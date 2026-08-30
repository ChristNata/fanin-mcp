#![cfg(feature = "probe-fixture")]

//! Phase B1 `fanin-mcp check` process contract.
//!
//! `check` is a CLI preflight, not an MCP stdio session. Every check assertion
//! uses process exit status plus stdout JSON. Process and log oracles prove the
//! negative side effects; N5 captures a real grandchild PID and polls that PID
//! for death after `check` returns.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::common;
use crate::common::fixtures as fx;

// Outer patience for a complete check subprocess under full nextest process
// parallelism. Per-server timeout_secs remains the behavior under test.
const CHECK_DEADLINE: Duration = Duration::from_secs(30);
// Kept equal to process_lifetime.rs::CLEANUP_INTERVAL. The probe descendant
// lives for 30s, so survival through this 12s poll is an unambiguous orphan.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(12);
// This descendant proof must observe the marker before its configured timeout
// can kill the process tree. The call and cleanup deadlines start separately.
const DESCENDANT_TIMEOUT_SECS: u64 = 8;
const MARKER_READY: Duration = Duration::from_secs(4);
const PROBE_TOOL_NAMES: [&str; 16] = [
    "always_error",
    "dangerous_noop",
    "echo_env",
    "echo_image",
    "echo_ok",
    "mutate_tools",
    "needs_elicitation",
    "needs_roots",
    "needs_sampling",
    "poison_meta",
    "poison_schema",
    "poison_validation",
    "self_pid",
    "slow_tool",
    "spawn_grandchild",
    "toggle_long_tool",
];

fn toml_literal(value: &str) -> String {
    value.replace('\0', "").replace('\'', "\\'")
}

fn raw_stdio_server(
    name: &str,
    args: &[&str],
    timeout_secs: Option<u64>,
    log_file: Option<&str>,
    cwd: Option<&str>,
) -> String {
    let probe = toml_literal(&fx::probe_bin_path());
    let args = args
        .iter()
        .map(|arg| format!("'{}'", toml_literal(arg)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out =
        format!("[servers.{name}]\ntransport = \"stdio\"\ncommand = '{probe}'\nargs = [{args}]\n");
    if let Some(timeout) = timeout_secs {
        out.push_str(&format!("timeout_secs = {timeout}\n"));
    }
    if let Some(log) = log_file {
        out.push_str(&format!("log_file = '{}'\n", toml_literal(log)));
    }
    if let Some(cwd) = cwd {
        out.push_str(&format!("cwd = '{}'\n", toml_literal(cwd)));
    }
    out.push('\n');
    out
}

fn namespace(name: &str, servers: &[&str], tools: &[(&str, &[&str])]) -> String {
    let servers = servers
        .iter()
        .map(|server| format!("\"{server}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!("[namespaces.{name}]\nservers = [{servers}]\n");
    if !tools.is_empty() {
        out.push_str(&format!("[namespaces.{name}.tools]\n"));
        for (server, names) in tools {
            let names = names
                .iter()
                .map(|tool| format!("\"{tool}\""))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("{server} = [{names}]\n"));
        }
    }
    out.push('\n');
    out
}

fn check_args(
    config_path: Option<&str>,
    selected_namespace: Option<&str>,
    global_log: Option<&str>,
    local_args: &[&str],
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(path) = config_path {
        args.extend(["--config".to_string(), path.to_string()]);
    }
    if let Some(name) = selected_namespace {
        args.extend(["--namespace".to_string(), name.to_string()]);
    }
    if let Some(path) = global_log {
        args.extend(["--log-file".to_string(), path.to_string()]);
    }
    args.push("check".to_string());
    args.push("--json".to_string());
    args.extend(local_args.iter().map(|arg| (*arg).to_string()));
    args
}

fn assert_check_command_recognized(output: &common::CliOutput) {
    let clap_rejected_check =
        output.stderr.contains("unrecognized subcommand") && output.stderr.contains("check");
    assert!(
        !clap_rejected_check,
        "RED reason: Phase B1 is absent because clap rejects the `check` subcommand; stderr: {}",
        output.stderr
    );
}

async fn run_check(args: &[String]) -> common::CliOutput {
    run_check_with_env(args, &[]).await
}

async fn run_check_with_env(args: &[String], child_env: &[(&str, &OsStr)]) -> common::CliOutput {
    let output = common::run_fanin_cli_with_env(args, None, CHECK_DEADLINE, child_env).await;
    assert_check_command_recognized(&output);
    assert!(
        output.status.is_some(),
        "check must return within {CHECK_DEADLINE:?}; stderr: {}",
        output.stderr
    );
    output
}

async fn assert_check_command_available() {
    let output = common::run_fanin_cli(
        &["check".to_string(), "--help".to_string()],
        None,
        CHECK_DEADLINE,
    )
    .await;
    assert_check_command_recognized(&output);
    assert!(
        output.status.is_some_and(|status| status.success()),
        "`check --help` must succeed once the B1 command exists; stderr: {}",
        output.stderr
    );
}

fn parse_stdout_json(output: &common::CliOutput) -> Value {
    serde_json::from_str(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "check --json stdout must be one JSON body; stdout: {:?}; stderr: {:?}; error: {error}",
            output.stdout, output.stderr
        )
    })
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("expected JSON object, got {value:?}"))
        .keys()
        .map(String::as_str)
        .collect()
}

