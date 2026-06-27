//! Phase 6 OSS-readiness — literal-secret header redaction (H-3).
//
//! The test mirrors the structure of the existing
//! `http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs`
//! but supplies a *literal* secret (no `${VAR}`) so that the regression
//! fixed by unconditional registration is exercised.

use std::time::Duration;

use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

const HTTP_DEADLINE: Duration = Duration::from_secs(15);

#[tokio::test]
async fn literal_secret_header_value_is_registered_for_redaction() {
    let server = format!("http-literal-{}", fx::phase3_unique_seq());
    let secret = fx::phase3_sentinel_value();
    let expected = format!("Bearer {secret}");
    let log = fx::empty_log_file_path();

    // Literal header value — no ${...} interpolation template.
    let cfg = http_literal_config(&server, &expected, &log);

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

    // The literal secret must be redacted in the log file.
    let logs = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !logs.contains(&secret),
        "literal secret header value must be redacted; log contained sentinel:\n{logs}"
    );
    assert!(
        logs.contains("[REDACTED]"),
        "redaction marker must appear for literal secret header"
    );
}

/// Minimal HTTP config with a literal (non-templated) Authorization header.
fn http_literal_config(server: &str, header_value: &str, log: &str) -> fx::ConfigFile {
    let toml = format!(
        r#"
[servers.{server}]
transport = "streamable-http"
endpoint = "http://127.0.0.1:9"   # unreachable — we only need lazy spawn + redaction
log_file = '{log}'

[servers.{server}.headers]
Authorization = "{header_value}"

[namespaces.default]
servers = ["{server}"]
"#
    );
    fx::raw_config_file(&toml)
}
