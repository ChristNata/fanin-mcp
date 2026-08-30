#![cfg(feature = "probe-fixture")]

//! Phase B2 reconstructible capability-cache contract.
//!
//! Every spawned child receives a unique `FANIN_MCP_CACHE_DIR` through its own
//! `Command` environment. The parent test process is never mutated, so both
//! thread-per-test `cargo test` and process-per-test nextest remain race-free.
//! All cache reads and writes use the required
//! `<override>/fanin-mcp/capabilities/<namespace>.json` path; the real user
//! cache is never touched.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::common;
use crate::common::fixtures as fx;

// Outer subprocess patience aligned with the check contract under full load.
const CHECK_DEADLINE: Duration = Duration::from_secs(30);
const NAMESPACE: &str = "reviewer";
const SERVER: &str = "probe";
const ALLOWED_TOOL: &str = "echo_ok";
const DENIED_TOOL: &str = "dangerous_noop";
const ALLOWED_SUMMARY: &str = "Echoes the supplied input back in a successful tool result.";
const CACHE_DIR_ENV: &str = "FANIN_MCP_CACHE_DIR";

struct IsolatedCache {
    _temp: TempDir,
    root: PathBuf,
}

impl IsolatedCache {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create isolated B2 cache directory");
        let root = temp.path().to_path_buf();
        Self { _temp: temp, root }
    }

    fn child_env(&self) -> (&'static str, &OsStr) {
        (CACHE_DIR_ENV, self.root.as_os_str())
    }

    fn path(&self) -> PathBuf {
        self.root
            .join("fanin-mcp")
            .join("capabilities")
            .join(format!("{NAMESPACE}.json"))
    }

    fn assert_expected_path(&self, path: &Path) {
        assert!(
            path.starts_with(&self.root),
            "cache path must remain under the isolated override: {}",
            path.display()
        );
        assert_eq!(
            path,
            self.path(),
            "B2 cache must use <override>/fanin-mcp/capabilities/<namespace>.json"
        );
    }
}

fn toml_literal(value: &str) -> String {
    value.replace('\0', "").replace('\'', "\\'")
}

fn toml_basic(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn stdio_config(
    command: &str,
    args: &[&str],
    description: &str,
    allowed_tools: &[&str],
    log_file: Option<&str>,
    env: &[(&str, &str)],
) -> fx::ConfigFile {
    let args = args
        .iter()
        .map(|arg| format!("'{}'", toml_literal(arg)))
        .collect::<Vec<_>>()
        .join(", ");
    let tools = allowed_tools
        .iter()
        .map(|tool| format!("\"{}\"", toml_basic(tool)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut toml = format!(
        "[servers.{SERVER}]\ntransport = \"stdio\"\ncommand = '{}'\nargs = [{args}]\ndescription = \"{}\"\n",
        toml_literal(command),
        toml_basic(description),
    );
    if let Some(log_file) = log_file {
        toml.push_str(&format!("log_file = '{}'\n", toml_literal(log_file)));
    }
    if !env.is_empty() {
        toml.push_str(&format!("[servers.{SERVER}.env]\n"));
        for (key, value) in env {
            toml.push_str(&format!("{key} = \"{}\"\n", toml_basic(value)));
        }
    }
    toml.push_str(&format!(
        "\n[namespaces.{NAMESPACE}]\nservers = [\"{SERVER}\"]\n[namespaces.{NAMESPACE}.tools]\n{SERVER} = [{tools}]\n"
    ));
    fx::raw_config_file(&toml)
}

fn http_config(
    endpoint: &str,
    description: &str,
    header_value: Option<&str>,
    env_value: Option<&str>,
) -> fx::ConfigFile {
    let mut toml = format!(
        "[servers.{SERVER}]\ntransport = \"streamable-http\"\nendpoint = \"{}\"\ndescription = \"{}\"\n",
        toml_basic(endpoint),
        toml_basic(description),
    );
    if let Some(value) = env_value {
        toml.push_str(&format!(
            "[servers.{SERVER}.env]\nVISIBLE_ENV = \"{}\"\n",
            toml_basic(value)
        ));
    }
    if let Some(value) = header_value {
        toml.push_str(&format!(
            "[servers.{SERVER}.headers]\nAuthorization = \"{}\"\n",
            toml_basic(value)
        ));
    }
    toml.push_str(&format!(
        "\n[namespaces.{NAMESPACE}]\nservers = [\"{SERVER}\"]\n[namespaces.{NAMESPACE}.tools]\n{SERVER} = [\"{ALLOWED_TOOL}\"]\n"
    ));
    fx::raw_config_file(&toml)
}

async fn start_http_probe() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind B2 loopback HTTP probe");
    let addr = listener.local_addr().expect("read B2 HTTP probe address");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16_384];
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]);
                let Some((_, body)) = request.split_once("\r\n\r\n") else {
                    return;
                };
                let Ok(request) = serde_json::from_str::<Value>(body) else {
                    return;
                };
                let Some(id) = request.get("id").cloned() else {
                    let response =
                        "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                };
                let body = match request.get("method").and_then(Value::as_str) {
                    Some("initialize") => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "b2-http-probe", "version": "0.0.0"}
                        }
                    }),
                    Some("tools/list") => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"tools": [{
                            "name": ALLOWED_TOOL,
                            "description": "HTTP cache summary",
                            "inputSchema": {"type": "object", "properties": {}}
                        }]}
                    }),
                    _ => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "method not found"}
                    }),
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/mcp")
}

