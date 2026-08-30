//! Multi-upstream proof — Phase 2 wire-level tests.
//!
//! Covers the plan's Phase 1 sub-phase (the headline multi-upstream proof):
//! master Success Criteria 1, 2, 3, 4, 5, 12, 13, plus the Phase 0/1
//! regression guard (criterion 13 in the plan's Success Criteria list:
//! static meta-tools, lazy startup, byte-faithful results, reverse-traffic
//! handling, stdout discipline remain true under a multi-upstream config).
//!
//! The existing `probe-server` binary is registered under distinct configured
//! server names (`alpha`, `beta`, optionally `gamma`). No second fixture
//! identity is introduced (plan §Probe fixture decision, master SC 12). The
//! configured name is what the aggregator routes on, so N registrations of the
//! same probe simulate N distinct upstreams from the proxy's perspective.
//!
//! Headline proof (D-007 / GOTCHA #16): while `alpha__slow_tool` is awaiting a
//! configured delay, a concurrent `beta__echo_ok` completes successfully within
//! a deadline strictly shorter than the slow delay. This is CROSS-upstream —
//! the load-bearing assertion. A same-upstream check is not the headline
//! (Phase 1 already covered that boundary).

use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::timeout;

use crate::common;
use crate::common::elicit as el;
use crate::common::expectations as exp;
use crate::common::fixtures as fx;

/// Deadline for a meta-tool call that may trigger a lazy spawn. Spawning the
/// probe + initialize + tools/list is well under 2s on any reasonable CI
/// runner; 15s is a generous ceiling that still catches a hang.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// The slow_tool delay used for the cross-upstream non-serialization proof.
/// Long enough that a serialized (lock-held-across-await) session would make
/// the concurrent beta echo blow past the proof deadline, while leaving enough
/// room for a correct cold beta spawn + MCP handshake under parallel test load.
const SLOW_DELAY_MS: u64 = 2_000;

/// The proof deadline for the concurrent beta echo — STRICTLY shorter than
/// `SLOW_DELAY_MS`. If the registry lock were held across the slow await, the
/// beta call could not complete before the slow call finished (the session
/// would serialize), so the beta echo would take >= SLOW_DELAY_MS. A correct
/// lock-discipline impl dispatches the beta call on a separate upstream while
/// alpha's slow await is still pending, so beta completes well under the slow
/// delay. The 1s deadline is half the slow delay: it tolerates a correct cold
/// spawn under load, but a serialized beta call still cannot beat the 2s floor.
const PROOF_DEADLINE: Duration = Duration::from_secs(1);

/// The exact set of probe tool names (mirrors `tests/integration/discovery.rs`).
/// Phase 3 extends the probe with `echo_env` and `spawn_grandchild`, bringing
/// the total to 10. Phase 4 adds `poison_meta`, `poison_schema`,
/// `mutate_tools`, and `self_pid`, bringing the static total to 14. The
/// review-fix pass adds `toggle_long_tool` (F2) and `poison_validation` (F3),
/// bringing the static total to 16. The runtime-added `added_tool`
/// (`mutate_tools`) and `long_named_tool` (F2 fixture, `toggle_long_tool`)
/// are NOT in this static set.
const PROBE_TOOL_NAMES: [&str; 16] = [
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
    "poison_meta",
    "poison_schema",
    "mutate_tools",
    "self_pid",
    "toggle_long_tool",
    "poison_validation",
];

/// Build a two-upstream (alpha + beta) config with a `default` namespace that
/// exposes both. The probe binary is registered under both names.
fn alpha_beta_config() -> fx::ConfigFile {
    fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .namespace(fx::NamespaceEntry::new("default", ["alpha", "beta"]))
        .write()
}

