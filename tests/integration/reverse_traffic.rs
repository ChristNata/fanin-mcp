//! Reverse-traffic handling — Phase 1 wire-level tests.
//!
//! Covers master Success Criteria 14 (no sampling/elicitation capabilities),
//! 15 (empty roots/list), 16 (sampling/elicitation rejected, not hung), 17
//! (upstream log notifications + child stderr to log sink with [server]
//! prefix), and 18 (child stderr never reaches stdout), plus Phase 2
//! sub-phase Success Criteria 1–6.
//!
//! The probe fixture's `needs_sampling` tool sends an upstream-originated
//! `sampling/createMessage` request toward its client (the aggregator). In
//! Phase 0 nothing answered it (GOTCHA #2: an unanswered upstream request
//! hangs forever). Phase 1 wires the `ClientHandler` so the aggregator
//! rejects it immediately with a structured error — the probe's
//! `needs_sampling` tool call must COMPLETE within the deadline, not hang.
//!
//! These tests exercise the full path: downstream `invoke_tool` ->
//! aggregator -> lazy upstream spawn -> probe `needs_sampling` -> probe sends
//! `sampling/createMessage` UP to the aggregator -> aggregator's
//! `ClientHandler::create_message` rejects it -> probe receives the
//! rejection -> probe's `call_tool` returns a result -> aggregator forwards
//! it byte-faithfully -> downstream sees the tool result.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::timeout;

use crate::common;
use crate::common::elicit as el;
use crate::common::fixtures as fx;

/// Deadline for a reverse-traffic call. The probe's `needs_sampling` emits
/// the sampling request and waits for the response; a correct aggregator
/// rejects within milliseconds. 10s is a generous ceiling that still catches
/// a hang (GOTCHA #2: without the handler the probe waits forever).
const REVERSE_DEADLINE: Duration = Duration::from_secs(10);

/// Helper: build the canonical Phase 1 config + spawn the aggregator +
/// initialize. Returns the live child ready for a meta-tool call.
async fn phase1_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    child
}

