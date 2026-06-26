//! End-to-end invoke_tool forwarding — Phase 1 wire-level tests.
//!
//! Covers master Success Criteria 8 (invoke_tool parses on first `__`,
//! forwards raw arguments, returns probe__echo_ok output), 9 (upstream
//! success/error results pass through as CallToolResult; tool-level failures
//! are isError:true, not JSON-RPC errors), 10 (content arrays never
//! stringified; non-text blocks stay structured), and 12 (concurrent first
//! calls spawn exactly once — covered in registry.rs), plus Phase 5
//! sub-phase Success Criteria 1–6.
//!
//! The probe's `echo_ok` echoes its input; `always_error` returns a
//! structured isError:true; `dangerous_noop` is a success no-op. These
//! tests exercise the full forward path: downstream invoke_tool -> parse ->
//! namespace check -> lazy connect -> raw-arg forward -> byte-faithful
//! result.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for an invoke_tool call. The first call may spawn the probe.
const INVOKE_DEADLINE: Duration = Duration::from_secs(15);

/// Helper: spawn the aggregator with the canonical Phase 1 config and
/// initialize. Returns the live child.
async fn phase1_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    child
}

/// Extract the joined text of a CallToolResult's content array.
fn result_text(result: &Value) -> String {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array"));
    content
        .iter()
        .filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Master criterion 8 / P5.SC1: `invoke_tool probe__echo_ok` returns the
/// probe's success result. The observable effect is a non-error CallToolResult
/// whose text content contains the echoed input.
#[tokio::test]
async fn invoke_tool_probe_echo_ok_returns_probe_success() {
    let mut child = phase1_child().await;

    let payload = "echo-payload-4f2a";
    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__echo_ok must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__echo_ok");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "echo_ok success must not set isError:true"
        );
    }
    let text = result_text(&result);
    assert!(
        text.contains(payload),
        "invoke_tool probe__echo_ok must echo the supplied input; got text: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 8 / P5.SC2: the EXACT raw arguments sent to
/// `probe__echo_ok` are visible in the echoed result without proxy-side
/// transformation. The probe's `echo_ok` echoes the `message` field verbatim.
/// We send a message with structural characters that a naive proxy might
/// mangle (quotes, braces, unicode) and assert it round-trips unchanged.
#[tokio::test]
async fn invoke_tool_forwards_raw_arguments_unchanged() {
    let mut child = phase1_child().await;

    // A message with characters that would break under naive re-serialization
    // or stringification: nested quotes, braces, a non-ASCII char, a newline
    // escape. The probe echoes the `message` string verbatim.
    let payload = r#"{"k":"v\"\\","n":[1,2,3]} — héllo — \n"#;
    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("invoke_tool raw-arg round-trip must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool raw-arg");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool raw-arg returned no result"));
    let text = result_text(&result);
    assert!(
        text.contains(payload),
        "raw arguments must round-trip unchanged (D-004); expected `{payload}` \
         in text: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 9 / P5.SC3: `invoke_tool` against a probe error tool
/// (`always_error`) returns `isError: true` content from the upstream, NOT a
/// JSON-RPC error. The upstream's structured error must pass through
/// byte-faithfully (D-005, GOTCHA #3).
#[tokio::test]
async fn invoke_tool_probe_always_error_returns_upstream_is_error_content() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__always_error",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__always_error must complete");
    // The load-bearing assertion: tool-level failure stays in the
    // conversation as a structured CallToolResult, never as a JSON-RPC error.
    common::assert_no_rpc_error(&resp, "invoke_tool probe__always_error");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool always_error returned no result"));
    common::assert_is_error_result(&result, "invoke_tool probe__always_error");

    // The probe's always_error payload includes a `code: "always_error"` JSON
    // body. Assert it round-trips through the proxy byte-faithfully — the
    // text content must contain the probe's error code.
    let text = result_text(&result);
    assert!(
        text.contains("always_error"),
        "upstream error content must pass through byte-faithfully; \
         expected `always_error` in text: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 8 / P5.SC4: tool names containing additional `__` after
/// the server delimiter are treated as part of the upstream tool name.
/// Parsing splits on the FIRST `__` only (GOTCHA #15). The probe has no tool
/// named `echo__ok` (with a double-underscore), so this must return a
/// structured "tool not found" error — proving the split kept `echo__ok` as
/// the tool name rather than splitting into `probe` / `echo` / `ok`.
#[tokio::test]
async fn invoke_tool_splits_on_first_double_underscore_only() {
    let mut child = phase1_child().await;

    // `probe__echo__ok` splits on the FIRST `__`: server=`probe`,
    // tool=`echo__ok`. The probe has no `echo__ok` tool, so this is a
    // structured "tool not found" error. If the proxy split on EVERY `__`,
    // it would try server=`probe`, tool=`echo` (also not found) — the error
    // message would differ, but the load-bearing observable is that the call
    // does not crash and returns a structured error.
    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo__ok",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool with extra __ must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__echo__ok");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool probe__echo__ok returned no result"));
    common::assert_is_error_result(&result, "invoke_tool probe__echo__ok");

    // The error content should reference the full tool name `echo__ok`, not
    // a truncated `echo`. This is the direct assertion that the split kept
    // the trailing `__ok` as part of the tool name.
    let text = result_text(&result);
    assert!(
        text.contains("echo__ok"),
        "the structured error must reference the full upstream tool name \
         `echo__ok` (split on first __ only, GOTCHA #15); got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 9 / P5.SC5: a denied/unknown namespace/server/tool
/// failure returns structured `isError: true` content, not a JSON-RPC error.
/// This covers the unknown-SERVER path (a server not in the config).
#[tokio::test]
async fn invoke_tool_unknown_server_returns_structured_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "no_such_server__echo_ok",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool unknown server must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool unknown server");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool unknown server returned no result"));
    common::assert_is_error_result(&result, "invoke_tool unknown server");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 9 / P5.SC5: an unknown TOOL on a known server returns
/// structured `isError: true` content.
#[tokio::test]
async fn invoke_tool_known_server_unknown_tool_returns_structured_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__does_not_exist",
                "arguments": {},
            }),
        )
        ,
    )
    .await
    .expect("invoke_tool unknown tool must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool unknown tool");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool unknown tool returned no result"));
    common::assert_is_error_result(&result, "invoke_tool unknown tool");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 8 edge / P5.SC1: `invoke_tool` with a malformed name (no
/// `__` delimiter) returns a structured error, not a JSON-RPC error and not
/// a hang. The parse must fail fast.
#[tokio::test]
async fn invoke_tool_name_without_delimiter_returns_structured_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "echo_ok",
                "arguments": {},
            }),
        )
        ,
    )
    .await
    .expect("invoke_tool without delimiter must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool no delimiter");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool no delimiter returned no result"));
    common::assert_is_error_result(&result, "invoke_tool no delimiter");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 8 edge / P5.SC1: `invoke_tool` with an empty name returns
