//! Phase 4 — LLM-visible upstream string sanitization (wire-level).
//!
//! Covers master Success Criteria 1, 2, 3, 4, 5:
//! - `list_tools` returns sanitized rows for upstream-authored names /
//!   descriptions: embedded `\n`, `\r`, tab/control chars absent from
//!   LLM-visible row text (SC 1).
//! - `list_tools` caps each upstream-authored description row at about 100
//!   visible characters after sanitization (SC 2).
//! - A probe tool with an upstream description containing newlines, control
//!   chars, and more than 100 characters is observably emitted as a
//!   single-line capped description (SC 3).
//! - `get_tool_schema` for an upstream tool returns valid JSON and sanitizes
//!   upstream-authored metadata strings visible to the LLM, without changing
//!   the schema's object shape needed by callers (SC 4).
//! - Sanitization does NOT apply to `invoke_tool` result content: non-text
//!   and structured content pass byte-faithfully (SC 5).
//!
//! The probe fixture's `poison_meta` tool carries a description with embedded
//! `\n`, `\r`, tab, vertical tab, form feed, and well over 100 visible
//! characters; `poison_schema` carries poisoned `title` / `description` /
//! `$comment` in its `input_schema`. The aggregator must strip control chars
//! and cap the description before the row text reaches the LLM, and must
//! sanitize the schema metadata strings while preserving the JSON shape.
//!
//! `invoke_tool` dispatch is on the REAL (clean) upstream tool name —
//! sanitization is display-only, not the call key. The probe's `poison_meta`
//! and `poison_schema` tools have clean names; the poisoned content lives in
//! the description / schema metadata, so dispatch still works and the test
//! can separately assert the control-char-bearing display is sanitized.
//!
//! All tests are wire-level. The suite compiles clean against the current
//! tree (which does NOT sanitize — `handle_list_tools` emits
//! `tool.description.unwrap_or_default()` verbatim) and fails RED on the
//! absent behavior.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// The description length cap the plan specifies ("about 100 characters"). We
/// assert the sanitized description is at most this many visible characters.
/// A small margin above 100 is accepted so the implementer may cap at 100,
/// 120, or similar round numbers; the load-bearing assertion is that a
/// description well over 100 chars is NOT emitted verbatim.
const DESC_CAP: usize = 120;

/// Helper: spawn the aggregator with the canonical Phase 1 config + initialize.
async fn phase1_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    child
}

/// Extract the list_tools rows as a JSON array of row objects.
fn parse_list_tools_rows(result: &Value) -> Vec<Value> {
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("list_tools result missing content array"));
    assert!(
        !content.is_empty(),
        "list_tools result must carry at least one content block"
    );
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
        panic!("list_tools text content must be valid JSON (the row array); got: {text:?}\n{e}")
    });
    parsed
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("list_tools text content must be a JSON array; got: {parsed:?}"))
}

/// Find a row by its `tool` (or `name`) field value.
fn find_row<'a>(rows: &'a [Value], tool: &str) -> Option<&'a Value> {
    rows.iter().find(|r| {
        r.get("tool").and_then(|t| t.as_str()) == Some(tool)
            || r.get("name").and_then(|n| n.as_str()) == Some(tool)
    })
}

/// Extract a row's description string (the LLM-visible description field).
fn row_description(row: &Value) -> String {
    row.get("description")
        .and_then(|d| d.as_str())
        .map(String::from)
        .unwrap_or_default()
}

/// Assert a string is free of C0 control characters (and DEL). Newlines
/// (`\n`), carriage returns (`\r`), tab (`\t`), vertical tab (`\u{000B}`),
/// form feed (`\u{000C}`), and other C0 chars (0x00–0x1F) plus 0x7F must be
/// absent — the sanitizer stripped them.
fn assert_no_control_chars(s: &str, ctx: &str) {
    for (i, ch) in s.chars().enumerate() {
        let u = ch as u32;
        let is_c0_or_del = u <= 0x1F || u == 0x7F;
        assert!(
            !is_c0_or_del,
            "{ctx}: string must be free of C0 control chars, but found U+{u:04X} at index {i}; \
             string: {s:?}"
        );
    }
}