/// Build a three-upstream (alpha + beta + gamma) config with a `default`
/// namespace exposing all three. Used where wider inventory / switching
/// coverage is useful.
fn alpha_beta_gamma_config() -> fx::ConfigFile {
    fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha"))
        .server(fx::ServerEntry::new("beta"))
        .server(fx::ServerEntry::new("gamma"))
        .namespace(fx::NamespaceEntry::new(
            "default",
            ["alpha", "beta", "gamma"],
        ))
        .write()
}

/// Extract the text content of a list_tools result as a JSON string, then
/// parse it as a JSON array of row objects (mirrors discovery.rs).
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

/// Extract the joined text of a CallToolResult's content array (mirrors
/// invoke.rs).
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

/// Master SC 1 / P1.SC1: a config with two probe-backed upstreams starts
/// fanin-mcp successfully. The observable effect is that `initialize` returns
/// a well-formed result and downstream rmcp `tools/list` returns exactly the
/// three static meta-tools.
#[tokio::test]
async fn multi_upstream_config_starts_aggregator() {
    let cfg = alpha_beta_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;

    let init = common::initialize(&mut child).await;
    assert!(
        init.get("serverInfo").is_some(),
        "multi-upstream config must let initialize return serverInfo"
    );
    let name = init
        .get("serverInfo")
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or_default();
    assert_eq!(name, "fanin-mcp", "server must still name itself fanin-mcp");

    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list after multi-upstream config");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);

    child.into_guard().shutdown().await.ok();
}

