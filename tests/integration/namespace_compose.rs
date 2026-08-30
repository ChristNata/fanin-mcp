#![cfg(feature = "probe-fixture")]

//! Phase C1 namespace composition contract.
//!
//! The resolver is exercised through production config loading, MCP wire
//! behavior, advertisement, and `check`. In particular, an empty tool-filter
//! intersection must remain a present empty filter: dropping that key changes
//! the existing absent-key meaning to ALL and fails the critical guard below.

use std::collections::BTreeSet;
use std::mem::discriminant;
use std::time::Duration;

use serde_json::Value;

use crate::common;
use crate::common::fixtures as fx;
use crate::error::StartupError;

const CHECK_DEADLINE: Duration = Duration::from_secs(20);
const INHERITED_FILTER_SERVER: &str = "probe";
const CACHED_ECHO_SUMMARY: &str = "Echoes the supplied input back in a successful tool result.";

fn inherited_filter_config(parent_tools: &[&str]) -> fx::ConfigFile {
    fx::MultiConfigBuilder::new()
        .server(
            fx::ServerEntry::new(INHERITED_FILTER_SERVER)
                .with_description("Inherited cache fingerprint probe"),
        )
        .namespace(
            fx::NamespaceEntry::new("parent", [INHERITED_FILTER_SERVER])
                .with_tools(INHERITED_FILTER_SERVER, parent_tools.iter().copied()),
        )
        .namespace(fx::NamespaceEntry::empty("reviewer").with_extends(["parent"]))
        .write()
}

fn list_tools_description(response: &Value) -> &str {
    response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some("list_tools"))
        })
        .and_then(|tool| tool.get("description"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("protocol tools/list must describe list_tools: {response:?}"))
}

fn result_text(result: &Value) -> &str {
    result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content.iter().find_map(|block| {
                (block.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| block.get("text").and_then(Value::as_str))
                    .flatten()
            })
        })
        .unwrap_or_else(|| panic!("CallToolResult must contain text content: {result:?}"))
}

fn list_rows(response: &Value) -> Vec<Value> {
    common::assert_no_rpc_error(response, "composed list_tools");
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("list_tools response must contain result: {response:?}"));
    assert_ne!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "list_tools must succeed for an effectively allowed server: {result:?}"
    );
    serde_json::from_str(result_text(result))
        .unwrap_or_else(|error| panic!("list_tools text must be a JSON row array: {error}"))
}

fn row_servers(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .filter_map(|row| row.get("server").and_then(Value::as_str).map(String::from))
        .collect()
}

fn row_tools(rows: &[Value], server: &str) -> Vec<String> {
    let mut names = rows
        .iter()
        .filter(|row| row.get("server").and_then(Value::as_str) == Some(server))
        .filter_map(|row| {
            row.get("tool")
                .and_then(Value::as_str)
                .or_else(|| row.get("name").and_then(Value::as_str))
                .map(String::from)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn list_all_rows(child: &mut common::JsonRpcChild) -> Vec<Value> {
    let response = common::call_tool(child, "list_tools", serde_json::json!({})).await;
    list_rows(&response)
}

async fn list_server_tools(child: &mut common::JsonRpcChild, server: &str) -> Vec<String> {
    let response =
        common::call_tool(child, "list_tools", serde_json::json!({ "server": server })).await;
    let rows = list_rows(&response);
    row_tools(&rows, server)
}

async fn assert_schema_error_code(
    child: &mut common::JsonRpcChild,
    name: &str,
    expected_code: &str,
) {
    let response = common::call_tool(
        child,
        "get_tool_schema",
        serde_json::json!({ "name": name }),
    )
    .await;
    common::assert_no_rpc_error(&response, "composed get_tool_schema");
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("get_tool_schema must contain result: {response:?}"));
    common::assert_is_error_result(result, "composed get_tool_schema");
    let body: Value = serde_json::from_str(result_text(result))
        .unwrap_or_else(|error| panic!("tool error text must be structured JSON: {error}"));
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some(expected_code),
        "unexpected ACL outcome for {name}: {body:?}"
    );
}

async fn assert_invoke_error_code(
    child: &mut common::JsonRpcChild,
    name: &str,
    expected_code: &str,
) {
    let response = common::call_tool(
        child,
        "invoke_tool",
        serde_json::json!({ "name": name, "arguments": {} }),
    )
    .await;
    common::assert_no_rpc_error(&response, "composed invoke_tool");
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("invoke_tool must contain result: {response:?}"));
    common::assert_is_error_result(result, "composed invoke_tool");
    let body: Value = serde_json::from_str(result_text(result))
        .unwrap_or_else(|error| panic!("tool error text must be structured JSON: {error}"));
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some(expected_code),
        "unexpected ACL outcome for {name}: {body:?}"
    );
}