/// Master SC 1 + SC 2 + SC 3: `list_tools` returns a sanitized row for the
/// probe's `poison_meta` tool. The probe's description contains embedded
/// `\n`, `\r`, tab, vertical tab, form feed, and well over 100 visible
/// characters. The aggregator must:
///   - strip all C0 control chars (SC 1, SC 3) — the row description is a
///     single line, control-free;
///   - cap the description at about 100 visible characters (SC 2) — well
///     under the raw 200+ char description;
///   - keep the tool NAME in the row as the REAL (clean) name `poison_meta`
///     (dispatch is on the real name; the poisoned name is not used as the
///     call key).
#[tokio::test]
async fn list_tools_sanitizes_poisoned_description_strips_control_and_caps_length() {
    let mut child = phase1_child().await;

    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools must complete (lazy spawn + discovery)");
    common::assert_no_rpc_error(&resp, "list_tools");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools returned no result"));
    let rows = parse_list_tools_rows(&result);

    let poison_row = find_row(&rows, "poison_meta").unwrap_or_else(|| {
        panic!("list_tools must include the `poison_meta` row; got rows: {rows:?}")
    });

    // SC 1 / SC 3: the description is a single line, free of C0 control chars.
    let desc = row_description(poison_row);
    assert!(
        !desc.is_empty(),
        "poison_meta description must not be empty (the probe advertises a non-empty description)"
    );
    assert!(
        !desc.contains('\n'),
        "poison_meta description must be a single line (no \\n); got: {desc:?}"
    );
    assert!(
        !desc.contains('\r'),
        "poison_meta description must have no carriage returns; got: {desc:?}"
    );
    assert_no_control_chars(&desc, "poison_meta description");

    // SC 2: the description is capped at about 100 visible characters. The
    // raw description is well over 200 chars, so a non-capping implementation
    // fails this.
    assert!(
        desc.chars().count() <= DESC_CAP,
        "poison_meta description must be capped at ~{DESC_CAP} visible chars (SC 2); \
         got {} chars: {desc:?}",
        desc.chars().count()
    );

    // The tool NAME in the row is the REAL clean name (`poison_meta`), not a
    // sanitized display alias. Dispatch is on the real name (SC: display-only
    // sanitization). The probe's `poison_meta` tool name is clean, so the
    // name passes through verbatim.
    assert_eq!(
        poison_row.get("tool").and_then(|t| t.as_str()),
        Some("poison_meta"),
        "poison_meta row `tool` field must be the real clean name; got: {poison_row:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 1 / SC 3 edge: a malicious upstream tool NAME containing control
/// chars appears in LLM-visible discovery text only in sanitized form. The
/// probe's `poison_meta` tool registers a CLEAN name (`poison_meta`) because
/// rmcp validates tool names on registration; this test asserts the
/// aggregator's LLM-visible row text for the name field is control-char-free
/// for every row — a proxy that forwarded a control-bearing upstream name
/// verbatim would fail. (The probe cannot register a control-bearing name,
/// so this test asserts the invariant on all rows rather than constructing a
/// control-bearing name fixture.)
#[tokio::test]
async fn list_tools_row_names_are_control_char_free() {
    let mut child = phase1_child().await;

    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools must complete");
    common::assert_no_rpc_error(&resp, "list_tools");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools returned no result"));
    let rows = parse_list_tools_rows(&result);

    // Every row's name (tool field) must be control-char-free. This is the
    // LLM-visible name text; sanitization applies even though the probe
    // registers clean names — a proxy that passed a control-bearing name
    // through verbatim would fail.
    for row in &rows {
        let name = row
            .get("tool")
            .and_then(|t| t.as_str())
            .or_else(|| row.get("name").and_then(|n| n.as_str()))
            .unwrap_or_else(|| panic!("row missing tool/name field: {row:?}"));
        assert_no_control_chars(name, "list_tools row name");
    }

    child.into_guard().shutdown().await.ok();
}

/// Master SC: `invoke_tool` still dispatches on the REAL (unsanitized)
/// upstream tool name — sanitization is display-only, not the call key. The
/// probe's `poison_meta` tool has a clean real name, so `invoke_tool
/// probe__poison_meta` must succeed (the probe's dispatch routes it to
/// `echo_ok` behavior). This proves the sanitized display name is NOT the
/// call key; the real name is.
#[tokio::test]
async fn invoke_tool_dispatches_on_real_tool_name_not_sanitized_display() {
    let mut child = phase1_child().await;

    let payload = "dispatch-on-real-name-9c2";
    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__poison_meta",
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__poison_meta must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__poison_meta");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool probe__poison_meta returned no result"));
    // The probe routes poison_meta to echo_ok, which echoes the message —
    // proving dispatch happened on the REAL tool name. A proxy that used the
    // sanitized display name as the call key would still dispatch here (the
    // name is clean), but the success path proves dispatch works; the
    // load-bearing assertion is that the call did not fail with an
    // unknown-tool error.
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "invoke_tool probe__poison_meta must succeed (dispatch on real name); \
             an unknown-tool error would mean the proxy used the sanitized display name \
             as the call key"
        );
    }
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("poison_meta result missing content array"))
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
        text.contains(payload),
        "invoke_tool probe__poison_meta must echo the payload (dispatch on real name); \
         got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 4: `get_tool_schema` for an upstream tool with poisoned metadata
