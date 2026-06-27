//! Phase 4 — `notifications/tools/list_changed` cache invalidation (wire-level).
//!
//! Covers master Success Criteria 10 and 11:
//! - An upstream `notifications/tools/list_changed` invalidates ONLY that
//!   upstream's cached inventory (SC 10 — per-server scope).
//! - A second `list_tools` / `inventory()` after the notification reflects
//!   the changed upstream tool inventory WITHOUT restarting fanin-mcp (SC 11
//!   — lazy refetch on next `inventory()` / `list_tools`, per state.json
//!   `decisions.cache-shape`).
//!
//! The probe fixture's `mutate_tools` tool toggles a runtime-added `added_tool`
//! in the probe's tool list, then emits `notifications/tools/list_changed`
//! toward the aggregator. The aggregator's
//! `UpstreamClientHandler::on_tool_list_changed` must mark that server's
//! cached inventory stale; the next `list_tools` / `inventory()` refetches
//! via `list_all_tools()` and reflects the new tool.
//!
//! The sibling-isolation half (SC 10): with TWO upstreams (`probe` + `probe2`),
//! triggering `mutate_tools` on `probe` must NOT invalidate `probe2`'s cache.
//! We assert `probe2`'s inventory is unchanged across the notification (its
//! row count and tool set are stable), while `probe`'s inventory grows by one
//! (the `added_tool`).
//!
//! All tests are wire-level. The suite compiles clean against the current
//! tree (`UpstreamClientHandler` has no `on_tool_list_changed` handler and
//! `UpstreamEntry.tools` is immutable) and fails RED on the absent behavior:
//! the second `list_tools` returns the same cached inventory (the
//! notification was not observed), so the `added_tool` row is missing.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn or a refetch.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// The runtime-added tool name the probe's `mutate_tools` toggles.
const ADDED_TOOL: &str = "added_tool";

/// Build a two-upstream config (`probe` + `probe2`) with a `default` namespace
/// exposing both. Used by the per-server-scope invalidation proof.
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

/// Extract the list_tools rows as a JSON array of row objects.
fn parse_list_tools_rows(result: &Value) -> Vec<Value> {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("list_tools result missing content array"));
    assert!(
        !content.is_empty(),
        "list_tools result must carry at least one content block"
    );
    let text = content
        .iter()
        .find_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("list_tools result must carry a text content block"));
    let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("list_tools text content must be valid JSON (the row array); got: {text:?}\n{e}")
    });
    parsed
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("list_tools text content must be a JSON array; got: {parsed:?}"))
}

/// Collect the set of tool names for a given server from list_tools rows.
fn tool_names_for_server(rows: &[Value], server: &str) -> Vec<String> {
    rows.iter()
        .filter_map(|r| {
            let s = r.get("server").and_then(|s| s.as_str());
            if s != Some(server) {
                return None;
            }
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|n| n.as_str()))
                .map(String::from)
        })
        .collect()
}

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