/// Compile-time proof that validation failures stay on the typed startup-error
/// channel. The plan intentionally does not prescribe new variant names.
fn assert_typed_startup_error(error: StartupError) {
    drop(error);
}

#[tokio::test]
async fn extends_single_and_transitive_inheritance_union_servers() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("atlas"))
        .server(fx::ServerEntry::new("beacon"))
        .namespace(fx::NamespaceEntry::new("repo-base", ["beacon"]))
        .namespace(fx::NamespaceEntry::new("repo-read", ["atlas"]).with_extends(["repo-base"]))
        .namespace(fx::NamespaceEntry::empty("reviewer").with_extends(["repo-read"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("reviewer")).await;
    common::initialize(&mut child).await;

    let rows = list_all_rows(&mut child).await;
    assert_eq!(
        row_servers(&rows),
        BTreeSet::from(["atlas".to_string(), "beacon".to_string()]),
        "single and transitive extends must union every inherited server"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn extends_multiple_parents_intersect_same_server_tools() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(
            fx::NamespaceEntry::new("analyst", ["postgres"])
                .with_tools("postgres", ["query", "describe_table"]),
        )
        .namespace(
            fx::NamespaceEntry::new("auditor", ["postgres"]).with_tools("postgres", ["query"]),
        )
        .namespace(fx::NamespaceEntry::empty("reviewer").with_extends(["analyst", "auditor"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("reviewer")).await;
    common::initialize(&mut child).await;

    // `query` survives the ACL intersection and reaches live inventory, where
    // the generic probe correctly reports it as an unknown upstream tool.
    assert_schema_error_code(&mut child, "postgres__query", "unknown_tool").await;
    assert_schema_error_code(&mut child, "postgres__describe_table", "namespace_denied").await;

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn extends_unfiltered_and_restricted_parent_is_restricted() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(fx::NamespaceEntry::new("unfiltered", ["postgres"]))
        .namespace(
            fx::NamespaceEntry::new("restricted", ["postgres"]).with_tools("postgres", ["echo_ok"]),
        )
        .namespace(fx::NamespaceEntry::empty("reviewer").with_extends(["unfiltered", "restricted"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("reviewer")).await;
    common::initialize(&mut child).await;

    assert_eq!(
        list_server_tools(&mut child, "postgres").await,
        ["echo_ok"],
        "All must be intersection identity; the restricted parent wins"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn extends_disjoint_filters_are_none() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(
            fx::NamespaceEntry::new("reader", ["postgres"]).with_tools("postgres", ["echo_ok"]),
        )
        .namespace(
            fx::NamespaceEntry::new("writer", ["postgres"])
                .with_tools("postgres", ["dangerous_noop"]),
        )
        .namespace(fx::NamespaceEntry::empty("reviewer").with_extends(["reader", "writer"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("reviewer")).await;
    common::initialize(&mut child).await;

    // Critical fail-open oracle: if the empty intersection key is dropped,
    // absent means ALL and this concrete invoke succeeds instead of denying.
    assert_invoke_error_code(&mut child, "postgres__echo_ok", "namespace_denied").await;
    assert_invoke_error_code(&mut child, "postgres__dangerous_noop", "namespace_denied").await;
    assert!(
        list_server_tools(&mut child, "postgres").await.is_empty(),
        "postgres must remain server-allowed while its present-empty filter denies every live tool"
    );

    child.into_guard().shutdown().await.ok();
}

#[test]
fn extends_unknown_parent_is_typed_startup_error() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(
            fx::NamespaceEntry::new("reviewer", ["postgres"]).with_extends(["missing-parent"]),
        )
        .write();
    let error = crate::config_model::load_and_validate(&config.path, "reviewer")
        .expect_err("an unknown extends target must fail before any spawn");

    assert_typed_startup_error(error);
}

#[test]
fn extends_cycle_is_typed_startup_error() {
    let build = || {
        fx::MultiConfigBuilder::new()
            .server(fx::ServerEntry::new("postgres"))
            .namespace(fx::NamespaceEntry::empty("a").with_extends(["b"]))
            .namespace(fx::NamespaceEntry::empty("b").with_extends(["c"]))
            .namespace(fx::NamespaceEntry::empty("c").with_extends(["a"]))
            .write()
    };
    let first_config = build();
    let second_config = build();
    let first = crate::config_model::load_and_validate(&first_config.path, "a")
        .expect_err("A -> B -> C -> A must fail validation");
    let second = crate::config_model::load_and_validate(&second_config.path, "a")
        .expect_err("the same cycle must fail deterministically");

    assert_eq!(
        discriminant(&first),
        discriminant(&second),
        "the same cycle must deterministically return the same typed error variant"
    );
    assert_typed_startup_error(first);
    assert_typed_startup_error(second);
}

#[tokio::test]
async fn extends_diamond_is_not_a_cycle() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(fx::NamespaceEntry::new("d", ["postgres"]))
        .namespace(fx::NamespaceEntry::empty("b").with_extends(["d"]))
        .namespace(fx::NamespaceEntry::empty("c").with_extends(["d"]))
        .namespace(fx::NamespaceEntry::empty("a").with_extends(["b", "c"]))
        .write();
    crate::config_model::load_and_validate(&config.path, "a")
        .expect("a diamond DAG is valid and must not be classified as a cycle");
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("a")).await;
    common::initialize(&mut child).await;

    assert!(
        !list_server_tools(&mut child, "postgres").await.is_empty(),
        "the diamond leaf server must resolve through both completed branches"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn extends_unlisted_parent_is_identity() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(
            fx::NamespaceEntry::new("broad", ["postgres"])
                .with_tools("postgres", ["echo_ok", "dangerous_noop"]),
        )
        .namespace(
            fx::NamespaceEntry::new("narrow", ["postgres"]).with_tools("postgres", ["echo_ok"]),
        )
        .namespace(fx::NamespaceEntry::empty("unrelated"))
        .namespace(fx::NamespaceEntry::empty("reviewer").with_extends([
            "broad",
            "narrow",
            "unrelated",
        ]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("reviewer")).await;
    common::initialize(&mut child).await;

    assert_eq!(
        list_server_tools(&mut child, "postgres").await,
        ["echo_ok"],
        "a parent that does not list postgres is All identity, not NONE"
    );

    child.into_guard().shutdown().await.ok();
}

#[test]
fn effective_set_validation_replaces_raw_local_check() {
    let legal = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(fx::NamespaceEntry::new("parent", ["postgres"]))
        .namespace(
            fx::NamespaceEntry::empty("child")
                .with_extends(["parent"])
                .with_tools("postgres", ["query"]),
        )
        .write();
    crate::config_model::load_and_validate(&legal.path, "child")
        .expect("a child may further-restrict an inherited server without re-listing it locally");

    let bad_filter = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("postgres"))
        .namespace(fx::NamespaceEntry::new("parent", ["postgres"]))
        .namespace(
            fx::NamespaceEntry::empty("child")
                .with_extends(["parent"])
                .with_tools("ghost", ["query"]),
        )
        .write();
    let error = crate::config_model::load_and_validate(&bad_filter.path, "child")
        .expect_err("a tool-filter key outside the effective server set must fail");
    assert!(
        matches!(
            &error,
            StartupError::ToolFilterUnknownServer { namespace, server }
                if namespace == "child" && server == "ghost"
        ),
        "effective tool-filter validation must identify child/ghost; got {error:?}"
    );

    let missing_server = fx::MultiConfigBuilder::new()
        .namespace(fx::NamespaceEntry::new("parent", ["ghost"]))
        .namespace(fx::NamespaceEntry::empty("child").with_extends(["parent"]))
        .write();
    let error = crate::config_model::load_and_validate(&missing_server.path, "child")
        .expect_err("every effective server must exist in top-level [servers]");
    assert_typed_startup_error(error);
}

#[tokio::test]
async fn resolved_namespace_preserves_absent_all_and_present_empty_none() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(
            fx::NamespaceEntry::new("parent", ["alpha", "beta"])
                .with_tools("beta", std::iter::empty::<&str>()),
        )
        .namespace(fx::NamespaceEntry::empty("child").with_extends(["parent"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("child")).await;
    common::initialize(&mut child).await;

    assert!(
        !list_server_tools(&mut child, "alpha").await.is_empty(),
        "absent alpha filter must remain ALL after resolution"
    );
    assert!(
        list_server_tools(&mut child, "beta").await.is_empty(),
        "present-empty beta filter must remain NONE after resolution"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn advertisement_follows_extends() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("atlas").with_description("Inherited atlas capability"))
        .server(fx::ServerEntry::new("beacon").with_description("Local beacon capability"))
        .namespace(fx::NamespaceEntry::new("repo-read", ["atlas"]))
        .namespace(fx::NamespaceEntry::new("reviewer", ["beacon"]).with_extends(["repo-read"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), Some("reviewer")).await;
    let init = common::initialize(&mut child).await;
    let instructions = init
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("composed namespace must produce initialize instructions"));

    assert!(
        instructions.contains("atlas") && instructions.contains("beacon"),
        "advertisement must name the inherited/local effective union: {instructions:?}"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn check_follows_extends() {
    let atlas_log = fx::empty_log_file_path();
    let beacon_log = fx::empty_log_file_path();
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("atlas").with_log_file(&atlas_log))
        .server(fx::ServerEntry::new("beacon").with_log_file(&beacon_log))
        .namespace(fx::NamespaceEntry::new("repo-read", ["atlas"]))
        .namespace(fx::NamespaceEntry::new("reviewer", ["beacon"]).with_extends(["repo-read"]))
        .write();
    let args = vec![
        "--config".to_string(),
        config.path_str(),
        "--namespace".to_string(),
        "reviewer".to_string(),
        "check".to_string(),
        "--json".to_string(),
        "--no-cache-write".to_string(),
    ];
    let output = common::run_fanin_cli(&args, None, CHECK_DEADLINE).await;
    assert!(
        output.status.is_some_and(|status| status.success()),
        "check over the effective inherited union must exit zero; stderr: {}",
        output.stderr
    );
    let body: Value = serde_json::from_str(&output.stdout)
        .unwrap_or_else(|error| panic!("check --json must emit JSON: {error}"));
    let names = body
        .get("servers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("check JSON must contain servers[]: {body:?}"))
        .iter()
        .filter_map(|server| server.get("name").and_then(Value::as_str))
        .map(String::from)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from(["atlas".to_string(), "beacon".to_string()]),
        "check must inventory the effective inherited/local server union"
    );
    for (name, log_path) in [("atlas", atlas_log), ("beacon", beacon_log)] {
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(
            log.contains(&format!("[{name}]")),
            "check must actually connect and inventory effective server {name}; log: {log}"
        );
    }
}

#[tokio::test]
async fn check_rejects_missing_tool_inherited_from_parent() {
    let missing_tool = "required_but_absent";
    let config = inherited_filter_config(&[missing_tool]);
    let args = vec![
        "--config".to_string(),
        config.path_str(),
        "--namespace".to_string(),
        "reviewer".to_string(),
        "check".to_string(),
        "--json".to_string(),
        "--no-cache-write".to_string(),
    ];
    let output = common::run_fanin_cli(&args, None, CHECK_DEADLINE).await;
    assert!(
        output.status.is_some_and(|status| !status.success()),
        "RED reason: check ignores the child's inherited parent tool filter and exits zero; stdout: {:?}; stderr: {:?}",
        output.stdout,
        output.stderr
    );
    let body: Value = serde_json::from_str(&output.stdout)
        .unwrap_or_else(|error| panic!("check --json must emit one JSON body: {error}"));
    assert_eq!(
        body.get("ok").and_then(Value::as_bool),
        Some(false),
        "an inherited configured-tool failure must emit ok:false: {body:?}"
    );
    assert!(
        body.get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| errors.iter().any(|error| {
                error.get("code").and_then(Value::as_str)
                    == Some("configured_tool_missing")
                    && error.get("server").and_then(Value::as_str)
                        == Some(INHERITED_FILTER_SERVER)
                    && error.get("tool").and_then(Value::as_str) == Some(missing_tool)
            })),
        "errors[] must identify inherited {INHERITED_FILTER_SERVER}/{missing_tool} with configured_tool_missing: {body:?}"
    );
}

#[tokio::test]
async fn parent_tool_filter_change_invalidates_child_capability_cache() {
    let cache_root = tempfile::tempdir().expect("create isolated inherited-filter cache directory");
    std::env::set_var("FANIN_MCP_CACHE_DIR", cache_root.path());

    let baseline = inherited_filter_config(&["echo_ok"]);
    let check_args = vec![
        "--config".to_string(),
        baseline.path_str(),
        "--namespace".to_string(),
        "reviewer".to_string(),
        "check".to_string(),
        "--json".to_string(),
    ];
    let check = common::run_fanin_cli(&check_args, None, CHECK_DEADLINE).await;
    assert!(
        check.status.is_some_and(|status| status.success()),
        "cache precondition: full check over the inherited filter must succeed; stdout: {:?}; stderr: {:?}",
        check.stdout,
        check.stderr
    );
    let cache_path = cache_root
        .path()
        .join("fanin-mcp")
        .join("capabilities")
        .join("reviewer.json");
    assert!(
        cache_path.is_file(),
        "full check must write the child namespace cache under FANIN_MCP_CACHE_DIR: {}",
        cache_path.display()
    );

    // This is the only semantic config change: the parent's inherited filter
    // broadens from one live tool to two. The child still has no local filter.
    let changed_parent = inherited_filter_config(&["echo_ok", "dangerous_noop"]);
    let mut child =
        common::spawn_fanin_with_config(&changed_parent.path_str(), Some("reviewer")).await;
    let init = common::initialize(&mut child).await;
    let protocol_list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(
        &protocol_list,
        "parent-filter fingerprint invalidation tools/list",
    );
    let advertisement = format!(
        "{}\n{}",
        init.get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        list_tools_description(&protocol_list)
    );
    assert!(
        advertisement.contains("Inherited cache fingerprint probe"),
        "fingerprint miss must retain the current config-only server description: {advertisement:?}"
    );
    assert!(
        !advertisement.contains("echo_ok") && !advertisement.contains(CACHED_ECHO_SUMMARY),
        "RED reason: changing only the parent's inherited tool filter must invalidate the child cache; stale cached echo_ok summary was reused: {advertisement:?}"
    );

    child.into_guard().shutdown().await.ok();
}