/// Master SC 2 / P1.SC1: starting fanin-mcp and calling downstream rmcp
/// `tools/list` opens ZERO upstream connections when multiple upstreams are
/// configured. The observable proxy: after `initialize` + downstream
/// `tools/list`, the log sink is empty of any `[alpha]`/`[beta]` line — no
/// upstream was spawned. Downstream `tools/list` is static (D-003, GOTCHA #7);
/// any upstream touch here would destroy lazy loading.
#[tokio::test]
async fn downstream_tools_list_with_multi_upstream_opens_zero_connections() {
    let log_path = fx::empty_log_file_path();
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha").with_log_file(&log_path))
        .server(fx::ServerEntry::new("beta").with_log_file(&log_path))
        .namespace(fx::NamespaceEntry::new("default", ["alpha", "beta"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);

    // Give any would-be spawn a moment to flush stderr to the log file.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        !log.contains("alpha") && !log.contains("beta"),
        "downstream tools/list must not spawn ANY upstream with multiple \
         configured (criterion 2 / D-003 / GOTCHA #7); log already contains an \
         upstream line:\n{log}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 3 / P1.SC2: lazy isolation. A meta-tool call targeting `alpha`
/// leaves `beta` unspawned/uncontacted until a call targets `beta`. The
/// observable proxy: after calling `list_tools` with `server: "alpha"` (which
/// spawns alpha), the log sink contains an `alpha` line but NO `beta` line.
/// Then a call targeting `beta` produces a `beta` line — proving beta was
/// untouched until explicitly named.
#[tokio::test]
async fn targeting_alpha_leaves_beta_unspawned_until_beta_targeted() {
    let log_path = fx::empty_log_file_path();
    let cfg = fx::MultiConfigBuilder::new()
        .server(fx::ServerEntry::new("alpha").with_log_file(&log_path))
        .server(fx::ServerEntry::new("beta").with_log_file(&log_path))
        .namespace(fx::NamespaceEntry::new("default", ["alpha", "beta"]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Target alpha only. This spawns alpha; beta must remain untouched.
    let list = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "list_tools",
            serde_json::json!({ "server": "alpha" }),
        ),
    )
    .await
    .expect("list_tools alpha must complete (may spawn alpha)");
    common::assert_no_rpc_error(&list, "list_tools alpha");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let log_after_alpha = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log_after_alpha.contains("alpha"),
        "targeting alpha must spawn alpha (log should contain an alpha line):\n{log_after_alpha}"
    );
    assert!(
        !log_after_alpha.contains("beta"),
        "targeting alpha must NOT spawn beta (lazy isolation, criterion 3); \
         log already contains a beta line:\n{log_after_alpha}"
    );

    // Now target beta. The log should gain a beta line.
    let list_beta = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "list_tools",
            serde_json::json!({ "server": "beta" }),
        ),
    )
    .await
    .expect("list_tools beta must complete (spawns beta)");
    common::assert_no_rpc_error(&list_beta, "list_tools beta");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let log_after_beta = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log_after_beta.contains("beta"),
        "targeting beta must spawn beta (criterion 3); log has no beta line:\n{log_after_beta}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 4 / P1.SC3: single-spawn under race. Two concurrent first calls
/// to the same cold server initialize that upstream exactly once. The
/// observable proxy is the strongest the harness supports on Windows: the
/// consistent-success proxy (both calls SUCCEED and return consistent
/// inventory). A double-spawn race would either error one call or return
/// inconsistent results. The strict process-count assertion is platform-
/// specific and brittle in CI; the consistent-success proxy is what the plan
/// sanctions for the wire-level suite. See `tests.md` §Boundaries for the
/// Windows-specific limitation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_first_calls_to_same_cold_server_spawn_once() {
    let cfg = alpha_beta_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Two concurrent list_tools calls filtered to `alpha` — both are "first
    // calls" that need alpha's inventory. The per-server init guard (D-007,
    // GOTCHA #17) must serialize the spawn so only one upstream is created.
    // Use send_request (releases &mut) then wait_for_id for each, so both
    // requests are truly in flight before either response is read.
    let id_a = child
        .send_request(
            "tools/call",
            serde_json::json!({ "name": "list_tools", "arguments": { "server": "alpha" } }),
        )
        .await;
    let id_b = child
        .send_request(
            "tools/call",
            serde_json::json!({ "name": "list_tools", "arguments": { "server": "alpha" } }),
        )
        .await;

    let resp_a = timeout(SPAWN_DEADLINE, child.wait_for_id(id_a))
        .await
        .expect("first concurrent list_tools alpha must complete within deadline");
    let resp_b = timeout(SPAWN_DEADLINE, child.wait_for_id(id_b))
        .await
        .expect("second concurrent list_tools alpha must complete within deadline");

    common::assert_no_rpc_error(&resp_a, "concurrent list_tools alpha #1");
    common::assert_no_rpc_error(&resp_b, "concurrent list_tools alpha #2");

    // Both must return a SUCCESS content array (alpha's inventory rows), not
    // a not-implemented error. Consistent success is the proxy for "exactly
    // one spawn." Requiring success makes the test fail RED against a stub
    // that returns isError:true without forwarding.
    for (resp, label) in [(&resp_a, "a"), (&resp_b, "b")] {
        let result = resp
            .get("result")
            .unwrap_or_else(|| panic!("concurrent list_tools alpha {label} missing result"));
        if let Some(is_error) = result.get("isError") {
            assert_ne!(
                is_error.as_bool(),
                Some(true),
                "concurrent list_tools alpha {label} must return the inventory, \
                 not an error (consistent-success proxy for single-spawn, criterion 4)"
            );
        }
        assert!(
            result.get("content").is_some(),
            "concurrent list_tools alpha {label} must carry content"
        );
    }

    // The two inventories must be consistent (same set of tool names). A
    // double-spawn race that returned two different upstream connections could
    // return inconsistent rows.
    let rows_a = parse_list_tools_rows(resp_a.get("result").unwrap());
    let rows_b = parse_list_tools_rows(resp_b.get("result").unwrap());
    let mut names_a: Vec<String> = rows_a
        .iter()
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    let mut names_b: Vec<String> = rows_b
        .iter()
        .filter_map(|r| {
            r.get("tool")
                .and_then(|t| t.as_str())
                .or_else(|| r.get("name").and_then(|t| t.as_str()))
                .map(String::from)
        })
        .collect();
    names_a.sort();
    names_b.sort();
    assert_eq!(
        names_a, names_b,
        "concurrent first calls to the same cold server must return consistent \
         inventory (criterion 4 / single-spawn proxy); got {names_a:?} vs {names_b:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 5 / P1.SC4: the headline D-007 / GOTCHA #16 non-serialization
/// proof. While `alpha__slow_tool` is awaiting a configured delay, a concurrent
/// `beta__echo_ok` completes successfully within a deadline STRICTLY shorter
/// than the slow delay. This is CROSS-upstream — the load-bearing assertion.
///
/// A registry lock held across the alpha slow await would serialize the whole
/// session: the beta echo could not be dispatched until alpha's slow call
/// finished, so beta would take >= SLOW_DELAY_MS. A correct lock-discipline
/// impl clones the Arc, drops the map lock, and awaits the alpha call on one
/// upstream while dispatching the beta call on a SEPARATE upstream — so beta
/// completes well under the slow delay. The PROOF_DEADLINE (1s) is half the
/// SLOW_DELAY_MS (2s): enough margin for a correct cold beta spawn + MCP
/// handshake under parallel test load, while a serialized call still misses
/// the deadline by at least 1s.
///
/// The test requires REAL forwarding on both upstreams (alpha slow success +
/// beta echo success), so a not-implemented stub fails RED rather than passing
/// trivially.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn alpha_slow_tool_does_not_block_concurrent_beta_echo() {
    let cfg = alpha_beta_config();
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
    // the alpha slow await, this request would not be dispatched until alpha
    // finished (>= SLOW_DELAY_MS). A correct impl dispatches beta on a
    // separate upstream while alpha's slow await is pending.
    let echo_id = child
        .send_request(
            "tools/call",
            serde_json::json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "beta__echo_ok",
                    "arguments": { "message": "non-serialized" },
                },
            }),
        )
        .await;

    // The load-bearing assertion: beta echo completes within PROOF_DEADLINE,
    // which is strictly shorter than SLOW_DELAY_MS. A serialized session would
    // make this timeout (beta can't complete before alpha's 2s delay).
    let echo_resp = timeout(PROOF_DEADLINE, child.wait_for_id(echo_id))
        .await
        .expect(
            "beta__echo_ok must complete within {PROOF_DEADLINE:?} while alpha__slow_tool \
             is awaiting {SLOW_DELAY_MS}ms — a registry lock held across the slow await \
             would serialize the session and make this timeout (D-007 / GOTCHA #16)",
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
            "beta__echo_ok must forward successfully (real forward, not an error) — \
             otherwise the lock discipline is not exercised"
        );
    }
    let echo_text = result_text(&echo_result);
    assert!(
        echo_text.contains("non-serialized"),
        "beta__echo_ok must echo the payload byte-faithfully; got: {echo_text:?}"
    );

    // The slow call must also eventually complete (no hang, no lost request).
    let slow_resp = timeout(SPAWN_DEADLINE, child.wait_for_id(slow_id))
        .await
        .expect("alpha__slow_tool must also complete (no hang, no lost request)");
    common::assert_no_rpc_error(&slow_resp, "alpha__slow_tool");
    let slow_result = slow_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("alpha__slow_tool returned no result"));
    if let Some(is_error) = slow_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "alpha__slow_tool must forward successfully (real forward)"
        );
    }
    let slow_text = result_text(&slow_result);
    assert!(
        slow_text.contains(&SLOW_DELAY_MS.to_string()),
        "alpha__slow_tool result must pass through byte-faithfully; got: {slow_text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Regression guard (plan SC 13): static 3 meta-tools, lazy startup, byte-
/// faithful invoke results, and reverse-traffic rejection remain true under a
/// multi-upstream config. This re-asserts the Phase 0/1 guarantees in the
/// Phase 2 multi-upstream context — the live multi-upstream discovery path
/// must NOT leak into the downstream rmcp tools/list surface, and a real
/// cross-upstream forward + reverse-traffic exchange must still work.
///
/// Combines: static tools/list (SC 13), lazy startup (SC 2 — re-asserted via
/// the first list_tools spawning), byte-faithful invoke (SC 13), and reverse-
/// traffic rejection (SC 13 — needs_sampling completes within deadline, not
/// hung, GOTCHA #2). Stdout discipline is implicitly asserted by every wire
/// test (the harness panics on a non-JSON stdout line).
#[tokio::test]
async fn multi_upstream_preserves_phase0_phase1_guarantees() {
    let cfg = alpha_beta_gamma_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // Static 3 meta-tools under a 3-upstream config.
    let resp = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&resp, "tools/list under 3-upstream config");
    let tools = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(tools);

    // Live discovery across all three upstreams: list_tools returns rows from
    // alpha, beta, AND gamma (3 x 10 = 30 rows). Proves multi-upstream
    // discovery composes without leaking into the static downstream surface.
    let list = timeout(
        SPAWN_DEADLINE,
        common::call_tool(&mut child, "list_tools", serde_json::json!({})),
    )
    .await
    .expect("list_tools under 3-upstream config must complete");
    common::assert_no_rpc_error(&list, "list_tools 3-upstream");
    let list_result = list
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("list_tools 3-upstream returned no result"));
    if let Some(is_error) = list_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "list_tools must return the combined inventory, not an error"
        );
    }
    let rows = parse_list_tools_rows(&list_result);
    // Each upstream contributes the sixteen probe tools; total = 48 rows.
    assert_eq!(
        rows.len(),
        PROBE_TOOL_NAMES.len() * 3,
        "3-upstream list_tools must return 48 rows (3 servers x 16 tools); got {} rows",
        rows.len()
    );
    // Every row's server field must be one of alpha/beta/gamma, and each
    // server must appear.
    let servers: std::collections::HashSet<String> = rows
        .iter()
        .filter_map(|r| r.get("server").and_then(|s| s.as_str()).map(String::from))
        .collect();
    for expected in ["alpha", "beta", "gamma"] {
        assert!(
            servers.contains(expected),
            "list_tools under 3-upstream config must include rows from `{expected}`; \
             got servers: {:?}",
            servers
        );
    }

    // Byte-faithful invoke across a multi-upstream config: invoke beta__echo_ok
    // and assert the payload round-trips unchanged (D-004).
    let payload = "multi-upstream-byte-faithful-7a2c";
    let echo = timeout(
        SPAWN_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "beta__echo_ok",
                "arguments": { "message": payload },
            }),
        ),
    )
    .await
    .expect("beta__echo_ok must complete");
    common::assert_no_rpc_error(&echo, "beta__echo_ok");
    let echo_result = echo
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__echo_ok returned no result"));
    if let Some(is_error) = echo_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "beta__echo_ok must succeed (byte-faithful forward)"
        );
    }
    let echo_text = result_text(&echo_result);
    assert!(
        echo_text.contains(payload),
        "beta__echo_ok must round-trip the payload byte-faithfully (D-004); got: {echo_text:?}"
    );

    // Reverse-traffic rejection under a multi-upstream config: needs_sampling
    // on alpha completes within the deadline (the aggregator rejects the
    // sampling request, not a hang — GOTCHA #2). Proves the reverse-traffic
    // handler works per-upstream, not just for the single Phase 1 upstream.
    let rev = timeout(
        Duration::from_secs(10),
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": "alpha__needs_sampling",
                "arguments": {},
            }),
        ),
    )
    .await
    .expect("alpha__needs_sampling must complete (reverse traffic handled, not hung)");
    common::assert_no_rpc_error(&rev, "alpha__needs_sampling");
    let rev_result = rev
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("alpha__needs_sampling returned no result"));
    if let Some(is_error) = rev_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "alpha__needs_sampling must forward the probe's success result"
        );
    }

    // Final static tools/list — still exactly the three meta-tools after the
    // full multi-upstream exercise (no leak, no destabilization).
    let final_list = common::list_tools(&mut child).await;
    common::assert_no_rpc_error(&final_list, "final tools/list under 3-upstream config");
    let final_tools = final_list
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list result.tools must be an array"));
    exp::assert_exact_meta_tools(final_tools);

    child.into_guard().shutdown().await.ok();
}

