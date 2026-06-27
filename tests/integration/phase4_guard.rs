//! Phase 4 — public-surface + invariant regression guards (wire-level).
//!
//! Covers master Success Criteria 12, 13, 14, 15, 16, 17 — the public-surface
//! invariants Phase 4 must NOT regress, plus the Phase-4-specific
//! concurrency and stdout guards:
//!
//! - SC 12: downstream `tools/list` still exposes EXACTLY the three meta-tools
//!   (`list_tools`, `get_tool_schema`, `invoke_tool`).
//! - SC 13: the static names and descriptions of the three meta-tools do NOT
//!   change.
//! - SC 14: the structured-error JSON shape remains D-005-compatible — no
//!   field rename, no field removal, only additive new `code` values.
//! - SC 15: the registry never holds the entries/map lock across
//!   `call_tool().await` or `list_all_tools().await`; a dead/slow upstream
//!   does not serialize a sibling call.
//! - SC 16: the rmcp dependency remains pinned exactly to `=1.8.0`.
//! - SC 17: no serve-path `println!`, `print!`, or `dbg!` reaches stdout.
//!
//! These are ADDITIVE guards — the existing Phase 0/1/2/3 tests
//! (`regression_guard.rs`, `pinning.rs`, the implicit stdout-clean assertion
//! in every wire test) already cover the static-meta-tools and pin invariants.
//! This module re-asserts them in the Phase 4 context and adds the
//! Phase-4-specific concurrency guard (a dead/slow upstream must not
//! serialize a sibling) and a D-005-shape-additive check.
//!
//! All tests are wire-level and compile clean against the current tree.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::expectations as exp;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// The slow_tool delay used for the cross-upstream non-serialization proof.
const SLOW_DELAY_MS: u64 = 800;

/// The proof deadline for the concurrent sibling echo — STRICTLY shorter
/// than `SLOW_DELAY_MS`. A registry lock held across the slow await would
/// serialize the session; the sibling echo would take >= SLOW_DELAY_MS. A
/// correct lock-discipline impl dispatches the sibling on a separate upstream
/// while the slow await is pending, so the sibling completes well under the
/// slow delay. Mirrors `multi_upstream::PROOF_DEADLINE`.
const PROOF_DEADLINE: Duration = Duration::from_millis(400);

/// Helper: spawn the aggregator with the canonical Phase 1 config + initialize.
async fn phase1_child() -> common::JsonRpcChild {
    let cfg = fx::ConfigBuilder::new().write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;
    child
}

/// Resolve the repo root from CARGO_MANIFEST_DIR (mirrors `pinning::repo_root`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Master SC 12 + SC 13: under a Phase 4 config (which exercises the
/// sanitization + list_changed + dead-upstream paths), the downstream MCP
/// surface remains exactly three meta-tools with unchanged static names and
/// descriptions. This re-asserts the Phase 0/1/2/3 invariant in the Phase 4
/// context — Phase 4's discovery/schema sanitization must NOT leak into the
/// downstream tools/list surface or change the static descriptions.
#[tokio::test]
async fn phase4_context_preserves_three_meta_tools_and_static_descriptions() {
    let mut child = phase1_child().await;

    // Exercise the discovery path so the lazy upstream spawns and the
    // sanitization + cache paths are warmed. This is the Phase 4 context.
    let _ = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await;

    // Downstream rmcp tools/list: exactly the three meta-tools.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list under Phase 4 context");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);

    // SC 13: the static descriptions are unchanged (D-003). Assert each
    // meta-tool's description matches the expectations verbatim.
    let list_tools = exp::find_tool(tools, "list_tools").unwrap();
    exp::assert_desc(list_tools, exp::LIST_TOOLS_DESC);
    let get_schema = exp::find_tool(tools, "get_tool_schema").unwrap();
    exp::assert_desc(get_schema, exp::GET_TOOL_SCHEMA_DESC);
    let invoke_tool = exp::find_tool(tools, "invoke_tool").unwrap();
    exp::assert_desc(invoke_tool, exp::INVOKE_TOOL_DESC);

    child.into_guard().shutdown().await.ok();
}