fn check_args(config: &fx::ConfigFile, extra: &[&str], env_store: bool) -> Vec<String> {
    let mut args = vec![
        "--config".to_string(),
        config.path_str(),
        "--namespace".to_string(),
        NAMESPACE.to_string(),
    ];
    if env_store {
        args.extend(["--credential-store".to_string(), "env".to_string()]);
    }
    args.extend(["check".to_string(), "--json".to_string()]);
    args.extend(extra.iter().map(|arg| (*arg).to_string()));
    args
}

async fn run_successful_check(
    cache: &IsolatedCache,
    config: &fx::ConfigFile,
    extra: &[&str],
    env_store: bool,
    extra_child_env: &[(&str, &OsStr)],
) -> common::CliOutput {
    let mut child_env = vec![cache.child_env()];
    child_env.extend_from_slice(extra_child_env);
    let output = common::run_fanin_cli_with_env(
        &check_args(config, extra, env_store),
        None,
        CHECK_DEADLINE,
        &child_env,
    )
    .await;
    assert!(
        output.status.is_some_and(|status| status.success()),
        "B2 cache precondition: healthy check must succeed; stdout: {:?}; stderr: {:?}",
        output.stdout,
        output.stderr
    );
    let body: Value = serde_json::from_str(&output.stdout)
        .unwrap_or_else(|error| panic!("check --json must emit one JSON body: {error}"));
    assert_eq!(
        body.get("ok").and_then(Value::as_bool),
        Some(true),
        "cache writes are permitted only after an ok:true check: {body:?}"
    );
    output
}

fn read_cache(cache: &IsolatedCache) -> Value {
    let path = cache.path();
    cache.assert_expected_path(&path);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "RED reason: B2 must write the successful-check cache at {}: {error}",
            path.display()
        )
    });
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("cache file must contain valid JSON: {error}; raw: {raw}"))
}

fn write_cache(cache: &IsolatedCache, body: &Value) {
    let path = cache.path();
    cache.assert_expected_path(&path);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create isolated cache parent");
    std::fs::write(&path, serde_json::to_vec_pretty(body).unwrap())
        .expect("write isolated cache fixture");
}

fn append_cached_tool(body: &mut Value, name: &str, description: &str) {
    let tools = body
        .get_mut("servers")
        .and_then(Value::as_array_mut)
        .and_then(|servers| servers.first_mut())
        .and_then(|server| server.get_mut("tools"))
        .and_then(Value::as_array_mut)
        .expect("generated cache must have a first server tools array");
    if !tools
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
    {
        tools.push(serde_json::json!({
            "name": name,
            "description": description
        }));
    }
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("expected JSON object, got {value:?}"))
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_rfc3339ish(value: &str) {
    let bytes = value.as_bytes();
    assert!(
        bytes.len() >= 20
            && bytes.get(4) == Some(&b'-')
            && bytes.get(7) == Some(&b'-')
            && bytes.get(10) == Some(&b'T')
            && (value.ends_with('Z')
                || value
                    .get(11..)
                    .is_some_and(|time| time.contains('+') || time.contains('-'))),
        "generated_at must be an RFC3339 timestamp, got {value:?}"
    );
}