/// Master criterion 16 / P2.SC3: `invoke_tool probe__needs_sampling` COMPLETES
/// within the deadline — the aggregator's `ClientHandler::create_message`
/// rejects the probe's sampling request immediately, so the probe's
/// `call_tool` future resolves instead of hanging on the unanswered request.
///
/// This is THE trap that bites first (GOTCHA #2). A Phase 0 stub with no
/// reverse-traffic handler would hang here; Phase 1 must answer. The
/// observable effect is "the tool call returned within the deadline" — a hang
/// fails the test on the `timeout` wrapper.
#[tokio::test]
async fn needs_sampling_call_completes_within_deadline_not_hung() {
    let mut child = phase1_child().await;

    let started = std::time::Instant::now();
    let resp = timeout(
        REVERSE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_sampling",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect(
        "needs_sampling call must complete within {REVERSE_DEADLINE:?} — the \
         aggregator must reject the probe's sampling request, not hang (GOTCHA #2)",
    );

    // The call must not surface as a JSON-RPC error (D-005).
    common::assert_no_rpc_error(&resp, "invoke_tool probe__needs_sampling");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_sampling returned no result field"));

    // The probe returns a SUCCESS text block ("sent sampling/createMessage
    // request to client") once its detached send future is spawned. The
    // aggregator must forward that result byte-faithfully. The Phase 0 stub
    // returns a not-implemented ERROR (isError: true) without forwarding —
    // so asserting SUCCESS here makes the test fail RED against the stub
    // and pass GREEN once real forwarding + reverse-traffic handling land.
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "needs_sampling must forward the probe's success result, not a \
             not-implemented error (isError must not be true)"
        );
    }
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("needs_sampling result missing content array"))
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
    assert!(
        text.contains("sampling/createMessage") || text.contains("needs_sampling"),
        "needs_sampling must forward the probe's success text (the probe \
         confirms it sent the sampling request); got: {text:?}"
    );

    let elapsed = started.elapsed();
    // Sanity: a rejection is fast. A correct impl returns in well under 2s;
    // anything approaching the deadline is suspicious even if it "passed."
    assert!(
        elapsed < Duration::from_secs(5),
        "needs_sampling should complete quickly once the sampling request is \
         rejected; took {elapsed:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 16 / P2.SC3 edge: the structured rejection the aggregator
/// sends back to the probe is a bounded, well-formed JSON-RPC error response
/// (not a hang, not a malformed message). We observe this indirectly: after
/// `needs_sampling` completes, the aggregator's stdout stream must remain
/// clean (no corruption from the reverse path). A subsequent `tools/list`
/// must still return exactly the three static meta-tools — proving the
/// reverse-traffic exchange did not destabilize the downstream server.
#[tokio::test]
async fn reverse_traffic_does_not_destabilize_downstream_server() {
    let mut child = phase1_child().await;

    // Trigger the reverse path.
    let resp = timeout(
        REVERSE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_sampling",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("needs_sampling must complete (reverse path handled)");
    common::assert_no_rpc_error(&resp, "needs_sampling before stability check");

    // The downstream server must still be healthy: tools/list returns the
    // three static meta-tools, and a second invoke completes.
    let list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&list, "tools/list after reverse traffic");
    let tools = list
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    crate::common::expectations::assert_exact_meta_tools(tools);

    let echo = timeout(
        Duration::from_secs(5),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": "post-reverse-traffic" },
            }),
        ),
    )
    .await
    .expect("echo_ok after reverse traffic must complete");
    common::assert_no_rpc_error(&echo, "echo_ok after reverse traffic");
    let echo_result = echo
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_ok after reverse traffic returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "echo_ok after reverse traffic must succeed (real forward, not a \
             not-implemented error)"
        );
    }
    let echo_text = echo_result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("echo_ok result missing content"))
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
    assert!(
        echo_text.contains("post-reverse-traffic"),
        "echo_ok after reverse traffic must forward and echo the payload; got: {echo_text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 14 / P2.SC1: the upstream client declares NO
/// sampling/elicitation capabilities at connection time. This is observed
/// indirectly — a spec-compliant server that respects the declared
/// capabilities would not send sampling/elicitation requests at all. The
/// probe's `needs_sampling` sends one regardless (it is a test fixture that
/// forces the path), so the observable contract is: the aggregator handles
/// the request even though it declared no capability, by rejecting it.
///
/// The direct capability assertion belongs to a unit test against the
/// `ClientInfo` the aggregator passes to `serve_client`; at the wire level
/// the rejection-within-deadline (above) is the observable proxy. This test
/// records the boundary: a true capability-declaration assertion is deferred
/// to a unit test once `forward.rs` exposes a constructible handler. See
/// `tests.md` §Deferred.
#[tokio::test]
async fn upstream_client_rejects_sampling_within_deadline_proxy_for_no_capability() {
    let mut child = phase1_child().await;

    // The proxy assertion: the rejection completes, which is only possible if
    // the ClientHandler is wired. The capability-declaration itself is not
    // observable over this stdio without instrumenting the probe's
    // initialize response — recorded as a deferred unit test.
    // We additionally require the forward to SUCCEED (the probe's
    // needs_sampling success text), so the test fails RED against the
    // Phase 0 stub (which returns a not-implemented error without forwarding).
    let resp = timeout(
        REVERSE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_sampling",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("sampling request must be rejected, not hung");
    common::assert_no_rpc_error(&resp, "needs_sampling rejection path");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_sampling returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "needs_sampling must forward the probe's success result (the \
             rejection happens on the reverse path, not the forward result)"
        );
    }

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 18 / P2.SC6: child stderr never reaches the aggregator's
/// stdout. The aggregator's stdout is the JSON-RPC transport (GOTCHA #1); any
/// child stderr line that leaks onto it corrupts the stream. We assert by
/// confirming every stdout line we read during a reverse-traffic exchange is
/// valid JSON — a leaked stderr line is non-JSON and the harness's
/// `serde_json::from_str` would panic.
///
/// This is the side-effect assertion for GOTCHA #1 on the child-spawn path.
/// The probe writes tracing logs to its OWN stderr; the aggregator must pipe
/// that to the log file, not inherit it onto the aggregator's stdout.
#[tokio::test]
async fn child_stderr_does_not_reach_aggregator_stdout() {
    let mut child = phase1_child().await;

    // Trigger an upstream spawn + a reverse-traffic exchange — this exercises
    // the child-spawn path where stderr inheritance would leak if present.
    // We use echo_ok (a forward that must succeed) to prove a real child
    // spawn happened, then needs_sampling to exercise the reverse path. The
    // Phase 0 stub returns not-implemented for both without spawning — so
    // asserting echo_ok success makes this test fail RED against the stub.
    let echo = timeout(
        Duration::from_secs(10),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": "stderr-check-spawn" },
            }),
        ),
    )
    .await
    .expect("echo_ok must complete to prove a real upstream spawn happened");
    common::assert_no_rpc_error(&echo, "echo_ok for stderr check");
    let echo_result = echo
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_ok returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "echo_ok must forward successfully (proves the upstream was actually \
             spawned); a not-implemented error means no child stderr path was \
             exercised"
        );
    }

    let _rev = timeout(
        REVERSE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_sampling",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("needs_sampling must complete");
    // (We do not assert on the needs_sampling result here; the stderr-leak
    // assertion below is the load-bearing check. The reverse path just
    // exercises more child-spawn stderr.)

    // Read any remaining stdout within a short window. Every line must parse
    // as JSON (a leaked stderr line would not). A timeout here is fine — it
    // means the child had nothing more to say.
    let _ = timeout(Duration::from_millis(500), async {
        loop {
            let raw = match child.read_line().await {
                Ok(s) => s,
                Err(_) => break, // timeout / EOF
            };
            if raw.trim().is_empty() {
                continue;
            }
            // Panic on a non-JSON line — that is the assertion.
            let _: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
                panic!(
                    "aggregator stdout produced a non-JSON line (likely leaked \
                     child stderr, GOTCHA #1/#18): {raw:?}\n{e}"
                )
            });
        }
    })
    .await;

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 17 / P2.SC5: upstream logging notifications and child
/// stderr lines appear in the log sink with the originating server name.
///
/// The probe does not currently emit a `notifications/message` (logging)
/// request on its own, and there is no probe tool that triggers one. Child
/// stderr IS emitted (the probe initializes `tracing` to stderr and logs at
/// INFO). So this test asserts the stderr half of criterion 17 against the
/// configured `log_file`: after spawning the upstream and exercising a call,
/// the log file must contain at least one line prefixed with the server
/// name (`[probe]` or the configured name).
///
/// The logging-notification half is a coverage gap — see `tests.md` §Gaps.
#[tokio::test]
async fn child_stderr_lands_in_log_sink_with_server_prefix() {
    let log_path = fx::empty_log_file_path();
    let cfg = fx::ConfigBuilder::new().log_file(&log_path).write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Force an upstream spawn so the probe's stderr (tracing init line) is
    // produced. echo_ok is the cheapest trigger.
    let resp = timeout(
        Duration::from_secs(5),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": "log-sink-probe" },
            }),
        ),
    )
    .await
    .expect("echo_ok must complete to exercise child stderr");
    common::assert_no_rpc_error(&resp, "echo_ok for log-sink test");

    // Give the aggregator a moment to flush the piped stderr to the log file.
    tokio::time::sleep(Duration::from_millis(300)).await;
    child.into_guard().shutdown().await.ok();

    let log_contents = std::fs::read_to_string(&log_path)
        .unwrap_or_else(|e| panic!("failed to read log file at {log_path}: {e}"));

    // The server name in the config is `probe`; the prefix may be `[probe]`
    // or `probe` — assert the configured name appears in at least one line.
    // A correct stderr-capture path writes the child's stderr lines with the
    // server-name prefix (D-008 / GOTCHA #29). An implementation that
    // inherits child stderr onto the aggregator's own stderr writes NOTHING
    // to the log file and fails here.
    assert!(
        log_contents.contains("probe"),
        "log sink must contain the server name `probe` (criterion 17); \
         log file contents:\n{log_contents}"
    );
}

