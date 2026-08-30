//! Phase 3 — upstream call timeouts and cancellation (wire-level).
//!
//! Covers master Success Criteria 12–18:
//! - `timeout_secs` per server with default 60 (SC 12).
//! - every upstream tool call wrapped in the effective timeout (SC 13).
//! - timed-out call returns `CallToolResult { isError: true }` with JSON
//!   `code: "upstream_timeout"`, server, tool, message, `recoverable: true`
//!   (SC 14, D-005).
//! - timeout is NOT a JSON-RPC error (SC 15).
//! - downstream cancellation frees local resources without waiting the full
//!   upstream duration (SC 16).
//! - cancellation forwarding when rmcp exposes the request identity (SC 17).
//! - registry locks never held across spawn/init/list/call/timeout/cancel
//!   awaits (SC 18 — concurrency regression, same proof shape as Phase 2).
//!
//! The default-60 case (SC 12) is config-observable: a server with NO
//! `timeout_secs` uses 60. The wire-level suite does NOT wait 60s; it
//! asserts a server with `timeout_secs = 1` times out at 1s, and a server
//! with no `timeout_secs` is exercised via a fast successful call (the
//! default-60 value is the implementer's config-unit concern, surfaced in
//! tests.md as a coverage boundary).
//!
//! All tests are wire-level. The suite compiles clean against the current
//! tree (no timeout wrapping in `registry.rs`) and fails RED on the absent
//! behavior, not on missing symbols.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::timeout;

use crate::common;
use crate::common::elicit as el;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// The configured `timeout_secs` for the timeout proof. 1s is long enough
/// that the timeout is unambiguous (the probe sleeps well past it) and
/// short enough that the test stays fast.
const TIMEOUT_SECS: u64 = 1;

/// The probe `slow_tool` delay MUST exceed the configured timeout so the
/// call times out before the probe returns. 3s is comfortably past 1s and
/// keeps the test fast.
const SLOW_DELAY_MS: u64 = 3000;

/// The proof deadline for the cancellation test. A cancelled in-flight call
/// must free local resources WITHOUT waiting the full upstream duration
/// (SC 16). The test issues a slow call, then cancels it (by dropping the
/// response future / closing the request), then issues a SECOND fast call
/// and asserts the second completes within a deadline strictly shorter
/// than the slow delay. Half the 3s slow window tolerates a correct call under
/// parallel-suite load, while a resource-holding implementation still cannot
/// complete before the 3s floor.
///
/// Note: MCP `notifications/cancelled` is the wire-level cancellation
/// signal. The test sends it after issuing the slow call; the aggregator
/// must abort the local in-flight future and (when rmcp exposes the
/// request identity) forward the cancellation upstream.
const CANCEL_PROOF_DEADLINE: Duration = Duration::from_millis(1_500);

/// The timeout floor for the cross-upstream non-serialization proof. This test
/// uses a wider floor than the 1s timeout-shape proof so a load-tolerant sibling
/// deadline can remain strictly below the point where a serialized call would
/// be released.
const CONCURRENT_TIMEOUT_SECS: u64 = 3;

/// The probe delay for the cross-upstream timeout proof. It must exceed
/// `CONCURRENT_TIMEOUT_SECS` so the timeout, not probe completion, releases a
/// serialized sibling.
const CONCURRENT_SLOW_DELAY_MS: u64 = 6_000;

/// The sibling proof deadline is half the 3s timeout floor. This tolerates a
/// correct cold spawn + MCP handshake under parallel-suite load, while a
/// registry lock held until the timeout fires still cannot satisfy the proof.
const TIMEOUT_CONCURRENCY_PROOF_DEADLINE: Duration = Duration::from_millis(1_500);

/// Extract the joined text of a CallToolResult's content array.
fn result_text(result: &Value) -> String {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array"));
    content
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Parse the structured-error JSON from a CallToolResult's text content.
fn parse_error_json(result: &Value) -> Value {
    let text = result_text(result);
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("structured error text content must be valid JSON; got: {text:?}\n{e}")
    })
}

