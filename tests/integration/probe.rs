//! Probe-server fixture — Phase 0 wire-level tests.
//!
//! Covers master Success Criteria 6 (probe build/run), 7 (probe inventory),
//! and 8 (probe behavior), plus P0.3 Phase Success Criteria. The probe is an
//! in-repo rmcp stdio binary (D-016) that later integration tests use as a
//! stand-in for real upstreams; Phase 0 proves it builds and behaves.
//!
//! Every test spawns the probe over stdio with no Node/npx (master.md §P0.3
//! Key Behaviors). needs_sampling sends an outbound request that nothing in
//! Phase 0 answers — bounded by RPC_DEADLINE so it never hangs (GOTCHA #2).

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::time::timeout;

use crate::common;

/// The exact, ordered set of probe tool names (master criterion 7). Phase 3
/// extends the probe with `echo_env` (env isolation proof) and
/// `spawn_grandchild` (hard-kill orphan proof), bringing the total to 10.
const PROBE_TOOL_NAMES: [&str; 10] = [
    "echo_ok",
    "always_error",
    "slow_tool",
    "dangerous_noop",
    "needs_sampling",
    "echo_image",
    "needs_elicitation",
    "needs_roots",
    "echo_env",
    "spawn_grandchild",
];

fn find_probe_tool<'a>(tools: &'a [Value], name: &str) -> Option<&'a Value> {
    tools
        .iter()
        .find(|t| t.get("name").and_then(|n| n.as_str()) == Some(name))
}

/// Criterion 6 (Probe build/run gate): the probe fixture builds and runs
/// standalone over stdio with no Node or npx. This test spawns it via the
/// cargo-injected path and proves the binary answers initialize — the build
/// itself is enforced by the cargo build/clippy gate (criterion 1), and the
/// no-Node requirement is structural (the probe is a Rust bin target).
#[tokio::test]
async fn probe_builds_and_runs_over_stdio_without_node() {
    let mut child = common::spawn_bin("probe-server").await;
    let init = common::initialize(&mut child).await;
    assert!(
        init.get("serverInfo").is_some(),
        "probe initialize result must carry serverInfo"
    );

    // A reachable tools/list proves the probe is a live stdio MCP server, not
    // just a buildable binary.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "probe tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("probe tools/list result.tools must be an array"));
    assert!(!tools.is_empty(), "probe must expose at least one tool");

    child.into_guard().shutdown().await.ok();
}

/// Criterion 7 (Probe inventory gate): the probe exposes exactly ten tools
/// over stdio: echo_ok, always_error, slow_tool, dangerous_noop,
/// needs_sampling, echo_image, needs_elicitation, needs_roots (D-016,
/// master.md §P0.3).
#[tokio::test]
async fn probe_exposes_exactly_eight_named_tools() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "probe tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("probe tools/list result.tools must be an array"));

    assert_eq!(
        tools.len(),
        10,
        "probe must expose exactly 10 tools, got {}: {tools:?}",
        tools.len()
    );
    for name in PROBE_TOOL_NAMES {
        assert!(
            find_probe_tool(tools, name).is_some(),
            "probe is missing the `{name}` tool"
        );
    }
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    let mut expected = PROBE_TOOL_NAMES.to_vec();
    expected.sort();
    assert_eq!(
        sorted, expected,
        "probe returned unexpected tool names: {names:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Criterion 8 (Probe behavior gate, echo_ok): echo_ok echoes the supplied
/// input in a successful tool result (master.md §P0.3 Key Behaviors).
#[tokio::test]
async fn probe_echo_ok_returns_supplied_input() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    let payload = serde_json::json!({ "message": "probe-payload-7c9f" });
    let resp = common::call_tool(&mut child, "echo_ok", payload.clone()).await;
    common::assert_no_rpc_error(&resp, "probe echo_ok");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("probe echo_ok returned no result field"));

    // Success path: isError is false or absent (never true).
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "echo_ok must not set isError:true on a successful echo"
        );
    }
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("probe echo_ok result missing content array"));
    assert!(!content.is_empty(), "echo_ok must return content");

    // The echoed input must appear in the text content — a stub that returns
    // empty content fails here. We search for the payload substring; the exact
    // framing is not part of the public contract.
    let text: String = content
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        text.contains("probe-payload-7c9f"),
        "echo_ok must echo the supplied input; got text: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Criterion 8 (Probe behavior gate, always_error): always_error returns a
/// structured tool result with isError:true (D-005, master.md §P0.3). Asserts
/// the result is a tool-level error, NOT a JSON-RPC error.
#[tokio::test]
async fn probe_always_error_returns_structured_is_error_result() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    let resp = common::call_tool(&mut child, "always_error", serde_json::json!({})).await;
    // The load-bearing assertion: tool-level failure stays in the conversation
    // as a structured CallToolResult, never surfaces as a JSON-RPC error
    // (D-005, GOTCHA #3). A probe that returned a JSON-RPC error fails here.
    common::assert_no_rpc_error(&resp, "probe always_error");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("probe always_error returned no result field"));
    common::assert_is_error_result(&result, "probe always_error");

    child.into_guard().shutdown().await.ok();
}

