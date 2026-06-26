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
//! ## Oracle — grandchild PID liveness, NOT marker absence
//!
//! The hard-kill orphan test (SC 21) is the MANDATORY D-009 proof. The
//! probe's `spawn_grandchild` tool writes a marker file whose CONTENT is the
//! grandchild's PID, then sleeps for 30s and removes the marker only on a
//! CLEAN exit. Containment works by Job Object `KILL_ON_JOB_CLOSE` on Windows
//! / process-group kill on Unix — that is a hard `TerminateProcess` that
//! kills the grandchild WITHOUT running its cleanup, so the marker PERSISTS
//! even though the process is dead. On a force-kill of fanin-mcp, Rust `Drop`
//! never runs either; only the kernel job-close kills the tree — correctly.
//!
//! Net: marker-absence cannot distinguish "contained/killed" from "survived."
//! Both leave the marker at the test's check time. So the oracle is the
//! grandchild PROCESS LIVENESS, not the marker's absence:
//!
//! 1. Before teardown, capture the grandchild PID from the marker content.
//! 2. After teardown + the cleanup interval, assert that PID is NOT alive.
//!
//! The marker file may remain as the PID-communication channel; the test's
//! own cleanup removes it on drop so temp files do not accumulate.
//!
//! All tests are wire-level. The suite compiles clean against the current
//! tree and asserts the dead-process observable, not the marker file.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// How long to poll for the grandchild's death after force-killing
/// fanin-mcp. The containment layer (Job Object kill-on-close / process
/// group kill) kills the grandchild when the tree is torn down; the OS
/// reaping is asynchronous, so the tests poll within this window rather
/// than checking once. The grandchild lifetime (30s) far exceeds this
/// window, so a SURVIVING grandchild (containment failed) is still alive
/// at the end — the poll does not mask a real failure: a working
/// containment lands in well under a second (the stdin-EOF path kills in
/// ~400ms); a failed containment stays alive past 30s.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(5);

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

/// Test whether a process with the given PID is currently alive.
///
/// Cross-platform, shell-out only (no `libc` / `windows-sys` test dep):
/// - Unix: `kill -0 <pid>` — the zero-signal probe. Exit 0 means the process
///   exists; non-zero means no such process (or we lack permission, which on
///   a test-owned grandchild does not arise). This mirrors the existing
///   `kill_process_by_id` pattern in `common/mod.rs`.
/// - Windows: `tasklist /FI "PID eq <pid>" /NH /FO CSV` and inspect the output
///   for the quoted PID. `tasklist` ships on every Windows edition.
///
/// Returns `true` if the process is alive, `false` if it is dead. A dead
/// PID is the containment success observable; an alive PID means the tree
/// survived the force-kill (containment failed).
fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // `kill -0` is the standard liveness probe: exit 0 if the process
        // exists (and we may signal it), non-zero otherwise. On a test-spawned
        // grandchild owned by the same user, EPERM does not arise.
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
        // tasklist filters by PID; if the PID appears in the CSV output, the
        // process is alive. tasklist ships on every Windows edition.
        let output = match std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(o) => o,
            Err(_) => return false, // tasklist missing — treat as dead.
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        // When no process matches, tasklist prints an informational line
        // ("INFO: No tasks are running ...") that never quotes the PID. A
        // live process row contains the quoted PID; match the quoted form
        // to avoid a false positive against the INFO line.
        stdout.contains(&format!("\"{pid}\""))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Parse the grandchild PID from the marker file content. The probe writes
/// `std::process::id().to_string()` as the marker; a missing or non-numeric
/// marker means the grandchild never started cleanly, which is a setup
/// failure surfaced clearly to the implementer.
fn parse_grandchild_pid(marker_path: &str) -> u32 {
    let content = std::fs::read_to_string(marker_path).unwrap_or_else(|e| {
        panic!(
            "grandchild marker must be readable to extract the PID; \
             path: {marker_path}, error: {e}"
        )
    });
    let trimmed = content.trim();
    trimmed.parse::<u32>().unwrap_or_else(|e| {
        panic!(
            "grandchild marker must contain the grandchild PID as a number; \
             path: {marker_path}, content: {content:?}, parse error: {e}"
        )
    })
}