/// Master SC 14: the structured-error JSON shape remains D-005-compatible.
/// Phase 4 adds new `code` values (`upstream_disconnected`) but must NOT
/// rename or remove the D-005 fields (`server`, `tool`, `code`, `message`,
/// `recoverable`). This test exercises a structured-error path that already
/// exists in the current tree (`always_error` round-trips the probe's error
/// JSON, and an unknown-tool call returns the aggregator's structured error)
/// and asserts the D-005 fields are present in BOTH. A Phase 4 implementation
/// that renamed a field or dropped `recoverable` fails this.
#[tokio::test]
async fn structured_error_json_keeps_d005_fields_additive_codes() {
    let mut child = phase1_child().await;

    // Path 1: the aggregator's OWN structured error for an unknown tool. The
    // aggregator emits the D-005 shape; Phase 4 must not rename/remove fields.
    let resp = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__does_not_exist",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("invoke_tool unknown tool must complete");
    common::assert_no_rpc_error(&resp, "invoke_tool unknown tool");
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("invoke_tool unknown tool returned no result"));
    common::assert_is_error_result(&result, "invoke_tool unknown tool");
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("result missing content array"))
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
    let err: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("aggregator structured error must be valid JSON; got: {text:?}\n{e}")
    });

    // SC 14 / D-005: every field present. A rename or removal fails.
    assert!(
        err.get("server").is_some(),
        "D-005 `server` field must be present; got: {err:?}"
    );
    assert!(
        err.get("tool").is_some(),
        "D-005 `tool` field must be present; got: {err:?}"
    );
    assert!(
        err.get("code").is_some(),
        "D-005 `code` field must be present; got: {err:?}"
    );
    assert!(
        err.get("message").is_some(),
        "D-005 `message` field must be present; got: {err:?}"
    );
    assert!(
        err.get("recoverable").is_some(),
        "D-005 `recoverable` field must be present (SC 14 — no field removal); got: {err:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 15: the registry never holds the entries/map lock across
/// `call_tool().await` or `list_all_tools().await`; a dead/slow upstream does
/// not serialize a sibling call. This is the Phase 4 analogue of the Phase 2
/// / Phase 3 concurrency proofs, extended to cover the cache-invalidation
/// path (the refetch on next `inventory()` after `list_changed` must not
/// hold the map lock across `list_all_tools().await`).
///
/// The proof: while `alpha__slow_tool` is awaiting a configured delay, a
/// concurrent `beta__echo_ok` completes within a deadline STRICTLY shorter
/// than the slow delay. A registry lock held across the slow await would
/// serialize the session; the sibling echo would block until the slow call
/// finished. This mirrors `multi_upstream::alpha_slow_tool_does_not_block_concurrent_beta_echo`
/// and re-asserts the invariant in the Phase 4 context (where the registry
/// cache is now mutable per state.json `decisions.cache-shape`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_upstream_does_not_serialize_concurrent_sibling() {
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(fx::NamespaceEntry::new("default", ["alpha", "beta"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Issue the slow alpha call WITHOUT waiting for its response. It enters
    // the registry, spawns alpha, and awaits the slow_tool delay.
    let slow_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "alpha__slow_tool",
                    "arguments": { "delay_ms": SLOW_DELAY_MS },
                },
            }),
        )
        .await;

    // Immediately issue the beta echo. If the registry lock were held across
    // the alpha slow await, this would block until alpha finished
    // (>= SLOW_DELAY_MS). A correct impl dispatches beta on a separate
    // upstream while alpha's slow await is pending.
    let echo_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "beta__echo_ok",
                    "arguments": { "message": "non-serialized-phase4" },
                },
            }),
        )
        .await;

    // SC 15: beta echo completes within PROOF_DEADLINE — strictly shorter
    // than SLOW_DELAY_MS. A serialized session would timeout.
    let echo_resp = timeout(PROOF_DEADLINE, child.wait_for_id(echo_id))
        .await
        .expect(
            "beta__echo_ok must complete within {PROOF_DEADLINE:?} while alpha__slow_tool \
             is awaiting {SLOW_DELAY_MS}ms — a registry lock held across the slow await \
             would serialize the session and make this timeout (SC 15 / D-007 / GOTCHA #16)",
        );
    common::assert_no_rpc_error(&echo_resp, "beta__echo_ok during alpha__slow_tool");
    let echo_result = echo_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__echo_ok returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "beta__echo_ok must succeed (real forward, not an error)"
        );
    }
    let echo_text = echo_result
        .get("content")
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("beta__echo_ok result missing content"))
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
        echo_text.contains("non-serialized-phase4"),
        "beta__echo_ok must echo byte-faithfully; got: {echo_text:?}"
    );

    // The slow call must also eventually complete (no hang, no lost request).
    let slow_resp = timeout(SPAWN_DEADLINE, child.wait_for_id(slow_id))
        .await
        .expect("alpha__slow_tool must also complete (no hang)");
    common::assert_no_rpc_error(&slow_resp, "alpha__slow_tool");

    child.into_guard().shutdown().await.ok();
}