fn assert_no_sensitive_keys(value: &Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized = key.to_ascii_lowercase().replace('_', "");
                assert!(
                    !normalized.contains("schema")
                        && normalized != "env"
                        && normalized != "headers"
                        && normalized != "credentials"
                        && normalized != "secret",
                    "cache must not persist schema/env/header/credential keys; found {key:?}"
                );
                assert_no_sensitive_keys(child);
            }
        }
        Value::Array(values) => values.iter().for_each(assert_no_sensitive_keys),
        _ => {}
    }
}

fn assert_required_cache_shape(body: &Value) {
    assert_eq!(
        object_keys(body),
        BTreeSet::from([
            "config_fingerprint",
            "format_version",
            "generated_at",
            "namespace",
            "servers",
        ]),
        "cache top-level keys must exactly match the B2 Required Pattern"
    );
    assert_eq!(body.get("format_version").and_then(Value::as_u64), Some(1));
    assert_eq!(
        body.get("namespace").and_then(Value::as_str),
        Some(NAMESPACE)
    );
    assert!(
        body.get("config_fingerprint").is_some_and(Value::is_object),
        "config_fingerprint must be a JSON object as specified by the Required Pattern"
    );
    assert_rfc3339ish(
        body.get("generated_at")
            .and_then(Value::as_str)
            .expect("generated_at must be a string"),
    );
    let servers = body
        .get("servers")
        .and_then(Value::as_array)
        .expect("servers must be an array");
    assert!(
        !servers.is_empty(),
        "healthy check cache must retain servers"
    );
    for server in servers {
        assert_eq!(
            object_keys(server),
            BTreeSet::from(["description", "name", "tools"]),
            "cached server keys must be name/description/tools only"
        );
        assert!(server.get("name").is_some_and(Value::is_string));
        assert!(server.get("description").is_some_and(Value::is_string));
        let tools = server
            .get("tools")
            .and_then(Value::as_array)
            .expect("cached server tools must be an array");
        assert!(
            !tools.is_empty(),
            "healthy inventory must retain tool summaries"
        );
        for tool in tools {
            assert_eq!(
                object_keys(tool),
                BTreeSet::from(["description", "name"]),
                "cached tool keys must be name/description only"
            );
            assert!(tool.get("name").is_some_and(Value::is_string));
            assert!(tool.get("description").is_some_and(Value::is_string));
        }
    }
    assert_no_sensitive_keys(body);
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

async fn advertisement(cache: &IsolatedCache, config: &fx::ConfigFile) -> (String, String) {
    let mut child = common::spawn_fanin_with_config_and_env(
        &config.path_str(),
        Some(NAMESPACE),
        &[cache.child_env()],
    )
    .await;
    let init = common::initialize(&mut child).await;
    let instructions = init
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tools = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&tools, "B2 cache-aware protocol tools/list");
    let list_description = list_tools_description(&tools).to_string();
    child.into_guard().shutdown().await.ok();
    (instructions, list_description)
}

fn result_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("CallToolResult must contain content: {result:?}"))
        .iter()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn file_count(path: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                file_count(&path)
            } else {
                1
            }
        })
        .sum()
}

#[tokio::test]
async fn successful_check_writes_required_cache_shape_without_sensitive_material() {
    let cache = IsolatedCache::new();
    let endpoint = start_http_probe().await;
    let env_marker = "B2-ENV-MATERIAL-MUST-NOT-PERSIST";
    let header_marker = "B2-HEADER-MATERIAL-MUST-NOT-PERSIST";
    let config = http_config(
        &endpoint,
        "HTTP capability cache source",
        Some(header_marker),
        Some(env_marker),
    );

    run_successful_check(&cache, &config, &[], false, &[]).await;
    let body = read_cache(&cache);
    assert_required_cache_shape(&body);
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(
        !serialized.contains(env_marker) && !serialized.contains(header_marker),
        "CK-006: cache must omit configured env and header values"
    );
}

