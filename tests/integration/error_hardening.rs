//! Phase 4 — upstream error hardening and mid-session death (wire-level).
//!
//! Covers master Success Criteria 6, 7, 8, 9 (error model finalization +
//! dead-upstream coverage) and contributes to SC 14 / SC 15 (D-005 shape
//! compatibility + no JSON-RPC-error drift).
//!
//! The headline proof (SC 6, 7): start fanin-mcp with TWO configured upstreams
//! (`probe` + `probe2`, both the probe-server binary under distinct
//! configured names), discover both, then KILL the first upstream process
//! mid-session and assert:
//!   - a subsequent `invoke_tool probe__echo_ok` returns
//!     `CallToolResult { isError: true }` with the D-005 fields and
//!     `code: "upstream_disconnected"` (the accepted default per state.json
//!     `decisions` — no silent reconnect);
//!   - a concurrent / subsequent `invoke_tool probe2__echo_ok` still
//!     SUCCEEDS (sibling isolation — a dead upstream does not poison its
//!     siblings or serialize the session).
//!
//! The probe's `self_pid` tool returns the probe's own PID so the test can
//! address and kill the specific grandchild upstream (the probe is a
//! grandchild of the test, spawned by fanin-mcp; the test has no direct handle
//! to it). The side-effect assertion is the dead PROCESS (the killed PID is
//! reaped), plus the structured error returned for the dead server.
//!
//! All tests are wire-level. The suite compiles clean against the current
//! (pre-Phase-4) tree and fails RED on the absent behavior: the current
//! `registry.rs::call_tool` returns `upstream_call_failed` for a broken pipe,
//! not `upstream_disconnected`; the dead-upstream code does not exist yet.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// Deadline for a dead-upstream call. A dead upstream must return the
/// structured `upstream_disconnected` error promptly, not hang. 5s is a
/// generous ceiling that still catches a hang (the registry would await a
/// broken pipe forever if it did not detect the death).
const DEAD_DEADLINE: Duration = Duration::from_secs(5);

/// Deadline for the `needs_sampling` clean-rejection proof (SC 9). The
/// aggregator must reject the probe's sampling request immediately; 10s is
/// the same ceiling as the Phase 1 reverse-traffic tests and catches a hang.
const REJECT_DEADLINE: Duration = Duration::from_secs(10);

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

/// Test whether a process with the given PID is currently alive. Cross-
/// platform, shell-out only (mirrors `process_lifetime::process_is_alive`).
fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let status = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        matches!(status, Ok(s) if s.success())
    }
    #[cfg(windows)]
    {
        let output = match std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(o) => o,
            Err(_) => return false,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains(&format!("\"{pid}\""))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Kill a process by PID (cross-platform, shell-out only). Used to simulate
/// mid-session upstream death: the test kills the probe process directly,
/// not the aggregator, so the aggregator observes a broken pipe / dead
/// connection on the next call to that server.
fn kill_process_by_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string(), "/T"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

/// Poll for a process to die within a bounded window (mirrors
/// `process_lifetime::wait_for_process_death`).
async fn wait_for_process_death(pid: u32, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    let poll = Duration::from_millis(100);
    loop {
        if !process_is_alive(pid) {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(poll).await;
    }
}

/// Build a two-upstream config: `probe` + `probe2`, both the probe-server
/// binary under distinct configured names, in a `default` namespace exposing
/// both. Mirrors `multi_upstream::alpha_beta_config` but keeps the names
/// `probe` / `probe2` so the test's invoke paths read naturally.
fn probe_probe2_config() -> fx::ConfigFile {
    fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("probe"))
        .server(fx::ServerEntry::new("probe2"))
        .namespace(fx::NamespaceEntry::new("default", ["probe", "probe2"]))
        .write()
}

/// Helper: spawn the aggregator with the single-`probe` config + initialize.
async fn phase1_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    child
}

