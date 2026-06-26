//! Namespace ACL completeness — Phase 2 wire-level tests.
//!
//! Covers the plan's Phase 2 sub-phase (namespace ACL matrix) and the
//! `namespace_denied` error shape: master Success Criteria 6, 7, 8, 9, 10,
//! plus Phase 2 sub-phase Success Criteria 1–5.
//!
//! The resolved Open Question #1 schema is the binding contract here: under a
//! namespace table, `servers = [...]` is the server allow-list, and an
//! optional `[namespaces.<name>.tools]` sub-table with `<server> = ["tool",
//! ...]` entries is the per-server name-level tool allow-list. A server
//! present in `servers` with NO tools entry exposes ALL its tools; a server
//! with a tools entry exposes EXACTLY the listed tool names. Name-level only —
//! no parameter-level ACL (D-006, GOTCHA #31).
//!
//! The probe binary is registered under distinct configured names (`alpha`,
//! `beta`) — the same fixture as multi_upstream.rs. No second fixture identity.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const ACL_DEADLINE: Duration = Duration::from_secs(15);

/// The exact set of probe tool names (mirrors discovery.rs).
const PROBE_TOOL_NAMES: [&str; 8] = [
    "echo_ok",
    "always_error",
    "slow_tool",
    "dangerous_noop",
    "needs_sampling",
    "echo_image",
    "needs_elicitation",
    "needs_roots",
];

/// Extract the text content of a list_tools result as a JSON array of rows.
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

/// Parse the structured-error JSON from a `namespace_denied` CallToolResult's
/// text content. Returns the parsed object so the caller can assert on
/// `code`, `server`, `tool`, `message`, `recoverable`.
fn parse_error_json(result: &Value) -> Value {
    let text = result_text(result);
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "namespace_denied text content must be valid JSON (the structured \
             error body); got: {text:?}\n{e}"
        )
    })
}

/// Collect the tool names from a list_tools row set (under `tool` or `name`).
#[allow(dead_code)] // helper kept for symmetric use with row_server_names.
fn row_tool_names(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect()
}

/// Collect the server names from a list_tools row set.
fn row_server_names(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| r.get("server").and_then(|s| s.as_str()).map(String::from))
        .collect()
}

