//! Phase 3 — regression guard for Phase 0/1/2 guarantees (wire-level).
//!
//! Covers master Success Criterion 24: the public downstream MCP surface
//! remains exactly three meta-tools (`list_tools`, `get_tool_schema`,
//! `invoke_tool`), and the Phase 0/1/2 guarantees (lazy startup, namespace
//! filtering, byte-faithful forwarding, reverse-traffic rejection, stdout
//! discipline) remain true under a Phase 3 config (with `timeout_secs` and
//! env vars present).
//!
//! This is the Phase 3 analogue of the Phase 2 regression guard. It
//! re-asserts the static meta-tool surface and a byte-faithful forward
//! under a config that carries Phase 3 fields, proving the new fields do
//! not destabilize the existing surface. Phase 0/1/2 tests are read-only
//! and unchanged — this test is ADDITIVE, not a replacement.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::expectations as exp;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

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

/// Master SC 24: under a Phase 3 config (with `timeout_secs` and env vars),
/// the downstream MCP surface remains exactly three meta-tools, a
/// byte-faithful forward still works, and a reverse-traffic exchange still
/// completes (not hung). Stdout discipline is implicitly asserted by every
/// wire test (the harness panics on a non-JSON stdout line).
#[tokio::test]
async fn phase3_config_preserves_phase012_guarantees() {
    let alpha = format!("alpha-{}", fx::phase3_unique_seq());
    let beta = format!("beta-{}", fx::phase3_unique_seq());

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(alpha.clone())
                .with_timeout_secs(30)
                .env("ALPHA_LIT", "alpha-literal"),
        )
        .server(fx::Phase3ServerEntry::new(beta.clone()))
        .namespace(fx::NamespaceEntry::new(
            "default",
            [alpha.as_str(), beta.as_str()],
        ))
        .write();

    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Static 3 meta-tools under a Phase 3 config.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list under Phase 3 config");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);

    // Byte-faithful forward under a Phase 3 config: invoke beta__echo_ok and
    // assert the payload round-trips unchanged (D-004).
    let payload = "phase3-regression-byte-faithful-4f1";
    let echo = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{beta}__echo_ok"),
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("beta__echo_ok must complete (Phase 3 regression)");
    common::assert_no_rpc_error(&echo, "beta__echo_ok Phase 3 regression");
    let echo_result = echo
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__echo_ok returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "beta__echo_ok must succeed (byte-faithful forward under Phase 3 config)"
        );
    }
    let echo_text = result_text(&echo_result);
    assert!(
        echo_text.contains(payload),
        "beta__echo_ok must round-trip the payload byte-faithfully (D-004); got: {echo_text:?}"
    );

    // Reverse-traffic rejection under a Phase 3 config: needs_sampling on
    // alpha completes within the deadline (the aggregator rejects the
    // sampling request, not a hang — GOTCHA #2).
    let rev = timeout(
        Duration::from_secs(10),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{alpha}__needs_sampling"),
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("alpha__needs_sampling must complete (reverse traffic handled, not hung)");
    common::assert_no_rpc_error(&rev, "alpha__needs_sampling Phase 3 regression");
    let rev_result = rev
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("alpha__needs_sampling returned no result"));
    if let Some(is_error) = rev_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "alpha__needs_sampling must forward the probe's success result"
        );
    }

    // Final static tools/list — still exactly the three meta-tools after the
    // full Phase 3 exercise.
    let final_list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&final_list, "final tools/list under Phase 3 config");
    let final_tools = final_list
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(final_tools);

    child.into_guard().shutdown().await.ok();
}