/// Master SC 6 + SC 7: kill the first upstream mid-session, then assert a
/// subsequent call to the DEAD upstream returns `upstream_disconnected`
/// while a sibling upstream stays callable.
///
/// Side-effect assertions:
///   - The killed probe PROCESS is dead (the test killed it; the aggregator's
///     containment layer is NOT involved here — the test kills the upstream
///     directly, simulating an external crash. The aggregator must OBSERVE
///     the death on the next call, not prevent it.)
///   - The dead-upstream call returns `CallToolResult { isError: true }`
///     with the D-005 fields and `code: "upstream_disconnected"`.
///   - The sibling upstream's `echo_ok` still succeeds in the SAME aggregator
///     session (sibling isolation — a dead upstream does not poison the
///     registry or serialize the session).
///
/// The current tree returns `upstream_call_failed` for a broken pipe (the
/// dead-upstream code does not exist yet), so the `code` assertion fails RED
/// until the implementer adds `upstream_disconnected`.
#[tokio::test]
async fn dead_upstream_returns_structured_error_and_sibling_stays_callable() {
    let cfg = probe_probe2_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Discover both upstreams so they are lazily spawned. This gives the
    // aggregator a cached inventory for each AND spawns the probe processes.
    let list = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools must complete (lazy spawn of both upstreams)");
    common::assert_no_rpc_error(&list, "list_tools both upstreams");

    // Ask the FIRST upstream for its own PID via the probe's `self_pid` tool.
    // The probe is a grandchild of the test (spawned by fanin-mcp); the test
    // has no direct handle to it, so the probe reports its PID over the wire.
    let pid_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__self_pid",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("probe__self_pid must complete so the test can address the probe PID");
    common::assert_no_rpc_error(&pid_resp, "probe__self_pid");
    let pid_result = pid_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("probe__self_pid returned no result"));
    let pid_text = result_text(&pid_result);
    let probe_pid: u32 = pid_text.trim().parse().unwrap_or_else(|e| {
        panic!("probe__self_pid must return a numeric PID; got text: {pid_text:?}\n{e}")
    });

    // Sanity: the probe process is alive before we kill it.
    assert!(
        process_is_alive(probe_pid),
        "probe (pid {probe_pid}) must be alive before the kill; setup failure"
    );

    // Kill the first upstream mid-session. This simulates an external crash
    // (the aggregator's containment layer is NOT involved — we are killing
    // the upstream directly, not the aggregator). The aggregator must
    // OBSERVE the death on the next call to that server.
    kill_process_by_pid(probe_pid);

    // Side-effect: the killed probe PROCESS must be dead. The test owns the
    // kill; the aggregator does not clean up the killed upstream (it is an
    // external crash, not a containment teardown). Poll for death within a
    // short window so the subsequent call assertion runs against a confirmed
    // dead upstream.
    assert!(
        wait_for_process_death(probe_pid, Duration::from_secs(2)).await,
        "killed probe (pid {probe_pid}) must be dead before the dead-upstream call; \
         the test killed it directly"
    );

    // SC 6: a subsequent call to the DEAD upstream returns a structured
    // `upstream_disconnected` error, not a JSON-RPC error, not a hang, and
    // not a silent reconnect (state.json `decisions.reconnect-policy`: no
    // silent reconnect; surface `upstream_disconnected`).
    let dead_resp = timeout(
        DEAD_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": "after-death" },
            }),
        ),
    )
    .await
    .expect(
        "invoke_tool probe__echo_ok after upstream death must complete within \
         {DEAD_DEADLINE:?} — a hang means the aggregator did not detect the dead \
         connection (SC 6)",
    );
    // SC 15 / D-005: the dead-upstream failure must NOT be a JSON-RPC error.
    common::assert_no_rpc_error(&dead_resp, "probe__echo_ok after death");
    let dead_result = dead_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("probe__echo_ok after death returned no result"));
    common::assert_is_error_result(&dead_result, "probe__echo_ok after death");

    let dead_err = parse_error_json(&dead_result);
    // SC 6 + SC 14: the error carries the D-005 fields and the finalized
    // dead-upstream code `upstream_disconnected` (state.json decision).
    assert_eq!(
        dead_err.get("code").and_then(|c| c.as_str()),
        Some("upstream_disconnected"),
        "dead-upstream call must carry code `upstream_disconnected` (state.json \
         decision; no silent reconnect); got: {dead_err:?}"
    );
    assert_eq!(
        dead_err.get("server").and_then(|s| s.as_str()),
        Some("probe"),
        "dead-upstream error must name the server `probe`; got: {dead_err:?}"
    );
    assert!(
        dead_err.get("tool").and_then(|t| t.as_str()).is_some(),
        "dead-upstream error must carry the tool field; got: {dead_err:?}"
    );
    assert!(
        dead_err.get("message").and_then(|m| m.as_str()).is_some(),
        "dead-upstream error must carry a message; got: {dead_err:?}"
    );
    assert!(
        dead_err.get("recoverable").is_some(),
        "dead-upstream error must carry the recoverable field (D-005); got: {dead_err:?}"
    );

    // SC 7: the sibling upstream stays callable in the SAME session. A dead
    // upstream must not poison the registry or serialize the session. The
    // sibling `probe2` was spawned earlier and must still answer echo_ok.
    let sib_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe2__echo_ok",
                "arguments": { "message": "sibling-alive" },
            }),
        ),
    )
    .await
    .expect(
        "probe2__echo_ok must complete within {SPAWN_DEADLINE:?} after probe died — \
         a dead upstream must not serialize the session or poison siblings (SC 7)",
    );
    common::assert_no_rpc_error(&sib_resp, "probe2__echo_ok after probe death");
    let sib_result = sib_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("probe2__echo_ok returned no result"));
    if let Some(is_error) = sib_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "probe2__echo_ok must succeed after probe died (sibling isolation, SC 7)"
        );
    }
    let sib_text = result_text(&sib_result);
    assert!(
        sib_text.contains("sibling-alive"),
        "probe2__echo_ok must echo byte-faithfully after probe death; got: {sib_text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 8: `probe__always_error` round-trips the probe's OWN structured
/// error content byte-faithfully. It is NOT re-wrapped as `upstream_call_failed`,
/// NOT stringified into a generic message, and NOT converted into a JSON-RPC
/// error.
///
/// The probe's `always_error` returns `CallToolResult::error` with text content
/// `{"code":"always_error","message":"...","recoverable":false}`. The
/// aggregator must forward that result unchanged (D-004 byte-faithfulness).
/// A re-wrapping implementation would replace the content with its own
/// `upstream_call_failed` JSON and fail the `code: "always_error"` assertion.
#[tokio::test]
async fn always_error_round_trips_upstream_error_content_byte_faithfully() {
    let mut child = phase1_child().await;

    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__always_error",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__always_error must complete");
    // SC 15 / D-005: the upstream's tool-level failure must NOT surface as a
    // JSON-RPC error.
    common::assert_no_rpc_error(&resp, "invoke_tool probe__always_error");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool always_error returned no result"));
    common::assert_is_error_result(&result, "invoke_tool probe__always_error");

    // SC 8: the probe's OWN error content round-trips byte-faithfully. Parse
    // the text content as JSON and assert the probe's `code: "always_error"`
    // is present — a re-wrapping stub that replaces the content with its own
    // `upstream_call_failed` JSON fails this assertion.
    let text = result_text(&result);
    let probe_err: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "always_error content must be the probe's structured JSON, not a \
             re-wrapped string; got: {text:?}\n{e}"
        )
    });
    assert_eq!(
        probe_err.get("code").and_then(|c| c.as_str()),
        Some("always_error"),
        "always_error must round-trip the probe's `code: always_error` byte-faithfully \
         (SC 8 — not re-wrapped as upstream_call_failed); got: {probe_err:?}"
    );
    assert!(
        probe_err.get("message").and_then(|m| m.as_str()).is_some(),
        "always_error must round-trip the probe's message; got: {probe_err:?}"
    );
    // The probe sets recoverable: false. A re-wrapping stub that always sets
    // recoverable: true fails this.
    assert_eq!(
        probe_err.get("recoverable").and_then(|r| r.as_bool()),
        Some(false),
        "always_error must round-trip the probe's `recoverable: false`; got: {probe_err:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 9: `probe__needs_sampling` receives a clean rejection path and
/// completes without hanging. This re-asserts the Phase 1 reverse-traffic
/// contract in the Phase 4 context — Phase 4 must not regress the clean
/// rejection. The probe sends `sampling/createMessage` upstream; the
/// aggregator's `ClientHandler::create_message` rejects immediately, so the
/// probe's `call_tool` resolves.
#[tokio::test]
async fn needs_sampling_completes_without_hanging() {
    let mut child = phase1_child().await;

    let started = std::time::Instant::now();
    let resp = timeout(
        REJECT_DEADLINE,
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
        "needs_sampling must complete within {REJECT_DEADLINE:?} — the aggregator \
         must reject the probe's sampling request, not hang (SC 9 / GOTCHA #2)",
    );
    common::assert_no_rpc_error(&resp, "invoke_tool probe__needs_sampling");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("needs_sampling returned no result"));
    // The probe returns a SUCCESS text block once its detached send future is
    // spawned. The rejection happens on the reverse path, not the forward
    // result. Asserting SUCCESS makes the test fail RED against a stub that
    // returns a not-implemented error without forwarding.
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "needs_sampling must forward the probe's success result (SC 9)"
        );
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "needs_sampling should complete quickly once the sampling request is \
         rejected; took {elapsed:?}"
    );

    child.into_guard().shutdown().await.ok();
}

