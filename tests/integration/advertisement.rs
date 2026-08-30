//! Phase A2 capability advertisement — wire-level contract.
//!
//! `initialize.instructions` and the config-aware `list_tools` description
//! advertise only the active namespace. Both paths are config-only: neither
//! initialize nor protocol `tools/list` may spawn an upstream. The distinct
//! `list_tools` meta-tool call remains the lazy inventory boundary (N1).

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::common;
use crate::common::expectations as exp;
use crate::common::fixtures as fx;

const ALLOWED: [(&str, &str); 2] = [
    ("atlas", "A2 allowed atlas capability description"),
    ("beacon", "A2 allowed beacon capability description"),
];
const DENIED: [(&str, &str); 2] = [
    ("cinder", "A2 denied cinder capability description"),
    ("drift", "A2 denied drift capability description"),
];

struct ServerFixture {
    name: &'static str,
    log_path: String,
}

fn advertisement_config() -> (fx::ConfigFile, Vec<ServerFixture>) {
    let mut servers = Vec::new();
    let mut builder = fx::MultiConfigBuilder::new();
    for (name, description) in ALLOWED.into_iter().chain(DENIED) {
        let log_path = fx::empty_log_file_path();
        builder = builder.server(
            fx::ServerEntry::new(name)
                .with_description(description)
                .with_log_file(&log_path),
        );
        servers.push(ServerFixture { name, log_path });
    }
    builder = builder.namespace(fx::NamespaceEntry::new(
        "default",
        ALLOWED.map(|(name, _)| name),
    ));
    (builder.write(), servers)
}

fn response_tools(response: &Value) -> &[Value] {
    response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array: {response:?}"))
}

fn tool_description<'a>(tools: &'a [Value], name: &str) -> &'a str {
    exp::find_tool(tools, name)
        .unwrap_or_else(|| panic!("missing {name} meta-tool"))
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} description must be a string"))
}

fn list_tools_inventory_rows(response: &Value) -> Vec<Value> {
    common::assert_no_rpc_error(response, "list_tools meta-tool");
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("list_tools meta-tool returned no result: {response:?}"));
    assert_ne!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "list_tools meta-tool must return live inventory, not an error: {result:?}"
    );
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content.iter().find_map(|block| {
                (block.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| block.get("text").and_then(Value::as_str))
                    .flatten()
            })
        })
        .unwrap_or_else(|| panic!("list_tools meta-tool must return text content: {result:?}"));
    serde_json::from_str::<Vec<Value>>(text)
        .unwrap_or_else(|error| panic!("list_tools inventory must be a JSON row array: {error}"))
}

async fn wait_for_child_log_line(server: &ServerFixture) -> String {
    let marker = format!("[{}]", server.name);
    let started = Instant::now();
    loop {
        let log = std::fs::read_to_string(&server.log_path).unwrap_or_default();
        if log.contains(&marker) || started.elapsed() >= Duration::from_secs(3) {
            return log;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A2.1 / CA-001 and A2.3 / CA-004: initialize advertises exactly the two
/// namespace-visible server identities and their configured descriptions.
#[tokio::test]
async fn initialize_advertises_only_allowed_servers_with_config_descriptions() {
    let (cfg, _) = advertisement_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    let init = common::initialize(&mut child).await;
    let instructions = init
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!("A2 initialize result must contain non-empty config-derived instructions")
        });

    for (name, description) in ALLOWED {
        assert!(
            instructions.contains(name),
            "instructions must name allowed server {name}: {instructions:?}"
        );
        assert!(
            instructions.contains(description),
            "instructions must use {name}'s configured description when no cache exists: {instructions:?}"
        );
    }
    for (name, description) in DENIED {
        assert!(
            !instructions.contains(name) && !instructions.contains(description),
            "instructions must not leak denied server {name}: {instructions:?}"
        );
    }

    child.into_guard().shutdown().await.ok();
}

/// A2.2 / CA-002 and A2.6 / N1: initialize plus protocol `tools/list` writes
/// no child-log prefix. The `list_tools` meta-tool then returns real inventory
/// and creates allowed-server child-log lines in the same session.
#[tokio::test]
async fn initialize_and_protocol_tools_list_do_not_spawn_but_meta_list_tools_does() {
    let (cfg, servers) = advertisement_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;

    common::initialize(&mut child).await;
    let protocol_list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&protocol_list, "protocol tools/list");
    exp::assert_exact_meta_tools(response_tools(&protocol_list));

    tokio::time::sleep(Duration::from_millis(300)).await;
    // These files are unique to this test, so their child-line absence is the
    // authoritative no-spawn oracle. A negative PPID query is unsound here:
    // macOS can reuse this fanin PID while a prior probe is still being reaped.
    for server in &servers {
        let log = std::fs::read_to_string(&server.log_path).unwrap_or_default();
        assert!(
            !log.contains(&format!("[{}]", server.name)),
            "initialize + protocol tools/list must write no [{}] child lines; log: {log}",
            server.name
        );
    }

    let meta_list = common::call_tool(&mut child, "list_tools", serde_json::json!({})).await;
    let rows = list_tools_inventory_rows(&meta_list);
    assert!(
        !rows.is_empty(),
        "list_tools meta-tool must reach inventory and return upstream rows"
    );

    for server in servers.iter().filter(|server| {
        ALLOWED
            .iter()
            .any(|(allowed_name, _)| *allowed_name == server.name)
    }) {
        let log = wait_for_child_log_line(server).await;
        assert!(
            log.contains(&format!("[{}]", server.name)),
            "list_tools meta-tool must inventory {} and produce a child log line; log: {log}",
            server.name
        );
    }

    child.into_guard().shutdown().await.ok();
}

