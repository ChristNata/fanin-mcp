//! Live discovery meta-tools — Phase 1 wire-level tests.
//!
//! Covers master Success Criteria 6 (list_tools returns the probe's tool rows
//! for the active namespace), 7 (get_tool_schema returns the probe tool's
//! input schema), plus Phase 4 sub-phase Success Criteria 1–5.
//!
//! These replace the Phase 0 "not-implemented" stub behavior for the
//! discovery meta-tools with live inventory reads from the lazy-cached
//! upstream. The probe exposes exactly ten tools (`echo_ok`, `always_error`,
//! `slow_tool`, `dangerous_noop`, `needs_sampling`, `echo_image`,
//! `needs_elicitation`, `needs_roots`); `list_tools` must return
//! one row per probe tool, and `get_tool_schema probe__echo_ok` must return
//! the probe's advertised input schema.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a discovery meta-tool call. The first call may trigger a lazy
/// spawn (probe + initialize + tools/list); subsequent calls hit the cache.
const DISCOVER_DEADLINE: Duration = Duration::from_secs(15);

/// The exact set of probe tool names (mirrors `tests/integration/probe.rs`).
/// Phase 3 extends the probe with `echo_env` and `spawn_grandchild`, bringing
/// the total to 10.
const PROBE_TOOL_NAMES: [&str; 10] = [
    "echo_ok",
    "always_error",
    "slow_tool",
    "dangerous_noop",
    "needs_sampling",
    "echo_image",
    "needs_elicitation",
    "needs_roots",
    "echo_env",
    "spawn_grandchild",
];

/// Helper: spawn the aggregator with the canonical Phase 1 config and
/// initialize. Returns the live child.
async fn phase1_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    child
}

/// Extract the text content of a list_tools result as a JSON string, then
/// parse it as a JSON array of row objects. The implementer may choose the
/// exact row shape; we assert the load-bearing fields: each row carries a
/// `tool` name (and likely a `server`), and the set of tool names matches
/// the probe's ten. See `tests.md` §Schema choices for the row shape.
fn parse_list_tools_rows(result: &Value) -> Vec<Value> {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("list_tools result missing content array"));
    assert!(
        !content.is_empty(),
        "list_tools result must carry at least one content block"
    );

    // The rows are serialized as JSON inside a text content block (the
    // meta-tool returns structured data as text, per the meta-tool contract).
    // Find the text block and parse it.
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
        panic!(
            "list_tools text content must be valid JSON (the row array); \
             got: {text:?}\n{e}"
        )
    });
    parsed
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("list_tools text content must be a JSON array; got: {parsed:?}"))
}

/// Master criterion 6 / P4.SC1: `list_tools` meta-tool returns the probe
/// server's tool rows for the active namespace. The ten probe tool names
/// must all appear in the returned rows.
#[tokio::test]
async fn list_tools_returns_probe_tool_rows() {
    let mut child = phase1_child().await;

    let resp = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools meta-tool must complete within deadline");
    common::assert_no_rpc_error(&resp, "list_tools");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools returned no result field"));
    // isError must be false or absent — this is a success path.
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "list_tools must not set isError:true on success"
        );
    }

    let rows = parse_list_tools_rows(&result);
    let tool_names: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();

    for expected in PROBE_TOOL_NAMES {
        assert!(
            tool_names.iter().any(|n| n == expected),
            "list_tools rows must include the probe tool `{expected}`; got: {tool_names:?}"
        );
    }
    assert_eq!(
        tool_names.len(),
        PROBE_TOOL_NAMES.len(),
        "list_tools must return exactly the ten probe tool rows; got {tool_names:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 6 / P4.SC2: `list_tools` with a specific configured
/// server returns only that server's rows. Passing `server: "probe"` must
/// return the same ten rows (the only configured server); the filter must
/// not drop any.
#[tokio::test]
async fn list_tools_filtered_by_server_returns_only_that_server_rows() {
    let mut child = phase1_child().await;

    let resp = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(
            &mut child,
            "list_tools",
            serde_json::json!({ "server": "probe" }),
        ),
    )
    .await
    .expect("filtered list_tools must complete");
    common::assert_no_rpc_error(&resp, "list_tools filtered by server");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("filtered list_tools returned no result"));
    let rows = parse_list_tools_rows(&result);

    // Every row must belong to the `probe` server (if the row carries a
    // `server` field). The tool set is still the ten probe tools.
    for row in &rows {
        if let Some(srv) = row.get("server").and_then(|s| s.as_str()) {
            assert_eq!(
                srv, "probe",
                "filtered list_tools row must belong to server `probe`; got `{srv}`"
            );
        }
    }
    let tool_names: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    assert_eq!(
        tool_names.len(),
        PROBE_TOOL_NAMES.len(),
        "filtered list_tools for the only configured server must still return ten rows"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 6 edge / P4.SC2: `list_tools` with an UNKNOWN server
/// filter returns a structured `isError: true` result (not a JSON-RPC error,
/// not an empty success). The unknown-server path is a tool-level failure
/// (D-005).
#[tokio::test]
async fn list_tools_filtered_by_unknown_server_returns_structured_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(
            &mut child,
            "list_tools",
            serde_json::json!({ "server": "does-not-exist" }),
        ),
    )
    .await
    .expect("list_tools with unknown server must complete");
    common::assert_no_rpc_error(&resp, "list_tools unknown server");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools unknown server returned no result"));
    common::assert_is_error_result(&result, "list_tools unknown server");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 7 / P4.SC3: `get_tool_schema` for `probe__echo_ok`