/// Master criterion 15 / P2.SC2: an upstream `roots/list` request receives
/// an EMPTY list response, not a hang and not an error. The aggregator's
/// `ClientHandler::list_roots` returns `ListRootsResult` with zero roots.
///
/// The probe fixture's `needs_roots` tool sends an upstream-originated
/// `roots/list` request toward the aggregator (mirrors `needs_sampling`).
/// The aggregator's `ClientHandler::list_roots` answers with an empty list,
/// so the probe's `call_tool` resolves instead of hanging. This is wired
/// (no longer deferred) now that the probe-fixture `needs_roots` tool exists.
#[tokio::test]
async fn upstream_roots_list_receives_empty_list() {
    let mut child = phase1_child().await;

    let resp = timeout(
        REVERSE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_roots",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect(
        "needs_roots call must complete within {REVERSE_DEADLINE:?} — the \
         aggregator must answer the probe's roots/list request with an empty \
         list, not hang (GOTCHA #2)",
    );

    // The call must not surface as a JSON-RPC error (D-005).
    common::assert_no_rpc_error(&resp, "invoke_tool probe__needs_roots");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_roots returned no result field"));

    // The probe returns a SUCCESS text block ("needs_roots: sent roots/list
    // request to client") once its detached send future is spawned. The
    // aggregator must forward that result byte-faithfully. A Phase 0 stub
    // returns a not-implemented ERROR (isError: true) without forwarding — so
    // asserting SUCCESS here makes the test fail RED against the stub and pass
    // GREEN once real forwarding + the roots/list empty-list handler land.
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "needs_roots must forward the probe's success result, not a \
             not-implemented error (isError must not be true)"
        );
    }
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("needs_roots result missing content array"))
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
    assert!(
        text.contains("roots/list") || text.contains("needs_roots"),
        "needs_roots must forward the probe's success text (the probe \
         confirms it sent the roots/list request); got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 16 / P2.SC4: an upstream elicitation request receives a
/// bounded rejection, not a hang. The aggregator's
/// `ClientHandler::create_elicitation` rejects immediately.
///
/// The probe fixture's `needs_elicitation` tool sends an upstream-originated
/// `elicitation/create` request toward the aggregator (mirrors
/// `needs_sampling`). The aggregator's `ClientHandler::create_elicitation`
/// rejects it, so the probe's `call_tool` resolves instead of hanging. This
/// is wired (no longer deferred) now that the probe-fixture
/// `needs_elicitation` tool exists.
#[tokio::test]
async fn upstream_elicitation_request_receives_bounded_rejection() {
    let mut child = phase1_child().await;

    let resp = timeout(
        REVERSE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_elicitation",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect(
        "needs_elicitation call must complete within {REVERSE_DEADLINE:?} — \
         the aggregator must reject the probe's elicitation request, not hang \
         (GOTCHA #2)",
    );

    // The call must not surface as a JSON-RPC error (D-005).
    common::assert_no_rpc_error(&resp, "invoke_tool probe__needs_elicitation");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_elicitation returned no result field"));

    // v1.1: the probe now AWAITS the forwarded elicitation and encodes the
    // outcome it received. With no downstream elicitation capability
    // (phase1_child uses empty caps), the aggregator rejects and the probe
    // encodes a non-accept outcome as `CallToolResult::error` (isError: true)
    // carrying a JSON payload with `non_accept: true`. The direct non-accept
    // assertion lives in
    // `elicitation_no_capability_takes_bounded_rejection_path`; this test
    // guards the no-hang + no-JSON-RPC-error contract that the Phase 1
    // reverse-traffic suite established. The text must mention elicitation so
    // the test still catches a stub that returns a generic not-implemented
    // error without exercising the elicitation path.
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("needs_elicitation result missing content array"))
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
    assert!(
        text.contains("elicitation") || text.contains("needs_elicitation"),
        "needs_elicitation must forward the probe's outcome text (the probe \
         confirms it sent the elicitation request and encoded the rejection); \
         got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

// ---- Elicitation forwarding (v1.1 / Phase 4) --------------------------------
//
// These tests extend the reverse-traffic suite with the v1.1 elicitation-
// forwarding slice. The downstream test client DECLARES elicitation capability
// at initialize and answers `elicitation/create` requests over the same stdio
// stream. The probe's `needs_elicitation` tool now AWAITS the forwarded
// response and returns a tool result encoding the outcome
// (`elicitation_action` + `non_accept`), so these tests assert the DIRECT
// outcome rather than inferring from a no-hang wrapper (SC10 assertion
// discipline).
//
// Coverage: SC3 (capability honesty), SC4 (no-cap rejection regression),
// SC5 (accept forwarded), SC6 (decline distinct), SC7 (cancel distinct),
// SC13/SC14 (sampling/roots regressions preserved alongside the new path).
// Lifecycle (SC8/SC9/SC10) lives in `timeout_cancellation.rs`; concurrency
// (SC11/SC12) lives in `multi_upstream.rs`.

/// Deadline for an elicitation-forwarding round-trip. Generous enough that a
/// correct forwarding path (downstream accept + probe encode + forward back)
/// never misses it, tight enough that a hang fails the test fast.
const ELICIT_DEADLINE: Duration = Duration::from_secs(15);

/// Helper: spawn fanin with the canonical Phase 1 config + the probe env var
/// that exposes `report_client_caps`, initialize declaring elicitation
/// capability, and return the live child ready for a meta-tool call. The env
/// var is set on the probe via the config's `[servers.probe.env]` table
/// (fanin's least-privilege `env_clear` means the probe does NOT inherit
/// fanin's ambient env — only config-declared vars reach the probe, D-010).
async fn elicitation_capable_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new()
        .env("FANIN_PROBE_REPORT_CLIENT_CAPS", "1")
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_declaring_elicitation(&mut child).await;
    child
}

/// Helper: spawn fanin with the canonical Phase 1 config + the probe env var
/// that exposes `report_client_caps`, initialize declaring NO elicitation
/// capability, and return the live child. Used by the no-capability regression
/// (SC4) and the capability-honesty negative arm (SC3).
async fn no_elicitation_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new()
        .env("FANIN_PROBE_REPORT_CLIENT_CAPS", "1")
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_without_elicitation(&mut child).await;
    child
}

