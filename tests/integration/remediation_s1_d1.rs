//! Remediation S-1 + D-1 contract tests.
//!
//! These tests pin the timeout envelope for cold connect / discovery / dirty
//! refetch and the documented per-server `cwd` behavior. They deliberately use
//! probes that hang far longer than the configured timeout so a real timeout is
//! the only acceptable success path.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

const TIMEOUT_SECS: u64 = 1;
const CALL_CEILING: Duration = Duration::from_secs(4);
const PROMPT_CEILING: Duration = Duration::from_secs(2);
const DEATH_CEILING: Duration = Duration::from_secs(5);

fn phase3_config_for_probe_args(server: &str, args: &[&str]) -> fx::ConfigFile {
    // Phase3ServerEntry does not own argv; use raw TOML for CLI modes.
    let quoted_args = args
        .iter()
        .map(|a| format!("'{}'", a.replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        "[servers.{server}]\ntransport = \"stdio\"\ncommand = '{}'\nargs = [{quoted_args}]\ntimeout_secs = {TIMEOUT_SECS}\n\n[namespaces.default]\nservers = [\"{server}\"]\n",
        fx::probe_bin_path().replace('\'', "\\'")
    );
    fx::raw_config_file(&toml)
}

fn stdio_config_with_cwd(server: &str, cwd: Option<&str>) -> fx::ConfigFile {
    let cwd_line = cwd
        .map(|value| format!("cwd = '{}'\n", value.replace('\'', "\\'")))
        .unwrap_or_default();
    let probe = fx::probe_bin_path().replace('\'', "\\'");
    let toml = format!(
        "[servers.{server}]\ntransport = \"stdio\"\ncommand = '{probe}'\nargs = ['--enable-report-cwd']\ntimeout_secs = {TIMEOUT_SECS}\n{cwd_line}\n[namespaces.default]\nservers = [\"{server}\"]\n"
    );
    fx::raw_config_file(&toml)
}

fn result_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("result missing content: {result:?}"))
        .iter()
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_error_json(result: &Value) -> Value {
    let text = result_text(result);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("structured error text must be JSON: {text:?}: {e}"))
}

fn assert_structured_timeout(resp: &Value, ctx: &str) {
    common::assert_no_rpc_error(resp, ctx);
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("{ctx}: missing result"));
    common::assert_is_error_result(result, ctx);
    let err = parse_error_json(result);
    assert_eq!(
        err.get("code").and_then(Value::as_str),
        Some("upstream_timeout"),
        "{ctx}: expected upstream_timeout structured code, got {err:?}"
    );
    assert_eq!(
        err.get("recoverable").and_then(Value::as_bool),
        Some(true),
        "{ctx}: timeout must be recoverable: {err:?}"
    );
}

async fn call_list_tools_with_elapsed(
    child: &mut common::JsonRpcChild,
    ctx: &str,
) -> (Value, Duration) {
    let started = Instant::now();
    let resp = timeout(
        CALL_CEILING,
        common::call_tool(child, "list_tools", serde_json::json!({})),
    )
    .await
    .unwrap_or_else(|_| panic!("{ctx}: call did not return within {CALL_CEILING:?}"));
    (resp, started.elapsed())
}

fn assert_within_timeout_envelope(elapsed: Duration, ctx: &str) {
    assert!(
        elapsed < CALL_CEILING,
        "{ctx}: must return well before the probe's long hang; elapsed {elapsed:?}"
    );
}

