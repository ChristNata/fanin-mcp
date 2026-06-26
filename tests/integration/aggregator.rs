//! Aggregator stdio server — Phase 0 wire-level tests.
//!
//! Covers master Success Criteria 2 (static discovery), 3 (annotation gate),
//! 4 (startup laziness), and 5 (stub call gate), plus P0.2 Phase Success
//! Criteria. Each test spawns the built `fanin-mcp` binary and speaks raw
//! JSON-RPC over stdio, asserting on the wire JSON — decoupling the contract
//! from rmcp's fast-moving Rust API (D-015).
//!
//! Out of scope (master.md §Out): real upstream proxying, reverse-traffic
//! handling, config parsing, credential logic. These tests assert only what
//! Phase 0 ships.

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::expectations as exp;

/// Criterion 2 (Static discovery gate): initialize succeeds and tools/list
/// returns EXACTLY the three meta-tools named list_tools, get_tool_schema,
/// invoke_tool with the exact static descriptions from the Required Pattern
/// table (master.md §Required Pattern, D-003).
#[tokio::test]
async fn static_discovery_returns_three_meta_tools_with_exact_descriptions() {
    let mut child = common::spawn_bin("fanin-mcp").await;
    let init = common::initialize(&mut child).await;
    assert!(
        init.get("serverInfo").is_some(),
        "initialize result must carry serverInfo"
    );

    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("tools/list returned no result field"));
    let tools = result
        .get("tools")
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));

    exp::assert_exact_meta_tools(tools);

    let list = exp::find_tool(tools, "list_tools").unwrap();
    exp::assert_desc(list, exp::LIST_TOOLS_DESC);

    let schema = exp::find_tool(tools, "get_tool_schema").unwrap();
    exp::assert_desc(schema, exp::GET_TOOL_SCHEMA_DESC);

    let invoke = exp::find_tool(tools, "invoke_tool").unwrap();
    exp::assert_desc(invoke, exp::INVOKE_TOOL_DESC);

    // No upstream-derived content — tools/list is fully static in Phase 0.
    // Any tool beyond the three meta-tools is scope creep (D-002).
    assert_eq!(tools.len(), 3, "no extra tools may appear in Phase 0");

    child.into_guard().shutdown().await.ok();
}

/// Criterion 3 (Annotation gate): the invoke_tool definition includes
/// destructiveHint=true, readOnlyHint=false, openWorldHint=true (D-006).
#[tokio::test]
async fn invoke_tool_carries_conservative_annotations() {
    let mut child = common::spawn_bin("fanin-mcp").await;
    common::initialize(&mut child).await;

    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));

    let invoke = exp::find_tool(tools, "invoke_tool")
        .unwrap_or_else(|| panic!("invoke_tool meta-tool missing"));
    let annotations = invoke
        .get("annotations")
        .unwrap_or_else(|| panic!("invoke_tool must carry annotations"));

    let destructive = annotations
        .get("destructiveHint")
        .unwrap_or_else(|| panic!("invoke_tool missing destructiveHint"));
    assert_eq!(
        destructive.as_bool(),
        Some(true),
        "destructiveHint must be true (D-006)"
    );

    let readonly = annotations
        .get("readOnlyHint")
        .unwrap_or_else(|| panic!("invoke_tool missing readOnlyHint"));
    assert_eq!(
        readonly.as_bool(),
        Some(false),
        "readOnlyHint must be false (D-006)"
    );

    let open_world = annotations
        .get("openWorldHint")
        .unwrap_or_else(|| panic!("invoke_tool missing openWorldHint"));
    assert_eq!(
        open_world.as_bool(),
        Some(true),
        "openWorldHint must be true (D-006)"
    );

    child.into_guard().shutdown().await.ok();
}

