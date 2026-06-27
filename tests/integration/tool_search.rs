//! Phase 5 P4 — Claude Code Tool Search composition.

use std::time::Duration;

use serde_json::Value;

use crate::common;
use crate::common::expectations;
use crate::common::fixtures as fx;

#[tokio::test]
async fn downstream_tools_list_returns_exactly_three_meta_tools_and_does_not_spawn_upstreams() {
    let log = fx::empty_log_file_path();
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha").with_log_file(&log))
        .server(fx::ServerEntry::new("beta").with_log_file(&log))
        .namespace(fx::NamespaceEntry::new("default", ["alpha", "beta"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let response = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&response, "downstream tools/list composition");
    let tools = response
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("tools/list result missing tools array: {response:?}"));
    expectations::assert_exact_meta_tools(tools);
    for tool in tools {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
        assert!(
            expectations::META_TOOL_NAMES.contains(&name),
            "downstream tools/list must not expose upstream schemas at startup: {tools:?}"
        );
    }

    tokio::time::sleep(Duration::from_millis(250)).await;
    let upstream_log = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        upstream_log.trim().is_empty(),
        "downstream tools/list must not lazy-spawn upstreams; child stderr log was written: {upstream_log}"
    );
    child.into_guard().shutdown().await.ok();
}