/// Master SC 11: a second `list_tools` after the upstream emits
/// `notifications/tools/list_changed` reflects the changed tool inventory
/// WITHOUT restarting fanin-mcp.
///
/// The probe's `mutate_tools` adds `added_tool` to its runtime list and emits
/// the notification. The aggregator must invalidate `probe`'s cached inventory
/// and refetch on the next `list_tools` — so the `added_tool` row appears.
///
/// The current tree's `UpstreamClientHandler` has no `on_tool_list_changed`
/// handler and `UpstreamEntry.tools` is immutable, so the second `list_tools`
/// returns the stale cached inventory and the `added_tool` row is missing —
/// RED until the implementer wires the handler + mutable cache.
#[tokio::test]
async fn list_changed_notification_invalidates_cache_reflects_new_inventory() {
    let mut child = phase1_child().await;

    // Baseline: list_tools returns the probe's static tool set (no
    // `added_tool`). This caches the inventory in the aggregator.
    let baseline_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("baseline list_tools must complete (lazy spawn + discovery)");
    common::assert_no_rpc_error(&baseline_resp, "baseline list_tools");
    let baseline_result = baseline_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("baseline list_tools returned no result"));
    let baseline_rows = parse_list_tools_rows(&baseline_result);
    let baseline_probe_tools = tool_names_for_server(&baseline_rows, "probe");
    assert!(
        !baseline_probe_tools.contains(&ADDED_TOOL.to_string()),
        "baseline list_tools must NOT include the runtime-added `{ADDED_TOOL}` \
         (it is added by mutate_tools); got: {baseline_probe_tools:?}"
    );
    let baseline_count = baseline_probe_tools.len();

    // Trigger the upstream list_changed: invoke probe__mutate_tools, which
    // toggles `added_tool` ON and emits `notifications/tools/list_changed`.
    let mutate_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__mutate_tools",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__mutate_tools must complete");
    common::assert_no_rpc_error(&mutate_resp, "invoke_tool probe__mutate_tools");
    let mutate_result = mutate_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("mutate_tools returned no result"));
    let mutate_text = result_text(&mutate_result);
    assert!(
        mutate_text.contains("added"),
        "mutate_tools must report it added the tool; got: {mutate_text:?}"
    );

    // Give the aggregator a moment to process the list_changed notification
    // (the handler marks the cache stale; the refetch is lazy on the next
    // inventory() call).
    tokio::time::sleep(Duration::from_millis(300)).await;

    // SC 11: a second list_tools reflects the changed inventory — the
    // `added_tool` row now appears for `probe`, WITHOUT restarting fanin-mcp.
    let after_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("post-notification list_tools must complete (lazy refetch)");
    common::assert_no_rpc_error(&after_resp, "post-notification list_tools");
    let after_result = after_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("post-notification list_tools returned no result"));
    let after_rows = parse_list_tools_rows(&after_result);
    let after_probe_tools = tool_names_for_server(&after_rows, "probe");
    assert!(
        after_probe_tools.contains(&ADDED_TOOL.to_string()),
        "post-notification list_tools must include the runtime-added `{ADDED_TOOL}` \
         for `probe` (SC 11 — cache invalidated + lazy refetch); got: {after_probe_tools:?}"
    );
    assert_eq!(
        after_probe_tools.len(),
        baseline_count + 1,
        "post-notification list_tools for `probe` must have exactly one more tool \
         (the added `{ADDED_TOOL}`); got {} vs baseline {}",
        after_probe_tools.len(),
        baseline_count
    );

    // Toggle the tool OFF again (mutate_tools is a toggle) and assert the
    // cache invalidates again — proving the wiring is repeatable, not a
    // one-shot.
    let mutate_resp2 = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__mutate_tools",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("second invoke_tool probe__mutate_tools must complete");
    common::assert_no_rpc_error(&mutate_resp2, "second mutate_tools");
    let mutate_text2 = result_text(
        &mutate_resp2
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("second mutate_tools returned no result")),
    );
    assert!(
        mutate_text2.contains("removed"),
        "second mutate_tools must report it removed the tool; got: {mutate_text2:?}"
    );
    assert!(
        mutate_text2.contains("removed"),
        "second mutate_tools must report it removed the tool; got: {mutate_text2:?}"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    let after2_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("post-second-notification list_tools must complete");
    common::assert_no_rpc_error(&after2_resp, "post-second-notification list_tools");
    let after2_rows = parse_list_tools_rows(
        &after2_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("post-second-notification list_tools returned no result")),
    );
    let after2_probe_tools = tool_names_for_server(&after2_rows, "probe");
    assert!(
        !after2_probe_tools.contains(&ADDED_TOOL.to_string()),
        "after the second list_changed (toggle off), list_tools must NOT include \
         `{ADDED_TOOL}`; got: {after2_probe_tools:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 10: an upstream `notifications/tools/list_changed` invalidates
/// ONLY that upstream's cached inventory. A SIBLING upstream's inventory is
/// NOT refetched or invalidated because another server sent `list_changed`.
///
/// With two upstreams (`probe` + `probe2`), triggering `mutate_tools` on
/// `probe` must invalidate `probe`'s cache (the `added_tool` appears on the
/// next list_tools for `probe`) but must NOT change `probe2`'s inventory —
/// `probe2`'s row count and tool set stay stable across the notification.
#[tokio::test]
async fn list_changed_invalidates_only_that_server_not_sibling() {
    let cfg = probe_probe2_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Baseline: list_tools returns both upstreams' inventories.
    let baseline_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("baseline list_tools must complete (lazy spawn of both upstreams)");
    common::assert_no_rpc_error(&baseline_resp, "baseline list_tools");
    let baseline_rows = parse_list_tools_rows(
        &baseline_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("baseline list_tools returned no result")),
    );
    let baseline_probe2_tools = tool_names_for_server(&baseline_rows, "probe2");
    let baseline_probe2_count = baseline_probe2_tools.len();
    assert!(
        !baseline_probe2_tools.contains(&ADDED_TOOL.to_string()),
        "baseline `probe2` inventory must not include the runtime-added tool; \
         got: {baseline_probe2_tools:?}"
    );

    // Trigger list_changed on `probe` only (mutate_tools adds `added_tool`).
    let mutate_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__mutate_tools",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__mutate_tools must complete");
    common::assert_no_rpc_error(&mutate_resp, "invoke_tool probe__mutate_tools");

    tokio::time::sleep(Duration::from_millis(300)).await;

    // SC 10: the sibling `probe2`'s inventory is NOT refetched/invalidated.
    // After `probe`'s list_changed, a list_tools returns `probe`'s updated
    // inventory (with `added_tool`) but `probe2`'s inventory is STABLE — the
    // same tool set and count as baseline. A per-server-scoped invalidation
    // touches only `probe`; a global/registry-wide invalidation would
    // refetch `probe2` too (which would still return the same set, but the
    // load-bearing observable is that `probe2`'s set is unchanged — a
    // registry that held a map lock across `probe`'s refetch or that
    // refetched ALL servers on one notification would still pass this
    // specific assertion; the concurrency guard in phase4_guard.rs covers
    // the lock-discipline half).
    let after_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("post-notification list_tools must complete");
    common::assert_no_rpc_error(&after_resp, "post-notification list_tools");
    let after_rows = parse_list_tools_rows(
        &after_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("post-notification list_tools returned no result")),
    );
    let after_probe_tools = tool_names_for_server(&after_rows, "probe");
    let after_probe2_tools = tool_names_for_server(&after_rows, "probe2");

    // `probe`'s inventory grew by one (the added_tool) — cache invalidated.
    assert!(
        after_probe_tools.contains(&ADDED_TOOL.to_string()),
        "post-notification `probe` inventory must include `{ADDED_TOOL}` (its \
         cache was invalidated); got: {after_probe_tools:?}"
    );

    // SC 10: `probe2`'s inventory is UNCHANGED — same count, same set (as a
    // sorted vec). The sibling was not invalidated/refetched because of
    // `probe`'s notification.
    assert_eq!(
        after_probe2_tools.len(),
        baseline_probe2_count,
        "sibling `probe2` inventory count must be unchanged after `probe`'s \
         list_changed (SC 10 — per-server scope); got {} vs baseline {}",
        after_probe2_tools.len(),
        baseline_probe2_count
    );
    assert!(
        !after_probe2_tools.contains(&ADDED_TOOL.to_string()),
        "sibling `probe2` inventory must NOT include the runtime-added tool \
         (it was not invalidated by `probe`'s notification); got: {after_probe2_tools:?}"
    );
    let mut baseline_sorted = baseline_probe2_tools.clone();
    baseline_sorted.sort();
    let mut after_sorted = after_probe2_tools.clone();
    after_sorted.sort();
    assert_eq!(
        baseline_sorted, after_sorted,
        "sibling `probe2` inventory tool set must be unchanged (SC 10 — per-server scope)"
    );

    child.into_guard().shutdown().await.ok();
}

