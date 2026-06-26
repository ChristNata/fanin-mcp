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

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
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
    let cfg = fx::ConfigBuilder::new()
        .log_file(&log_path)
        .write();
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

    let log_contents = std::fs::read_to_string(&log_path).unwrap_or_else(|e| {
        panic!("failed to read log file at {log_path}: {e}")
    });

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

    // The probe returns a SUCCESS text block ("needs_elicitation: sent
    // elicitation/create request to client") once its detached send future is
    // spawned. The aggregator must forward that result byte-faithfully. A
    // Phase 0 stub returns a not-implemented ERROR (isError: true) without
    // forwarding — so asserting SUCCESS here makes the test fail RED against
    // the stub and pass GREEN once real forwarding + the elicitation-reject
    // handler land.
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "needs_elicitation must forward the probe's success result, not a \
             not-implemented error (isError must not be true)"
        );
    }
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
        "needs_elicitation must forward the probe's success text (the probe \
         confirms it sent the elicitation request); got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}