/// A2.4 / CA-006 and A2.7 / Critical #2: protocol `tools/list` remains exactly
/// three meta-tools. Only config-aware `list_tools` gains a non-empty ToC
/// suffix; schema/invoke descriptions remain exact.
#[tokio::test]
async fn configured_protocol_tools_list_keeps_three_meta_tools_and_only_list_tools_gains_toc_suffix(
) {
    let (cfg, _) = advertisement_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let response = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&response, "config-aware protocol tools/list");
    let tools = response_tools(&response);
    exp::assert_exact_meta_tools(tools);

    let list_tool = exp::find_tool(tools, "list_tools").unwrap();
    exp::assert_desc_prefix(list_tool, exp::LIST_TOOLS_DESC);
    let list_description = tool_description(tools, "list_tools");
    let suffix = list_description
        .strip_prefix(exp::LIST_TOOLS_DESC)
        .expect("assert_desc_prefix already proved the stable prefix");
    assert!(
        !suffix.trim().is_empty(),
        "config-aware list_tools description must append a ToC suffix"
    );
    for (name, description) in ALLOWED {
        assert!(
            suffix.contains(name) && suffix.contains(description),
            "list_tools ToC suffix must contain allowed server {name} and its description: {suffix:?}"
        );
    }
    for (name, description) in DENIED {
        assert!(
            !suffix.contains(name) && !suffix.contains(description),
            "list_tools ToC suffix must not leak denied server {name}: {suffix:?}"
        );
    }

    exp::assert_desc(
        exp::find_tool(tools, "get_tool_schema").unwrap(),
        exp::GET_TOOL_SCHEMA_DESC,
    );
    exp::assert_desc(
        exp::find_tool(tools, "invoke_tool").unwrap(),
        exp::INVOKE_TOOL_DESC,
    );

    child.into_guard().shutdown().await.ok();
}

/// A2.5 edge / Phase 0 compatibility: no config omits instructions and keeps
/// the original list_tools description as an exact string, not just a prefix.
#[tokio::test]
async fn no_config_initialize_omits_instructions_and_list_tools_description_stays_exact() {
    let mut child = common::spawn_bin("fanin-mcp").await;
    let init = common::initialize(&mut child).await;
    assert!(
        matches!(init.get("instructions"), None | Some(Value::Null)),
        "no-config initialize.instructions must be absent or null: {init:?}"
    );

    let response = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&response, "no-config protocol tools/list");
    let tools = response_tools(&response);
    exp::assert_exact_meta_tools(tools);
    exp::assert_desc(
        exp::find_tool(tools, "list_tools").unwrap(),
        exp::LIST_TOOLS_DESC,
    );

    child.into_guard().shutdown().await.ok();
}