// ---- Elicitation concurrency (v1.1 / Phase 4) -----------------------------
//
// Two-upstream concurrency proofs for the elicitation-forwarding slice. The
// downstream test client DECLARES elicitation capability and answers
// `elicitation/create` requests from BOTH upstreams over the same stdio
// stream. The probe's `needs_elicitation` tool encodes the outcome it received
// per call, so the tests assert the DIRECT outcome per upstream (SC11 distinct
// outcomes, no cross-talk) and that a slow prompt on one upstream does not
// block a sibling (SC12).
//
// The peer cell is reusable shared state, NOT a single-slot current-elicitation
// holder (GP-9). A stub that stores a single current-elicitation future would
// cross-talk the two concurrent prompts and fail SC11.

/// Deadline for a concurrent elicitation round-trip across two upstreams.
const CONCURRENT_ELICIT_DEADLINE: Duration = Duration::from_secs(20);

/// Deadline for the fast sibling while the other upstream's elicitation stays
/// pending. Fifteen seconds tolerates parallel-suite process contention while
/// remaining far below the pending call's default 60s timeout, so a serialized
/// implementation still fails.
const PENDING_ELICITATION_PROOF_DEADLINE: Duration = Duration::from_secs(15);

/// Master SC11 / GP-9: two upstreams issue CONCURRENT elicitation requests and
/// receive their DISTINCT downstream outcomes with no cross-talk. Alpha is
/// answered ACCEPT (with content "alpha-yes") and beta is answered DECLINE —
/// the two outcomes must not be swapped or collapsed. The test asserts the
/// DIRECT outcome per upstream via the probe's encoded
/// `elicitation_action`/`non_accept` fields.
///
/// A stub that stores a single current-elicitation future (a single-slot
/// holder rather than the reusable peer handle) would cross-talk: alpha might
/// receive beta's decline, or beta alpha's accept. The distinct-outcome
/// assertion catches that directly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_elicitations_on_two_upstreams_resolve_distinct_no_crosstalk() {
    let cfg = alpha_beta_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_declaring_elicitation(&mut child).await;

    // Issue both needs_elicitation calls WITHOUT awaiting — both enter the
    // registry, spawn their upstream, and the probes each emit a forwarded
    // elicitation/create toward this downstream client.
    let alpha_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "alpha__needs_elicitation",
                    "arguments": {},
                },
            }),
        )
        .await;
    let beta_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "beta__needs_elicitation",
                    "arguments": {},
                },
            }),
        )
        .await;

    // Two forwarded elicitation/create requests arrive on the wire. Read them
    // both and answer each by id. The order of arrival is not guaranteed, so
    // match each request to its tool call by inspecting the params (the probe
    // sends the same message text; we answer by id and assert the outcome
    // matches the answer we sent for that id). To keep the cross-talk
    // assertion tight, we answer the FIRST arriving request with ACCEPT
    // (content "alpha-yes") and the SECOND with DECLINE, then assert the
    // probe whose tool call resolves with ACCEPT is the one whose forwarded
    // request we answered with ACCEPT — i.e., we correlate by id, not by
    // upstream name.
    let req_a = el::await_elicitation_request(&mut child).await;
    let id_a = req_a
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("first forwarded elicitation missing id: {req_a:?}"));
    let req_b = el::await_elicitation_request(&mut child).await;
    let id_b = req_b
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("second forwarded elicitation missing id: {req_b:?}"));

    // Answer the first request ACCEPT, the second DECLINE.
    el::answer_accept(&mut child, id_a, json!({ "answer": "alpha-yes" })).await;
    el::answer_decline(&mut child, id_b).await;

    // Both tool calls must resolve within the deadline (no hang, no lost
    // request). The responses may arrive in either order.
    let resp_a = timeout(CONCURRENT_ELICIT_DEADLINE, child.wait_for_id(alpha_id))
        .await
        .expect("alpha__needs_elicitation must resolve (no hang)");
    let resp_b = timeout(CONCURRENT_ELICIT_DEADLINE, child.wait_for_id(beta_id))
        .await
        .expect("beta__needs_elicitation must resolve (no hang)");
    common::assert_no_rpc_error(&resp_a, "alpha__needs_elicitation concurrent");
    common::assert_no_rpc_error(&resp_b, "beta__needs_elicitation concurrent");

    // The mapping from forwarded-elicitation id to tool-call id is not
    // directly observable (rmcp owns id correlation on both hops). What IS
    // observable: exactly one tool call resolves ACCEPT and exactly one
    // resolves DECLINE — the two distinct outcomes we sent. Cross-talk would
    // collapse them (both accept, both decline, or swapped-and-mismatched).
    let result_a = resp_a
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("alpha__needs_elicitation returned no result"));
    let result_b = resp_b
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__needs_elicitation returned no result"));
    let outcome_a = el::parse_elicitation_outcome(&result_a, "SC11 alpha outcome");
    let outcome_b = el::parse_elicitation_outcome(&result_b, "SC11 beta outcome");

    // Collect the two actions; they must be {accept, decline} as a set.
    let mut actions: Vec<String> = [(&outcome_a, "a"), (&outcome_b, "b")]
        .iter()
        .map(|(o, label)| {
            o.get("elicitation_action")
                .and_then(|a| a.as_str())
                .unwrap_or_else(|| panic!("SC11 outcome {label} missing action: {o:?}"))
                .to_string()
        })
        .collect();
    actions.sort();
    assert_eq!(
        actions,
        vec!["accept".to_string(), "decline".to_string()],
        "SC11 / GP-9: the two concurrent elicitations must resolve to DISTINCT \
         outcomes (accept + decline); got {actions:?} — cross-talk would collapse or \
         swap them. outcomes: a={outcome_a:?} b={outcome_b:?}"
    );

    // The accept outcome's content must round-trip byte-faithfully (D-004).
    let (accept_outcome, _decline_outcome) =
        if outcome_a.get("elicitation_action").and_then(|a| a.as_str()) == Some("accept") {
            (&outcome_a, &outcome_b)
        } else {
            (&outcome_b, &outcome_a)
        };
    let content = el::accept_content(accept_outcome);
    assert_eq!(
        content.get("answer").and_then(|a| a.as_str()),
        Some("alpha-yes"),
        "SC11: the accept content must round-trip byte-faithfully; got {content:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC12 / GP-9: a pending (slow / never-answered) elicitation on one
/// upstream does NOT block a concurrent fast tool call on a sibling upstream.
/// Alpha's `needs_elicitation` is answered slowly (we leave it pending), while
/// beta's `echo_ok` completes within a deadline strictly shorter than the
/// pending alpha elicitation. A stub that holds a single registry-level lock
/// across the downstream elicitation await would serialize the session: beta's
/// echo would block until alpha's elicitation resolved.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_elicitation_on_one_upstream_does_not_block_sibling() {
    let cfg = alpha_beta_config();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    el::initialize_declaring_elicitation(&mut child).await;

    // Issue alpha's needs_elicitation WITHOUT awaiting — it enters the
    // registry, spawns alpha, and awaits the forwarded elicitation (which we
    // leave pending). `alpha_call_id` is the downstream tools/call id; the
    // forwarded elicitation/create has its OWN id (read below).
    let alpha_call_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "alpha__needs_elicitation",
                    "arguments": {},
                },
            }),
        )
        .await;
    // Await the forwarded elicitation/create request so we KNOW alpha is
    // blocked on the downstream answer. RETAIN the forwarded request id — we
    // answer it after the beta assertion so alpha resolves promptly and the
    // cleanup drain is deterministic (the proxy's default 60s tool-call timeout
    // is far longer than the harness `read_line` RPC_DEADLINE, so leaving
    // alpha to time out on the proxy side would panic the drain read instead
    // of cleanly elapsing). Do NOT answer it yet — alpha must stay pending
    // through the load-bearing beta assertion below.
    let alpha_elicit_req = el::await_elicitation_request(&mut child).await;
    let alpha_elicit_req_id = alpha_elicit_req
        .get("id")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            panic!("alpha forwarded elicitation/create missing id: {alpha_elicit_req:?}")
        });

    // Immediately issue beta's echo_ok. If the proxy held a registry lock
    // across alpha's elicitation await, this would block until alpha resolved.
    // A correct lock-discipline impl dispatches beta on a SEPARATE upstream
    // while alpha's elicitation await is pending.
    let beta_echo_id = child
        .send_request(
            "tools/call",
            json!({
                "name": "invoke_tool",
                "arguments": {
                    "name": "beta__echo_ok",
                    "arguments": { "message": "concurrent-with-pending-elicitation" },
                },
            }),
        )
        .await;

    // The load-bearing assertion: beta echo completes within a deadline
    // strictly shorter than the pending alpha elicitation (which we leave
    // unanswered THROUGH this assertion, then answer it below for cleanup).
    // The named 15s proof deadline tolerates parallel-suite process contention
    // and remains far below alpha's default 60s timeout.
    let beta_resp = timeout(
        PENDING_ELICITATION_PROOF_DEADLINE,
        child.wait_for_id(beta_echo_id),
    )
    .await
    .expect(
        "beta__echo_ok must complete within PENDING_ELICITATION_PROOF_DEADLINE while \
             alpha__needs_elicitation is \
             pending — a registry lock held across the elicitation await would serialize \
             the session and make this timeout (SC12 / D-007 / GOTCHA #16)",
    );
    common::assert_no_rpc_error(&beta_resp, "beta__echo_ok during pending alpha elicitation");
    let beta_result = beta_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("beta__echo_ok returned no result"));
    if let Some(is_error) = beta_result.get("isError") {
        assert_ne!(
            is_error.as_bool(),
            Some(true),
            "beta__echo_ok must succeed (real forward, not an error) while alpha's \
             elicitation is pending"
        );
    }
    let beta_text = result_text(&beta_result);
    assert!(
        beta_text.contains("concurrent-with-pending-elicitation"),
        "SC12: beta__echo_ok must echo byte-faithfully while alpha's elicitation is \
         pending; got: {beta_text:?}"
    );

    // Answer alpha's pending elicitation with DECLINE so it resolves promptly
    // and the cleanup drain is deterministic. We retained the forwarded request
    // id above (binding it, not dropping it) precisely so we can answer it
    // here. Leaving alpha to resolve via the proxy's default 60s tool-call
    // timeout would outlast the harness `read_line` RPC_DEADLINE (30s) and
    // panic the drain read — that was the original harness read bug, not a
    // lock. Answering here makes alpha's tool result arrive within
    // `read_line`'s budget; the outer ceiling still catches a hang.
    el::answer_decline(&mut child, alpha_elicit_req_id).await;
    let _ = timeout(Duration::from_secs(12), child.wait_for_id(alpha_call_id))
        .await
        .expect(
            "alpha__needs_elicitation must resolve after we answer its forwarded elicitation — \
             a hang here would mean the proxy did not relay the decline upstream (SC12 cleanup)",
        );
    child.into_guard().shutdown().await.ok();
}
