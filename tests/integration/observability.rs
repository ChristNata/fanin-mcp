//! Phase 5 P1 — redacted JSON observability contract.
//!
//! These tests are wire/CLI level. They assert effects: bytes on stdout,
//! newline-delimited JSON in the file sink, and absence of sentinel secrets in
//! every diagnostic sink.

use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

const OBS_DEADLINE: Duration = Duration::from_secs(15);

fn log_file_path(label: &str) -> String {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "fanin-mcp-phase5-{label}-{}-{}.ndjson",
        std::process::id(),
        fx::phase3_unique_seq()
    ));
    let _ = std::fs::remove_file(&p);
    p.to_string_lossy().into_owned()
}

fn result_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("result missing content array: {result:?}"))
        .iter()
        .filter_map(|b| {
            (b.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| b.get("text").and_then(Value::as_str).map(str::to_owned))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("")
}

fn read_json_lines(path: &str) -> Vec<Value> {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("log file {path} must be readable: {e}"));
    assert!(!raw.trim().is_empty(), "log file must not be empty");
    raw.lines()
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .unwrap_or_else(|e| panic!("log line must be JSON: {line:?}\n{e}"))
        })
        .collect()
}

async fn drain_stderr(mut child: common::JsonRpcChild) -> String {
    let mut stderr = child
        .take_stderr()
        .expect("spawn helper must pipe fanin-mcp stderr");
    child.into_guard().shutdown().await.ok();
    let mut buf = Vec::new();
    let _ = timeout(Duration::from_secs(2), stderr.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn log_file_writes_ndjson_and_stdout_stays_json_rpc_only() {
    let log = log_file_path("serve");
    let cfg = fx::ConfigBuilder::new().write();
    let args = vec![
        "--config".to_string(),
        cfg.path_str(),
        "--log-file".to_string(),
        log.clone(),
    ];
    let mut child = common::spawn_fanin_with_args(&args).await;
    common::initialize(&mut child).await;

    let list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&list, "tools/list while JSON logging");
    let result = list.get("result").expect("tools/list result");
    assert!(
        result.get("tools").is_some(),
        "stdout must carry MCP JSON-RPC, not diagnostics; got: {list:?}"
    );

    let stderr = drain_stderr(child).await;
    assert!(
        !stderr.contains("{\""),
        "structured JSON diagnostics belong in the file sink, not stderr/stdout; stderr: {stderr}"
    );
    let lines = read_json_lines(&log);
    assert!(
        lines.iter().any(|v| v.get("level").is_some()),
        "NDJSON entries must carry structured fields: {lines:?}"
    );
}

#[tokio::test]
async fn log_level_debug_includes_debug_events_and_invalid_level_fails_before_serve() {
    let log = log_file_path("debug");
    let cfg = fx::ConfigBuilder::new().write();
    let args = vec![
        "--config".to_string(),
        cfg.path_str(),
        "--log-file".to_string(),
        log.clone(),
        "--log-level".to_string(),
        "debug".to_string(),
    ];
    let mut child = common::spawn_fanin_with_args(&args).await;
    common::initialize(&mut child).await;
    let _ = common::list_tools(&mut child).await;
    child.into_guard().shutdown().await.ok();
    let lines = read_json_lines(&log);
    assert!(
        lines.iter().any(|v| {
            v.get("level")
                .and_then(Value::as_str)
                .map(|s| s.eq_ignore_ascii_case("debug"))
                .unwrap_or(false)
        }),
        "--log-level debug must include debug-level structured events: {lines:?}"
    );

    let bad = common::run_fanin_cli(
        &[
            "--config".to_string(),
            cfg.path_str(),
            "--log-level".to_string(),
            "definitely-not-a-level".to_string(),
        ],
        None,
        Duration::from_secs(5),
    )
    .await;
    let status = bad.status.expect("invalid log-level run must exit");
    assert!(!status.success(), "invalid --log-level must exit non-zero");
    assert!(
        bad.stdout.is_empty(),
        "invalid --log-level must fail before serve(stdio()) and write no stdout bytes; stdout: {:?}",
        bad.stdout
    );
}

#[tokio::test]
async fn sentinel_secret_absent_from_stderr_and_json_file_sink() {
    let log = log_file_path("redaction");
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let env_name = fx::phase3_env_var_name("OBS");
    let secret = fx::phase3_sentinel_value();
    std::env::set_var(&key, &secret);

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(server.clone()).env(env_name.clone(), format!("${{{key}}}")),
        )
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let args = vec![
        "--config".to_string(),
        cfg.path_str(),
        "--log-file".to_string(),
        log.clone(),
    ];
    let mut child = common::spawn_fanin_with_args(&args).await;
    common::initialize(&mut child).await;
    let resp = timeout(
        OBS_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": env_name },
            }),
        ),
    )
    .await
    .expect("invoke_tool with resolved secret must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool redaction sentinel");
    let text = result_text(resp.get("result").expect("invoke_tool result"));
    assert!(
        text.contains(&secret),
        "probe must receive the resolved secret"
    );

    let stderr = drain_stderr(child).await;
    let file = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !stderr.contains(&secret),
        "stderr leaked sentinel: {stderr}"
    );
    assert!(
        !file.contains(&secret),
        "JSON log file leaked sentinel: {file}"
    );
    std::env::remove_var(&key);
}

#[tokio::test]
async fn invoke_tool_logs_success_and_failure_without_args_or_secrets() {
    let log = log_file_path("calls");
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let secret_arg = fx::phase3_sentinel_value();
    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server.clone()))
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let args = vec![
        "--config".to_string(),
        cfg.path_str(),
        "--log-file".to_string(),
        log.clone(),
    ];
    let mut child = common::spawn_fanin_with_args(&args).await;
    common::initialize(&mut child).await;

    let ok = timeout(
        OBS_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_ok"),
                "arguments": { "message": "success-path" },
            }),
        ),
    )
    .await
    .expect("success invoke must complete");
    common::assert_no_rpc_error(&ok, "success invoke");

    let fail = timeout(
        OBS_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__always_error"),
                "arguments": { "token": secret_arg },
            }),
        ),
    )
    .await
    .expect("failing invoke must complete");
    common::assert_no_rpc_error(&fail, "failing invoke");
    child.into_guard().shutdown().await.ok();

    let lines = read_json_lines(&log);
    let call_entries: Vec<&Value> = lines
        .iter()
        .filter(|v| v.get("server").and_then(Value::as_str) == Some(server.as_str()))
        .filter(|v| v.get("tool").is_some() && v.get("latency_ms").is_some())
        .collect();
    assert!(
        call_entries.iter().any(|v| {
            v.get("tool").and_then(Value::as_str) == Some("echo_ok")
                && v.get("latency_ms").and_then(Value::as_f64).is_some()
                && v.get("outcome").and_then(Value::as_str) == Some("success")
        }),
        "successful invoke log must carry server/tool/numeric latency/success outcome: {lines:?}"
    );
    assert!(
        call_entries.iter().any(|v| {
            v.get("tool").and_then(Value::as_str) == Some("always_error")
                && v.get("outcome").and_then(Value::as_str) == Some("failure")
        }),
        "failing invoke log must carry failure outcome: {lines:?}"
    );
    let raw = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !raw.contains(&secret_arg),
        "failure log leaked arguments/secrets: {raw}"
    );
}