/// Master SC 6 / P2.SC1: omitting `--namespace` selects `default`, and
/// `default` exposes exactly the servers declared in `[namespaces.default]`.
/// Here `default` lists only `alpha`; `beta` is configured but NOT in the
/// default namespace, so it must be hidden from `list_tools` and denied on
/// direct access.
#[tokio::test]
async fn omitted_namespace_selects_default_exposing_only_declared_servers() {
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(fx::NamespaceEntry::new("default", ["alpha"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // list_tools (no server filter) returns only alpha's rows — beta is
    // denied in the default namespace.
    let list = timeout(
        ACL_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools default namespace must complete");
    common::assert_no_rpc_error(&list, "list_tools default namespace");
    let list_result = list
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools returned no result"));
    if let Some(is_error) = list_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "list_tools in the default namespace must succeed"
        );
    }
    let rows = parse_list_tools_rows(&list_result);
    let servers = row_server_names(&rows);
    assert!(
        servers.iter().all(|s| s == "alpha"),
        "default namespace exposes only `alpha`; got servers: {servers:?}"
    );
    assert!(
        !servers.iter().any(|s| s == "beta"),
        "default namespace must NOT expose `beta` (not in [namespaces.default]); \
         got servers: {servers:?}"
    );

    // Direct access to beta is denied with the structured namespace_denied
    // error (not a JSON-RPC error).
    let denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "beta__echo_ok",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("denied beta invoke must complete (denied before lazy connect)");
    common::assert_no_rpc_error(&denied, "denied beta invoke");
    let denied_result = denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("denied beta invoke returned no result"));
    common::assert_is_error_result(&denied_result, "denied beta invoke");

    child.into_guard().shutdown().await.ok();
}

/// Master SC 7 / P2.SC2: server-level visibility matrix. A server visible in
/// namespace `open` appears in `list_tools` and is invokable; the SAME server
/// denied in namespace `restricted` is hidden from `list_tools` and returns
/// structured `namespace_denied` from BOTH `get_tool_schema` and
/// `invoke_tool`.
///
/// Two aggregator sessions are spawned against the same config, one with
/// `--namespace open` and one with `--namespace restricted`. `alpha` is in
/// both; `beta` is in `open` only.
#[tokio::test]
async fn server_visibility_matrix_across_namespaces() {
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(fx::NamespaceEntry::new("open", ["alpha", "beta"]))
        .namespace(fx::NamespaceEntry::new("restricted", ["alpha"]))
        .write();

    // --- open namespace: beta is visible and invokable ---
    let mut open = common::spawn_fanin_with_config(&cfg.path_str(), Some("open")).await;
    common::initialize(&mut open).await;

    let list = timeout(
        ACL_DEADLINE,
        common::call_tool(&mut open, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools open namespace must complete");
    common::assert_no_rpc_error(&list, "list_tools open namespace");
    let list_result = list
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools open returned no result"));
    let rows = parse_list_tools_rows(&list_result);
    let open_servers = row_server_names(&rows);
    assert!(
        open_servers.iter().any(|s| s == "beta"),
        "open namespace must expose beta in list_tools; got servers: {open_servers:?}"
    );

    // beta is invokable in open.
    let echo = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut open,
            "invoke_tool",
            serde_json::json!({
                "name": "beta__echo_ok",
                "arguments": { "message": "open-ns" },
            }),
        ),
    )
    .await
    .expect("beta__echo_ok in open namespace must complete");
    common::assert_no_rpc_error(&echo, "beta__echo_ok open namespace");
    let echo_result = echo
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__echo_ok open returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "beta__echo_ok must succeed in the open namespace"
        );
    }
    open.into_guard().shutdown().await.ok();

    // --- restricted namespace: beta is hidden and denied ---
    let mut restricted = common::spawn_fanin_with_config(&cfg.path_str(), Some("restricted")).await;
    common::initialize(&mut restricted).await;

    let list_r = timeout(
        ACL_DEADLINE,
        common::call_tool(&mut restricted, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools restricted namespace must complete");
    common::assert_no_rpc_error(&list_r, "list_tools restricted namespace");
    let list_r_result = list_r
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools restricted returned no result"));
    let rows_r = parse_list_tools_rows(&list_r_result);
    let restricted_servers = row_server_names(&rows_r);
    assert!(
        !restricted_servers.iter().any(|s| s == "beta"),
        "restricted namespace must HIDE beta from list_tools; got servers: {restricted_servers:?}"
    );
    assert!(
        restricted_servers.iter().any(|s| s == "alpha"),
        "restricted namespace must still expose alpha; got servers: {restricted_servers:?}"
    );

    // get_tool_schema for beta__echo_ok returns namespace_denied (not a
    // JSON-RPC error, not a schema).
    let schema_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut restricted,
            "get_tool_schema",
            serde_json::json!({ "name": "beta__echo_ok" }),
        ),
    )
    .await
    .expect("get_tool_schema beta in restricted must complete");
    common::assert_no_rpc_error(&schema_denied, "get_tool_schema beta restricted");
    let schema_denied_result = schema_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema beta restricted returned no result"));
    common::assert_is_error_result(&schema_denied_result, "get_tool_schema beta restricted");

    // invoke_tool for beta__echo_ok returns namespace_denied.
    let invoke_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut restricted,
            "invoke_tool",
            serde_json::json!({
                "name": "beta__echo_ok",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool beta in restricted must complete");
    common::assert_no_rpc_error(&invoke_denied, "invoke_tool beta restricted");
    let invoke_denied_result = invoke_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool beta restricted returned no result"));
    common::assert_is_error_result(&invoke_denied_result, "invoke_tool beta restricted");

    restricted.into_guard().shutdown().await.ok();
}

/// Master SC 8 / P2.SC3: tool-level matrix (per the resolved Open Question
/// #1). In a namespace where `alpha` lists only `echo_ok`, `list_tools` shows
/// `echo_ok` and HIDES `dangerous_noop`; `get_tool_schema` returns the schema
/// for `echo_ok` and `namespace_denied` for `dangerous_noop`; `invoke_tool`
/// succeeds for `echo_ok` and returns `namespace_denied` for `dangerous_noop`.
/// A server with NO tools entry (here `beta`) exposes all its tools.
#[tokio::test]
async fn tool_level_acl_filters_list_schema_and_invoke() {
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(
            fx::NamespaceEntry::new("filtered", ["alpha", "beta"]).with_tools("alpha", ["echo_ok"]),
        )
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), Some("filtered")).await;
    common::initialize(&mut child).await;

    // list_tools: alpha rows contain ONLY echo_ok; beta rows contain ALL eight
    // probe tools (no tools entry => all visible).
    let list = timeout(
        ACL_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools filtered namespace must complete");
    common::assert_no_rpc_error(&list, "list_tools filtered namespace");
    let list_result = list
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools filtered returned no result"));
    let rows = parse_list_tools_rows(&list_result);

    let alpha_tools: Vec<String> = rows
        .iter()
        .filter(|r| r.get("server").and_then(|s| s.as_str()) == Some("alpha"))
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    assert_eq!(
        alpha_tools,
        vec!["echo_ok".to_string()],
        "alpha tool filter must expose ONLY echo_ok in list_tools; got: {alpha_tools:?}"
    );

    let beta_tools: Vec<String> = rows
        .iter()
        .filter(|r| r.get("server").and_then(|s| s.as_str()) == Some("beta"))
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    let mut beta_expected = PROBE_TOOL_NAMES.to_vec();
    beta_expected.sort();
    let mut beta_sorted = beta_tools.clone();
    beta_sorted.sort();
    assert_eq!(
        beta_sorted, beta_expected,
        "beta has no tools entry => ALL its tools visible; got: {beta_tools:?}"
    );

    // get_tool_schema: echo_ok returns a schema; dangerous_noop returns
    // namespace_denied.
    let schema_ok = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "alpha__echo_ok" }),
        ),
    )
    .await
    .expect("get_tool_schema alpha__echo_ok must complete");
    common::assert_no_rpc_error(&schema_ok, "get_tool_schema alpha__echo_ok");
    let schema_ok_result = schema_ok
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema alpha__echo_ok returned no result"));
    if let Some(is_error) = schema_ok_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "get_tool_schema for an allowed tool must return the schema, not an error"
        );
    }

    let schema_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "alpha__dangerous_noop" }),
        ),
    )
    .await
    .expect("get_tool_schema alpha__dangerous_noop must complete");
    common::assert_no_rpc_error(&schema_denied, "get_tool_schema alpha__dangerous_noop");
    let schema_denied_result = schema_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema alpha__dangerous_noop returned no result"));
    common::assert_is_error_result(
        &schema_denied_result,
        "get_tool_schema alpha__dangerous_noop",
    );

    // invoke_tool: echo_ok succeeds; dangerous_noop returns namespace_denied.
    let invoke_ok = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "alpha__echo_ok",
                "arguments": { "message": "tool-acl-allowed" },
            }),
        ),
    )
    .await
    .expect("invoke_tool alpha__echo_ok must complete");
    common::assert_no_rpc_error(&invoke_ok, "invoke_tool alpha__echo_ok");
    let invoke_ok_result = invoke_ok
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool alpha__echo_ok returned no result"));
    if let Some(is_error) = invoke_ok_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "alpha__echo_ok must succeed (it is in the tool allow-list)"
        );
    }
    let ok_text = result_text(&invoke_ok_result);
    assert!(
        ok_text.contains("tool-acl-allowed"),
        "alpha__echo_ok must echo the payload byte-faithfully; got: {ok_text:?}"
    );

    let invoke_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "alpha__dangerous_noop",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool alpha__dangerous_noop must complete");
    common::assert_no_rpc_error(&invoke_denied, "invoke_tool alpha__dangerous_noop");
    let invoke_denied_result = invoke_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool alpha__dangerous_noop returned no result"));
    common::assert_is_error_result(&invoke_denied_result, "invoke_tool alpha__dangerous_noop");

    child.into_guard().shutdown().await.ok();
}