/// returns the input schema the probe advertises for `echo_ok`. The probe's
/// `echo_ok` schema is an object with an optional `message` string property.
/// We assert the schema carries the `message` property — a stub returning
/// empty content fails.
#[tokio::test]
async fn get_tool_schema_returns_probe_echo_ok_input_schema() {
    let mut child = phase1_child().await;

    let resp = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "probe__echo_ok" }),
        ),
    )
    .await
    .expect("get_tool_schema probe__echo_ok must complete");
    common::assert_no_rpc_error(&resp, "get_tool_schema probe__echo_ok");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "get_tool_schema for a known tool must not be an error"
        );
    }

    // The schema is serialized as JSON inside a text content block.
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("get_tool_schema result missing content array"));
    let text = content
        .iter()
        .find_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("get_tool_schema result must carry a text content block"));

    let schema: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("get_tool_schema text content must be valid JSON (the schema); got: {text:?}\n{e}")
    });

    // The probe's echo_ok schema is an object with a `message` string property.
    assert_eq!(
        schema.get("type").and_then(|t| t.as_str()),
        Some("object"),
        "echo_ok schema must be type=object; got: {schema:?}"
    );
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .unwrap_or_else(|| panic!("echo_ok schema must carry properties; got: {schema:?}"));
    assert!(
        props.contains_key("message"),
        "echo_ok schema must include the `message` property; got keys: {:?}",
        props.keys().collect::<Vec<_>>()
    );
    let msg = &props["message"];
    assert_eq!(
        msg.get("type").and_then(|t| t.as_str()),
        Some("string"),
        "echo_ok `message` property must be type=string; got: {msg:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 7 / P4.SC4: `get_tool_schema` for an UNKNOWN server/tool
/// returns `CallToolResult { isError: true }`, NOT a JSON-RPC error (D-005).
/// The unknown-tool path is a tool-level failure that stays in the
/// conversation.
#[tokio::test]
async fn get_tool_schema_unknown_server_returns_structured_error_not_rpc_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "no_such_server__no_such_tool" }),
        ),
    )
    .await
    .expect("get_tool_schema unknown server must complete");
    common::assert_no_rpc_error(&resp, "get_tool_schema unknown server");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema unknown server returned no result"));
    common::assert_is_error_result(&result, "get_tool_schema unknown server");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 7 edge: `get_tool_schema` for a known server but unknown
/// tool returns a structured error (not a JSON-RPC error). The server exists
/// (`probe`), the tool does not (`does_not_exist`).
#[tokio::test]
async fn get_tool_schema_known_server_unknown_tool_returns_structured_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "probe__does_not_exist" }),
        ),
    )
    .await
    .expect("get_tool_schema unknown tool must complete");
    common::assert_no_rpc_error(&resp, "get_tool_schema unknown tool");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema unknown tool returned no result"));
    common::assert_is_error_result(&result, "get_tool_schema unknown tool");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 4 / P4.SC5: downstream rmcp `tools/list` still returns
/// exactly the three static meta-tools after the Phase 1 discovery path is
/// wired. This re-asserts the Phase 0 guarantee in the Phase 1 context —
/// the live discovery meta-tools do NOT leak into the downstream rmcp
/// tools/list surface.
#[tokio::test]
async fn downstream_tools_list_still_returns_three_static_meta_tools() {
    let mut child = phase1_child().await;

    // First, exercise the live discovery path (which may spawn the upstream).
    let _ = timeout(
        DISCOVER_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await;

    // Then assert the downstream rmcp tools/list is unchanged.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list after live discovery");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    crate::common::expectations::assert_exact_meta_tools(tools);

    child.into_guard().shutdown().await.ok();
}