/// Master SC 13 + SC 14 + SC 15: a server with `timeout_secs = 1` calling a
/// probe tool that sleeps longer than 1s returns `CallToolResult { isError:
/// true }` with JSON containing `code: "upstream_timeout"`, server, tool,
/// message, `recoverable: true`. The timeout is NOT a JSON-RPC error.
///
/// The current tree has no timeout wrapping (`registry.rs::call_tool` awaits
/// the upstream call directly), so the slow call completes at 3s instead of
/// timing out at 1s — RED until the implementer wraps the call in
/// `tokio::time::timeout`.
#[tokio::test]
async fn timeout_secs_wraps_upstream_call_and_returns_structured_error() {
    let server = format!("srv-{}", fx::phase3_unique_seq());

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_timeout_secs(TIMEOUT_SECS))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Issue a slow_tool call that sleeps well past the configured timeout.
    // The call MUST time out at 1s, not complete at 3s. A generous ceiling
    // (12s) tolerates subprocess starvation and lets a non-timeout impl
    // complete so the assertion fails on error shape, not harness impatience.
    let resp = timeout(
        Duration::from_secs(12),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__slow_tool"),
                "arguments": { "delay_ms": SLOW_DELAY_MS },
            }),
        ),
    )
    .await
    .expect("slow_tool call must complete (either timeout or full delay) within 5s");

    // SC 15: the timeout must NOT be a JSON-RPC error.
    common::assert_no_rpc_error(&resp, "slow_tool timed-out call");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("slow_tool timed-out call returned no result"));

    // SC 14: isError: true with structured JSON.
    common::assert_is_error_result(&result, "slow_tool timed-out call");
    let err = parse_error_json(&result);

    assert_eq!(
        err.get("code").and_then(|c| c.as_str()),
        Some("upstream_timeout"),
        "timed-out call must carry code `upstream_timeout`; got: {err:?}"
    );
    assert_eq!(
        err.get("server").and_then(|s| s.as_str()),
        Some(server.as_str()),
        "timed-out call error must name the server; got: {err:?}"
    );
    assert_eq!(
        err.get("tool").and_then(|t| t.as_str()),
        Some("slow_tool"),
        "timed-out call error must name the tool; got: {err:?}"
    );
    assert!(
        err.get("message").and_then(|m| m.as_str()).is_some(),
        "timed-out call error must carry a message; got: {err:?}"
    );
    assert_eq!(
        err.get("recoverable").and_then(|r| r.as_bool()),
        Some(true),
        "timed-out call error must carry recoverable: true; got: {err:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 13 (fast call passes through): a successful fast upstream call
/// still passes through byte-faithfully and is NOT wrapped as an error when
/// it completes within the timeout. This guards against an implementer who
/// wraps every call in an error result regardless of whether the timeout
/// fired.
#[tokio::test]
async fn fast_call_within_timeout_passes_through_byte_faithfully() {
    let server = format!("srv-{}", fx::phase3_unique_seq());

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_timeout_secs(10))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let payload = "fast-byte-faithful-9c3";
    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_ok"),
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("fast echo_ok must complete within deadline");
    common::assert_no_rpc_error(&resp, "fast echo_ok");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("fast echo_ok returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "fast call within timeout must NOT be wrapped as an error"
        );
    }
    let text = result_text(&result);
    assert!(
        text.contains(payload),
        "fast call must pass through byte-faithfully; got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 16: a cancelled downstream request frees fanin-mcp's local call
/// resources without waiting for the upstream's full duration.
///
/// The observable: after issuing a slow call (3s) and sending a
/// `notifications/cancelled` for its request id, a SECOND fast call on the
/// SAME server completes within a deadline strictly shorter than the slow
/// delay. If the cancellation did not free local resources (the in-flight
/// future kept running and held resources), the second call would either
/// block or be affected by the still-running slow call.
///
/// Note: the wire-level cancellation signal is `notifications/cancelled`
/// carrying the request id. rmcp `=1.8.0` may or may not expose the upstream
/// request identity for forwarding (OQ3); the test asserts the LOCAL
/// observable (the second call is not blocked), not the forwarded
/// cancellation. See tests.md §Coverage gaps for the SC 17 boundary.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_frees_local_resources_without_waiting_full_upstream() {
    let server = format!("srv-{}", fx::phase3_unique_seq());

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_timeout_secs(30))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Issue a slow call WITHOUT awaiting its response. It enters the
    // registry, spawns the server, and awaits the slow_tool delay.
    let slow_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{server}__slow_tool"),
                    "arguments": { "delay_ms": SLOW_DELAY_MS },
                },
            }),
        )
        .await;

    // Send a cancellation notification for the slow call's request id. The
    // aggregator must abort the local in-flight future.
    child
        .notify(
            "notifications/cancelled",
            serde_json::json!({
                "requestId": slow_id,
                "reason": "test cancellation",
            }),
        )
        .await;

    // Immediately issue a SECOND fast call on the SAME server. If the
    // cancellation freed local resources, this completes within
    // CANCEL_PROOF_DEADLINE (well under the slow delay). If the slow call
    // still held resources / serialized the session, the fast call would
    // block until the slow call finished (>= 3s).
    let fast_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{server}__echo_ok"),
                    "arguments": { "message": "after-cancel" },
                },
            }),
        )
        .await;

    let fast_resp = timeout(CANCEL_PROOF_DEADLINE, child.wait_for_id(fast_id))
        .await
        .expect(
            "fast call after cancellation must complete within CANCEL_PROOF_DEADLINE — \
             a still-running slow call holding resources would serialize the session \
             and make this timeout (SC 16)",
        );
    common::assert_no_rpc_error(&fast_resp, "fast call after cancellation");
    let fast_result = fast_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("fast call after cancellation returned no result"));
    if let Some(is_error) = fast_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "fast call after cancellation must succeed (not an error)"
        );
    }
    let fast_text = result_text(&fast_result);
    assert!(
        fast_text.contains("after-cancel"),
        "fast call after cancellation must echo byte-faithfully; got: {fast_text:?}"
    );

    // The slow call's response may or may not arrive (the aggregator may
    // return a cancelled result or drop the request). Drain it with a
    // generous timeout so it does not leak into the next test; do NOT
    // assert on its shape — the SC 16 contract is the local-resource
    // observable, not the cancelled-response shape.
    let _ = timeout(Duration::from_secs(12), child.wait_for_id(slow_id)).await;

    child.into_guard().shutdown().await.ok();
}