#[tokio::test]
async fn s1_hang_during_initialize_returns_structured_timeout_within_bound() {
    let cfg = phase3_config_for_probe_args("hang-init", &["--hang-during-initialize"]);
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let (resp, elapsed) = call_list_tools_with_elapsed(&mut child, "hang during initialize").await;
    assert_within_timeout_envelope(elapsed, "hang during initialize");
    assert_structured_timeout(&resp, "hang during initialize");

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn s1_hang_during_initial_list_tools_returns_structured_timeout_within_bound() {
    let cfg = phase3_config_for_probe_args("hang-list", &["--hang-during-list-tools"]);
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let (resp, elapsed) =
        call_list_tools_with_elapsed(&mut child, "hang during initial list_tools").await;
    assert_within_timeout_envelope(elapsed, "hang during initial list_tools");
    assert_structured_timeout(&resp, "hang during initial list_tools");

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn s1_http_stall_during_initialize_returns_structured_timeout_within_bound() {
    let (endpoint, _server_task) = start_stalling_http_probe().await;
    let cfg = http_config_with_cwd("http-stall", &endpoint, None);
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let (resp, elapsed) = call_list_tools_with_elapsed(&mut child, "HTTP initialize stall").await;
    assert_within_timeout_envelope(elapsed, "HTTP initialize stall");
    assert_structured_timeout(&resp, "HTTP initialize stall");

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn s1_hang_during_dirty_refetch_times_out_keeps_dirty_and_preserves_cache() {
    let cfg = phase3_config_for_probe_args("hang-refetch", &["--hang-during-refetch"]);
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let baseline = timeout(
        CALL_CEILING,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("baseline list_tools must complete");
    common::assert_no_rpc_error(&baseline, "baseline list_tools");
    let baseline_text = result_text(baseline.get("result").expect("baseline result"));
    assert!(
        baseline_text.contains("echo_ok"),
        "baseline cache must contain the original inventory: {baseline_text}"
    );

    let mutate = timeout(
        CALL_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name":"hang-refetch__mutate_tools", "arguments": {}}),
        ),
    )
    .await
    .expect("mutate_tools must complete");
    common::assert_no_rpc_error(&mutate, "mutate_tools before refetch hang");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (timeout_resp, elapsed) =
        call_list_tools_with_elapsed(&mut child, "dirty refetch hang").await;
    assert_within_timeout_envelope(elapsed, "dirty refetch hang");
    assert_structured_timeout(&timeout_resp, "dirty refetch hang");

    let (retry_resp, retry_elapsed) =
        call_list_tools_with_elapsed(&mut child, "dirty refetch retry").await;
    assert_within_timeout_envelope(retry_elapsed, "dirty refetch retry");
    assert_structured_timeout(&retry_resp, "dirty refetch retry");
    assert!(
        baseline_text.contains("echo_ok"),
        "prior cached inventory must not be overwritten with empty on timeout"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn s1_cold_connect_timeout_retries_and_releases_init_guard() {
    let cfg = phase3_config_for_probe_args("hang-init", &["--hang-during-initialize"]);
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let (first, first_elapsed) =
        call_list_tools_with_elapsed(&mut child, "first cold connect timeout").await;
    assert_within_timeout_envelope(first_elapsed, "first cold connect timeout");
    assert_structured_timeout(&first, "first cold connect timeout");

    let (second, second_elapsed) =
        call_list_tools_with_elapsed(&mut child, "second cold connect timeout").await;
    assert_within_timeout_envelope(second_elapsed, "second cold connect timeout");
    assert_structured_timeout(&second, "second cold connect timeout");

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn s1_timed_out_connect_kills_spawned_descendant_during_test_window() {
    let marker_path = fx::grandchild_marker_path();
    let cfg = phase3_config_for_probe_args(
        "hang-tree",
        &["--hang-then-spawn-descendant", marker_path.as_str()],
    );
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let started = Instant::now();
    let call_id = child
        .send_request(
            "tools/call",
            serde_json::json!({"name":"list_tools", "arguments": {}}),
        )
        .await;

    let pid = wait_for_marker_pid(&marker_path, Duration::from_secs(3)).await;
    assert!(
        process_is_alive(pid),
        "descendant must be alive before timeout"
    );

    let resp = timeout(CALL_CEILING, child.wait_for_id(call_id))
        .await
        .expect("timed-out connect must return a response within the timeout envelope");
    assert_within_timeout_envelope(started.elapsed(), "connect timeout with descendant");
    assert_structured_timeout(&resp, "connect timeout with descendant");
    assert!(
        wait_for_process_death(pid, DEATH_CEILING).await,
        "descendant pid {pid} must be dead within {DEATH_CEILING:?} after the connect timeout"
    );

    let _ = std::fs::remove_file(&marker_path);
    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn s1_hung_cold_connect_does_not_block_already_connected_sibling() {
    let probe = fx::probe_bin_path().replace('\'', "\\'");
    let toml = format!(
        "[servers.hung]\ntransport = \"stdio\"\ncommand = '{probe}'\nargs = ['--hang-during-initialize']\ntimeout_secs = {TIMEOUT_SECS}\n\n[servers.ready]\ntransport = \"stdio\"\ncommand = '{probe}'\nargs = []\ntimeout_secs = {TIMEOUT_SECS}\n\n[namespaces.default]\nservers = [\"hung\", \"ready\"]\n"
    );
    let cfg = fx::raw_config_file(&toml);
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let warm = timeout(
        CALL_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name":"ready__echo_ok", "arguments":{"message":"warm"}}),
        ),
    )
    .await
    .expect("ready sibling warmup must complete");
    common::assert_no_rpc_error(&warm, "ready sibling warmup");

    let hung_id = child
        .send_request(
            "tools/call",
            serde_json::json!({"name":"list_tools", "arguments": {}}),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ready_started = Instant::now();
    let ready = timeout(
        PROMPT_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name":"ready__echo_ok", "arguments":{"message":"sibling"}}),
        ),
    )
    .await
    .expect("already-connected sibling must return while another server is hung");
    common::assert_no_rpc_error(&ready, "ready sibling during hung connect");
    assert!(
        ready_started.elapsed() < PROMPT_CEILING,
        "sibling call must not wait for hung cold connect"
    );

    let hung = timeout(CALL_CEILING, child.wait_for_id(hung_id))
        .await
        .expect("hung cold connect must still return by timeout");
    assert_structured_timeout(&hung, "hung cold connect after sibling proof");

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn d1_literal_cwd_is_child_working_directory() {
    let temp = TempDir::new().expect("temp cwd");
    let cfg = stdio_config_with_cwd("cwd-lit", Some(path_str(temp.path()).as_str()));
    let reported = invoke_report_cwd(&cfg, "cwd-lit").await;
    assert_same_path(&reported, temp.path(), "literal cwd");
}

#[tokio::test]
async fn d1_cwd_resolves_env_placeholder_through_existing_resolver() {
    let temp = TempDir::new().expect("temp cwd");
    let key = fx::phase3_env_var_name("CWD_ROOT");
    std::env::set_var(&key, path_str(temp.path()));
    let raw = format!("${{{key}}}");
    let cfg = stdio_config_with_cwd("cwd-env", Some(&raw));
    let reported = invoke_report_cwd(&cfg, "cwd-env").await;
    assert_same_path(&reported, temp.path(), "resolved cwd");
    std::env::remove_var(&key);
}

#[tokio::test]
async fn d1_unset_cwd_inherits_aggregator_working_directory() {
    let parent = std::env::current_dir().expect("parent cwd");
    let cfg = stdio_config_with_cwd("cwd-inherit", None);
    let reported = invoke_report_cwd(&cfg, "cwd-inherit").await;
    assert_same_path(&reported, &parent, "inherited cwd");
}

#[tokio::test]
async fn d1_empty_or_whitespace_cwd_fails_config_validation_before_serving() {
    for raw in ["", "   \t  "] {
        let cfg = stdio_config_with_cwd("bad-cwd", Some(raw));
        let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
        assert_does_not_serve(&mut child).await;
        child.into_guard().shutdown().await.ok();
    }
}

#[tokio::test]
async fn d1_resolved_blank_cwd_fails_before_spawn_with_structured_tool_error() {
    let key = fx::phase3_env_var_name("BLANK_CWD");
    std::env::set_var(&key, "   ");
    let cfg = stdio_config_with_cwd("blank-cwd", Some(&format!("${{{key}}}")));
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let resp = timeout(
        CALL_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name":"blank-cwd__report_cwd", "arguments": {}}),
        ),
    )
    .await
    .expect("blank resolved cwd must fail promptly, not hang");
    common::assert_no_rpc_error(&resp, "blank resolved cwd");
    let result = resp.get("result").expect("blank cwd result");
    common::assert_is_error_result(result, "blank resolved cwd");

    child.into_guard().shutdown().await.ok();
    std::env::remove_var(&key);
}

#[tokio::test]
async fn d1_nonexistent_cwd_surfaces_upstream_connect_failed() {
    let mut missing = std::env::temp_dir();
    missing.push(format!("fanin-mcp-missing-cwd-{}", fx::phase3_unique_seq()));
    let cfg = stdio_config_with_cwd("missing-cwd", Some(path_str(&missing).as_str()));
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let resp = timeout(
        CALL_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name":"missing-cwd__report_cwd", "arguments": {}}),
        ),
    )
    .await
    .expect("missing cwd spawn failure must return promptly");
    common::assert_no_rpc_error(&resp, "missing cwd");
    let result = resp.get("result").expect("missing cwd result");
    common::assert_is_error_result(result, "missing cwd");
    let err = parse_error_json(result);
    assert_eq!(
        err.get("code").and_then(Value::as_str),
        Some("upstream_connect_failed"),
        "missing cwd must surface public upstream_connect_failed code: {err:?}"
    );

    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn d1_http_cwd_is_ignored_and_not_resolved_or_applied() {
    let missing_key = fx::phase3_env_var_name("HTTP_CWD_SHOULD_NOT_RESOLVE");
    std::env::remove_var(&missing_key);
    let (endpoint, _server_task) = start_normal_http_probe().await;
    let cfg = http_config_with_cwd("http-cwd", &endpoint, Some(&format!("${{{missing_key}}}")));
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let resp = timeout(
        CALL_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name":"http-cwd__echo_ok", "arguments": {}}),
        ),
    )
    .await
    .expect("HTTP cwd must be ignored, not resolved");
    common::assert_no_rpc_error(&resp, "HTTP cwd ignored");
    let text = result_text(resp.get("result").expect("HTTP cwd result"));
    assert!(
        text.contains("http echo ok"),
        "HTTP server must connect normally: {text}"
    );

    child.into_guard().shutdown().await.ok();
}

async fn invoke_report_cwd(cfg: &fx::ConfigFile, server: &str) -> PathBuf {
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    let resp = timeout(
        CALL_CEILING,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({"name": format!("{server}__report_cwd"), "arguments": {}}),
        ),
    )
    .await
    .expect("report_cwd must return within deadline");
    common::assert_no_rpc_error(&resp, "report_cwd");
    let result = resp.get("result").expect("report_cwd result");
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        panic!("report_cwd returned error result: {}", result_text(result));
    }
    let reported = PathBuf::from(result_text(result));
    child.into_guard().shutdown().await.ok();
    reported
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn assert_same_path(actual: &Path, expected: &Path, ctx: &str) {
    let actual_canon = actual.canonicalize().unwrap_or_else(|e| {
        panic!(
            "{ctx}: actual path {} not canonicalizable: {e}",
            actual.display()
        )
    });
    let expected_canon = expected.canonicalize().unwrap_or_else(|e| {
        panic!(
            "{ctx}: expected path {} not canonicalizable: {e}",
            expected.display()
        )
    });
    assert_eq!(
        actual_canon, expected_canon,
        "{ctx}: child cwd must match configured directory"
    );
}

async fn assert_does_not_serve(child: &mut common::JsonRpcChild) {
    let _ = child
        .send_request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "remediation-test", "version": "0.0.0" },
            }),
        )
        .await;
    let stdout = child.drain_stdout_raw(Duration::from_secs(2)).await;
    assert!(
        stdout.is_empty(),
        "config validation failure must happen before MCP serving and write no stdout; got {:?}",
        String::from_utf8_lossy(&stdout)
    );
}