// ---- Review-fix coverage (F4) ----------------------------------------------
//
// F4 — send-side broken pipe misclassified as `upstream_call_failed`. The
// THOROUGH review (F4) found `map_service_error` matches only
// `ServiceError::TransportClosed`. A child that dies such that the failure
// is FIRST observed on the WRITE side surfaces as
// `ServiceError::TransportSend(...)` → reported as `upstream_call_failed`
// instead of the Phase 4 `upstream_disconnected`.

/// Review fix F4 — send-side death → `upstream_disconnected` (robustness,
/// best-effort). NOT DETERMINISTIC wire-level: whether a killed upstream's
/// next call surfaces as `ServiceError::TransportClosed` (the transport
/// worker already detected the EOF/closure) vs `ServiceError::TransportSend`
/// (the send fails before the worker notices) is a race between the OS
/// pipe-closure propagation and the aggregator's next `call_tool` send. On
/// Windows the pipe behavior differs from Unix. The existing
/// `dead_upstream_returns_structured_error_and_sibling_stays_callable` test
/// covers the `TransportClosed` path (it happens to surface that way on this
/// host/timing); this stub would cover the `TransportSend` path, but forcing
/// the send-side observation deterministically requires injecting into the
/// transport (a hook to make the send fail before the worker detects
/// closure), which is below the wire-level surface.
///
/// Unblock trigger: a wire-level harness that can deterministically force the
/// send-side failure surface (e.g. a transport wrapper that injects a broken
/// pipe on the next send without closing the read side first), OR a unit-level
/// test against `map_service_error` once that function is extracted/testable.
/// Until then, the F4 code fix (also map `TransportSend` from an established
/// upstream operation to `UpstreamDisconnected`) is verified by the existing
/// `TransportClosed` path test plus the code review — the send-side path is
/// the documented gap.
#[tokio::test]
#[ignore = "F4: send-side broken pipe surfaces as TransportSend vs TransportClosed \
            non-deterministically (OS pipe-closure race, Windows/Unix differ). The existing \
            dead_upstream_returns_structured_error_and_sibling_stays_callable test covers the \
            TransportClosed path; this stub covers the TransportSend path. Unblock: a wire-level \
            transport wrapper that forces send-side failure, or a unit test against \
            map_service_error once extracted. Reason: no flaky test."]