#[tokio::test]
async fn fingerprint_changes_fall_back_to_config_only_advertisement() {
    let cache = IsolatedCache::new();
    let probe = fx::probe_bin_path();
    let stable_description = "Current config-only fallback description";

    let baseline = stdio_config(&probe, &[], stable_description, &[ALLOWED_TOOL], None, &[]);
    run_successful_check(&cache, &baseline, &[], false, &[]).await;
    read_cache(&cache);
    let changed_command = stdio_config(
        "definitely-different-command-for-fingerprint",
        &[],
        stable_description,
        &[ALLOWED_TOOL],
        None,
        &[],
    );
    assert_cache_miss_advertisement(&cache, "command", &changed_command, stable_description).await;

    run_successful_check(&cache, &baseline, &[], false, &[]).await;
    let changed_args = stdio_config(
        &probe,
        &["--fingerprint-change"],
        stable_description,
        &[ALLOWED_TOOL],
        None,
        &[],
    );
    assert_cache_miss_advertisement(&cache, "args", &changed_args, stable_description).await;

    let endpoint = start_http_probe().await;
    let http_baseline = http_config(&endpoint, stable_description, None, None);
    run_successful_check(&cache, &http_baseline, &[], false, &[]).await;
    let changed_endpoint = http_config(
        "http://127.0.0.1:9/fingerprint-change",
        stable_description,
        None,
        None,
    );
    assert_cache_miss_advertisement(&cache, "endpoint", &changed_endpoint, stable_description)
        .await;

    run_successful_check(&cache, &baseline, &[], false, &[]).await;
    let mut acl_cache = read_cache(&cache);
    append_cached_tool(
        &mut acl_cache,
        DENIED_TOOL,
        "Current ACL tool from a stale cache",
    );
    write_cache(&cache, &acl_cache);
    let changed_acl = stdio_config(&probe, &[], stable_description, &[DENIED_TOOL], None, &[]);
    assert_cache_miss_advertisement(&cache, "ACL", &changed_acl, stable_description).await;

    run_successful_check(&cache, &baseline, &[], false, &[]).await;
    let changed_description_text = "Changed description must invalidate cache";
    let changed_description = stdio_config(
        &probe,
        &[],
        changed_description_text,
        &[ALLOWED_TOOL],
        None,
        &[],
    );
    assert_cache_miss_advertisement(
        &cache,
        "description",
        &changed_description,
        changed_description_text,
    )
    .await;
}

async fn assert_cache_miss_advertisement(
    cache: &IsolatedCache,
    changed_field: &str,
    config: &fx::ConfigFile,
    current_description: &str,
) {
    let (instructions, list_description) = advertisement(cache, config).await;
    let combined = format!("{instructions}\n{list_description}");
    assert!(
        combined.contains(current_description),
        "{changed_field} fingerprint mismatch must fall back to the current config description: {combined:?}"
    );
    assert!(
        !instructions.contains(ALLOWED_TOOL)
            && !list_description.contains(ALLOWED_TOOL)
            && !instructions.contains(DENIED_TOOL)
            && !list_description.contains(DENIED_TOOL)
            && !instructions.contains(ALLOWED_SUMMARY)
            && !list_description.contains(ALLOWED_SUMMARY),
        "{changed_field} fingerprint mismatch must not advertise stale cached tool summaries: {combined:?}"
    );
}

#[tokio::test]
async fn valid_cache_enriches_advertisement_with_allowed_summaries_only() {
    let cache = IsolatedCache::new();
    let config = stdio_config(
        &fx::probe_bin_path(),
        &[],
        "Filtered cache advertisement",
        &[ALLOWED_TOOL],
        None,
        &[],
    );
    run_successful_check(&cache, &config, &[], false, &[]).await;
    read_cache(&cache);

    let (instructions, list_description) = advertisement(&cache, &config).await;
    let combined = format!("{instructions}\n{list_description}");
    assert!(
        combined.contains(ALLOWED_TOOL) && combined.contains(ALLOWED_SUMMARY),
        "CA-005: a matching fresh cache must enrich at least one advertisement surface with compact allowed tool summaries: {combined:?}"
    );
    assert!(
        !instructions.contains(DENIED_TOOL) && !list_description.contains(DENIED_TOOL),
        "CA-003: namespace-denied tool names must never enter cache-enriched advertisement: {combined:?}"
    );
    assert!(
        !combined.contains("inputSchema")
            && !combined.contains("input_schema")
            && !combined.contains("\"properties\"")
            && !combined.contains("\"type\":\"object\""),
        "CA-005: advertisement may carry compact summaries, never full JSON schema: {combined:?}"
    );
}

