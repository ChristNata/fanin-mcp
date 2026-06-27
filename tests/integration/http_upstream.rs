//! Phase 5 P3 — Streamable-HTTP upstream + static header auth.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

const HTTP_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone, Default)]
struct HeaderSeen(Arc<Mutex<Vec<String>>>);

impl HeaderSeen {
    fn push(&self, value: String) {
        self.0.lock().unwrap().push(value);
    }

    fn contains(&self, expected: &str) -> bool {
        self.0.lock().unwrap().iter().any(|v| v == expected)
    }
}

async fn start_http_probe(expected_auth: String) -> (String, HeaderSeen) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback HTTP probe");
    let addr = listener.local_addr().expect("local addr");
    let seen = HeaderSeen::default();
    let seen_task = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let expected = expected_auth.clone();
            let seen = seen_task.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                for line in req.lines() {
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            seen.push(value.trim().to_string());
                        }
                    }
                }
                let status = if seen.contains(&expected) {
                    "200 OK"
                } else {
                    "401 Unauthorized"
                };
                let Some((_, body_text)) = req.split_once("\r\n\r\n") else {
                    return;
                };
                let Ok(request) = serde_json::from_str::<Value>(body_text) else {
                    return;
                };
                let id = request.get("id").cloned();
                let method = request.get("method").and_then(Value::as_str);
                let Some(id) = id else {
                    let status = if seen.contains(&expected) {
                        "202 Accepted"
                    } else {
                        "401 Unauthorized"
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                };
                let body = match method {
                    Some("initialize") => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "protocolVersion":"2024-11-05",
                            "capabilities":{"tools":{}},
                            "serverInfo":{"name":"http-probe","version":"0.0.0"}
                        }
                    }),
                    Some("tools/list") => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {"tools":[{"name":"echo_ok","description":"HTTP echo","inputSchema":{"type":"object","properties":{"message":{"type":"string"}}}}]}
                    }),
                    Some("tools/call") => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {"content":[{"type":"text","text":"http echo ok"}],"isError":false}
                    }),
                    _ => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "method not found"}
                    }),
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (format!("http://{addr}/mcp"), seen)
}

fn http_config(server: &str, endpoint: &str, header_value: &str, log: &str) -> fx::ConfigFile {
    let toml = format!(
        r#"
[servers.{server}]
transport = "streamable-http"
endpoint = "{endpoint}"
log_file = '{log}'

[servers.{server}.headers]
Authorization = "{header_value}"

[namespaces.default]
servers = ["{server}"]
"#
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

#[tokio::test]
async fn http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs() {
    let server = format!("http-{}", fx::phase3_unique_seq());
    let token_key = format!("FANIN_HTTP_TOKEN_{}", fx::phase3_unique_seq());
    let token = fx::phase3_sentinel_value();
    let expected = format!("Bearer {token}");
    let log = fx::empty_log_file_path();
    std::env::set_var(&token_key, &token);
    let (endpoint, seen) = start_http_probe(expected.clone()).await;
    let cfg = http_config(
        &server,
        &endpoint,
        &format!("Bearer ${{{token_key}}}"),
        &log,
    );

    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    let resp = timeout(
        HTTP_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_ok"),
                "arguments": { "message": "from-http" },
            }),
        ),
    )
    .await
    .expect("HTTP invoke must complete");
    common::assert_no_rpc_error(&resp, "HTTP invoke");
    let text = result_text(resp.get("result").expect("HTTP result"));
    assert!(
        text.contains("http echo ok"),
        "HTTP mock result must return: {text}"
    );
    assert!(
        seen.contains(&expected),
        "HTTP mock must observe the resolved Authorization header value"
    );
    child.into_guard().shutdown().await.ok();
    let logs = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !logs.contains(&token),
        "JSON/log output leaked HTTP token: {logs}"
    );
    std::env::remove_var(&token_key);
}

#[tokio::test]
async fn missing_http_header_credential_returns_structured_error_without_connecting() {
    let server = format!("http-{}", fx::phase3_unique_seq());
    let missing_key = format!("FANIN_HTTP_MISSING_{}", fx::phase3_unique_seq());
    std::env::remove_var(&missing_key);
    let expected = "Bearer should-not-arrive".to_string();
    let (endpoint, seen) = start_http_probe(expected).await;
    let log = fx::empty_log_file_path();
    let cfg = http_config(
        &server,
        &endpoint,
        &format!("Bearer ${{{missing_key}}}"),
        &log,
    );

    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    let resp = timeout(
        HTTP_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_ok"),
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("missing HTTP credential must fail promptly");
    common::assert_no_rpc_error(&resp, "missing HTTP credential");
    let result = resp.get("result").expect("result");
    common::assert_is_error_result(result, "missing HTTP credential");
    let text = result_text(result);
    let err: Value = serde_json::from_str(&text).expect("structured credential error JSON");
    assert_eq!(
        err.get("code").and_then(Value::as_str),
        Some("credential_resolution_failed"),
        "missing header credential must use the existing structured error shape: {err:?}"
    );
    assert!(
        !seen.contains("Bearer should-not-arrive"),
        "mock must not be contacted when header credential resolution fails"
    );
    child.into_guard().shutdown().await.ok();
}

#[tokio::test]
async fn stdio_upstream_still_lazy_and_namespace_filtered_after_http_support() {
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("stdio-a"))
        .server(fx::ServerEntry::new("stdio-b"))
        .namespace(fx::NamespaceEntry::new("default", ["stdio-a"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    let list = timeout(
        HTTP_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("stdio list_tools must still work");
    common::assert_no_rpc_error(&list, "stdio list_tools after HTTP support");
    let text = result_text(list.get("result").expect("list result"));
    assert!(
        text.contains("stdio-a"),
        "allowed stdio server must remain visible"
    );
    assert!(
        !text.contains("stdio-b"),
        "denied stdio server must remain hidden"
    );
    child.into_guard().shutdown().await.ok();
}