/// Master SC 18 + concurrency regression: a slow timed-out call on one
/// upstream does NOT block a concurrent fast call on a sibling upstream.
/// This is the Phase 3 analogue of the Phase 2 D-007 / GOTCHA #16 proof,
/// extended to cover the timeout-wrapping path. A registry lock held across
/// the timeout await would serialize the session: the sibling fast call
/// would block until the slow call's timeout fired (3s). The 1.5s proof
/// deadline is half that timeout floor, so a serialized session still fails
/// while a correct cold sibling spawn has load-tolerant headroom.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_timed_out_call_does_not_block_concurrent_sibling() {
    let alpha = format!("alpha-{}", fx::phase3_unique_seq());
    let beta = format!("beta-{}", fx::phase3_unique_seq());

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(alpha.clone()).with_timeout_secs(CONCURRENT_TIMEOUT_SECS),
        )
        .server(fx::Phase3ServerEntry::new(beta.clone()))
        .namespace(fx::NamespaceEntry::new(
            "default",
            [alpha.as_str(), beta.as_str()],
        ))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Issue a slow alpha call WITHOUT awaiting. It spawns alpha and awaits
    // the slow_tool delay; the 3s timeout will fire before the 6s delay
    // completes.
    let slow_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{alpha}__slow_tool"),
                    "arguments": { "delay_ms": CONCURRENT_SLOW_DELAY_MS },
                },
            }),
        )
        .await;

    // Immediately issue a fast beta echo. If the registry lock were held
    // across the alpha timeout await, this would block until alpha's 3s
    // timeout fired. A correct lock-discipline impl dispatches beta on a
    // separate upstream while alpha's timeout is pending.
    let echo_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{beta}__echo_ok"),
                    "arguments": { "message": "concurrent-with-timeout" },
                },
            }),
        )
        .await;

    let echo_resp = timeout(
        TIMEOUT_CONCURRENCY_PROOF_DEADLINE,
        child.wait_for_id(echo_id),
    )
    .await
    .expect(
        "beta__echo_ok must complete within TIMEOUT_CONCURRENCY_PROOF_DEADLINE while \
             alpha__slow_tool is timing out — \
             a registry lock held across the timeout await would serialize the session and \
             make this timeout (SC 18 / D-007 / GOTCHA #16)",
    );
    common::assert_no_rpc_error(&echo_resp, "beta__echo_ok during alpha timeout");
    let echo_result = echo_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__echo_ok returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "beta__echo_ok must succeed (concurrent with alpha timeout)"
        );
    }
    let echo_text = result_text(&echo_result);
    assert!(
        echo_text.contains("concurrent-with-timeout"),
        "beta__echo_ok must echo byte-faithfully; got: {echo_text:?}"
    );

    // The slow alpha call must eventually return the timeout error (not a
    // hang, not a lost request). Drain it so it does not leak.
    let slow_resp = timeout(Duration::from_secs(12), child.wait_for_id(slow_id))
        .await
        .expect("alpha__slow_tool must eventually return (timeout result)");
    common::assert_no_rpc_error(&slow_resp, "alpha__slow_tool timeout");
    let slow_result = slow_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("alpha__slow_tool returned no result"));
    common::assert_is_error_result(&slow_result, "alpha__slow_tool timeout");
    let slow_err = parse_error_json(&slow_result);
    assert_eq!(
        slow_err.get("code").and_then(|c| c.as_str()),
        Some("upstream_timeout"),
        "alpha__slow_tool must time out with code upstream_timeout; got: {slow_err:?}"
    );

    child.into_guard().shutdown().await.ok();
}