/// Master SC 9 / P2.SC4 + error shape: `namespace_denied` is a tool-level
/// `CallToolResult { isError: true }` whose JSON text carries
/// `code: "namespace_denied"`, the server name, the denied tool when
/// applicable, a `message`, and `recoverable` — never a JSON-RPC error.
///
/// Two denial paths are asserted: a denied SERVER (no tool in the name) and a
/// denied TOOL (server allowed, tool denied by the tool filter). The error
/// JSON shape must be present in both, with the `tool` field set only for the
/// tool-denial path.
#[tokio::test]
async fn namespace_denied_error_shape_for_denied_server_and_tool() {
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(fx::NamespaceEntry::new("shaped", ["alpha"]).with_tools("alpha", ["echo_ok"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), Some("shaped")).await;
    common::initialize(&mut child).await;

    // --- denied SERVER path: beta is not in the `shaped` namespace. ---
    let server_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "beta__echo_ok",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("denied server invoke must complete");
    common::assert_no_rpc_error(&server_denied, "denied server invoke");
    let server_denied_result = server_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("denied server invoke returned no result"));
    common::assert_is_error_result(&server_denied_result, "denied server invoke");
    let server_err = parse_error_json(&server_denied_result);
    assert_eq!(
        server_err.get("code").and_then(|c| c.as_str()),
        Some("namespace_denied"),
        "denied server error must carry code `namespace_denied`; got: {server_err:?}"
    );
    assert_eq!(
        server_err.get("server").and_then(|s| s.as_str()),
        Some("beta"),
        "denied server error must carry the denied server name `beta`; got: {server_err:?}"
    );
    // Server-level denial has no tool in the denied name; the `tool` field may
    // be null or absent. The load-bearing assertion is that code/server are
    // present and isError is true (asserted above).
    assert!(
        server_err.get("message").and_then(|m| m.as_str()).is_some(),
        "denied server error must carry a message; got: {server_err:?}"
    );
    assert!(
        server_err.get("recoverable").is_some(),
        "denied server error must carry a recoverable field; got: {server_err:?}"
    );

    // --- denied TOOL path: alpha is allowed, dangerous_noop is not in the
    // tool allow-list. ---
    let tool_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "alpha__dangerous_noop",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("denied tool invoke must complete");
    common::assert_no_rpc_error(&tool_denied, "denied tool invoke");
    let tool_denied_result = tool_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("denied tool invoke returned no result"));
    common::assert_is_error_result(&tool_denied_result, "denied tool invoke");
    let tool_err = parse_error_json(&tool_denied_result);
    assert_eq!(
        tool_err.get("code").and_then(|c| c.as_str()),
        Some("namespace_denied"),
        "denied tool error must carry code `namespace_denied`; got: {tool_err:?}"
    );
    assert_eq!(
        tool_err.get("server").and_then(|s| s.as_str()),
        Some("alpha"),
        "denied tool error must carry the server name `alpha`; got: {tool_err:?}"
    );
    assert_eq!(
        tool_err.get("tool").and_then(|t| t.as_str()),
        Some("dangerous_noop"),
        "denied tool error must carry the denied tool name `dangerous_noop`; got: {tool_err:?}"
    );
    assert!(
        tool_err.get("message").and_then(|m| m.as_str()).is_some(),
        "denied tool error must carry a message; got: {tool_err:?}"
    );
    assert!(
        tool_err.get("recoverable").is_some(),
        "denied tool error must carry a recoverable field; got: {tool_err:?}"
    );

    // The same shape must hold for get_tool_schema on a denied tool.
    let schema_denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "alpha__dangerous_noop" }),
        ),
    )
    .await
    .expect("get_tool_schema denied tool must complete");
    common::assert_no_rpc_error(&schema_denied, "get_tool_schema denied tool");
    let schema_denied_result = schema_denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema denied tool returned no result"));
    common::assert_is_error_result(&schema_denied_result, "get_tool_schema denied tool");
    let schema_err = parse_error_json(&schema_denied_result);
    assert_eq!(
        schema_err.get("code").and_then(|c| c.as_str()),
        Some("namespace_denied"),
        "get_tool_schema denied tool must carry code `namespace_denied`; got: {schema_err:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 10 / P2.SC4: a denied server is NOT spawned merely to reject the
/// call — denial precedes lazy connection. The observable: a denied call
/// touches no upstream process/log. We point the aggregator at a log file and
/// assert that after a denied `beta__echo_ok` call (beta is not in the
/// namespace), the log contains NO beta line. A stub that connects-then-denies
/// would spawn beta and leave a log line — failing the assertion.
#[tokio::test]
async fn denied_server_is_not_spawned_to_reject_call() {
    let log_path = fx::empty_log_file_path();
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha").with_log_file(&log_path))
        .server(fx::ServerEntry::new("beta").with_log_file(&log_path))
        .namespace(fx::NamespaceEntry::new("default", ["alpha"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Issue a denied call to beta. The namespace check must happen BEFORE any
    // lazy connection attempt, so beta is never spawned.
    let denied = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "beta__echo_ok",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("denied beta invoke must complete (denied before lazy connect)");
    common::assert_no_rpc_error(&denied, "denied beta invoke");
    let denied_result = denied
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("denied beta invoke returned no result"));
    common::assert_is_error_result(&denied_result, "denied beta invoke");

    // Give any would-be spawn a moment to flush stderr to the log file.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        !log.contains("beta"),
        "denied server beta must NOT be spawned to reject the call (criterion 10 — \
         denial precedes lazy connection); log contains a beta line:\n{log}"
    );

    // Sanity: alpha IS allowed, so a call to alpha DOES spawn alpha and leave
    // a log line. This proves the log-capture path works and the absence of a
    // beta line is meaningful (not a broken log sink).
    let _allowed = timeout(
        ACL_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "alpha__echo_ok",
                "arguments": { "message": "allowed-spawn-check" },
            }),
        ),
    )
    .await
    .expect("alpha__echo_ok must complete");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log_after_alpha = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log_after_alpha.contains("alpha"),
        "alpha (allowed) must spawn and leave a log line — proves the log sink works; \
         log:\n{log_after_alpha}"
    );
    // beta STILL must not be present (the denied call earlier did not spawn it).
    assert!(
        !log_after_alpha.contains("beta"),
        "beta must remain unspawned even after alpha was spawned; log:\n{log_after_alpha}"
    );

    child.into_guard().shutdown().await.ok();
}
