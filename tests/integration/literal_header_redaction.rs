//! Phase 6 OSS-readiness — literal-secret header redaction (H-3).
//!
//! This test BITES the H-3 contract: a *literal* (non-`${VAR}`) secret header
//! value must be registered for redaction unconditionally. To observe redaction
//! end-to-end, the loopback HTTP probe responds to the `tools/call` with a
//! Streamable-HTTP **SSE** stream that carries a `notifications/message` whose
//! `data` echoes the `Authorization` value the upstream received, followed by
//! the tool result. fanin-mcp's `forward.rs::on_logging_message` redacts that
//! notification through `process::redact` before writing it to the per-server
//! log file. With H-3 the echoed `Bearer <secret>` is registered, so the log
//! line shows `[REDACTED]`; WITHOUT H-3 (literal value not registered) the raw
//! value would appear — so this test fails if `registry.rs`'s
//! `register_secret(&resolved)` for headers were removed or re-guarded.

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

/// Loopback Streamable-HTTP probe. Plain-JSON responses for `initialize` /
/// `tools/list`; an SSE stream for `tools/call` that first sends a logging
/// notification echoing the observed `Authorization` value, then the result.
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
                let mut observed_auth = String::new();
                for line in req.lines() {
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            observed_auth = value.trim().to_string();
                            seen.push(observed_auth.clone());
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

                // `tools/call` → SSE: a logging notification that echoes the
                // observed Authorization value (so the registered literal header
                // value flows through forward.rs's redacted log path), then the
                // tool result.
                if method == Some("tools/call") {
                    let notif = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/message",
                        "params": {
                            "level": "info",
                            "logger": "http-probe",
                            "data": format!("upstream received authorization header: {observed_auth}")
                        }
                    })
                    .to_string();
                    let result = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"content":[{"type":"text","text":"http echo ok"}],"isError":false}
                    })
                    .to_string();
                    let sse = format!("data: {notif}\n\ndata: {result}\n\n");
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{sse}"
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                let body = match method {
                    Some("initialize") => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {
                            "protocolVersion":"2024-11-05",
                            "capabilities":{"tools":{},"logging":{}},
                            "serverInfo":{"name":"http-probe","version":"0.0.0"}
                        }
                    }),
                    Some("tools/list") => serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": id,
                        "result": {"tools":[{"name":"echo_ok","description":"HTTP echo","inputSchema":{"type":"object","properties":{"message":{"type":"string"}}}}]}
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
/// Behavioral redaction proof for a LITERAL header secret (H-3): the value is
/// registered unconditionally and is `[REDACTED]` when it reaches a log line.
async fn literal_secret_header_value_is_redacted_in_logs() {
    let secret = fx::phase3_sentinel_value();
    let expected = format!("Bearer {secret}");
    let log = fx::empty_log_file_path();

    // Reachable loopback HTTP probe so the literal header is actually transmitted
    // and echoed back through the upstream logging-notification path.
    let (endpoint, seen) = start_http_probe(expected.clone()).await;

    let server = format!("http-literal-{}", fx::phase3_unique_seq());
    let cfg = http_literal_config(&server, &endpoint, &expected, &log);

    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

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
    assert!(
        seen.contains(&expected),
        "probe must observe the literal Authorization header value"
    );

    // The upstream logging notification -> redacted-log write is asynchronous:
    // forward.rs routes it to an mpsc-backed, per-line-flushed writer task in the
    // child. Poll the log WHILE THE CHILD IS ALIVE until the redaction marker
    // lands, so the assertion does not race the async write (it raced on macOS
    // CI with a single immediate read). The writer flushes per line, so once the
    // marker is present it is durable.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut logs = String::new();
    while std::time::Instant::now() < deadline {
        logs = std::fs::read_to_string(&log).unwrap_or_default();
        if logs.contains("[REDACTED]") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    child.into_guard().shutdown().await.ok();

    // With H-3 the registered literal `Bearer <secret>` is redacted; without it
    // the raw value would appear and the marker would never land (the poll then
    // times out and this assertion fails — the test still bites).
    assert!(
        logs.contains("[REDACTED]"),
        "redaction marker must appear for the literal header value echoed via the upstream logging notification; log:\n{logs}"
    );
    assert!(
        !logs.contains(&secret),
        "literal secret header value must be redacted, not leaked; log:\n{logs}"
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