async fn f4_send_side_death_returns_upstream_disconnected_not_call_failed() {
    // Stub: the deterministic wire-level sequence would be:
    //   1. spawn aggregator with probe upstream, discover (lazy spawn).
    //   2. kill the probe PID.
    //   3. RACE: wait just long enough that the OS has closed the pipe but the
    //      aggregator's transport worker has NOT yet detected the EOF, then
    //      call invoke_tool so the SEND fails first → TransportSend.
    //   4. assert the structured error code is `upstream_disconnected`.
    // Step 3 is a race we cannot make deterministic without a transport hook.
    // Left ignored with the reason + unblock trigger above.
}

/// Phase 5 CARRY-4 deterministic guard. This is the compile-safe contract
/// available until the implementer exposes `map_service_error` to tests: the
/// source must explicitly classify `ServiceError::TransportSend(_)` with the
/// same `upstream_disconnected` branch as `TransportClosed`.
///
/// Test-needs-impl dependency recorded in `tests.md`: replace this source guard
/// with a direct call against an exposed `map_service_error`/wrapper once the
/// source surface exists. The test is intentionally NOT ignored so the Phase 5
/// gate has an always-run CARRY-4 proof.
#[test]
fn service_error_transport_send_maps_to_upstream_disconnected_deterministically() {
    let registry = std::fs::read_to_string("src/registry.rs")
        .expect("src/registry.rs must be readable for the CARRY-4 source guard");
    assert!(
        registry.contains("ServiceError::TransportSend"),
        "map_service_error must explicitly match ServiceError::TransportSend(_)"
    );
    assert!(
        registry.contains("ServiceError::TransportClosed | ServiceError::TransportSend")
            || registry.contains("ServiceError::TransportSend(_) => ToolError::UpstreamDisconnected")
            || registry.contains("ServiceError::TransportSend(_)\n")
                && registry.contains("ToolError::UpstreamDisconnected"),
        "ServiceError::TransportSend must map to ToolError::UpstreamDisconnected / public upstream_disconnected; registry.rs:\n{registry}"
    );
}