// ---- Review-fix coverage (F5) ----------------------------------------------
//
// F5 — failed lazy refetch clears dirty → stale cache served later. The
// THOROUGH review (F5) found `ensure_fresh` does `dirty.swap(false)` BEFORE
// `list_all_tools().await`. If the refetch fails (upstream died between the
// notification and the refetch), dirty stays false; a later `inventory()` /
// `list_tools` fast-paths past the (now-clean) flag and serves the STALE
// pre-notification inventory. The list_changed signal is silently lost.

/// Test whether a process with the given PID is currently alive (cross-
/// platform, shell-out only — mirrors `error_hardening::process_is_alive`).
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
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Kill a process by PID (cross-platform, shell-out only — mirrors
/// `error_hardening::kill_process_by_pid`).
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
/// `error_hardening::wait_for_process_death`).
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

/// Parse the structured-error JSON from a CallToolResult's text content.
fn parse_error_json(result: &Value) -> Value {
    let text = result_text(result);
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("structured error text content must be valid JSON; got: {text:?}\n{e}")
    })
}

/// Review fix F5 — failed refetch retries (no stale cache). The F5 bug:
/// `ensure_fresh` does `dirty.swap(false)` BEFORE `list_all_tools().await`. If
/// the refetch fails (upstream died after the list_changed notification but
/// before the refetch), dirty stays false → a later `list_tools` fast-paths
/// past the (now-clean) flag and serves the STALE pre-notification inventory.
/// The list_changed signal is silently lost.
///
/// This test forces the post-`list_changed` refetch to fail deterministically
/// by killing the probe upstream AFTER it emits `list_changed` (so the
/// aggregator marks dirty) but BEFORE the next `list_tools` (so the refetch
/// fails). The sequence:
///   1. Baseline `list_tools` (caches the pre-mutate inventory).
///   2. `invoke_tool probe__mutate_tools` (toggles `added_tool` ON, emits
///      `notifications/tools/list_changed` → aggregator marks `probe` dirty).
///   3. Kill the probe upstream (so the next refetch will fail with a broken
///      pipe / closed transport).
///   4. `list_tools` → `ensure_fresh` sees dirty=true, swaps to false,
///      refetch fails (probe dead) → returns `upstream_disconnected` error.
///      Under the current bug: dirty is now false.
///   5. `list_tools` AGAIN → `ensure_fresh` sees dirty=false → fast-paths →
///      serves the STALE pre-mutate inventory (no `added_tool`). Under the
///      fix: dirty was restored to true on the failure, so this RETRIES the
///      refetch (and fails again with `upstream_disconnected`, NOT stale).
///
/// The assertion: the SECOND `list_tools` (step 5) returns an ERROR
/// (`upstream_disconnected`), NOT a successful stale inventory. RED against
/// the current tree (step 5 succeeds with the stale pre-mutate inventory —
/// the `added_tool` is absent AND the call succeeds, which is the F5 bug:
/// the list_changed signal was silently lost).
#[tokio::test]
async fn f5_failed_refetch_retries_does_not_serve_stale_inventory() {
    let mut child = phase1_child().await;

    // Step 1: baseline list_tools caches the probe's pre-mutate inventory.
    let baseline_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("baseline list_tools must complete (lazy spawn + discovery)");
    common::assert_no_rpc_error(&baseline_resp, "baseline list_tools");
    let baseline_result = baseline_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("baseline list_tools returned no result"));
    let baseline_rows = parse_list_tools_rows(&baseline_result);
    let baseline_probe_tools = tool_names_for_server(&baseline_rows, "probe");
    assert!(
        !baseline_probe_tools.contains(&ADDED_TOOL.to_string()),
        "baseline list_tools must NOT include the runtime-added `{ADDED_TOOL}`; \
         got: {baseline_probe_tools:?}"
    );
    let baseline_count = baseline_probe_tools.len();

    // Ask the probe for its own PID so the test can address and kill it.
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
    let pid_text = result_text(
        &pid_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("probe__self_pid returned no result")),
    );
    let probe_pid: u32 = pid_text.trim().parse().unwrap_or_else(|e| {
        panic!("probe__self_pid must return a numeric PID; got text: {pid_text:?}\n{e}")
    });
    assert!(
        process_is_alive(probe_pid),
        "probe (pid {probe_pid}) must be alive before the kill; setup failure"
    );

    // Step 2: trigger list_changed on `probe` (mutate_tools adds `added_tool`
    // and emits the notification → aggregator marks `probe` dirty).
    let mutate_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__mutate_tools",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__mutate_tools must complete");
    common::assert_no_rpc_error(&mutate_resp, "invoke_tool probe__mutate_tools");
    let mutate_text = result_text(
        &mutate_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("mutate_tools returned no result")),
    );
    assert!(
        mutate_text.contains("added"),
        "mutate_tools must report it added the tool; got: {mutate_text:?}"
    );

    // Give the aggregator a moment to process the list_changed notification
    // (the handler marks the cache dirty; the refetch is lazy on the next
    // inventory() call).
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Step 3: kill the probe upstream so the next refetch fails. This
    // deterministically forces the post-list_changed refetch to fail (the
    // aggregator observes a broken pipe / closed transport on the next
    // list_all_tools()).
    kill_process_by_pid(probe_pid);
    assert!(
        wait_for_process_death(probe_pid, Duration::from_secs(2)).await,
        "killed probe (pid {probe_pid}) must be dead before the refetch; the test killed it"
    );

    // Step 4: the first list_tools after the kill. ensure_fresh sees
    // dirty=true, swaps to false, refetch fails (probe dead) → returns the
    // structured upstream_disconnected error. Under the current bug: dirty
    // is now false (the swap already happened).
    let first_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("first post-kill list_tools must complete (or fail with structured error)");
    let first_result = first_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("first post-kill list_tools returned no result"));
    common::assert_is_error_result(&first_result, "first post-kill list_tools");
    let first_err = parse_error_json(&first_result);
    assert_eq!(
        first_err.get("code").and_then(|c| c.as_str()),
        Some("upstream_disconnected"),
        "F5 step 4: first post-kill list_tools must return upstream_disconnected \
         (the refetch failed because the probe died); got: {first_err:?}"
    );

    // Step 5: the SECOND list_tools after the failed refetch. Under the F5
    // bug: dirty is false → ensure_fresh fast-paths → serves the STALE
    // pre-mutate inventory (no `added_tool`) as a SUCCESS. Under the fix:
    // dirty was restored to true on the failure → ensure_fresh retries →
    // fails again → returns upstream_disconnected (NOT stale inventory).
    //
    // The load-bearing assertion: the second list_tools returns an ERROR
    // (upstream_disconnected), NOT a successful stale inventory. RED against
    // the current tree (the second list_tools succeeds with the stale
    // pre-mutate inventory, which has the same count as baseline and no
    // `added_tool` — the list_changed signal was silently lost).
    let second_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("second post-kill list_tools must complete");
    let second_result = second_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("second post-kill list_tools returned no result"));

    // F5: the second call must NOT serve the stale inventory. Under the fix
    // it returns a structured error (the refetch was retried and failed
    // again). Under the bug it returns a SUCCESS with the stale pre-mutate
    // inventory (no `added_tool`, count == baseline).
    let is_error = second_result
        .get("isError")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    assert!(
        is_error,
        "F5: the second list_tools after a failed refetch must NOT serve the stale \
         pre-notification inventory (the list_changed signal must not be silently lost). \
         Under the fix it retries the refetch and returns upstream_disconnected again. \
         Under the F5 bug it succeeds with the stale inventory (count == baseline {baseline_count}, \
         no `{ADDED_TOOL}`). got: {second_result:?}"
    );
    let second_err = parse_error_json(&second_result);
    assert_eq!(
        second_err.get("code").and_then(|c| c.as_str()),
        Some("upstream_disconnected"),
        "F5 step 5: the retried refetch must surface upstream_disconnected (not a \
         generic call failure, not a stale success); got: {second_err:?}"
    );

    child.into_guard().shutdown().await.ok();
}
