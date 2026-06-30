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

/// Long clean schema annotation fixture. Kept in sync with the probe fixture's
/// `poison_schema.properties.long_clean.description` value. The distinctive
/// suffix is past `DESC_CAP`, so a reintroduced schema-annotation row cap fails
/// by exact equality, not by a weak length-only check.
const LONG_CLEAN_SCHEMA_DESCRIPTION: &str = "This clean schema annotation intentionally exceeds the old list row cap while containing no control characters, so get_tool_schema must relay the full text without truncation or mutation. DISTINCTIVE_TAIL_PAST_120_SCHEMA_RELAY_FIDELITY";

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

/// Master SC 5 / D-004: `invoke_tool` arguments are a byte-faithful channel.
/// A BEL U+0007 embedded in a string argument must reach the upstream echo path
/// and return in the tool result unchanged. Sanitization is display-only for
/// discovery/schema annotations, not invoke arguments or result content.
#[tokio::test]
async fn invoke_tool_arguments_with_control_chars_round_trip_verbatim() {
    let mut child = phase1_child().await;

    let payload = "wei\u{0007}rd";
    let resp = timeout(
        SPAWN_DEADLINE,
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
    .expect("invoke_tool probe__echo_ok with BEL-bearing argument must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool probe__echo_ok BEL round-trip");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool echo_ok returned no result"));
    if let Some(is_error) = result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "invoke_tool echo_ok must be a successful tool result, not isError"
        );
    }

    let text = result_text(&result);
    assert_eq!(
        text, payload,
        "invoke_tool must round-trip the BEL-bearing argument verbatim; BEL must not be \
         replaced, deleted, escaped into a different semantic value, or sanitized"
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
    // single-line). A non-sanitizing implementation would pass the poisoned
    // strings through verbatim. Full schema annotations are NOT row-capped;
    // only `list_tools` row descriptions use `DESC_CAP`.
    if let Some(title) = schema.get("title").and_then(|t| t.as_str()) {
        assert!(
            !title.contains('\n') && !title.contains('\r'),
            "poison_schema `title` must be a single line (sanitized); got: {title:?}"
        );
        assert_no_control_chars(title, "poison_schema title");
    }
    if let Some(desc) = schema.get("description").and_then(|d| d.as_str()) {
        assert!(
            !desc.contains('\n') && !desc.contains('\r'),
            "poison_schema `description` must be a single line; got: {desc:?}"
        );
        assert_no_control_chars(desc, "poison_schema description");
    }
    if let Some(comment) = schema.get("$comment").and_then(|c| c.as_str()) {
        assert!(
            !comment.contains('\n') && !comment.contains('\r'),
            "poison_schema `$comment` must be a single line; got: {comment:?}"
        );
        assert_no_control_chars(comment, "poison_schema $comment");
    }

    child.into_guard().shutdown().await.ok();
}

/// Master SC 1: `get_tool_schema` returns full annotation strings. This uses a
/// clean property description longer than the old `DESC_CAP`, with a distinctive
/// suffix beyond character 120. Exact equality fails on truncation and on
/// mid-string mutation.
#[tokio::test]
async fn get_tool_schema_preserves_full_length_annotations_without_row_cap() {
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
    common::assert_no_rpc_error(
        &resp,
        "get_tool_schema probe__poison_schema long annotation",
    );
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
    let schema: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("get_tool_schema text content must be valid JSON; got: {text:?}\n{e}")
    });

    let long_desc = schema
        .get("properties")
        .and_then(|p| p.get("long_clean"))
        .and_then(|p| p.get("description"))
        .and_then(|d| d.as_str())
        .unwrap_or_else(|| {
            panic!("poison_schema must expose properties.long_clean.description; got: {schema:?}")
        });
    assert_eq!(
        long_desc,
        LONG_CLEAN_SCHEMA_DESCRIPTION,
        "get_tool_schema must relay the full clean annotation exactly; expected {} chars with \
         the distinctive tail past {DESC_CAP}, got {} chars: {long_desc:?}",
        LONG_CLEAN_SCHEMA_DESCRIPTION.chars().count(),
        long_desc.chars().count()
    );
    assert!(
        long_desc.contains("DISTINCTIVE_TAIL_PAST_120_SCHEMA_RELAY_FIDELITY"),
        "long annotation must include the distinctive suffix past the old cap; got: {long_desc:?}"
    );

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