fn errors(body: &Value) -> &[Value] {
    body.get("errors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("check JSON must contain errors[]: {body:?}"))
}

fn error_with_code<'a>(body: &'a Value, code: &str) -> &'a Value {
    errors(body)
        .iter()
        .find(|error| error.get("code").and_then(Value::as_str) == Some(code))
        .unwrap_or_else(|| panic!("check errors[] must contain code {code:?}: {body:?}"))
}

fn assert_failure(output: &common::CliOutput, body: &Value) {
    assert!(
        output.status.is_some_and(|status| !status.success()),
        "failed preflight must return a non-zero exit status; status: {:?}; stdout: {:?}; stderr: {:?}",
        output.status,
        output.stdout,
        output.stderr
    );
    assert_eq!(
        body.get("ok").and_then(Value::as_bool),
        Some(false),
        "failed preflight must emit ok:false: {body:?}"
    );
}

#[cfg(windows)]
fn probe_child_pids(parent_pid: u32) -> Vec<u32> {
    let script = format!(
        "$ErrorActionPreference = 'Stop'; Get-CimInstance Win32_Process -Filter \"ParentProcessId = {parent_pid}\" | Where-Object {{ $_.Name -like 'probe-server*' }} | ForEach-Object {{ $_.ProcessId }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("PowerShell process query must run");
    assert!(
        output.status.success(),
        "PowerShell process query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(unix)]
fn probe_child_pids(parent_pid: u32) -> Vec<u32> {
    let output = Command::new("ps")
        .args(["-eo", "pid=,ppid=,comm="])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("ps process query must run");
    assert!(
        output.status.success(),
        "ps process query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let ppid = fields.next()?.parse::<u32>().ok()?;
            let command = fields.next()?;
            (ppid == parent_pid && command.starts_with("probe-server")).then_some(pid)
        })
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn probe_child_pids(_parent_pid: u32) -> Vec<u32> {
    panic!("probe PID oracle is unsupported on this platform")
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        matches!(
            Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
            Ok(status) if status.success()
        )
    }
    #[cfg(windows)]
    {
        let output = match Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        {
            Ok(output) => output,
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

async fn wait_for_process_death(pid: u32) -> bool {
    let started = Instant::now();
    loop {
        if !process_is_alive(pid) {
            return true;
        }
        if started.elapsed() >= CLEANUP_INTERVAL {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn parse_pid_marker(path: &str) -> u32 {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("grandchild PID marker {path} must be readable: {error}"));
    content.trim().parse::<u32>().unwrap_or_else(|error| {
        panic!("grandchild marker must contain a numeric PID; content {content:?}: {error}")
    })
}

#[tokio::test]
async fn check_json_ok_on_healthy_namespace() {
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("zeta"))
        .server(fx::ServerEntry::new("alpha"))
        .namespace(fx::NamespaceEntry::new("reviewer", ["zeta", "alpha"]))
        .write();
    let output = run_check(&check_args(
        Some(&config.path_str()),
        Some("reviewer"),
        None,
        &["--refresh-cache", "--no-cache-write"],
    ))
    .await;
    assert!(
        output.status.is_some_and(|status| status.success()),
        "healthy check must exit zero; stderr: {}",
        output.stderr
    );
    let body = parse_stdout_json(&output);

    assert_eq!(
        object_keys(&body),
        BTreeSet::from(["errors", "namespace", "ok", "servers"]),
        "CK-005 top-level JSON shape must be exact"
    );
    assert_eq!(
        body.get("namespace").and_then(Value::as_str),
        Some("reviewer")
    );
    assert_eq!(body.get("ok").and_then(Value::as_bool), Some(true));
    assert!(
        errors(&body).is_empty(),
        "healthy check errors[] must be empty"
    );

    let servers = body
        .get("servers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("servers must be an array: {body:?}"));
    let names = servers
        .iter()
        .map(|server| server.get("name").and_then(Value::as_str).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["alpha", "zeta"],
        "servers must be sorted deterministically"
    );
    for server in servers {
        assert_eq!(
            object_keys(server),
            BTreeSet::from(["name", "status", "tools"]),
            "per-server JSON shape must be name/status/tools"
        );
        assert_eq!(server.get("status").and_then(Value::as_str), Some("ok"));
        let tools = server.get("tools").and_then(Value::as_array).unwrap();
        let tool_names = tools
            .iter()
            .map(|tool| {
                assert!(
                    tool.get("inputSchema").is_none() && tool.get("input_schema").is_none(),
                    "check JSON must not retain tool schemas: {tool:?}"
                );
                tool.get("name").and_then(Value::as_str).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tool_names, PROBE_TOOL_NAMES,
            "every healthy server must expose the sorted live probe inventory"
        );
    }
}

#[tokio::test]
async fn check_fails_on_missing_credential() {
    let key = fx::phase3_env_var_name("CHECK_MISSING");
    let server = "missing-cred";
    let config = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(server).env("TOKEN", format!("${{{key}}}")))
        .namespace(fx::NamespaceEntry::new("default", [server]))
        .write();
    let output = run_check(&check_args(Some(&config.path_str()), None, None, &[])).await;
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    let error = error_with_code(&body, "credential_resolution_failed");
    assert_eq!(error.get("server").and_then(Value::as_str), Some(server));
    assert_eq!(error.get("key").and_then(Value::as_str), Some(key.as_str()));
}

#[tokio::test]
async fn check_fails_on_invalid_cwd() {
    let mut missing_cwd = std::env::temp_dir();
    missing_cwd.push(format!(
        "fanin-check-missing-cwd-{}-{}",
        std::process::id(),
        fx::phase3_unique_seq()
    ));
    let missing_cwd = missing_cwd.to_string_lossy().into_owned();
    let server = "bad-cwd";
    let config = fx::ConfigBuilder::new()
        .server_name(server)
        .namespace_servers([server])
        .cwd(&missing_cwd)
        .write();
    let output = run_check(&check_args(Some(&config.path_str()), None, None, &[])).await;
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    let error = error_with_code(&body, "upstream_connect_failed");
    assert_eq!(error.get("server").and_then(Value::as_str), Some(server));
}

#[tokio::test]
async fn check_fails_on_configured_tool_missing() {
    let server = "filtered";
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new(server))
        .namespace(
            fx::NamespaceEntry::new("default", [server])
                .with_tools(server, ["echo_ok", "absent_live_tool"]),
        )
        .write();
    let output = run_check(&check_args(Some(&config.path_str()), None, None, &[])).await;
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    let error = error_with_code(&body, "configured_tool_missing");
    assert_eq!(error.get("server").and_then(Value::as_str), Some(server));
    assert_eq!(
        error.get("tool").and_then(Value::as_str),
        Some("absent_live_tool")
    );
}

#[tokio::test]
async fn check_does_not_connect_namespace_excluded_server() {
    let allowed_log = fx::empty_log_file_path();
    let denied_log = fx::empty_log_file_path();
    let denied_pid_marker = fx::grandchild_marker_path();
    let config = fx::raw_config_file(&format!(
        "{}{}{}",
        raw_stdio_server("allowed", &[], None, Some(&allowed_log), None),
        raw_stdio_server(
            "excluded",
            &["--spawn-immediate-descendant", &denied_pid_marker],
            None,
            Some(&denied_log),
            None,
        ),
        namespace("default", &["allowed"], &[]),
    ));
    let output = run_check(&check_args(Some(&config.path_str()), None, None, &[])).await;
    assert!(
        output.status.is_some_and(|status| status.success()),
        "the allowed server is healthy: {}",
        output.stderr
    );
    let allowed = std::fs::read_to_string(&allowed_log).unwrap_or_default();
    let denied = std::fs::read_to_string(&denied_log).unwrap_or_default();
    assert!(
        allowed.contains("[allowed]"),
        "positive oracle: check must contact the namespace-allowed server; log: {allowed}"
    );
    assert!(
        !denied.contains("[excluded]"),
        "excluded server must produce no child log line; log: {denied}"
    );
    assert!(
        std::fs::metadata(&denied_pid_marker).is_err(),
        "excluded server must never spawn its probe descendant or produce its PID marker"
    );
}

#[tokio::test]
async fn check_server_filter_denied_fails_closed_without_connections() {
    let allowed_log = fx::empty_log_file_path();
    let denied_log = fx::empty_log_file_path();
    let denied_pid_marker = fx::grandchild_marker_path();
    let config = fx::raw_config_file(&format!(
        "{}{}{}",
        raw_stdio_server("allowed", &[], None, Some(&allowed_log), None),
        raw_stdio_server(
            "denied",
            &["--spawn-immediate-descendant", &denied_pid_marker],
            None,
            Some(&denied_log),
            None,
        ),
        namespace("default", &["allowed"], &[]),
    ));
    let output = run_check(&check_args(
        Some(&config.path_str()),
        None,
        None,
        &["--server", "denied"],
    ))
    .await;
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    assert!(
        errors(&body)
            .iter()
            .any(|error| error.get("server").and_then(Value::as_str) == Some("denied")),
        "the closed denial must identify the requested server: {body:?}"
    );
    assert!(
        std::fs::read_to_string(&allowed_log)
            .unwrap_or_default()
            .is_empty(),
        "a denied --server filter must not connect other allowed servers"
    );
    assert!(
        !std::fs::read_to_string(&denied_log)
            .unwrap_or_default()
            .contains("[denied]")
            && std::fs::metadata(&denied_pid_marker).is_err(),
        "a denied --server filter must not spawn the denied probe"
    );
}

#[tokio::test]
async fn serve_initialize_still_lazy_after_check_exists() {
    assert_check_command_available().await;
    let first_log = fx::empty_log_file_path();
    let second_log = fx::empty_log_file_path();
    let config = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("first").with_log_file(&first_log))
        .server(fx::ServerEntry::new("second").with_log_file(&second_log))
        .namespace(fx::NamespaceEntry::new("default", ["first", "second"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&config.path_str(), None).await;
    let fanin_pid = child.process_id();
    common::initialize(&mut child).await;
    let list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&list, "serve protocol tools/list after check exists");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        probe_child_pids(fanin_pid).is_empty(),
        "serve initialize + protocol tools/list must leave zero probe-server child PIDs"
    );
    for (name, path) in [("first", first_log), ("second", second_log)] {
        let log = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            !log.contains(&format!("[{name}]")),
            "serve initialize + protocol tools/list must not contact {name}; log: {log}"
        );
    }
    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn check_leaves_no_orphan_processes() {
    assert_check_command_available().await;
    let marker = fx::grandchild_marker_path();
    let config = fx::raw_config_file(&format!(
        "{}{}",
        raw_stdio_server(
            "hung",
            &["--hang-then-spawn-descendant", &marker],
            Some(DESCENDANT_TIMEOUT_SECS),
            None,
            None,
        ),
        namespace("default", &["hung"], &[]),
    ));
    let args = check_args(Some(&config.path_str()), None, None, &[]);
    let path = env!("CARGO_BIN_EXE_fanin-mcp");
    let mut command = tokio::process::Command::new(path);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().expect("check process must spawn");

    let started = Instant::now();
    while std::fs::metadata(&marker).is_err() && started.elapsed() < MARKER_READY {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let grandchild_pid = parse_pid_marker(&marker);
    assert!(
        process_is_alive(grandchild_pid),
        "N5 precondition: the probe grandchild PID must be alive while check is running"
    );

    let raw = tokio::time::timeout(CHECK_DEADLINE, child.wait_with_output())
        .await
        .expect("check must return after its configured upstream timeout")
        .expect("check output must be collectable");
    let output = common::CliOutput {
        stdout: String::from_utf8_lossy(&raw.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&raw.stderr).into_owned(),
        status: Some(raw.status),
    };
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    error_with_code(&body, "upstream_timeout");
    assert!(
        wait_for_process_death(grandchild_pid).await,
        "N5: after check returns, grandchild PID {grandchild_pid} must be dead within {CLEANUP_INTERVAL:?}; marker absence is not the oracle"
    );
    let _ = std::fs::remove_file(&marker);
}

#[tokio::test]
async fn check_tools_list_timeout_returns_without_hanging() {
    let config = fx::raw_config_file(&format!(
        "{}{}",
        raw_stdio_server(
            "slow-list",
            &["--hang-during-list-tools"],
            Some(2),
            None,
            None,
        ),
        namespace("default", &["slow-list"], &[]),
    ));
    let started = Instant::now();
    let output = run_check(&check_args(Some(&config.path_str()), None, None, &[])).await;
    let elapsed = started.elapsed();
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    let error = error_with_code(&body, "upstream_timeout");
    assert_eq!(
        error.get("server").and_then(Value::as_str),
        Some("slow-list")
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "a 2s configured timeout must return well inside the 15s process ceiling; took {elapsed:?}"
    );
}

#[tokio::test]
async fn check_json_and_logs_do_not_leak_resolved_secret() {
    let key = fx::phase3_env_var_name("CHECK_SECRET");
    let sentinel = fx::phase3_sentinel_value();

    let global_log = fx::empty_log_file_path();
    let server_log = fx::empty_log_file_path();
    let probe = toml_literal(&fx::probe_bin_path());
    let config = fx::raw_config_file(&format!(
        "[servers.secret-probe]\ntransport = \"stdio\"\ncommand = '{probe}'\nargs = []\nlog_file = '{}'\n[servers.secret-probe.env]\nTOKEN = \"${{{key}}}\"\n\n{}",
        toml_literal(&server_log),
        namespace("default", &["secret-probe"], &[]),
    ));
    let mut args = check_args(Some(&config.path_str()), None, Some(&global_log), &[]);
    args.splice(0..0, ["--credential-store".to_string(), "env".to_string()]);
    let output = run_check_with_env(&args, &[(key.as_str(), OsStr::new(&sentinel))]).await;
    assert!(
        output.status.is_some_and(|status| status.success()),
        "secret resolution is valid and check should succeed: {}",
        output.stderr
    );
    let global = std::fs::read_to_string(&global_log).unwrap_or_default();
    let server = std::fs::read_to_string(&server_log).unwrap_or_default();
    assert!(
        !output.stdout.contains(&sentinel)
            && !output.stderr.contains(&sentinel)
            && !global.contains(&sentinel)
            && !server.contains(&sentinel),
        "D-010 sentinel must be absent from check JSON, stderr, global --log-file, and per-server log"
    );
}

#[tokio::test]
async fn check_requires_config_and_emits_json_failure() {
    let output = run_check(&check_args(None, None, None, &[])).await;
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    assert!(
        body.get("servers")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty),
        "missing --config must fail before any server can spawn: {body:?}"
    );
}

#[tokio::test]
async fn check_fails_on_unknown_namespace_without_spawning() {
    let log = fx::empty_log_file_path();
    let config = fx::ConfigBuilder::new().log_file(&log).write();
    let output = run_check(&check_args(
        Some(&config.path_str()),
        Some("does-not-exist"),
        None,
        &[],
    ))
    .await;
    let body = parse_stdout_json(&output);
    assert_failure(&output, &body);
    assert!(
        !std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("[probe]"),
        "unknown namespace must fail validation before spawning the probe"
    );
}