// ---- Elicitation lifecycle (v1.1 / Phase 4) --------------------------------
//
// Adversarial/lifecycle tests for the elicitation-forwarding slice. These
// cover SC8 (timeout-mid-prompt → non-accept, no hang), SC9 (disconnect-mid-
// prompt → non-accept, no hang), and SC10 (a dropped/cancelled elicitation is
// NEVER interpreted as accept — the direct default-deny assertion, not a
// no-hang inference).
//
// The downstream test client DECLARES elicitation capability and either never
// answers (SC8/SC10 timeout path), disconnects mid-prompt (SC9), or answers
// with a JSON-RPC error (SC10 protocol-error path). The probe's
// `needs_elicitation` tool encodes the non-accept outcome it received, so the
// tests assert the DIRECT outcome via `el::assert_non_accept`.

/// Deadline for the lifecycle tests. The proxy's per-server `timeout_secs` is
/// the binding boundary (GP-3); the tests configure a SHORT `timeout_secs` so
/// the timeout-mid-prompt case resolves quickly. 15s is a generous ceiling that
/// still catches a hang.
const ELICIT_LIFECYCLE_DEADLINE: Duration = Duration::from_secs(15);

/// The configured `timeout_secs` for the elicitation timeout-mid-prompt proof.
/// 2s is long enough that the forwarded elicitation arrives before the timeout
/// fires, short enough that the test stays fast. The probe's own await backstop
/// (30s) does not fire first.
const ELICIT_TIMEOUT_SECS: u64 = 2;