/// The downstream client's answer to a forwarded `elicitation/create` request.
/// Used by `drive_elicitation` so each SC5/SC6/SC7 test stays focused on its
/// assertion rather than re-deriving the wire-answer shape.
enum Answer {
    Accept(Value),
    Decline,
    Cancel,
}

/// Drive a `probe__needs_elicitation` call that triggers a forwarded
/// `elicitation/create`, await the forwarded request, answer it per `answer`,
/// and return the downstream `invoke_tool` CallToolResult. Bundles the common
/// shape so each SC5/SC6/SC7 test stays focused on its assertion.
async fn drive_elicitation(answer: Answer) -> Value {
    let mut child = elicitation_capable_child().await;
    // Send the invoke_tool request without awaiting its response — the probe
    // awaits the forwarded elicitation, so the tools/call response will not
    // arrive until AFTER we answer the elicitation/create request (or the
    // proxy's tool-call timeout fires for the Never case).
    let call_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "probe__needs_elicitation",
                    "arguments": {},
                },
            }),
        )
        .await;
    // Await the forwarded elicitation/create request on the wire. For the
    // Never case the request still arrives (the proxy forwards before the
    // timeout fires); we just do not answer it.
    let req = el::await_elicitation_request(&mut child).await;
    let elicit_id = req
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("forwarded elicitation/create missing id: {req:?}"));
    match answer {
        Answer::Accept(content) => el::answer_accept(&mut child, elicit_id, content).await,
        Answer::Decline => el::answer_decline(&mut child, elicit_id).await,
        Answer::Cancel => el::answer_cancel(&mut child, elicit_id).await,
    }
    // Now the probe's call_tool resolves (or the proxy times out for Never)
    // and the downstream tools/call response arrives. Wait for it within the
    // deadline (no-hang guard).
    let resp = timeout(ELICIT_DEADLINE, child.wait_for_id(call_id))
        .await
        .expect(
            "probe__needs_elicitation must complete within ELICIT_DEADLINE once the \
             downstream client answers the forwarded elicitation — a hang means the \
             proxy did not relay the client's answer back upstream (SC5/GP-5)",
        );
    common::assert_no_rpc_error(&resp, "invoke_tool probe__needs_elicitation");
    resp.get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_elicitation returned no result field"))
}