#[tokio::test]
async fn cache_cannot_authorize_namespace_denied_invoke() {
    let cache = IsolatedCache::new();
    let log = fx::empty_log_file_path();
    let config = stdio_config(
        &fx::probe_bin_path(),
        &[],
        "Cache is advisory only",
        &[ALLOWED_TOOL],
        Some(&log),
        &[],
    );
    run_successful_check(&cache, &config, &[], false, &[]).await;
    let mut body = read_cache(&cache);
    append_cached_tool(
        &mut body,
        DENIED_TOOL,
        "A cache entry must never grant permission",
    );
    write_cache(&cache, &body);
    std::fs::write(&log, "").expect("clear baseline check child log");

    let mut child = common::spawn_fanin_with_config_and_env(
        &config.path_str(),
        Some(NAMESPACE),
        &[cache.child_env()],
    )
    .await;
    let init = common::initialize(&mut child).await;
    let protocol_list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&protocol_list, "cache-not-auth protocol tools/list");
    let cache_read_text = format!(
        "{}\n{}",
        init.get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        list_tools_description(&protocol_list)
    );
    assert!(
        cache_read_text.contains(ALLOWED_TOOL),
        "RED reason: initialize and/or protocol tools/list must prove serve consumed the matching cache before cache-not-auth can be proved"
    );
    let response = common::call_tool(
        &mut child,
        "invoke_tool",
        serde_json::json!({
            "name": format!("{SERVER}__{DENIED_TOOL}"),
            "arguments": {}
        }),
    )
    .await;
    common::assert_no_rpc_error(&response, "cache-listed namespace-denied invoke");
    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("invoke_tool must return a CallToolResult: {response:?}"));
    common::assert_is_error_result(result, "cache-listed namespace-denied invoke");
    let error: Value = serde_json::from_str(&result_text(result))
        .expect("namespace denial content must be structured JSON");
    assert_eq!(
        error.get("code").and_then(Value::as_str),
        Some("namespace_denied"),
        "CK-008: cache content must not change the live namespace ACL: {error:?}"
    );
    assert_eq!(error.get("server").and_then(Value::as_str), Some(SERVER));
    assert_eq!(error.get("tool").and_then(Value::as_str), Some(DENIED_TOOL));
    let log_body = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !log_body.contains(&format!("[{SERVER}]")),
        "cache-authorized denial must happen before any upstream spawn/call; log: {log_body}"
    );
    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn no_cache_write_suppresses_an_otherwise_working_write() {
    let cache = IsolatedCache::new();
    let config = stdio_config(
        &fx::probe_bin_path(),
        &[],
        "No-cache-write control",
        &[ALLOWED_TOOL],
        None,
        &[],
    );
    run_successful_check(&cache, &config, &[], false, &[]).await;
    read_cache(&cache);
    std::fs::remove_file(cache.path()).expect("remove positive-control cache file");

    run_successful_check(&cache, &config, &["--no-cache-write"], false, &[]).await;
    let path = cache.path();
    cache.assert_expected_path(&path);
    assert!(
        !path.exists() && file_count(&cache.root) == 0,
        "--no-cache-write must leave no new cache file under the isolated override"
    );
}

#[tokio::test]
async fn refresh_cache_replaces_stale_file_after_success() {
    let cache = IsolatedCache::new();
    let config = stdio_config(
        &fx::probe_bin_path(),
        &[],
        "Refresh-cache control",
        &[ALLOWED_TOOL],
        None,
        &[],
    );
    run_successful_check(&cache, &config, &[], false, &[]).await;
    read_cache(&cache);
    let stale = serde_json::json!({
        "format_version": 999,
        "namespace": NAMESPACE,
        "config_fingerprint": {},
        "generated_at": "2000-01-01T00:00:00Z",
        "servers": []
    });
    write_cache(&cache, &stale);

    run_successful_check(&cache, &config, &["--refresh-cache"], false, &[]).await;
    let refreshed = read_cache(&cache);
    assert_ne!(
        refreshed, stale,
        "--refresh-cache must ignore and replace the stale cache on successful check"
    );
    assert_required_cache_shape(&refreshed);
}

#[tokio::test]
async fn credential_secret_is_absent_from_cache_and_json_stdout() {
    let cache = IsolatedCache::new();
    let key = fx::phase3_env_var_name("B2_CACHE_SECRET");
    let sentinel = fx::phase3_sentinel_value();
    let placeholder = format!("${{{key}}}");
    let config = stdio_config(
        &fx::probe_bin_path(),
        &[],
        "Credential redaction cache",
        &[ALLOWED_TOOL],
        None,
        &[("TOKEN", &placeholder)],
    );

    let output = run_successful_check(
        &cache,
        &config,
        &[],
        true,
        &[(key.as_str(), OsStr::new(&sentinel))],
    )
    .await;
    let body = read_cache(&cache);
    let cache_text = serde_json::to_string(&body).unwrap();
    assert!(
        !cache_text.contains(&sentinel) && !output.stdout.contains(&sentinel),
        "D-010: a credential-path sentinel must be absent from cache and check --json stdout"
    );
}