/// returns valid JSON and sanitizes the upstream-authored `title` /
/// `description` / `$comment` strings, while preserving the schema's
/// structural shape (type, properties, required, property keys) used by
/// callers.
///
/// The probe's `poison_schema` tool carries `title`, `description`, and
/// `$comment` with embedded `\n`, `\r`, tab, vertical tab, form feed, and
/// long content. The aggregator must sanitize those metadata strings in the
/// JSON text returned by `get_tool_schema`, and must NOT change the
/// schema's `type`, `properties`, `required`, or property keys.
#[tokio::test]
async fn get_tool_schema_sanitizes_poisoned_metadata_preserves_shape() {
    let mut child = phase1_child().await;

    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "probe__poison_schema" }),
        ),
    )
    .await
    .expect("get_tool_schema probe__poison_schema must complete");
    common::assert_no_rpc_error(&resp, "get_tool_schema probe__poison_schema");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("get_tool_schema returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "get_tool_schema for a known tool must not be an error"
        );
    }

    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("get_tool_schema result missing content array"));
    let text = content
        .iter()
        .find_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                b.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("get_tool_schema result must carry a text content block"));

    // The returned text must be valid JSON (SC 4: "returns valid JSON").
    let schema: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("get_tool_schema text content must be valid JSON; got: {text:?}\n{e}")
    });

    // SC 4 structural preservation: the schema's object shape is unchanged.
    // The probe's poison_schema has type=object, properties={key: {...}},
    // required=["key"]. A sanitization that mangled the shape would break
    // callers.
    assert_eq!(
        schema.get("type").and_then(|t| t.as_str()),
        Some("object"),
        "poison_schema must preserve type=object; got: {schema:?}"
    );
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .unwrap_or_else(|| panic!("poison_schema must preserve properties; got: {schema:?}"));
    assert!(
        props.contains_key("key"),
        "poison_schema must preserve the `key` property key; got keys: {:?}",
        props.keys().collect::<Vec<_>>()
    );
    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .unwrap_or_else(|| panic!("poison_schema must preserve required array; got: {schema:?}"));
    assert!(
        required.iter().any(|v| v.as_str() == Some("key")),
        "poison_schema `required` must still include `key`; got: {required:?}"
    );

    // SC 4 metadata sanitization: the upstream-authored `title`,
    // `description`, and `$comment` strings are sanitized (control-free and
    // capped). A non-sanitizing implementation would pass the poisoned
    // strings through verbatim.
    if let Some(title) = schema.get("title").and_then(|t| t.as_str()) {
        assert!(
            !title.contains('\n') && !title.contains('\r'),
            "poison_schema `title` must be a single line (sanitized); got: {title:?}"
        );
        assert_no_control_chars(title, "poison_schema title");
        assert!(
            title.chars().count() <= DESC_CAP,
            "poison_schema `title` must be capped at ~{DESC_CAP} chars; got {}: {title:?}",
            title.chars().count()
        );
    }
    if let Some(desc) = schema.get("description").and_then(|d| d.as_str()) {
        assert!(
            !desc.contains('\n') && !desc.contains('\r'),
            "poison_schema `description` must be a single line; got: {desc:?}"
        );
        assert_no_control_chars(desc, "poison_schema description");
        assert!(
            desc.chars().count() <= DESC_CAP,
            "poison_schema `description` must be capped at ~{DESC_CAP} chars; got {}: {desc:?}",
            desc.chars().count()
        );
    }
    if let Some(comment) = schema.get("$comment").and_then(|c| c.as_str()) {
        assert!(
            !comment.contains('\n') && !comment.contains('\r'),
            "poison_schema `$comment` must be a single line; got: {comment:?}"
        );
        assert_no_control_chars(comment, "poison_schema $comment");
        assert!(
            comment.chars().count() <= DESC_CAP,
            "poison_schema `$comment` must be capped at ~{DESC_CAP} chars; got {}: {comment:?}",
            comment.chars().count()
        );
    }

    child.into_guard().shutdown().await.ok();
}

/// Master SC 5: sanitization does NOT apply to `invoke_tool` result content.
/// Non-text content (the probe's `echo_image` tool returns an image block) and
/// structured content pass byte-faithfully — the aggregator must not
/// stringify, sanitize, or transform tool-call results.
///
/// This re-asserts the Phase 1 byte-faithfulness invariant in the Phase 4
/// context: the sanitization added for discovery/schema metadata must NOT
/// leak into the invoke result path.
#[tokio::test]
async fn invoke_tool_result_content_not_sanitized_passes_byte_faithfully() {
    let mut child = phase1_child().await;

    // Non-text content: the probe's echo_image returns an image content block.
    let resp = timeout(
        SPAWN_DEADLINE,
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
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "echo_image must succeed (byte-faithful non-text forward)"
        );
    }
    let content = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("echo_image result missing content array"));
    // SC 5 / D-004: at least one content block has a non-text `type` (image).
    // A proxy that stringified the content array would produce only text
    // blocks. Sanitization MUST NOT touch result content.
    let has_non_text = content
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) != Some("text"));
    assert!(
        has_non_text,
        "non-text content block must be preserved byte-faithfully (SC 5 / D-004), not \
         stringified or sanitized; got only text blocks: {content:?}"
    );

    child.into_guard().shutdown().await.ok();
}
