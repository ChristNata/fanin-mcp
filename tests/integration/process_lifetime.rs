//! Phase 3 — process-tree lifetime (wire-level).
//!
//! Covers master Success Criteria 19–23:
//! - Windows Job Object / Unix process group containment (SC 19, 20).
//! - hard-kill orphan test: force-kill fanin-mcp and observe zero surviving
//!   upstream descendants (SC 21, D-009, GOTCHA #11/#14).
//! - stdin-EOF teardown terminates the full upstream tree (SC 22).
//! - stderr capture intact after process wrapping: `[server]`-prefixed
//!   redacted lines still reach the log file (SC 23).
//!
//! The hard-kill orphan test (SC 21) is the MANDATORY D-009 proof. It uses
//! the probe's `spawn_grandchild` tool to start a long-lived descendant
//! process that writes a presence marker; after force-killing fanin-mcp, the
//! test asserts the marker is gone (the grandchild was killed by the
//! containment layer). An uncontained tree leaves the grandchild alive and
//! the marker persists — the failure the test catches.
//!
//! All tests are wire-level. The suite compiles clean against the current
//! tree (no Job Object / process group in `process.rs`) and fails RED on
//! the absent containment, not on missing symbols.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// How long to wait after force-killing fanin-mcp before checking the
/// grandchild marker. The containment layer (Job Object kill-on-close /
// process group kill) is observed synchronously on the kill, but the OS
// reaping + the grandchild's own shutdown (marker removal on clean exit)
// may take a moment. The grandchild lifetime (30s) is far longer than this
/// interval, so a surviving grandchild keeps the marker; a contained
/// grandchild is killed before the lifetime elapses, so the marker is
/// removed (or never written).
const CLEANUP_INTERVAL: Duration = Duration::from_secs(2);

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

/// Master SC 19 / SC 20 / SC 21: the mandatory hard-kill orphan test (D-009).
///
/// The test:
/// 1. Spawns `fanin-mcp` with a single configured upstream.
/// 2. Invokes `spawn_grandchild` on the upstream, which starts a long-lived
///    descendant process that writes a presence marker at a known path.
/// 3. Force-kills `fanin-mcp` (kill the aggregator, not the probe — the
///    probe is a child of the aggregator).
/// 4. Waits `CLEANUP_INTERVAL` for the OS to reap the tree.
/// 5. Asserts the marker is GONE — the grandchild was killed by the
///    containment layer (Job Object kill-on-close on Windows, process group
///    kill on Unix). An uncontained tree leaves the grandchild alive and
///    the marker persists — the failure the test catches.
///
/// On Windows, this catches the `cmd /c npx` descendant shape (the
/// grandchild is a detached child of the probe, which is a child of
/// fanin-mcp; the Job Object must kill the whole tree). On Unix, the
/// process group kill must reach the grandchild.
///
/// The current tree has no containment (`process.rs` spawns a bare
/// `TokioChildProcess`), so the grandchild survives and the marker persists
/// — RED until the implementer installs Job Object / process group
/// containment.
#[tokio::test]
async fn hard_kill_orphan_test_no_surviving_descendants() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let marker_path = fx::grandchild_marker_path();

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Trigger the lazy spawn + grandchild. The probe's `spawn_grandchild`
    // tool starts a long-lived descendant that writes the marker.
    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__spawn_grandchild"),
                "arguments": { "marker_path": marker_path.clone() },
            }),
        ),
    )
    .await
    .expect("spawn_grandchild must complete within deadline");
    common::assert_no_rpc_error(&resp, "spawn_grandchild");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("spawn_grandchild returned no result"));
    let text = result_text(&result);
    assert!(
        text.contains("descendant started"),
        "spawn_grandchild must report the descendant started; got: {text:?}"
    );

    // Confirm the marker was written (the grandchild is alive).
    tokio::time::sleep(Duration::from_millis(300)).await;
    let marker_before = std::fs::read_to_string(&marker_path).unwrap_or_default();
    assert!(
        !marker_before.is_empty(),
        "grandchild marker must be present before fanin-mcp is killed; \
         path: {marker_path}"
    );

    // Force-kill fanin-mcp. Take the underlying tokio Child so we can kill
    // it directly without the clean-EOF shutdown path.
    let guard = child.into_guard();
    guard.kill_and_wait().await;

    // Wait for the OS to reap the tree. The grandchild lifetime (30s) is
    // far longer than this interval, so a surviving grandchild keeps the
    // marker; a contained grandchild is killed before the lifetime elapses.
    tokio::time::sleep(CLEANUP_INTERVAL).await;

    // SC 21: zero surviving upstream descendants. The marker must be GONE.
    let marker_after = std::fs::read_to_string(&marker_path).unwrap_or_default();
    assert!(
        marker_after.is_empty(),
        "hard-kill orphan test (SC 21 / D-009): after force-killing fanin-mcp, the \
         grandchild marker must be GONE (containment killed the descendant); \
         marker still present at {marker_path} with content:\n{marker_after}"
    );

    // Clean up any stale marker.
    let _ = std::fs::remove_file(&marker_path);
}

