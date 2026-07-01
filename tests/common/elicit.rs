//! Elicitation-forwarding harness helpers (Phase 4 / v1.1).
//!
//! Black-box helpers for tests that drive the downstream-client side of an
//! elicitation exchange over the fanin-mcp stdio transport. The downstream
//! test client declares elicitation capability at `initialize` time and answers
//! `elicitation/create` requests by writing JSON-RPC responses back onto the
//! same stdio stream. The helpers never call rmcp's server-role send API —
//! fanin-mcp owns that internally; the test code is purely the downstream
//! client side (GOTCHA / task rule).
//!
//! Wire shape (MCP `elicitation/create`):
//! - request: `{"jsonrpc":"2.0","id":<n>,"method":"elicitation/create","params":{...}}`
//! - response: `{"jsonrpc":"2.0","id":<n>,"result":{"action":"accept"|"decline"|"cancel","content":...,"meta":...}}`
//! - error: `{"jsonrpc":"2.0","id":<n>,"error":{"code":...,"message":...}}`
//!
//! An accept MUST carry `content`; decline/cancel MUST NOT. A response with an
//! unknown action or a missing `content` on accept is malformed and the probe
//! surfaces a non-accept outcome (SC10 default-deny).

use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::timeout;

use crate::common::JsonRpcChild;

/// Deadline for awaiting an upstream-originated `elicitation/create` request
/// on the wire. Generous enough that a correct forwarding path never misses it
/// (the proxy forwards within the tool-call deadline), tight enough that a
/// missing forwarder fails the test fast instead of stalling CI.
const ELICIT_REQUEST_DEADLINE: Duration = Duration::from_secs(15);

/// `initialize` with a configurable downstream client `capabilities` object.
/// Mirrors `common::initialize` but lets a test declare `elicitation` (or any
/// other capability) so fanin-mcp's capability-honesty + forwarding gate can
/// observe it. Sends the MCP `initialize` request, asserts a result is present,
/// sends the `notifications/initialized` notification, and returns the
/// server's initialize result (with its advertised capabilities).
pub async fn initialize_with_capabilities(child: &mut JsonRpcChild, capabilities: Value) -> Value {
    let result = timeout(
        Duration::from_secs(10),
        child.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": capabilities,
                "clientInfo": {
                    "name": "fanin-mcp-elicitation-test-harness",
                    "version": "0.0.0",
                },
            }),
        ),
    )
    .await
    .expect("initialize did not return within 10s")
    .get("result")
    .cloned()
    .unwrap_or_else(|| panic!("initialize returned no result field"));

    assert!(
        result.get("protocolVersion").is_some(),
        "initialize result must carry a protocolVersion"
    );

    child
        .notify(
            "notifications/initialized",
            Value::Object(Default::default()),
        )
        .await;

    result
}

/// `initialize` declaring downstream `elicitation` capability. The capability
/// object mirrors rmcp's `ClientCapabilities::builder().enable_elicitation()`
/// wire form: `{"elicitation": {}}` (an empty object signals support with no
/// per-mode settings, matching the rmcp builder's empty `ElicitationCapability`).
pub async fn initialize_declaring_elicitation(child: &mut JsonRpcChild) -> Value {
    initialize_with_capabilities(child, json!({ "elicitation": {} })).await
}

/// `initialize` declaring NO elicitation capability (the existing Phase 1
/// shape: empty capabilities). Used by the no-capability regression (SC4) and
/// the capability-honesty negative arm (SC3).
pub async fn initialize_without_elicitation(child: &mut JsonRpcChild) -> Value {
    initialize_with_capabilities(child, json!({})).await
}