/// a structured error (boundary — empty input).
#[tokio::test]
async fn invoke_tool_empty_name_returns_structured_error() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool empty name must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool empty name");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool empty name returned no result"));
    common::assert_is_error_result(&result, "invoke_tool empty name");

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 10 / P5.SC6: non-text content blocks returned by an
/// upstream fixture are preserved as structured content blocks, NOT
/// stringified (D-004, GOTCHA #4).
///
/// **COVERAGE GAP — DEFERRED.** The current probe fixture has NO tool that
/// returns a non-text content block (every probe tool returns
/// `Content::text(...)`). Asserting byte-faithful non-text preservation
/// requires a probe fixture update: a new tool (e.g. `echo_image`) that
/// returns a `Content::image(...)` or `Content::embedded_resource(...)` block.
/// This test is `#[ignore]`'d until that fixture lands. The orchestrator
/// should route a probe-fixture update to add such a tool; see `tests.md`
/// §Gaps.
#[tokio::test]
async fn invoke_tool_preserves_non_text_content_block_not_stringified() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_image",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__echo_image must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__echo_image");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool echo_image returned no result"));
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("echo_image result missing content array"));

    // The load-bearing assertion: at least one content block has a non-text
    // `type` (e.g. "image", "resource", "embedded_resource"). A proxy that
    // stringified the content array would produce only text blocks.
    let has_non_text = content
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) != Some("text"));
    assert!(
        has_non_text,
        "non-text content block must be preserved as structured content, not \
         stringified (D-004 / GOTCHA #4); got only text blocks: {content:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 10 edge: a successful result with MULTIPLE text content
/// blocks passes through with all blocks intact (not collapsed to one). The
/// probe's `echo_ok` returns a single text block, so this asserts the
/// multi-block path is not corrupted by asserting the content array length
/// matches what the probe sent. A proxy that `to_string()`'d the array would
/// collapse it to one block.
///
/// NOTE: the probe's `echo_ok` returns exactly one text block, so this test
/// effectively asserts "the single block arrives intact." A true multi-block
/// assertion needs a probe tool that returns multiple blocks; recorded as a
/// boundary in `tests.md`. The byte-faithful single-block pass-through is
/// still a meaningful assertion (a stringifying proxy would corrupt even one
/// block's framing).
#[tokio::test]
async fn invoke_tool_preserves_content_array_structure() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__echo_ok",
                "arguments": { "message": "structure-check" },
            }),
        ),
    )
    .await
    .expect("invoke_tool structure check must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool structure check");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool structure check returned no result"));
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array"));

    // The probe returns exactly one text block for echo_ok. Assert it arrives
    // as a well-formed content block with `type: "text"` and a string `text`.
    assert!(
        !content.is_empty(),
        "content array must not be empty after byte-faithful forward"
    );
    let block = &content[0];
    assert_eq!(
        block.get("type").and_then(|t| t.as_str()),
        Some("text"),
        "content block type must be preserved as `text`; got: {block:?}"
    );
    assert!(
        block.get("text").and_then(|t| t.as_str()).is_some(),
        "content block `text` field must be a string; got: {block:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 9 / P5 edge: a successful upstream tool that is NOT
/// echo_ok (`dangerous_noop`) passes through its success result
/// byte-faithfully. This widens the success-path coverage beyond echo_ok.
#[tokio::test]
async fn invoke_tool_probe_dangerous_noop_returns_success() {
    let mut child = phase1_child().await;

    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__dangerous_noop",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__dangerous_noop must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__dangerous_noop");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool dangerous_noop returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "dangerous_noop success must not set isError:true"
        );
    }
    let text = result_text(&result);
    assert!(
        text.contains("dangerous_noop"),
        "dangerous_noop result must pass through byte-faithfully; got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master criterion 8 / P5 edge: `invoke_tool` against `probe__slow_tool`
/// honors the requested delay — the wall-clock elapsed time reflects the
/// delay. A proxy that swallowed the arguments or returned instantly fails.
#[tokio::test]
async fn invoke_tool_probe_slow_tool_honors_delay() {
    let mut child = phase1_child().await;

    let requested_ms = 150u64;
    let started = std::time::Instant::now();
    let resp = timeout(
        INVOKE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__slow_tool",
                "arguments": { "delay_ms": requested_ms },
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__slow_tool must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__slow_tool");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(requested_ms),
        "slow_tool must honor the requested delay of {requested_ms}ms through \
         the proxy; returned in {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "slow_tool should not take dramatically longer than requested through \
         the proxy; took {elapsed:?}"
    );

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("slow_tool returned no result"));
    let text = result_text(&result);
    assert!(
        text.contains(&requested_ms.to_string()),
        "slow_tool result must pass through byte-faithfully; expected \
         `{requested_ms}` in text: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}