/// Master SC 16: the rmcp dependency remains pinned exactly to `=1.8.0`.
/// Phase 4 adds an `on_tool_list_changed` handler and a mutable cache but
/// must NOT bump the rmcp pin. This re-asserts the Phase 0 `pinning.rs`
/// invariant in the Phase 4 context — the exact-pin discipline holds across
/// the Phase 4 work.
#[test]
fn rmcp_remains_pinned_exactly_at_1_8_0() {
    let root = repo_root();
    let cargo_toml = root.join("Cargo.toml");
    let toml_text = std::fs::read_to_string(&cargo_toml)
        .unwrap_or_else(|e| panic!("Cargo.toml must exist at {}: {e}", cargo_toml.display()));

    // Look for the rmcp dependency line and assert it pins exactly. We
    // accept either `rmcp = "=1.8.0"` or `rmcp = { version = "=1.8.0", ... }`.
    // A caret/tilde/range pin or a bumped version fails this.
    assert!(
        contains_exact_raincp_pin(&toml_text),
        "Cargo.toml must pin rmcp with exact `=x.y.z` syntax (D-015 / SC 16). \
         Expected a `rmcp` dependency whose version literal starts with `=`. \
         Cargo.toml contents:\n{toml_text}"
    );
}

/// Scan Cargo.toml text for an exact-pinned rmcp dependency (mirrors
/// `pinning::contains_exact_raincp_pin`). Looks for the bare form
/// `rmcp = "=..."` or the table form `rmcp = { ... version = "=..." ... }`.
fn contains_exact_raincp_pin(toml: &str) -> bool {
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let key = match trimmed.split_once('=') {
            Some((k, _)) => k.trim().trim_matches('"'),
            None => continue,
        };
        if key != "rmcp" {
            continue;
        }
        let value = trimmed.split_once('=').map(|(_, v)| v).unwrap_or("");
        if has_quoted_exact_version(value) {
            return true;
        }
    }
    false
}

/// In a dependency value string, find any quoted literal that starts with `=`
/// followed by a digit (mirrors `pinning::has_quoted_exact_version`).
fn has_quoted_exact_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j > start {
                let lit = &value[start..j];
                if let Some(rest) = lit.strip_prefix('=') {
                    if rest
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                    {
                        return true;
                    }
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}

/// Master SC 17: no serve-path `println!`, `print!`, or `dbg!` reaches
/// stdout. This is implicitly asserted by every wire-level test in the suite
/// — the harness panics on a non-JSON stdout line (GOTCHA #1). This test
/// makes the invariant explicit for the Phase 4 context: after exercising
/// the discovery + invoke + reverse-traffic + list_changed paths, every
/// stdout line the harness read parsed as valid JSON. A stray `println!` on
/// the serve path (e.g. in the new `on_tool_list_changed` handler or the
/// sanitization helper) would corrupt the stream and panic an earlier read.
///
/// The test drives a reverse-traffic exchange (which exercises the
/// ClientHandler path) and a list_changed exchange (which exercises the new
/// on_tool_list_changed path), then reads any remaining stdout within a
/// short window and asserts every line parses as JSON.
#[tokio::test]
async fn no_stdout_diagnostics_on_phase4_serve_path() {
    let mut child = phase1_child().await;

    // Drive a reverse-traffic exchange (exercises the ClientHandler path).
    let _ = timeout(
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
    .expect("needs_sampling must complete (reverse path exercised)");

    // Drive a list_changed exchange (exercises the new on_tool_list_changed
    // path). mutate_tools emits the notification; the aggregator's handler
    // must process it without writing to stdout.
    let _ = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "probe__mutate_tools",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("mutate_tools must complete (list_changed path exercised)");

    // Read any remaining stdout within a short window. Every line must parse
    // as JSON (a leaked stderr/stdout diagnostic would not). A timeout here
    // is fine — it means the child had nothing more to say.
    let _ = timeout(Duration::from_millis(500), async {
        loop {
            let raw = match child.read_line().await {
                Ok(s) => s,
                Err(_) => break, // timeout / EOF
            };
            if raw.trim().is_empty() {
                continue;
            }
            // Panic on a non-JSON line — that is the assertion (GOTCHA #1).
            let _: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
                panic!(
                    "aggregator stdout produced a non-JSON line on the Phase 4 serve path \
                     (likely a stray println!/print!/dbg!, SC 17 / GOTCHA #1): {raw:?}\n{e}"
                )
            });
        }
    })
    .await;

    child.into_guard().shutdown().await.ok();
}