/// Await an upstream-originated `elicitation/create` request on the wire within
/// `ELICIT_REQUEST_DEADLINE`. Returns the request message (with its `id` and
/// `params`) so a test can inspect the forwarded params and answer by id.
/// Skips responses and notifications for other in-flight requests; buffers
/// responses for those ids so a later `wait_for_id` still resolves.
pub async fn await_elicitation_request(child: &mut JsonRpcChild) -> Value {
    let observed = timeout(ELICIT_REQUEST_DEADLINE, async {
        loop {
            let raw = child
                .read_line()
                .await
                .unwrap_or_else(|e| panic!("read elicitation request failed: {e}"));
            if raw.trim().is_empty() {
                continue;
            }
            let msg: Value = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("elicitation request not JSON: {raw}\n{e}"));
            if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                let method = msg.get("method").and_then(|m| m.as_str());
                if method == Some("elicitation/create") {
                    return msg;
                }
                // A response for a different in-flight request — buffer it.
                if msg.get("result").is_some() || msg.get("error").is_some() {
                    child.buffer_response(id, msg);
                    continue;
                }
                // Some other request method; keep scanning.
                continue;
            }
            // A notification (no id) — skip.
        }
    })
    .await;
    observed.expect(
        "no elicitation/create request observed on the wire within \
         {ELICIT_REQUEST_DEADLINE:?} — the proxy must forward the upstream \
         elicitation to this downstream client (SC5 / SC3)",
    )
}

/// Answer an `elicitation/create` request with an `accept` result carrying the
/// given content JSON. `content` is sent verbatim as the `content` field; it
/// must conform to the request's schema (the test is responsible for shape).
pub async fn answer_accept(child: &mut JsonRpcChild, id: u64, content: Value) {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "action": "accept",
            "content": content,
            "meta": null,
        },
    });
    child
        .send_raw(&resp.to_string())
        .await
        .expect("send accept response failed");
}

/// Answer an `elicitation/create` request with a `decline` result. Decline
/// carries no `content` (rmcp: content is Some only on Accept). Distinct from
/// cancel (GP-5 / SC6).
pub async fn answer_decline(child: &mut JsonRpcChild, id: u64) {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "action": "decline",
            "content": null,
            "meta": null,
        },
    });
    child
        .send_raw(&resp.to_string())
        .await
        .expect("send decline response failed");
}

/// Answer an `elicitation/create` request with a `cancel` result. Cancel
/// carries no `content`. Distinct from decline (GP-5 / SC7).
pub async fn answer_cancel(child: &mut JsonRpcChild, id: u64) {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "action": "cancel",
            "content": null,
            "meta": null,
        },
    });
    child
        .send_raw(&resp.to_string())
        .await
        .expect("send cancel response failed");
}

/// Answer an `elicitation/create` request with a JSON-RPC error (an explicit
/// non-accept outcome at the protocol level). Used where a test wants to assert
/// the proxy maps a downstream error to a non-accept upstream outcome.
pub async fn answer_error(child: &mut JsonRpcChild, id: u64, code: i32, message: &str) {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    });
    child
        .send_raw(&resp.to_string())
        .await
        .expect("send error response failed");
}

/// Never answer the `elicitation/create` request — leave it pending so the
/// enclosing tool-call timeout (GP-3) fires. The test asserts the eventual
/// downstream `invoke_tool` result is non-accept and that it arrives within
/// the tool-call deadline (no hang).
#[allow(dead_code)] // documentation-of-intent helper; tests call the inline never-answer shape directly.
pub async fn never_answer(_child: &mut JsonRpcChild, _id: u64) {
    // Intentionally a no-op: the request is left unanswered. Provided for
    // symmetry with the other answer helpers so a test's intent reads clearly.
}

/// Parse the probe's `needs_elicitation` tool-result JSON payload from a
/// downstream `invoke_tool` CallToolResult. Returns the decoded JSON object
/// (carrying `elicitation_action`, `non_accept`, `content`). Panics with a
/// clear message if the result text is absent or not valid JSON — the probe
/// always encodes JSON for this tool, so a non-JSON result is a contract
/// violation worth surfacing.
pub fn parse_elicitation_outcome(result: &Value, ctx: &str) -> Value {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("{ctx}: result missing content array"));
    let text = content
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "{ctx}: needs_elicitation result text must be valid JSON carrying the \
             elicitation outcome; got: {text:?}\n{e}"
        )
    })
}