/// Criterion 4 (Startup laziness gate): initialize returns in under 500ms and
/// opens zero upstream connections. Zero-upstream is observable because
/// Phase 0 ships no upstream config, registry, or child processes (master.md
/// §Out: "No real upstream connections"). A 1s ceiling is used for the spawn +
/// initialize wall-clock so a slow CI runner does not flake; the 500ms
/// assertion is on the initialize round-trip only, per the plan's budget.
#[tokio::test]
async fn initialize_returns_under_500ms_and_no_upstream_connections() {
    let mut child = common::spawn_bin("fanin-mcp").await;

    let started = Instant::now();
    let init = timeout(
        Duration::from_secs(1),
        common::initialize(&mut child),
    )
    .await
    .expect("initialize did not return within 1s ceiling")
    .get("serverInfo")
    .cloned()
    .unwrap_or_else(|| panic!("initialize result must carry serverInfo"));
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "initialize must return in under 500ms (master criterion 4), took {elapsed:?}"
    );

    // The server must name itself fanin-mcp (D-017, master.md §P0.2 Key
    // Behaviors: get_info advertises name/version).
    let name = init
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or_else(|| panic!("serverInfo.name must be a string: {init:?}"));
    assert_eq!(
        name, "fanin-mcp",
        "server must advertise its name as fanin-mcp"
    );

    // No upstream connections are observable in Phase 0: there is no config,
    // no registry wiring, and no spawned children. tools/list must succeed
    // immediately and return only the three static meta-tools — proving no
    // upstream fan-out happened on the discovery path (D-003, GOTCHA #7).
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);

    child.into_guard().shutdown().await.ok();
}

/// Criterion 5 (Stub call gate): calling each meta-tool returns a structured
/// not-implemented CallToolResult with isError:true; the process does NOT
/// panic and does NOT hang (bounded by RPC_DEADLINE). Phase 0 call_tool must
/// never proxy a request (master.md §Out) — every call returns the structured
/// not-implemented result (D-005: errors stay in the conversation, never
/// JSON-RPC errors).
#[tokio::test]
async fn calling_each_meta_tool_returns_structured_not_implemented() {
    let mut child = common::spawn_bin("fanin-mcp").await;
    common::initialize(&mut child).await;

    // Each meta-tool called with representative arguments. A stub that
    // panics kills the child (next read fails/hangs); a stub that hangs is
    // caught by RPC_DEADLINE inside request().
    let cases: [(&str, Value); 3] = [
        ("list_tools", serde_json::json!({})),
        ("get_tool_schema", serde_json::json!({ "name": "postgres__query" })),
        (
            "invoke_tool",
            serde_json::json!({ "name": "postgres__query", "arguments": {} }),
        ),
    ];

    for (name, args) in cases {
        let resp = common::call_tool(&mut child, name, args.clone()).await;
        common::assert_no_rpc_error(
            &resp,
            &format!("tools/call {name} must not return a JSON-RPC error (D-005)"),
        );
        let result = resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("tools/call {name} returned no result field"));
        common::assert_is_error_result(&result, &format!("tools/call {name}"));

        // The not-implemented content must be readable structured JSON so the
        // LLM can reason about it (D-005). Assert at least one text block
        // exists; the exact wording is not part of the public contract but
        // the shape is.
        let content = result.get("content").and_then(|c| c.as_array()).unwrap();
        assert!(
            content.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")),
            "tools/call {name} not-implemented result must carry a text content block"
        );
    }

    child.into_guard().shutdown().await.ok();
}

/// Criterion 5 edge: calling an unknown tool name returns a structured
/// not-implemented/error CallToolResult, not a JSON-RPC error and not a hang.
/// Phase 0 does not differentiate unknown-tool from not-implemented; both are
/// tool-level results.
#[tokio::test]
async fn calling_unknown_tool_returns_structured_result_not_rpc_error() {
    let mut child = common::spawn_bin("fanin-mcp").await;
    common::initialize(&mut child).await;

    let resp = common::call_tool(
        &mut child,
        "does_not_exist",
        serde_json::json!({}),
    )
    .await;
    // The key assertion: tool-level failures stay in the conversation, never
    // surface as JSON-RPC errors (D-005, GOTCHA #3).
    common::assert_no_rpc_error(&resp, "unknown tool call");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("unknown tool call returned no result field"));
    common::assert_is_error_result(&result, "unknown tool call");

    child.into_guard().shutdown().await.ok();
}