/// Criterion 8 (Probe behavior gate, slow_tool): slow_tool honors a short
/// requested delay (~100ms). Asserts the wall-clock elapsed time reflects the
/// delay — a stub that returns instantly fails. The upper bound is loose to
/// avoid CI flake; the lower bound is the real assertion.
#[tokio::test]
async fn probe_slow_tool_honors_requested_delay() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    let requested_ms = 100u64;
    let started = Instant::now();
    let resp = common::call_tool(
        &mut child,
        "slow_tool",
        serde_json::json!({ "delay_ms": requested_ms }),
    )
    .await;
    common::assert_no_rpc_error(&resp, "probe slow_tool");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(requested_ms),
        "slow_tool must honor the requested delay of {requested_ms}ms; returned in {elapsed:?}"
    );
    // Upper bound: generous enough for CI, tight enough to catch a hang-masking
    // stub that sleeps far longer than requested.
    assert!(
        elapsed < Duration::from_secs(3),
        "slow_tool should not take dramatically longer than requested; took {elapsed:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Criterion 8 (Probe behavior gate, dangerous_noop): dangerous_noop is
/// harmless but advertises destructive annotations (master.md §P0.3 Key
/// Behaviors, D-006). Asserts destructiveHint=true is present on the tool
/// definition.
#[tokio::test]
async fn probe_dangerous_noop_advertises_destructive_annotations() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "probe tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("probe tools/list result.tools must be an array"));
    let tool = find_probe_tool(tools, "dangerous_noop")
        .unwrap_or_else(|| panic!("probe missing dangerous_noop tool"));

    let annotations = tool
        .get("annotations")
        .unwrap_or_else(|| panic!("dangerous_noop must carry annotations"));
    let destructive = annotations
        .get("destructiveHint")
        .unwrap_or_else(|| panic!("dangerous_noop missing destructiveHint"));
    assert_eq!(
        destructive.as_bool(),
        Some(true),
        "dangerous_noop must advertise destructiveHint=true"
    );

    child.into_guard().shutdown().await.ok();
}

/// Criterion 8 (Probe behavior gate, needs_sampling): needs_sampling SENDS a
/// sampling/createMessage request on the wire. Nothing in Phase 0 answers it
/// (master.md §Out: no reverse-traffic handling; that starts in Phase 1), so
/// the test observes the OUTBOUND request directly rather than waiting for a
/// response. Bounded by a 5s deadline so the probe never hangs.
///
/// The probe, as the server side of its stdio connection, emits a
/// sampling/createMessage REQUEST (with an id) toward its client. We send the
/// tools/call request (write-only, no borrow held), then read raw messages off
/// the wire looking for the outbound sampling request. We do NOT wait for the
/// tools/call response (the probe awaits our sampling response, which we never
/// send); the child is force-killed on drop at the end of the test.
#[tokio::test]
async fn probe_needs_sampling_sends_sampling_create_message_on_wire() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    // Send the tools/call request that triggers the outbound sampling request.
    // send_request writes and returns without holding a borrow, so we can read
    // raw messages immediately afterward.
    let _call_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "needs_sampling",
                "arguments": {},
            }),
        )
        .await;

    // Read messages off the wire for up to 5s, looking for the outbound
    // sampling/createMessage request. A request has an `id` and method
    // `sampling/createMessage`; a response has an `id` and a `result`/`error`;
    // a notification has no `id`.
    let deadline = Duration::from_secs(5);
    let observed = timeout(deadline, async {
        loop {
            let msg = child.read_next_message().await;
            let method = msg.get("method").and_then(|m| m.as_str());
            let has_id = msg.get("id").is_some();
            if method == Some("sampling/createMessage") && has_id {
                return msg;
            }
            // Keep scanning — tools/call responses, notifications, etc.
        }
    })
    .await;

    assert!(
        observed.is_ok(),
        "needs_sampling must send a sampling/createMessage request on the wire \
         within {deadline:?}; observed no such request"
    );
    let msg = observed.unwrap();
    assert_eq!(
        msg.get("method").and_then(|m| m.as_str()),
        Some("sampling/createMessage"),
        "observed message must be sampling/createMessage"
    );
    assert!(
        msg.get("params").is_some(),
        "sampling/createMessage request must carry params"
    );

    child.into_guard().shutdown().await.ok();
}

/// P0.3 Phase Success Criterion: each of the five probe tools is reachable
/// over stdio (calling each returns a well-formed CallToolResult, success or
/// structured error — never a hang and never a JSON-RPC error). Complements
/// the per-behavior tests above with a reachability sweep.
#[tokio::test]
async fn probe_all_five_tools_reachable_over_stdio() {
    let mut child = common::spawn_bin("probe-server").await;
    common::initialize(&mut child).await;

    // needs_sampling is exercised by its dedicated test; here we confirm the
    // other four are reachable with a bounded call. slow_tool uses a small
    // delay to keep the test fast.
    let calls: [(&str, Value); 4] = [
        ("echo_ok", serde_json::json!({ "message": "reach" })),
        ("always_error", serde_json::json!({})),
        ("slow_tool", serde_json::json!({ "delay_ms": 20 })),
        ("dangerous_noop", serde_json::json!({})),
    ];

    for (name, args) in calls {
        let resp = common::call_tool(&mut child, name, args).await;
        common::assert_no_rpc_error(
            &resp,
            &format!(
                "probe {name} must be reachable and return a tool result, not a JSON-RPC error"
            ),
        );
        let result = resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("probe {name} returned no result field"));
        assert!(
            result.get("content").is_some(),
            "probe {name} result must carry content"
        );
    }

    child.into_guard().shutdown().await.ok();
}
