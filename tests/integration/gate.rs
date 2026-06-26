//! Phase 1 integration gate — wire-level test.
//!
//! Covers master Success Criteria 20 (full integration suite passes at 100%)
//! and Phase 6 sub-phase Success Criteria 1–5. This is the single
//! end-to-end test that exercises the FULL Phase 1 path in one session:
//! config load -> lazy spawn -> discovery -> schema -> invoke -> reverse
//! traffic -> stderr capture, then re-asserts the Phase 0 guarantees
//! (exactly three static meta-tools, conservative annotations, fast init,
//! no stdout diagnostics).
//!
//! A failure here means the phases do not compose — a regression in any
//! sub-phase surfaces at the gate. The per-area tests in the sibling modules
//! isolate the failure to its area; this test is the composition proof.

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::expectations as exp;
use crate::common::fixtures as fx;

/// Generous ceiling for the full-path test — it spawns the aggregator, loads
/// config, lazily spawns the probe, exercises discovery + schema + invoke +
/// reverse traffic, and reads the log file. 30s is well above any correct
/// impl and still catches a hang.
const GATE_DEADLINE: Duration = Duration::from_secs(30);

/// Parse list_tools rows from a CallToolResult (mirrors discovery.rs).
fn parse_list_tools_rows(result: &Value) -> Vec<Value> {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("list_tools result missing content array"));
    let text = content
        .iter()
        .find_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("list_tools result must carry a text content block"));
    let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("list_tools text content must be valid JSON; got: {text:?}\n{e}")
    });
    parsed
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("list_tools text content must be a JSON array; got: {parsed:?}"))
}

/// Master criterion 20 / P6.SC1: the full Phase 1 integration path passes
/// in a single session. This is the composition gate — every sub-phase's
/// observable behavior in one end-to-end run.
#[tokio::test]
async fn full_phase1_path_config_to_reverse_traffic_passes() {
    let log_path = fx::empty_log_file_path();
    let cfg = fx::ConfigBuilder::new().log_file(&log_path).write();
    let path_str = cfg.path_str();

    // 1. Config load + startup. Initialize must be fast (< 500ms, criterion 5)
    //    and open zero upstreams (criterion 11).
    let mut child = common::spawn_fanin_with_config(&path_str, None).await;
    let started = Instant::now();
    let init = timeout(Duration::from_secs(10), common::initialize(&mut child))
        .await
        .expect("initialize must complete within the gate deadline");
    assert!(
        init.get("serverInfo").is_some(),
        "initialize result must carry serverInfo"
    );
    let init_elapsed = started.elapsed();
    assert!(
        init_elapsed < Duration::from_millis(500),
        "initialize must return in under 500ms (criterion 5); took {init_elapsed:?}"
    );
    assert_eq!(
        init.get("serverInfo")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str()),
        Some("fanin-mcp"),
        "server must name itself fanin-mcp"
    );

    // 2. Downstream rmcp tools/list is static (criterion 4 / P6.SC2): exactly
    //    the three meta-tools, with conservative annotations on invoke_tool.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "gate: tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);
    let invoke_def = exp::find_tool(tools, "invoke_tool").unwrap();
    let ann = invoke_def
        .get("annotations")
        .expect("invoke_tool must carry annotations");
    assert_eq!(
        ann.get("destructiveHint").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        ann.get("readOnlyHint").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        ann.get("openWorldHint").and_then(|v| v.as_bool()),
        Some(true)
    );

    // 3. Lazy spawn: downstream tools/list must NOT have spawned the upstream.
    //    The log file must be empty of any probe line so far.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let log_before = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        !log_before.contains("probe"),
        "gate: downstream tools/list must not spawn the upstream (criterion 11); \
         log already has a probe line:\n{log_before}"
    );

    // 4. Live discovery: list_tools meta-tool returns the probe's five tool
    //    rows (criterion 6). This is the first meta-tool call -> lazy spawn.
    let list_resp = timeout(
        GATE_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("gate: list_tools must complete (lazy spawn + discovery)");
    common::assert_no_rpc_error(&list_resp, "gate: list_tools");
    let list_result = list_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("gate: list_tools returned no result"));
    let rows = parse_list_tools_rows(&list_result);
    let tool_names: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    for expected in [
        "echo_ok",
        "always_error",
        "slow_tool",
        "dangerous_noop",
        "needs_sampling",
    ] {
        assert!(
            tool_names.iter().any(|n| n == expected),
            "gate: list_tools must include probe tool `{expected}`; got: {tool_names:?}"
        );
    }

    // 5. After the lazy spawn, the log sink must contain a probe line
    //    (criterion 17 — child stderr captured with server prefix).
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log_after_spawn = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log_after_spawn.contains("probe"),
        "gate: child stderr must land in the log sink with the server name \
         (criterion 17); log contents:\n{log_after_spawn}"
    );

    // 6. Schema lookup: get_tool_schema probe__echo_ok returns the probe's
    //    input schema (criterion 7).
    let schema_resp = timeout(
        Duration::from_secs(10),
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "probe__echo_ok" }),
        ),
    )
    .await
    .expect("gate: get_tool_schema must complete");
    common::assert_no_rpc_error(&schema_resp, "gate: get_tool_schema");
    let schema_result = schema_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("gate: get_tool_schema returned no result"));
    if let Some(is_error) = schema_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "gate: get_tool_schema must succeed"
        );
    }

    // 7. Invoke end-to-end: invoke_tool probe__echo_ok returns the echoed
    //    input byte-faithfully (criterion 8, 9, 10).
    let echo_payload = "gate-echo-9d3f";
    let echo_resp = timeout(
        Duration::from_secs(10),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": echo_payload },
            }),
        ),
    )
    .await
    .expect("gate: invoke_tool echo_ok must complete");
    common::assert_no_rpc_error(&echo_resp, "gate: invoke_tool echo_ok");
    let echo_result = echo_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("gate: invoke_tool returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(is_error.as_bool(), Some(true), "gate: echo_ok must succeed");
    }
    let echo_text = echo_result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("gate: echo_ok missing content"))
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        echo_text.contains(echo_payload),
        "gate: echo_ok must round-trip the payload byte-faithfully; got: {echo_text:?}"
    );

    // 8. Reverse traffic: invoke_tool probe__needs_sampling completes within
    //    the deadline (criterion 16 / P6.SC3 — clean rejection, not a hang).
    let rev_resp = timeout(
        Duration::from_secs(10),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__needs_sampling",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("gate: needs_sampling must complete (reverse traffic handled, not hung)");
    common::assert_no_rpc_error(&rev_resp, "gate: needs_sampling");

    // 9. Phase 0 guarantees preserved AFTER the full Phase 1 exercise:
    //    downstream tools/list is still exactly the three static meta-tools
    //    (criterion 4 / P6.SC2), and the server is still healthy.
    let final_list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&final_list, "gate: final tools/list");
    let final_tools = final_list
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(final_tools);

    // 10. No stdout diagnostics: the whole session ran over stdout as
    //     JSON-RPC. Every line the harness read parsed as JSON (the harness
    //     panics on a non-JSON line). This is the implicit criterion 19
    //     assertion — a stray println! at any point in the gate would have
    //     panicked a prior read. No additional explicit assertion needed.

    child.into_guard().shutdown().await.ok();
}