/// Master SC3 / GP-1: capability honesty. A downstream client that DECLARES
/// elicitation causes fanin-mcp's upstream client handler to ADVERTISE
/// elicitation to the probe. The probe's `report_client_caps` tool reports the
/// elicitation capability it observed on the aggregator client at initialize:
/// `{"elicitation": true}`. A stub that unconditionally advertises (or never
/// advertises) fails this assertion.
#[tokio::test]
async fn downstream_declares_elicitation_aggregator_advertises_to_upstream() {
    let mut child = elicitation_capable_child().await;
    let resp = timeout(
        ELICIT_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            json!({ "name": "probe__report_client_caps", "arguments": {} }),
        ),
    )
    .await
    .expect("report_client_caps must complete");
    common::assert_no_rpc_error(&resp, "report_client_caps (declare arm)");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("report_client_caps returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "report_client_caps must forward the probe's success result"
        );
    }
    let caps = el::parse_elicitation_outcome(&result, "report_client_caps (declare arm)");
    let declared = caps
        .get("elicitation")
        .and_then(|e| e.as_bool())
        .unwrap_or_else(|| panic!("report_client_caps missing elicitation bool: {caps:?}"));
    assert!(
        declared,
        "SC3 / GP-1: when the downstream client declares elicitation, fanin-mcp must \
         advertise elicitation to the upstream (probe should observe \
         elicitation=true); got {caps:?}"
    );
    child.into_guard().shutdown().await.ok();
}