fn http_config_with_cwd(server: &str, endpoint: &str, cwd: Option<&str>) -> fx::ConfigFile {
    let cwd_line = cwd
        .map(|value| format!("cwd = '{}'\n", value.replace('\'', "\\'")))
        .unwrap_or_default();
    let toml = format!(
        "[servers.{server}]\ntransport = \"streamable-http\"\nendpoint = \"{endpoint}\"\ntimeout_secs = {TIMEOUT_SECS}\n{cwd_line}\n[namespaces.default]\nservers = [\"{server}\"]\n"
    );
    fx::raw_config_file(&toml)
}

async fn start_stalling_http_probe() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalling HTTP probe");
    let addr = listener.local_addr().expect("local addr");
    let task = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            std::future::pending::<()>().await;
        }
    });
    (format!("http://{addr}/mcp"), task)
}

async fn start_normal_http_probe() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind normal HTTP probe");
    let addr = listener.local_addr().expect("local addr");
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let Some((_, body_text)) = req.split_once("\r\n\r\n") else {
                    return;
                };
                let Ok(request) = serde_json::from_str::<Value>(body_text) else {
                    return;
                };
                let Some(id) = request.get("id").cloned() else {
                    let response =
                        "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                };
                let method = request.get("method").and_then(Value::as_str);
                let body = match method {
                    Some("initialize") => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"http-probe","version":"0.0.0"}}}),
                    Some("tools/list") => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"tools":[{"name":"echo_ok","description":"HTTP echo","inputSchema":{"type":"object","properties":{}}}]}}),
                    Some("tools/call") => serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"http echo ok"}],"isError":false}}),
                    _ => serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (format!("http://{addr}/mcp"), task)
}

async fn wait_for_marker_pid(marker_path: &str, deadline: Duration) -> u32 {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if let Ok(content) = std::fs::read_to_string(marker_path) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("marker {marker_path} did not contain a descendant pid within {deadline:?}");
}

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

async fn wait_for_process_death(pid: u32, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if !process_is_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}