/// Assert the parsed elicitation outcome is a specific `action` string
/// (`accept` / `decline` / `cancel`). Fails with a clear diff if the action
/// differs — this is the direct outcome assertion (SC5/SC6/SC7), not an
/// inference from a no-hang wrapper.
pub fn assert_action(outcome: &Value, expected: &str, ctx: &str) {
    let got = outcome
        .get("elicitation_action")
        .and_then(|a| a.as_str())
        .unwrap_or_else(|| panic!("{ctx}: outcome missing elicitation_action: {outcome:?}"));
    assert_eq!(
        got, expected,
        "{ctx}: expected elicitation_action `{expected}`, got `{got}`; outcome: {outcome:?}"
    );
}

/// Assert the parsed elicitation outcome is NON-accept — the direct default-
/// deny assertion (SC8/SC9/SC10). Checks `non_accept == true` AND the action is
/// not `accept`. A no-hang-only assertion is insufficient for SC10; this is the
/// load-bearing check that a dropped/timed-out/disconnected prompt is never
/// silently treated as accept.
pub fn assert_non_accept(outcome: &Value, ctx: &str) {
    let non_accept = outcome
        .get("non_accept")
        .and_then(|n| n.as_bool())
        .unwrap_or_else(|| panic!("{ctx}: outcome missing non_accept bool: {outcome:?}"));
    let action = outcome
        .get("elicitation_action")
        .and_then(|a| a.as_str())
        .unwrap_or_else(|| panic!("{ctx}: outcome missing elicitation_action: {outcome:?}"));
    assert!(
        non_accept && action != "accept",
        "{ctx}: outcome must be non-accept (SC10 default-deny); got action=`{action}` \
         non_accept={non_accept}; outcome: {outcome:?}"
    );
}

/// Assert the parsed elicitation outcome IS accept — the direct happy-path
/// assertion (SC5). Checks `non_accept == false` AND the action is `accept`.
pub fn assert_accept(outcome: &Value, ctx: &str) {
    let non_accept = outcome
        .get("non_accept")
        .and_then(|n| n.as_bool())
        .unwrap_or_else(|| panic!("{ctx}: outcome missing non_accept bool: {outcome:?}"));
    let action = outcome
        .get("elicitation_action")
        .and_then(|a| a.as_str())
        .unwrap_or_else(|| panic!("{ctx}: outcome missing elicitation_action: {outcome:?}"));
    assert!(
        !non_accept && action == "accept",
        "{ctx}: outcome must be accept (SC5); got action=`{action}` non_accept={non_accept}; \
         outcome: {outcome:?}"
    );
}

/// Assert a downstream `invoke_tool` CallToolResult is NON-accept, accepting
/// EITHER the probe's encoded non-accept outcome (`non_accept=true`) OR a
/// fanin structured error (`isError:true` with no probe outcome payload). The
/// structured-error arm fires when the proxy's per-server tool-call deadline
/// expires BEFORE the probe encodes its own outcome: the downstream then sees
/// fanin's `upstream_timeout` / `elicitation_non_accept` / `send_error`
/// structured error (D-005), not the probe's `{non_accept:true,...}` shape.
///
/// An `isError:true` result is definitively non-accept — the proxy never maps
/// a structured error to accept, so SC10 (default-deny) holds on this path
/// (SC8). The assertion STILL FAILS on an accept / success result
/// (`isError` absent-or-false AND the probe outcome is `accept`): the direct
/// default-deny discipline is preserved, only the *shape* of the non-accept is
/// broadened to the proxy's own structured error.
pub fn assert_non_accept_or_error(result: &Value, ctx: &str) {
    // A fanin structured error (isError:true) is a valid non-accept. The proxy
    // surfaces upstream timeouts / non-accepts / send errors as structured
    // CallToolResult errors (D-005); these are never accept.
    if result.get("isError").and_then(|e| e.as_bool()) == Some(true) {
        return;
    }
    let outcome = parse_elicitation_outcome(result, ctx);
    assert_non_accept(&outcome, ctx);
}

/// Extract the `content` JSON value from a parsed accept outcome (for tests
/// that assert the accept content round-trips byte-faithfully).
pub fn accept_content(outcome: &Value) -> Value {
    outcome
        .get("content")
        .cloned()
        .unwrap_or_else(|| panic!("accept outcome missing content: {outcome:?}"))
}