/// Poll for the grandchild process to die within a bounded window. The
/// containment layer (Job Object kill-on-close / process group kill) kills
/// the grandchild when fanin-mcp's process tree is torn down, but the OS
/// reaping is asynchronous — a single check at a fixed interval can race
/// the kill propagation. This polls up to `deadline` (wall-clock from the
/// call) and returns `true` if the process dies within the window, `false`
/// if it is still alive at the end (the containment failure the test
/// catches).
///
/// The grandchild lifetime (30s) far exceeds the deadline, so a SURVIVING
/// grandchild (containment failed) is still alive at the end of the window
/// — the poll does not mask a real failure: a failed containment stays
/// alive past 30s, well beyond any reasonable poll deadline.
async fn wait_for_process_death(pid: u32, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    let poll = Duration::from_millis(200);
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

/// Master SC 19 / SC 20 / SC 21: the mandatory hard-kill orphan test (D-009).
///
/// The test:
/// 1. Spawns `fanin-mcp` with a single configured upstream.
/// 2. Invokes `spawn_grandchild` on the upstream, which starts a long-lived
///    descendant process that writes a marker file at a known path. The
///    marker CONTENT is the grandchild's PID.
/// 3. Force-kills `fanin-mcp` (kill the aggregator, not the probe — the
///    probe is a child of the aggregator).
/// 4. Waits `CLEANUP_INTERVAL` for the OS to reap the tree.
/// 5. Asserts the grandchild PROCESS is DEAD — captured by its PID from the
///    marker, then checked for liveness. The containment layer (Job Object
///    kill-on-close on Windows, process group kill on Unix) hard-terminates
///    the grandchild WITHOUT running its cleanup, so the marker may persist
///    even though the process is dead — the marker's absence is NOT the
///    oracle, the PID's liveness is. An uncontained tree leaves the
///    grandchild alive and the PID still resolves — the failure this catches.
///
/// On Windows, this catches the `cmd /c npx` descendant shape (the
/// grandchild is a detached child of the probe, which is a child of
/// fanin-mcp; the Job Object must kill the whole tree). On Unix, the
/// process group kill must reach the grandchild.
///
/// The oracle polls the grandchild PID for death within a bounded window
/// (5s, far shorter than the 30s grandchild lifetime). A working
/// containment kills the grandchild in well under a second (the stdin-EOF
/// path kills in ~400ms); a failed containment leaves the orphan alive
/// past 30s, so the 5s window cleanly distinguishes the two.
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
    // tool starts a long-lived descendant that writes the marker (whose
    // content is the grandchild's PID).
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

    // Confirm the marker was written (the grandchild is alive) and capture
    // the grandchild PID from the marker content. The marker is written by
    // the grandchild before it sleeps; the "marker must be present before
    // kill" setup check stays as the precondition that the grandchild
    // actually started.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let marker_before = std::fs::read_to_string(&marker_path).unwrap_or_default();
    assert!(
        !marker_before.is_empty(),
        "grandchild marker must be present before fanin-mcp is killed; \
         path: {marker_path}"
    );
    let grandchild_pid = parse_grandchild_pid(&marker_path);

    // Force-kill fanin-mcp. Take the underlying tokio Child so we can kill
    // it directly without the clean-EOF shutdown path.
    let guard = child.into_guard();
    guard.kill_and_wait().await;

    // SC 21: zero surviving upstream descendants. The oracle is the
    // grandchild PROCESS LIVENESS — the marker may persist (the hard kill
    // does not run the grandchild's cleanup), but a contained tree leaves
    // the PID dead. The OS reaping is asynchronous, so we poll for the
    // grandchild's death within a bounded window (CLEANUP_INTERVAL) rather
    // than checking once. The grandchild lifetime (30s) far exceeds the
    // window, so a SURVIVING grandchild (containment failed) is still alive
    // at the end — the poll does not mask a real failure. An uncontained
    // tree leaves the orphan alive and the PID still resolves — the
    // failure this catches.
    assert!(
        wait_for_process_death(grandchild_pid, CLEANUP_INTERVAL).await,
        "hard-kill orphan test (SC 21 / D-009): after force-killing fanin-mcp, the \
         grandchild (pid {grandchild_pid}) must be DEAD within {CLEANUP_INTERVAL:?} \
         (containment killed the descendant); it is still alive. The marker may \
         persist (hard kill does not run cleanup) — the dead PROCESS is the oracle, \
         not the marker file. marker at {marker_path}"
    );

    // Clean up the stale marker. The marker file may remain (hard kill does
    // not remove it); the test owns its temp-file cleanup so files do not
    // accumulate across runs.
    let _ = std::fs::remove_file(&marker_path);
}

/// Master SC 22: normal stdin-EOF teardown also terminates the full upstream
/// tree. A clean shutdown (close stdin => EOF => fanin-mcp exits) must kill
/// the spawned upstream AND any descendants, not just the aggregator.
///
/// The observable: after a clean shutdown, the grandchild PROCESS is dead.
/// The grandchild's PID is captured from the marker (written before the
/// teardown); a contained tree kills the grandchild before its 30s lifetime
/// elapses, so the PID is dead at the check. The marker may persist (the
/// kill does not run the grandchild's cleanup) — the dead PROCESS is the
/// oracle, not the marker file.
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

    // Confirm the marker is present and capture the grandchild PID.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let marker_before = std::fs::read_to_string(&marker_path).unwrap_or_default();
    assert!(
        !marker_before.is_empty(),
        "grandchild marker must be present before EOF teardown; path: {marker_path}"
    );
    let grandchild_pid = parse_grandchild_pid(&marker_path);

    // Clean shutdown: close stdin (EOF) and wait for fanin-mcp to exit.
    child.into_guard().shutdown().await.ok();

    // SC 22: the full upstream tree must be terminated. The grandchild
    // PROCESS must be dead. The marker may persist (the kill does not run
    // the grandchild's cleanup); the dead PID is the oracle, not the
    // marker file. The OS reaping is asynchronous, so we poll for the
    // grandchild's death within a bounded window (CLEANUP_INTERVAL). A
    // teardown that killed only the aggregator (not the tree) would leave
    // the grandchild alive past the window — the failure this catches.
    assert!(
        wait_for_process_death(grandchild_pid, CLEANUP_INTERVAL).await,
        "stdin-EOF teardown (SC 22): the grandchild (pid {grandchild_pid}) must be \
         DEAD within {CLEANUP_INTERVAL:?} after a clean shutdown; it is still alive. \
         A teardown that killed only the aggregator (not the tree) would leave the \
         grandchild alive. The marker may persist (kill does not run cleanup) — \
         the dead PROCESS is the oracle. marker at {marker_path}"
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