// ---- Review-fix coverage (F1–F3) -------------------------------------------
//
// The THOROUGH review surfaced five targeted findings against the landed
// Phase 4 implementation. F1, F2, F3 are cleanly wire-testable and get RED
// tests below; F4 and F5 live in `error_hardening.rs` / `list_changed.rs`
// respectively (F4 as an `#[ignore]` stub — not deterministic wire-level; F5
// as a RED wire test). Each new test asserts the CORRECTED behavior; the
// current tree fails them, which is the contract for the debugger.

/// The Unicode/C1/bidi/zero-width code points the F1 fixture embeds in the
/// `poison_meta` description. The sanitizer must strip ALL of these — a C0-
/// only strip (the current tree) leaves them in the LLM-visible row text.
const F1_FORBIDDEN_CODEPOINTS: [u32; 7] = [
    0x2028, // LINE SEPARATOR
    0x2029, // PARAGRAPH SEPARATOR
    0x0085, // NEXT LINE (C1 control)
    0x0080, // PADDING CHARACTER (C1 control)
    0x202E, // RIGHT-TO-LEFT OVERRIDE (bidi)
    0x200B, // ZERO WIDTH SPACE
    0xFEFF, // BYTE ORDER MARK (ZWNBS) — included per review F1 list
];

/// Review fix F1 — Unicode separators / C1 / bidi / zero-width are stripped
/// (security). The probe's `poison_meta` description now embeds U+2028, U+2029,
/// C1 controls (U+0080, U+0085 NEL), a bidi override (U+202E), a zero-width
/// char (U+200B), and a BOM (U+FEFF). The aggregator's sanitizer currently
/// strips only C0 (U+0000–U+001F) + DEL (U+007F), so these pass through into
/// the LLM-visible row text — a real injection/format bypass (GOTCHA #20).
///
/// This test asserts the `list_tools` row description for `poison_meta`
/// contains NONE of these code points (and is still single-line + capped). RED
/// against the current tree (C0-only strip). The existing
/// `list_tools_sanitizes_poisoned_description_strips_control_and_caps_length`
/// test stays GREEN — its assertions (no `\n`/`\r`, C0-free, capped) remain
/// true under the current C0-only strip; THIS test is what catches the F1
/// gap.
#[tokio::test]
async fn f1_list_tools_strips_unicode_separators_c1_bidi_zero_width() {
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
    let desc = row_description(poison_row);

    // F1: NONE of the Unicode separators / C1 / bidi / zero-width / BOM code
    // points survive sanitization. A C0-only strip leaves them — RED now.
    for cp in F1_FORBIDDEN_CODEPOINTS {
        assert!(
            !desc.chars().any(|c| c as u32 == cp),
            "F1: poison_meta description must NOT contain U+{cp:04X} (Unicode \
             separator / C1 / bidi / zero-width / BOM); the sanitizer must strip \
             it; got: {desc:?}"
        );
    }

    // Re-assert the existing invariants still hold (single-line, C0-free,
    // capped) so this test is a strict superset of the F1 contract, not a
    // weaker substitute.
    assert!(
        !desc.is_empty(),
        "poison_meta description must not be empty"
    );
    assert!(
        !desc.contains('\n') && !desc.contains('\r'),
        "poison_meta description must be a single line; got: {desc:?}"
    );
    assert_no_control_chars(&desc, "poison_meta description (F1)");
    assert!(
        desc.chars().count() <= DESC_CAP,
        "poison_meta description must be capped at ~{DESC_CAP} chars; got {}: {desc:?}",
        desc.chars().count()
    );

    child.into_guard().shutdown().await.ok();
}

