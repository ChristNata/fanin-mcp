//! Phase 6 OSS-readiness — literal-secret header redaction (H-3).
//
//! The test mirrors the structure of the existing
//! `http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs`
//! but supplies a *literal* secret (no `${VAR}`) so that the regression
//! fixed by unconditional registration is exercised.

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
                    _ => {
                        serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}})
                    }
                };
                let body_bytes = serde_json::to_vec(&body).unwrap();
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body_bytes.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&body_bytes).await;
            });
        }
    });
    (format!("http://{addr}"), seen)
}

#[tokio::test]
async fn literal_secret_header_value_is_registered_for_redaction() {
    let secret = fx::phase3_sentinel_value();
    let expected = format!("Bearer {secret}");
    let log = fx::empty_log_file_path();

    // Reachable loopback HTTP probe so the literal header is actually transmitted.
    let (endpoint, _seen) = start_http_probe(expected.clone()).await;

    let server = format!("http-literal-{}", fx::phase3_unique_seq());
    let cfg = http_literal_config(&server, &endpoint, &expected, &log);

    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Trigger lazy spawn + a tool call so the header is processed and logged.
    let resp = timeout(
        HTTP_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_ok"),
                "arguments": { "message": "literal-header" },
            }),
        ),
    )
    .await
    .expect("literal-header invoke must complete");

    common::assert_no_rpc_error(&resp, "literal-header invoke");
    child.into_guard().shutdown().await.ok();

    // The literal secret must never appear raw in any log output.
    // This is the observable side-effect of H-3: the literal value is registered
    // for redaction at header-registration time, exactly as templated values are.
    let logs = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !logs.contains(&secret),
        "literal secret header value must be redacted; log contained sentinel:\n{logs}"
    );
}

/// Minimal HTTP config with a literal (non-templated) Authorization header.
fn http_literal_config(
    server: &str,
    endpoint: &str,
    header_value: &str,
    log: &str,
) -> fx::ConfigFile {
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