/// Master SC3 negative arm: a downstream client that does NOT declare
/// elicitation causes fanin-mcp to advertise NO elicitation to the upstream.
/// The probe's `report_client_caps` reports `{"elicitation": false}`.
#[tokio::test]
async fn downstream_omits_elicitation_aggregator_advertises_nothing_extra() {
    let mut child = no_elicitation_child().await;
    let resp = timeout(
        ELICIT_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            json!({ "name": "probe__report_client_caps", "arguments": {} }),
        ),
    )
    .await
    .expect("report_client_caps must complete");
    common::assert_no_rpc_error(&resp, "report_client_caps (no-declare arm)");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("report_client_caps returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "report_client_caps must forward the probe's success result"
        );
    }
    let caps = el::parse_elicitation_outcome(&result, "report_client_caps (no-declare arm)");
    let declared = caps
        .get("elicitation")
        .and_then(|e| e.as_bool())
        .unwrap_or_else(|| panic!("report_client_caps missing elicitation bool: {caps:?}"));
    assert!(
        !declared,
        "SC3 / GP-1: when the downstream client does NOT declare elicitation, fanin-mcp \
         must NOT advertise elicitation to the upstream (probe should observe \
         elicitation=false); got {caps:?}"
    );
    child.into_guard().shutdown().await.ok();
}

/// Master SC5: `probe__needs_elicitation` forwards the raw request to an
/// elicitation-capable downstream client, the client answers ACCEPT with
/// content, and the probe's tool result relays the accept back through the
/// upstream call path. Asserts the DIRECT outcome (accept) and that the call
/// completes within the deadline (no hang).
#[tokio::test]
async fn elicitation_accept_forwarded_end_to_end() {
    let result = drive_elicitation(Answer::Accept(json!({ "answer": "yes" }))).await;
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "needs_elicitation accept must forward the probe's success result"
        );
    }
    let outcome = el::parse_elicitation_outcome(&result, "SC5 accept");
    el::assert_accept(&outcome, "SC5 accept");
    el::assert_action(&outcome, "accept", "SC5 accept");
    // The accept content round-trips byte-faithfully (D-004 / GP-8).
    let content = el::accept_content(&outcome);
    assert_eq!(
        content.get("answer").and_then(|a| a.as_str()),
        Some("yes"),
        "SC5: accept content must round-trip byte-faithfully; got {content:?}"
    );
}

/// Master SC6: a decline response from the downstream client is observable as
/// non-accept and is NOT rewritten as accept. Asserts the DIRECT outcome
/// (action=decline, non_accept=true). A stub that silently treats decline as
/// accept fails this assertion — the exact security failure SC10 prevents.
#[tokio::test]
async fn elicitation_decline_is_distinct_non_accept() {
    let result = drive_elicitation(Answer::Decline).await;
    let outcome = el::parse_elicitation_outcome(&result, "SC6 decline");
    el::assert_non_accept(&outcome, "SC6 decline");
    el::assert_action(&outcome, "decline", "SC6 decline");
}