/// P6.SC5: the Phase 3 `--credential-store` global flag is ACCEPTED — it
/// does not cause a clap rejection, and selects the preferred backend. The
/// earlier Phase-1 "no scope creep" guard (which asserted the flag was an
/// unknown rejection) is now invalid: Phase 3 legitimately adds
/// `--credential-store keyring|env` as a global flag (see `src/main.rs` and
/// `src/credentials.rs::CredentialStoreChoice`).
///
/// The observable: passing `--credential-store keyring` alongside a valid
/// `--config` lets the server start and answer `initialize` with a valid
/// `serverInfo` — i.e. clap parsed the flag and the server began serving,
/// rather than exiting non-zero on a parse error. The test does NOT assert
/// on any secret value (D-010); it only asserts the flag is structurally
/// accepted. Recorded as a boundary in `tests.md`.
#[tokio::test]
async fn credential_store_flag_is_accepted() {
    let cfg = fx::ConfigBuilder::new().write();
    let args = vec![
        "--config".to_string(),
        cfg.path_str(),
        "--credential-store".to_string(),
        "keyring".to_string(),
    ];
    let mut child = common::spawn_fanin_with_args(&args).await;

    // clap accepted the flag => the server started serving. A clap rejection
    // would have exited non-zero BEFORE serving, so `initialize` would hang
    // (no response) and the gate's deadline would fail the test. We assert
    // the positive: initialize returns a well-formed result with serverInfo.
    let init = timeout(Duration::from_secs(10), common::initialize(&mut child))
        .await
        .expect(
            "credential_store_flag_is_accepted: --credential-store keyring must be accepted \
             (server starts serving); clap rejection would have exited before serving",
        );
    assert!(
        init.get("serverInfo").is_some(),
        "credential_store_flag_is_accepted: initialize result must carry serverInfo \
         (--credential-store keyring accepted, server serving)"
    );

    child.into_guard().shutdown().await.ok();
}