/// Master SC8 / GP-3: a timed-out downstream elicitation resolves to non-accept
/// and the upstream tool call returns without hanging. The downstream client
/// declares elicitation but NEVER answers the forwarded `elicitation/create`
/// request. The proxy's per-server `timeout_secs` (2s) fires, the pending
/// downstream elicitation is cancelled/dropped, and the probe's tool result
/// encodes a non-accept outcome. The test asserts the DIRECT non-accept outcome
/// (SC10 discipline) — a no-hang-only assertion is insufficient and is the
/// exact security failure to prevent (a dropped response silently treated as
/// accept).
#[tokio::test]
async fn elicitation_timeout_mid_prompt_resolves_non_accept_no_hang() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_timeout_secs(ELICIT_TIMEOUT_SECS))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_declaring_elicitation(&mut child).await;

    let started = std::time::Instant::now();
    // Send the invoke_tool request without awaiting — the probe awaits the
    // forwarded elicitation, which we never answer. The proxy's 2s timeout
    // fires and the call resolves to a non-accept outcome.
    let call_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{server}__needs_elicitation"),
                    "arguments": {},
                },
            }),
        )
        .await;
    // Await the forwarded elicitation/create request (it arrives before the
    // timeout fires). Do NOT answer it.
    let _req = el::await_elicitation_request(&mut child).await;
    // The proxy's tool-call timeout (2s) fires and the call resolves to a
    // non-accept outcome. Wait within a generous ceiling that still catches a
    // hang.
    let resp = timeout(ELICIT_LIFECYCLE_DEADLINE, child.wait_for_id(call_id))
        .await
        .expect(
            "needs_elicitation with a never-answered forwarded elicitation must resolve \
             via the proxy tool-call timeout, not hang (SC8 / GP-3)",
        );
    common::assert_no_rpc_error(&resp, "needs_elicitation timeout-mid-prompt");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_elicitation timeout returned no result"));
    // SC8 + SC10: assert the DIRECT non-accept outcome. The proxy's per-server
    // tool-call timeout (2s) fires BEFORE the probe's 30s backstop, so the
    // downstream result is fanin's structured `upstream_timeout` error
    // (`isError:true`, no probe outcome payload), not the probe's encoded
    // `{non_accept:true,...}` shape. An `isError:true` result is definitively
    // non-accept — the proxy never maps a timeout to accept, so SC10 holds.
    // `assert_non_accept_or_error` accepts EITHER shape and still FAILS on an
    // accept/success result (direct default-deny preserved).
    el::assert_non_accept_or_error(&result, "SC8 timeout-mid-prompt");
    let elapsed = started.elapsed();
    // The proxy's 2s timeout is the binding boundary; allow generous slack for
    // CI. Anything approaching the probe's 30s backstop is a bug (the proxy
    // did not apply its timeout).
    assert!(
        elapsed < Duration::from_secs(10),
        "SC8: timeout-mid-prompt must resolve near the {ELICIT_TIMEOUT_SECS}s tool \
         timeout; took {elapsed:?} (the proxy's tool-call timeout may not have fired)"
    );
    child.into_guard().shutdown().await.ok();
}