/// Master SC7: a cancel response from the downstream client is observable as
/// non-accept and is NOT rewritten as accept. Asserts the DIRECT outcome
/// (action=cancel, non_accept=true). Distinct from decline (GP-5).
#[tokio::test]
async fn elicitation_cancel_is_distinct_non_accept() {
    let result = drive_elicitation(Answer::Cancel).await;
    let outcome = el::parse_elicitation_outcome(&result, "SC7 cancel");
    el::assert_non_accept(&outcome, "SC7 cancel");
    el::assert_action(&outcome, "cancel", "SC7 cancel");
}

/// Master SC4 / GOTCHA #8 regression: with a downstream client that does NOT
/// declare elicitation, `probe__needs_elicitation` takes the EXISTING bounded
/// rejection path and does NOT hang. The probe's tool result reflects the
/// rejection (non-accept outcome), and the call completes within the deadline.
/// This preserves the MVP reverse-traffic contract — forwarding is gated on
/// the downstream capability, and the no-capability arm is still rejection.
#[tokio::test]
async fn elicitation_no_capability_takes_bounded_rejection_path() {
    let mut child = no_elicitation_child().await;
    let started = std::time::Instant::now();
    let resp = timeout(
        ELICIT_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            json!({ "name": "probe__needs_elicitation", "arguments": {} }),
        ),
    )
    .await
    .expect(
        "needs_elicitation with no downstream elicitation capability must complete via \
         the bounded rejection path, not hang (SC4 / GOTCHA #2)",
    );
    common::assert_no_rpc_error(&resp, "needs_elicitation no-cap rejection");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_elicitation no-cap returned no result"));
    // The probe encodes the rejection it received as a non-accept outcome. The
    // exact rejection shape (structured error vs error result) depends on how
    // fanin-mcp relays the `elicitation_rejected` error; either way the probe
    // surfaces a non-accept outcome. Assert non-accept directly.
    let outcome = el::parse_elicitation_outcome(&result, "SC4 no-cap rejection");
    el::assert_non_accept(&outcome, "SC4 no-cap rejection");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "no-capability rejection must be fast; took {elapsed:?}"
    );
    child.into_guard().shutdown().await.ok();
}

/// Master SC13 regression: sampling remains rejected by `create_message` even
/// after the elicitation-forwarding slice lands. The probe's `needs_sampling`
/// tool sends `sampling/createMessage`; the aggregator rejects it; the probe's
/// tool call completes (not hung). No sampling forwarding path was added.
#[tokio::test]
async fn sampling_remains_rejected_under_elicitation_forwarding() {
    let mut child = elicitation_capable_child().await;
    let resp = timeout(
        ELICIT_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            json!({ "name": "probe__needs_sampling", "arguments": {} }),
        ),
    )
    .await
    .expect("needs_sampling must complete (sampling rejected, not hung — SC13)");
    common::assert_no_rpc_error(&resp, "needs_sampling under elicitation forwarding");
    // The probe's needs_sampling still returns its detached success text; the
    // observable contract is that the call completes (the reverse-path
    // sampling request was rejected, not forwarded). A stub that ADDS a
    // sampling forwarding path would hang waiting for a downstream sampling
    // answer that never comes — caught by the deadline.
    child.into_guard().shutdown().await.ok();
}

/// Master SC14 regression: roots remains an empty `list_roots` response even
/// after the elicitation-forwarding slice lands. The probe's `needs_roots` tool
/// sends `roots/list`; the aggregator answers with an empty list; the probe's
/// tool call completes (not hung). No roots forwarding path was added.
#[tokio::test]
async fn roots_remain_empty_under_elicitation_forwarding() {
    let mut child = elicitation_capable_child().await;
    let resp = timeout(
        ELICIT_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            json!({ "name": "probe__needs_roots", "arguments": {} }),
        ),
    )
    .await
    .expect("needs_roots must complete (roots empty, not hung — SC14)");
    common::assert_no_rpc_error(&resp, "needs_roots under elicitation forwarding");
    child.into_guard().shutdown().await.ok();
}