/// Review fix F2 — A long upstream tool name stays dispatchable (correctness).
/// The probe's `toggle_long_tool` toggles ON a tool whose REAL name is 120
/// characters (longer than the aggregator's ~100-char description cap, under
/// rmcp 1.8.0's 128-char registration ceiling, valid `[A-Za-z0-9_.-]`). The
/// aggregator currently applies the same `sanitize_upstream_text` cap to the
/// `tool`/`name` dispatch-key field as it does to `description`, so the row
/// advertises a TRUNCATED 100-char name; a subsequent `invoke_tool` using that
/// advertised key fails `unknown_tool` (the real upstream name is 120 chars).
///
/// This test asserts:
///   - `list_tools` advertises the FULL 120-char real name in the row
///     `tool`/`name` field (not truncated to 100);
///   - `invoke_tool` using that advertised key SUCCEEDS (round-trips, not
///     `unknown_tool`).
/// RED against the current tree (name field capped at 100 → truncated →
/// dispatch fails). rmcp 1.8.0 registers names up to 128 chars, so the
/// 120-char name IS registerable (verified by `probe-tools` build + the
/// probe's `debug_assert!`s in `long_named_tool_tool`); the finding is NOT
/// moot.
#[tokio::test]
async fn f2_long_named_tool_advertised_full_and_dispatchable() {
    let mut child = phase1_child().await;

    // Baseline: the long-named tool is NOT visible (off by default). This
    // confirms the fixture starts in the default state.
    let baseline_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("baseline list_tools must complete");
    common::assert_no_rpc_error(&baseline_resp, "baseline list_tools");
    let baseline_rows = parse_list_tools_rows(
        &baseline_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("baseline list_tools returned no result")),
    );
    assert!(
        !baseline_rows.iter().any(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|n| n.as_str()))
                .map(|n| n.starts_with("long_named_tool_"))
                .unwrap_or(false)
        }),
        "baseline list_tools must NOT include the long-named tool (off by default)"
    );

    // Toggle the long-named tool ON via the probe's `toggle_long_tool`.
    let toggle_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__toggle_long_tool",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool probe__toggle_long_tool must complete");
    common::assert_no_rpc_error(&toggle_resp, "probe__toggle_long_tool");
    let toggle_text = result_text(
        &toggle_resp
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("toggle_long_tool returned no result")),
    );
    assert!(
        toggle_text.contains("added"),
        "toggle_long_tool must report it added the long-named tool; got: {toggle_text:?}"
    );

    // The aggregator's `toggle_long_tool` call does NOT emit list_changed, so
    // the cached inventory is stale relative to the probe's new tool list.
    // The next `list_tools` triggers a lazy refetch (dirty is NOT set by
    // toggle_long_tool, so this relies on the inventory being fresh enough
    // OR the aggregator refetching). To make this deterministic, we force a
    // refetch by... actually the current cache holds the pre-toggle list;
    // without a list_changed notification the aggregator will NOT refetch.
    // So we MUST trigger a refetch. The cleanest deterministic way: kill the
    // cached connection is wrong (that's F4/F5). Instead, the F2 test uses
    // `get_tool_schema` directly on the long name — that path does NOT
    // depend on the cached inventory; it looks up by name. But the F2
    // contract is that `list_tools` ADVERTISES the full name, which requires
    // the inventory to include it. The probe's `toggle_long_tool` does not
    // notify, so the aggregator's cache stays stale and list_tools will NOT
    // show the long tool.
    //
    // Resolution: the F2 fixture's `toggle_long_tool` SHOULD emit
    // `notifications/tools/list_changed` so the aggregator refetches — this
    // is the same mechanism `mutate_tools` uses, and it makes the F2 test
    // deterministic. The F2 contract is name dispatchability, not cache
    // invalidation, but the advertisement depends on the inventory
    // refreshing. Emitting list_changed is the clean way to refresh.
    //
    // (If `toggle_long_tool` did not notify, the test would be flaky — it
    // would depend on the aggregator happening to refetch. Emitting the
    // notification makes the next `list_tools` deterministically refetch.)
    //
    // NOTE: the probe fixture's `toggle_long_tool` dispatch was updated to
    // emit list_changed; see `toggle_long_tool` in the probe. The
    // `tokio::time::sleep` below gives the aggregator a moment to process
    // the notification before the next `list_tools`.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // F2 part 1: `list_tools` advertises the FULL 120-char real name in the
    // row `tool`/`name` field (not truncated to 100). RED against the
    // current tree (name capped at 100 → row shows a 100-char truncation,
    // not the 120-char real name).
    let after_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("post-toggle list_tools must complete (lazy refetch)");
    common::assert_no_rpc_error(&after_resp, "post-toggle list_tools");
    let after_result = after_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("post-toggle list_tools returned no result"));
    let after_rows = parse_list_tools_rows(&after_result);

    let long_row = after_rows
        .iter()
        .find(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|n| n.as_str()))
                .map(|n| n.starts_with("long_named_tool_"))
                .unwrap_or(false)
        })
        .unwrap_or_else(|| {
            panic!(
                "post-toggle list_tools must include the long-named tool row; got rows: {after_rows:?}"
            )
        });
    let advertised_name = long_row
        .get("tool")
        .and_then(|t| t.as_str())
        .or_else(|| long_row.get("name").and_then(|n| n.as_str()))
        .unwrap_or_else(|| panic!("long-named row missing tool/name field: {long_row:?}"));
    assert_eq!(
        advertised_name,
        long_full_name(),
        "F2: list_tools must advertise the FULL 120-char real name (not truncated to 100); \
         got {} chars: {advertised_name:?}",
        advertised_name.chars().count()
    );

    // F2 part 2: `invoke_tool` using the advertised key SUCCEEDS (round-trip,
    // not `unknown_tool`). RED against the current tree: the advertised key
    // is truncated to 100 chars, so `invoke_tool probe__<100-char-truncated>`
    // looks up a 100-char name the upstream does not have → `unknown_tool`.
    // After the F2 fix: the advertised key is the full 120-char name, which
    // the upstream recognizes → echo_ok round-trip.
    let payload = "f2-roundtrip-7d3";
    let invoke_resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("probe__{}", advertised_name),
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("invoke_tool on the long-named tool must complete");
    common::assert_no_rpc_error(&invoke_resp, "invoke_tool long-named tool");
    let invoke_result = invoke_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool long-named tool returned no result"));
    if let Some(is_error) = invoke_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "F2: invoke_tool on the long-named tool must SUCCEED (dispatch on the full real \
             name); an unknown-tool error means the proxy used the truncated name as the call key"
        );
    }
    let invoke_text = result_text(&invoke_result);
    assert!(
        invoke_text.contains(payload),
        "F2: invoke_tool on the long-named tool must echo the payload (round-trip on the full \
         real name); got: {invoke_text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// The full 120-char real name of the F2 long-named tool. Kept in sync with
/// the probe fixture's `LONG_TOOL_NAME` constant (both are 120 chars,
/// `long_named_tool_` + 104 `a`s). A shared constant would require a build-
/// time path between the probe bin and the test bin; duplicating with an
/// explicit length assert is the cleaner wire-level discipline (the test
/// asserts the advertised name equals THIS, pinning the contract).
fn long_full_name() -> String {
    let name = format!("long_named_tool_{}", "a".repeat(104));
    debug_assert_eq!(
        name.chars().count(),
        120,
        "F2 fixture: long_full_name must be exactly 120 chars; got {}",
        name.chars().count()
    );
    name
}

/// Review fix F3 — Schema validation data is preserved; only annotations are
/// sanitized. The probe's `poison_validation` tool has an `input_schema` with
/// BOTH (a) annotation fields (`title`, `description`) carrying control chars,
/// AND (b) validation fields carrying control-bearing string values — a
/// string `enum` member (`["clean", "wei\u{0007}rd"]`), a `default`
/// (`"def\u{000A}ault"`), and a `const` (`"const\u{000B}val"`).
///
/// The aggregator's current schema sanitizer treats `enum` as a metadata key
/// (per `is_schema_metadata_key`: `title|description|$comment|examples|enum`)
/// and sanitizes its string members, CORRUPTING the validation data — the
/// enum member `"wei\u{0007}rd"` becomes `"wei rd"` (control→space), so
/// `get_tool_schema` advertises a value the upstream does not accept.
/// `default`/`const` are NOT in the current key set, so they pass verbatim
/// today — but the F3 fix NARROWS the key set to annotations only, so they
/// continue to pass verbatim (the test pins that).
///
/// This test asserts `get_tool_schema` returns the `enum` members and
/// `default`/`const` values VERBATIM/unchanged (validation data preserved)
/// WHILE the `title`/`description` annotation IS sanitized (control-free).
/// RED against the current tree (enum member sanitized → control stripped).
#[tokio::test]
async fn f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations() {
    let mut child = phase1_child().await;

    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "get_tool_schema",
            serde_json::json!({ "name": "probe__poison_validation" }),
        ),
    )
    .await
    .expect("get_tool_schema probe__poison_validation must complete");
    common::assert_no_rpc_error(&resp, "get_tool_schema probe__poison_validation");
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

    let schema: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("get_tool_schema text content must be valid JSON; got: {text:?}\n{e}")
    });

    // Structural shape preserved (the F3 fix must not mangle the schema).
    assert_eq!(
        schema.get("type").and_then(|t| t.as_str()),
        Some("object"),
        "poison_validation must preserve type=object; got: {schema:?}"
    );
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .unwrap_or_else(|| panic!("poison_validation must preserve properties; got: {schema:?}"));
    assert!(
        props.contains_key("key"),
        "poison_validation must preserve the `key` property key; got keys: {:?}",
        props.keys().collect::<Vec<_>>()
    );
    let key_prop = &props["key"];

    // F3 part 1: VALIDATION DATA PRESERVED VERBATIM. The `enum` members,
    // `default`, and `const` carry control chars; the aggregator must NOT
    // sanitize them. RED against the current tree: `enum` is in
    // `is_schema_metadata_key`, so `"wei\u{0007}rd"` becomes `"wei rd"`.

    // enum: exactly ["clean", "wei\u{0007}rd"] — the control-bearing member
    // preserved verbatim.
    let enum_arr = key_prop
        .get("enum")
        .and_then(|e| e.as_array())
        .unwrap_or_else(|| {
            panic!("poison_validation `key.enum` must be an array; got: {key_prop:?}")
        });
    assert_eq!(
        enum_arr.len(),
        2,
        "poison_validation `key.enum` must have exactly 2 members; got: {enum_arr:?}"
    );
    assert_eq!(
        enum_arr[0].as_str(),
        Some("clean"),
        "poison_validation `key.enum[0]` must be `clean` (verbatim); got: {enum_arr:?}"
    );
    assert_eq!(
        enum_arr[1].as_str(),
        Some("wei\u{0007}rd"),
        "F3: poison_validation `key.enum[1]` must be `wei\\u{{0007}}rd` VERBATIM (validation data \
         preserved, not sanitized); got: {:?}",
        enum_arr[1].as_str()
    );

    // default: "def\u{000A}ault" — the newline preserved verbatim.
    assert_eq!(
        key_prop.get("default").and_then(|d| d.as_str()),
        Some("def\u{000A}ault"),
        "F3: poison_validation `key.default` must be `def\\u{{000A}}ault` VERBATIM (validation \
         data preserved); got: {:?}",
        key_prop.get("default")
    );

    // const: "const\u{000B}val" — the VT preserved verbatim.
    assert_eq!(
        key_prop.get("const").and_then(|c| c.as_str()),
        Some("const\u{000B}val"),
        "F3: poison_validation `key.const` must be `const\\u{{000B}}val` VERBATIM (validation \
         data preserved); got: {:?}",
        key_prop.get("const")
    );

    // F3 part 2: ANNOTATIONS SANITIZED. The `title` and `description` carry
    // control chars; the aggregator MUST sanitize them (control-free, single
    // line). Full schema annotations are NOT row-capped. This is the
    // annotation-only policy: sanitize labels, not validation constants.
    if let Some(title) = schema.get("title").and_then(|t| t.as_str()) {
        assert!(
            !title.contains('\n') && !title.contains('\r'),
            "F3: poison_validation `title` annotation must be a single line (sanitized); got: {title:?}"
        );
        assert_no_control_chars(title, "poison_validation title (F3)");
    }
    if let Some(desc) = schema.get("description").and_then(|d| d.as_str()) {
        assert!(
            !desc.contains('\n') && !desc.contains('\r'),
            "F3: poison_validation `description` annotation must be a single line; got: {desc:?}"
        );
        assert_no_control_chars(desc, "poison_validation description (F3)");
    }

    child.into_guard().shutdown().await.ok();
}