/// Master SC 22: normal stdin-EOF teardown also terminates the full upstream
/// tree. A clean shutdown (close stdin => EOF => fanin-mcp exits) must kill
/// the spawned upstream AND any descendants, not just the aggregator.
///
/// The observable: after a clean shutdown, the grandchild marker is gone.
/// The grandchild lifetime (30s) is far longer than the cleanup interval, so
/// a surviving grandchild keeps the marker; a contained tree removes it.
#[tokio::test]
async fn stdin_eof_teardown_terminates_full_upstream_tree() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let marker_path = fx::grandchild_marker_path();

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Spawn the grandchild via the probe.
    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__spawn_grandchild"),
                "arguments": { "marker_path": marker_path.clone() },
            }),
        ),
    )
    .await
    .expect("spawn_grandchild must complete within deadline");
    common::assert_no_rpc_error(&resp, "spawn_grandchild (eof teardown)");

    // Confirm the marker is present.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let marker_before = std::fs::read_to_string(&marker_path).unwrap_or_default();
    assert!(
        !marker_before.is_empty(),
        "grandchild marker must be present before EOF teardown; path: {marker_path}"
    );

    // Clean shutdown: close stdin (EOF) and wait for fanin-mcp to exit.
    child.into_guard().shutdown().await.ok();

    // Wait for the tree to be reaped.
    tokio::time::sleep(CLEANUP_INTERVAL).await;

    // SC 22: the full upstream tree must be terminated. The marker must be
    // gone.
    let marker_after = std::fs::read_to_string(&marker_path).unwrap_or_default();
    assert!(
        marker_after.is_empty(),
        "stdin-EOF teardown (SC 22): the grandchild marker must be GONE after a clean \
         shutdown; marker still present at {marker_path} with content:\n{marker_after}"
    );

    let _ = std::fs::remove_file(&marker_path);
}

/// Master SC 23: child stderr capture still writes `[server]`-prefixed
/// redacted lines to the configured log file after process wrapping. The
/// containment layer (Job Object / process group) must not break the stderr
/// capture path.
///
/// The observable: after invoking a tool that makes the probe write to its
/// stderr (the probe's tracing init writes to stderr), the log file contains
/// at least one `[server]`-prefixed line. The sentinel-redaction test
/// (cred_store.rs) proves the redaction half; this test proves the capture
/// half survives process wrapping.
#[tokio::test]
async fn stderr_capture_intact_after_process_wrapping() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let log_path = fx::empty_log_file_path();

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()).with_log_file(&log_path))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Trigger a lazy spawn so the probe starts and writes to its stderr
    // (the probe's tracing init + any runtime diagnostics). The aggregator's
    // stderr capture path must write `[server]`-prefixed lines to the log.
    let _ = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_ok"),
                "arguments": { "message": "stderr-capture-check" },
            }),
        ),
    )
    .await
    .expect("echo_ok must complete (stderr capture check)");

    // Give the stderr log task a moment to flush.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();

    // SC 23: the log must contain at least one `[server]`-prefixed line.
    // The probe writes tracing lines to its stderr on init; the aggregator
    // captures and prefixes them. A process wrapper that broke stderr
    // capture would leave the log empty.
    assert!(
        log.contains(&format!("[{server}]")),
        "stderr capture must write [{server}]-prefixed lines to the log file after \
         process wrapping; log content:\n{log}"
    );

    child.into_guard().shutdown().await.ok();
}