/// Master SC9 / GP-4: a downstream disconnect during elicitation resolves to
/// non-accept and the upstream tool call returns without hanging. The
/// downstream client declares elicitation, the forwarded `elicitation/create`
/// arrives, then the downstream client disconnects (we close stdin / EOF). The
/// proxy's forward await terminates (peer error/closed) and maps to non-accept.
///
/// Observable as PROCESS EXIT, not as a readable wire response. Closing the
/// downstream stdin sends the proxy EOF on ITS stdin, so the proxy's
/// `running.waiting()` returns and the proxy EXITS — which closes its stdout,
/// making `wait_for_id` fail with a read error (not a hang). The real SC9 / GP-4
/// invariant is that the awaiting upstream handler does NOT hang after the
/// disconnect: a hung forward-await would keep the proxy process alive past the
/// deadline. We assert the proxy exits cleanly within the deadline (no hang)
/// — the observable that survives the disconnect. This is a real no-hang
/// assertion, not a no-op: a regression that blocks the forward-await on the
/// dropped peer would keep the process alive and time out here.
#[tokio::test]
async fn elicitation_disconnect_mid_prompt_resolves_non_accept_no_hang() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    // A long per-server timeout (30s) so the proxy's tool-call deadline does
    // NOT fire first — the disconnect is the binding lifecycle event here. If
    // the proxy hung on the forward-await, this test would wait the full
    // deadline and fail, not exit via the tool-call timeout.
    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_timeout_secs(30))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_declaring_elicitation(&mut child).await;

    let _call_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{server}__needs_elicitation"),
                    "arguments": {},
                },
            }),
        )
        .await;
    // Await the forwarded elicitation/create request so we KNOW the proxy is
    // blocked on the downstream answer before we disconnect.
    let _req = el::await_elicitation_request(&mut child).await;
    // Disconnect the downstream client: close stdin (EOF). The proxy's stdio
    // transport observes the peer-close; the forwarded elicitation await must
    // terminate (peer error / closed) and map to non-accept (GP-4). The proxy
    // then drains and exits.
    child.close_stdin().await;
    // The load-bearing SC9 / GP-4 assertion: the proxy process EXITS within a
    // deadline after the disconnect — a hung forward-await (the exact failure
    // SC9 prevents) would keep it alive past the deadline. We cannot read the
    // probe's tool result on stdout after a client-stdin close (the proxy
    // exits and closes its stdout), so process exit is the observable. 10s is
    // generous for a clean drain and still catches a hang.
    let guard = child.into_guard();
    let status = guard
        .wait_for_exit_within(ELICIT_LIFECYCLE_DEADLINE)
        .await
        .expect(
            "needs_elicitation with a mid-prompt downstream disconnect must NOT hang the \
             proxy — the forward await must terminate on peer-close and the process must \
             exit within the deadline (SC9 / GP-4)",
        );
    // A clean exit (the proxy drained and stopped) is the success shape. A
    // crash exit is also acceptable here: the disconnect path is allowed to
    // terminate abruptly as long as it does not hang. What is NOT acceptable
    // is no exit within the deadline (handled by the `expect` above).
    assert!(
        status.code().is_some(),
        "SC9: proxy must exit (not be killed) within the deadline after a mid-prompt \
         disconnect; got a signal kill or no exit code"
    );
}

/// Master SC10: a downstream JSON-RPC error response to the forwarded
/// elicitation is never interpreted as accept. The downstream client answers
/// the `elicitation/create` request with an explicit JSON-RPC error; the proxy
/// relays the error to the upstream; the probe encodes a non-accept outcome.
/// This is the protocol-error path of the default-deny invariant — distinct
/// from timeout (SC8) and disconnect (SC9) but the same security guarantee: a
/// non-accept outcome is never silently treated as accept.
#[tokio::test]
async fn elicitation_downstream_error_response_never_treated_as_accept() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_timeout_secs(30))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_declaring_elicitation(&mut child).await;

    let call_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": format!("{server}__needs_elicitation"),
                    "arguments": {},
                },
            }),
        )
        .await;
    let req = el::await_elicitation_request(&mut child).await;
    let elicit_id = req
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("forwarded elicitation/create missing id: {req:?}"));
    // Answer with an explicit JSON-RPC error (a non-accept outcome at the
    // protocol level).
    el::answer_error(&mut child, elicit_id, -32000, "client declined to elicit").await;
    let resp = timeout(ELICIT_LIFECYCLE_DEADLINE, child.wait_for_id(call_id))
        .await
        .expect(
            "needs_elicitation with a downstream error response must resolve, not hang \
             (SC10)",
        );
    common::assert_no_rpc_error(&resp, "needs_elicitation downstream-error");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_elicitation downstream-error returned no result"));
    let outcome = el::parse_elicitation_outcome(&result, "SC10 downstream-error");
    el::assert_non_accept(&outcome, "SC10 downstream-error");
    child.into_guard().shutdown().await.ok();
}
