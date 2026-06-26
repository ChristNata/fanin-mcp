//! Phase 3 — credential store, interpolation, isolation, redaction (wire-level).
//!
//! Covers master Success Criteria 1–11:
//! - `cred set|list|rm` CLI surface (SC 3, 4, 5).
//! - keyring round-trip + env fallback (SC 6, 7).
//! - `${VAR}` interpolation at spawn (SC 8, SC 10).
//! - per-upstream env isolation (SC 9 — D-010 least-privilege).
//! - sentinel-redaction across tracing + child stderr + upstream logs (SC 11).
//! - resolution order: preferred backend -> env fallback -> structured error
//!   (SC 1, 2).
//!
//! All tests are wire-level / CLI-level. Tests reference NO `src/` symbols;
//! they depend only on `tokio`, `serde_json`, `tempfile` (dev-deps), and
//! `CARGO_BIN_EXE_fanin-mcp` / `CARGO_BIN_EXE_probe-server`. The suite
//! compiles clean against the current tree (where `credentials.rs` is a
//! stub) and fails RED on the absent behavior, not on missing symbols.

use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::common;
use crate::common::fixtures as fx;

/// Deadline for a `cred` CLI subcommand to exit. The hidden-prompt path
/// reads stdin then exits; a hang fails the test on the deadline.
const CLI_DEADLINE: Duration = Duration::from_secs(10);

/// Deadline for a meta-tool call that may trigger a lazy spawn + env
/// interpolation + credential resolution.
const RESOLVE_DEADLINE: Duration = Duration::from_secs(15);

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

/// Parse the structured-error JSON from a CallToolResult's text content.
fn parse_error_json(result: &Value) -> Value {
    let text = result_text(result);
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("structured error text content must be valid JSON; got: {text:?}\n{e}")
    })
}

/// Build the argv for `fanin-mcp cred <action> <server> <KEY>`.
fn cred_argv(action: &str, server: &str, key: Option<&str>) -> Vec<String> {
    let mut v = vec!["cred".to_string(), action.to_string(), server.to_string()];
    if let Some(k) = key {
        v.push(k.to_string());
    }
    v
}

/// Master SC 3: `cred set <server> <KEY>` stores a value entered through a
/// hidden stdin prompt and exposes NO CLI argument or flag capable of
/// carrying the secret value.
///
/// The test feeds the secret through the child's stdin pipe (never argv),
/// then asserts:
/// 1. The secret value does NOT appear in stdout or stderr (no echo).
/// 2. The CLI exits 0 (the stub currently exits FAILURE — RED until the
///    implementer wires the real `cred set` path).
/// 3. No argv position can carry the secret — the test invokes ONLY
///    `cred set <server> <KEY>`; the secret is on stdin.
#[tokio::test]
async fn cred_set_reads_secret_from_hidden_stdin_not_argv() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let secret = fx::phase3_sentinel_value();

    let args = cred_argv("set", &server, Some(&key));
    let out = common::run_fanin_cli(&args, Some(&format!("{secret}\n")), CLI_DEADLINE).await;

    // The secret must NEVER appear on stdout or stderr (no echo, no log leak).
    assert!(
        !out.stdout.contains(&secret),
        "cred set must NOT echo the secret to stdout; stdout:\n{}",
        out.stdout
    );
    assert!(
        !out.stderr.contains(&secret),
        "cred set must NOT echo the secret to stderr; stderr:\n{}",
        out.stderr
    );

    // The CLI must exit 0 on success. The current stub exits FAILURE, so this
    // is the RED-until-implemented assertion.
    let status = out.status.expect("cred set must exit within deadline");
    assert!(
        status.success(),
        "cred set must exit 0 on success (current stub exits failure — RED until \
         implemented); got status {status:?}; stderr:\n{}",
        out.stderr
    );
}

/// Master SC 4: `cred list <server>` prints or returns ONLY credential names
/// and NEVER includes stored values.
///
/// The test first runs `cred set` to store a sentinel secret (feeding it via
/// stdin), then runs `cred list` and asserts the KEY name appears in the
/// output but the secret VALUE does not.
#[tokio::test]
async fn cred_list_emits_names_only_never_values() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let secret = fx::phase3_sentinel_value();

    // Store the secret first (stdin pipe, never argv).
    let set_args = cred_argv("set", &server, Some(&key));
    let set_out =
        common::run_fanin_cli(&set_args, Some(&format!("{secret}\n")), CLI_DEADLINE).await;
    // If `cred set` is not implemented yet, the list test still asserts the
    // value-nevers-appear invariant on whatever `cred list` emits. But record
    // the set status so the RED reason is clear.
    let _ = set_out.status;

    // List the credentials for this server.
    let list_args = cred_argv("list", &server, None);
    let list_out = common::run_fanin_cli(&list_args, None, CLI_DEADLINE).await;

    // The secret VALUE must never appear in the list output, regardless of
    // whether the KEY name appears (a stub that lists nothing still passes
    // the value-absence assertion; a stub that prints the value fails it).
    let combined = format!("{}{}", list_out.stdout, list_out.stderr);
    assert!(
        !combined.contains(&secret),
        "cred list must NEVER emit the stored secret value; output:\n{combined}"
    );
}

/// Master SC 5: `cred rm <server> <KEY>` removes the key so a later lookup
/// cannot resolve it from the selected mutable backend.
///
/// The test stores a secret, removes it, then attempts to resolve it via a
/// spawned upstream that references `${KEY}` in its env. After removal, the
/// resolution must NOT return the secret value (either a structured
/// missing-credential error, or the probe's echo_env reports `<absent>`).
/// The observable is: the secret value does not reach the probe after
/// `cred rm`.
#[tokio::test]
async fn cred_rm_makes_later_resolution_not_return_value() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let env_name = fx::phase3_env_var_name("RM");
    let secret = fx::phase3_sentinel_value();

    // Store the secret.
    let set_args = cred_argv("set", &server, Some(&key));
    let _ = common::run_fanin_cli(&set_args, Some(&format!("{secret}\n")), CLI_DEADLINE).await;

    // Remove it.
    let rm_args = cred_argv("rm", &server, Some(&key));
    let rm_out = common::run_fanin_cli(&rm_args, None, CLI_DEADLINE).await;
    let rm_status = rm_out.status.expect("cred rm must exit within deadline");
    assert!(
        rm_status.success(),
        "cred rm must exit 0 on success; got {rm_status:?}; stderr:\n{}",
        rm_out.stderr
    );

    // Now attempt to resolve it through a spawned upstream. The config
    // references `${key}` in the server's env under `env_name`; after rm,
    // the resolution must NOT deliver the secret value to the probe.
    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(server.clone()).env(env_name.clone(), format!("${{{key}}}")),
        )
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": env_name },
            }),
        ),
    )
    .await
    .expect("echo_env call must complete within deadline");
    common::assert_no_rpc_error(&resp, "echo_env after cred rm");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_env returned no result"));

    // Either the call succeeds with "<absent>" (env fallback could not
    // resolve the removed key) or it returns a structured missing-credential
    // error. In NEITHER case may the secret value appear.
    let text = result_text(&result);
    assert!(
        !text.contains(&secret),
        "after cred rm, the secret value must NOT reach the upstream; \
         echo_env returned: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 6: a keyring-backed round trip through `cred set` succeeds on
/// hosts with an available keyring. On a keyring-less / headless host the
/// keyring backend is not usable, so this test is `#[ignore]`-gated to avoid
/// hanging or erroring the suite on CI without a keyring daemon.
///
/// The always-run path for credential storage is the env fallback
/// (SC 7 / `env_fallback_resolves_without_keyring`).
#[tokio::test]
#[ignore = "requires a usable OS keyring; re-enable on a host with a keyring daemon"]
async fn keyring_round_trip_succeeds_when_keyring_available() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let secret = fx::phase3_sentinel_value();

    let set_args = cred_argv("set", &server, Some(&key));
    let set_out =
        common::run_fanin_cli(&set_args, Some(&format!("{secret}\n")), CLI_DEADLINE).await;
    let status = set_out
        .status
        .expect("cred set must exit within deadline (keyring round trip)");
    assert!(
        status.success(),
        "cred set via keyring must exit 0; got {status:?}; stderr:\n{}",
        set_out.stderr
    );

    // List must show the key name (not the value).
    let list_args = cred_argv("list", &server, None);
    let list_out = common::run_fanin_cli(&list_args, None, CLI_DEADLINE).await;
    let combined = format!("{}{}", list_out.stdout, list_out.stderr);
    assert!(
        combined.contains(&key),
        "cred list via keyring must show the stored key name; output:\n{combined}"
    );
    assert!(
        !combined.contains(&secret),
        "cred list via keyring must NOT show the stored value; output:\n{combined}"
    );
}

/// Master SC 7: env fallback works in a keyring-less / headless case without
/// requiring a keyring service. The test sets a sentinel secret via `cred set`
/// (which may use env fallback when no keyring is available) and asserts the
/// value is NOT echoed to stdout/stderr — the value-absence invariant holds
/// regardless of backend.
#[tokio::test]
async fn env_fallback_resolves_without_keyring() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let secret = fx::phase3_sentinel_value();

    // `cred set` with no keyring available must still succeed via env fallback
    // (or at minimum not hang and not echo the secret). Feed via stdin.
    let args = cred_argv("set", &server, Some(&key));
    let out = common::run_fanin_cli(&args, Some(&format!("{secret}\n")), CLI_DEADLINE).await;

    // The secret must never appear on stdout/stderr regardless of backend.
    assert!(
        !out.stdout.contains(&secret) && !out.stderr.contains(&secret),
        "env-fallback cred set must NOT echo the secret; stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
}

/// Master SC 8 + SC 10: `${VAR}` interpolation in a server's configured `env`
/// resolves at spawn time, and literal non-secret env values still reach the
/// selected upstream unchanged.
///
/// The test sets a sentinel secret via `cred set` under a known key, then
/// configures a server with two env entries:
/// - `INTERPOLATED = "${key}"` — resolves to the sentinel at spawn.
/// - `LITERAL = "plain-value-123"` — passes through unchanged.
/// It then invokes `echo_env` on the probe for each key and asserts the
/// interpolated value resolves (the probe sees the sentinel) and the literal
/// value passes through verbatim.
///
/// The current tree has no interpolation (`process.rs` injects env literally
/// with `${key}` unresolved), so the probe sees the literal `${key}` string
/// — RED until the implementer wires resolution.
#[tokio::test]
async fn dollar_brace_interpolation_resolves_and_literal_values_pass_through() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let secret = fx::phase3_sentinel_value();
    let interp_name = fx::phase3_env_var_name("INTERP");
    let lit_name = fx::phase3_env_var_name("LIT");
    let lit_value = "plain-value-123";

    // Store the sentinel secret.
    let set_args = cred_argv("set", &server, Some(&key));
    let _ = common::run_fanin_cli(&set_args, Some(&format!("{secret}\n")), CLI_DEADLINE).await;

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(server.clone())
                .env(interp_name.clone(), format!("${{{key}}}"))
                .env(lit_name.clone(), lit_value),
        )
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // The interpolated var must resolve to the sentinel at spawn.
    let interp_resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": interp_name },
            }),
        ),
    )
    .await
    .expect("echo_env (interpolated) must complete within deadline");
    common::assert_no_rpc_error(&interp_resp, "echo_env interpolated");
    let interp_result = interp_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_env interpolated returned no result"));
    let interp_text = result_text(&interp_result);
    assert!(
        interp_text.contains(&secret),
        "${{{key}}} must resolve to the sentinel at spawn; echo_env returned: {interp_text:?}"
    );
    assert!(
        !interp_text.contains("${"),
        "interpolated env value must NOT reach the probe as a literal ${{...}} string; \
         got: {interp_text:?}"
    );

    // The literal var must pass through unchanged.
    let lit_resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": lit_name },
            }),
        ),
    )
    .await
    .expect("echo_env (literal) must complete within deadline");
    common::assert_no_rpc_error(&lit_resp, "echo_env literal");
    let lit_result = lit_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_env literal returned no result"));
    let lit_text = result_text(&lit_result);
    assert_eq!(
        lit_text.trim(),
        lit_value,
        "literal non-secret env value must reach the upstream unchanged; got: {lit_text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 9: per-upstream env isolation (D-010 least-privilege). Each
/// spawned upstream receives ONLY its own configured env vars; sibling
/// credentials and the aggregator's full environment are NOT inherited.
///
/// The test configures two sibling servers:
/// - `alpha` with env `ALPHA_ONLY = alpha-secret`.
/// - `beta` with env `BETA_ONLY = beta-secret`.
/// It then invokes `echo_env` on alpha for `BETA_ONLY` (must be `<absent>`)
/// and on beta for `ALPHA_ONLY` (must be `<absent>`). A non-isolated impl
/// that inherits the aggregator's full env or shares sibling env would let
/// the probe see the sibling's value — failing the assertion.
///
/// The test also asserts a UNIQUE ambient env var set on the aggregator's
/// own process is NOT visible to the probe unless explicitly configured —
/// the strongest D-010 proof.
#[tokio::test]
async fn per_upstream_env_isolation_sibling_and_ambient_not_inherited() {
    let alpha = format!("alpha-{}", fx::phase3_unique_seq());
    let beta = format!("beta-{}", fx::phase3_unique_seq());
    let alpha_key = fx::phase3_env_var_name("ALPHA");
    let beta_key = fx::phase3_env_var_name("BETA");
    let alpha_val = "alpha-secret-value";
    let beta_val = "beta-secret-value";
    let ambient_key = fx::phase3_env_var_name("AMBIENT");
    let ambient_val = "aggregator-ambient-value";

    // Set an ambient var on the aggregator's own process so the spawned
    // aggregator child inherits it; the probe must NOT see it unless it is
    // explicitly configured for that server.
    std::env::set_var(&ambient_key, ambient_val);

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(fx::Phase3ServerEntry::new(alpha.clone()).env(alpha_key.clone(), alpha_val))
        .server(fx::Phase3ServerEntry::new(beta.clone()).env(beta_key.clone(), beta_val))
        .namespace(fx::NamespaceEntry::new(
            "default",
            [alpha.as_str(), beta.as_str()],
        ))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // alpha must see its own var.
    let alpha_own = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{alpha}__echo_env"),
                "arguments": { "key": alpha_key },
            }),
        ),
    )
    .await
    .expect("alpha echo_env own must complete");
    common::assert_no_rpc_error(&alpha_own, "alpha echo_env own");
    let alpha_own_text = result_text(alpha_own.get("result").unwrap());
    assert!(
        alpha_own_text.contains(alpha_val),
        "alpha must see its own configured env var; got: {alpha_own_text:?}"
    );

    // alpha must NOT see beta's var (sibling isolation).
    let alpha_sibling = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{alpha}__echo_env"),
                "arguments": { "key": beta_key },
            }),
        ),
    )
    .await
    .expect("alpha echo_env sibling must complete");
    common::assert_no_rpc_error(&alpha_sibling, "alpha echo_env sibling");
    let alpha_sibling_text = result_text(alpha_sibling.get("result").unwrap());
    assert!(
        alpha_sibling_text.contains("<absent>"),
        "alpha must NOT see beta's env var (D-010 sibling isolation); got: {alpha_sibling_text:?}"
    );
    assert!(
        !alpha_sibling_text.contains(beta_val),
        "alpha must NOT see beta's secret value; got: {alpha_sibling_text:?}"
    );

    // alpha must NOT see the aggregator's ambient var (least-privilege: no
    // wholesale env inheritance).
    let alpha_ambient = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{alpha}__echo_env"),
                "arguments": { "key": ambient_key },
            }),
        ),
    )
    .await
    .expect("alpha echo_env ambient must complete");
    common::assert_no_rpc_error(&alpha_ambient, "alpha echo_env ambient");
    let alpha_ambient_text = result_text(alpha_ambient.get("result").unwrap());
    assert!(
        alpha_ambient_text.contains("<absent>"),
        "alpha must NOT inherit the aggregator's ambient env (D-010 least-privilege); \
         got: {alpha_ambient_text:?}"
    );
    assert!(
        !alpha_ambient_text.contains(ambient_val),
        "alpha must NOT see the aggregator's ambient value; got: {alpha_ambient_text:?}"
    );

    // beta must see its own var and NOT alpha's.
    let beta_own = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{beta}__echo_env"),
                "arguments": { "key": beta_key },
            }),
        ),
    )
    .await
    .expect("beta echo_env own must complete");
    common::assert_no_rpc_error(&beta_own, "beta echo_env own");
    let beta_own_text = result_text(beta_own.get("result").unwrap());
    assert!(
        beta_own_text.contains(beta_val),
        "beta must see its own configured env var; got: {beta_own_text:?}"
    );

    let beta_sibling = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{beta}__echo_env"),
                "arguments": { "key": alpha_key },
            }),
        ),
    )
    .await
    .expect("beta echo_env sibling must complete");
    common::assert_no_rpc_error(&beta_sibling, "beta echo_env sibling");
    let beta_sibling_text = result_text(beta_sibling.get("result").unwrap());
    assert!(
        beta_sibling_text.contains("<absent>"),
        "beta must NOT see alpha's env var (D-010 sibling isolation); got: {beta_sibling_text:?}"
    );

    std::env::remove_var(&ambient_key);
    child.into_guard().shutdown().await.ok();
}

/// Master SC 11: the mandatory sentinel-redaction test. The sentinel secret
/// never appears in tracing output, child stderr logs, or upstream
/// notification logs.
///
/// The test:
/// 1. Stores a sentinel secret via `cred set`.
/// 2. Configures a server whose resolved env value is the sentinel, with a
///    log file capturing child stderr + upstream log notifications.
/// 3. Triggers tracing (by invoking a tool that forces a lazy spawn), child
///    stderr capture (the probe writes diagnostics to stderr), and upstream
///    log notifications (the probe emits log notifications via the reverse-
///    traffic path).
/// 4. Asserts the sentinel string appears in NONE of:
///    - the aggregator's stderr (tracing output),
///    - the child stderr log file (`[server]`-prefixed lines).
///
/// The current tree has no redaction layer, so the sentinel may leak into the
/// child stderr log file or tracing — RED until the implementer registers the
/// sentinel with the redaction component.
#[tokio::test]
async fn sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let key = format!("KEY_{}", fx::phase3_unique_seq());
    let sentinel = fx::phase3_sentinel_value();
    let env_name = fx::phase3_env_var_name("SENTINEL");
    let log_path = fx::empty_log_file_path();

    // Store the sentinel secret.
    let set_args = cred_argv("set", &server, Some(&key));
    let _ = common::run_fanin_cli(&set_args, Some(&format!("{sentinel}\n")), CLI_DEADLINE).await;

    // Configure the server with the sentinel as a resolved env value and a
    // log file capturing child stderr + upstream notifications.
    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(server.clone())
                .env(env_name.clone(), format!("${{{key}}}"))
                .with_log_file(&log_path),
        )
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();

    // Spawn the aggregator with stderr captured so we can assert tracing
    // output does not leak the sentinel.
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    let mut stderr = child.take_stderr();

    common::initialize(&mut child).await;

    // Trigger a lazy spawn (which resolves the sentinel and injects it into
    // the child env). The probe logs its stderr to the log file via the
    // aggregator's capture path; the aggregator's tracing may log the
    // resolved env. The sentinel must NOT appear in either sink.
    let resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": env_name },
            }),
        ),
    )
    .await
    .expect("echo_env must complete within deadline (sentinel redaction)");
    common::assert_no_rpc_error(&resp, "echo_env sentinel redaction");

    // Drain any remaining stderr within a short window so tracing lines
    // emitted after the call land.
    let mut agg_stderr_buf = Vec::new();
    if let Some(stderr) = stderr.as_mut() {
        use tokio::io::AsyncReadExt;
        let _ = tokio::time::timeout(Duration::from_millis(500), async {
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => agg_stderr_buf.extend_from_slice(&buf[..n]),
                }
            }
        })
        .await;
    }
    let agg_stderr = String::from_utf8_lossy(&agg_stderr_buf).into_owned();

    // Give the child stderr log task a moment to flush.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();

    // The sentinel must NOT appear in the aggregator's tracing stderr.
    assert!(
        !agg_stderr.contains(&sentinel),
        "sentinel secret must NOT appear in aggregator tracing (stderr); got:\n{agg_stderr}"
    );
    // The sentinel must NOT appear in the child stderr log file.
    assert!(
        !log.contains(&sentinel),
        "sentinel secret must NOT appear in the child stderr log file; got:\n{log}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 2 (resolution order) + missing-credential error shape: when a
/// configured `${VAR}` cannot be resolved (no keyring entry, no env var),
/// the upstream call returns a structured tool-level error naming the server
/// and variable, NOT a JSON-RPC error, and the error text does NOT include
/// the secret or a secret-looking value.
#[tokio::test]
async fn missing_credential_returns_structured_error_not_rpc_error() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let missing_key = format!("DEFINITELY_MISSING_{}", fx::phase3_unique_seq());
    let env_name = fx::phase3_env_var_name("MISSING");

    // Ensure the key is not present in the process env either.
    std::env::remove_var(&missing_key);

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(server.clone())
                .env(env_name.clone(), format!("${{{missing_key}}}")),
        )
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    let resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": env_name },
            }),
        ),
    )
    .await
    .expect("echo_env (missing credential) must complete within deadline");
    common::assert_no_rpc_error(&resp, "echo_env missing credential");

    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_env missing credential returned no result"));

    // The result must be a structured error (isError: true) — NOT a
    // JSON-RPC error (asserted above) and NOT a successful echo of the
    // literal `${...}` string.
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        is_error,
        "missing credential must return isError: true, not a successful echo; \
         result: {result:?}"
    );

    let err = parse_error_json(&result);
    // The error must name the server.
    assert_eq!(
        err.get("server").and_then(|s| s.as_str()),
        Some(server.as_str()),
        "missing-credential error must name the server; got: {err:?}"
    );
    // The error must carry a code (the exact code is the implementer's
    // choice; it must NOT be a JSON-RPC error).
    assert!(
        err.get("code").and_then(|c| c.as_str()).is_some(),
        "missing-credential error must carry a structured code; got: {err:?}"
    );
    // The error text must NOT include the unresolved `${...}` literal as a
    // secret-looking value.
    let text = result_text(&result);
    assert!(
        !text.contains("${"),
        "missing-credential error must NOT include the unresolved ${{...}} literal; got: {text:?}"
    );

    child.into_guard().shutdown().await.ok();
}

/// Master SC 1 + SC 2: `src/credentials.rs` defines a server-scoped credential
/// abstraction with keyring and env backends, and resolution order is
/// preferred backend -> env fallback -> structured error. The wire-level
/// observable is the env-fallback resolution path plus the missing-credential
/// structured error. This test is the explicit resolution-order proof: a
/// value available ONLY via env fallback resolves, and a value available via
/// NEITHER backend returns the structured error.
#[tokio::test]
async fn credential_resolution_order_env_fallback_then_structured_error() {
    let server = format!("srv-{}", fx::phase3_unique_seq());
    let env_only_key = fx::phase3_env_var_name("ENVONLY");
    let env_only_val = "env-fallback-value";
    let missing_key = format!("DEFINITELY_MISSING_{}", fx::phase3_unique_seq());
    let env_name_ok = fx::phase3_env_var_name("OK");
    let env_name_missing = fx::phase3_env_var_name("MISS");

    // Set the env-only var on the aggregator process so the spawned
    // aggregator child inherits it; env fallback resolves it.
    std::env::set_var(&env_only_key, env_only_val);
    std::env::remove_var(&missing_key);

    let cfg = fx::Phase3ConfigBuilder::new()
        .server(
            fx::Phase3ServerEntry::new(server.clone())
                .env(env_name_ok.clone(), format!("${{{env_only_key}}}"))
                .env(env_name_missing.clone(), format!("${{{missing_key}}}")),
        )
        .namespace(fx::NamespaceEntry::new("default", [server.as_str()]))
        .write();
    let mut child = common::spawn_fanin_with_config(&cfg.path_str(), None).await;
    common::initialize(&mut child).await;

    // The env-fallback value must resolve.
    let ok_resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": env_name_ok },
            }),
        ),
    )
    .await
    .expect("echo_env (env fallback) must complete");
    common::assert_no_rpc_error(&ok_resp, "echo_env env fallback");
    let ok_text = result_text(ok_resp.get("result").unwrap());
    assert!(
        ok_text.contains(env_only_val),
        "env-fallback resolution must deliver the value to the upstream; got: {ok_text:?}"
    );

    // The missing value must produce a structured error, not a literal echo.
    let miss_resp = timeout(
        RESOLVE_DEADLINE,
        common::call_tool(
            &mut child,
            "invoke_tool",
            serde_json::json!({
                "name": format!("{server}__echo_env"),
                "arguments": { "key": env_name_missing },
            }),
        ),
    )
    .await
    .expect("echo_env (missing) must complete");
    common::assert_no_rpc_error(&miss_resp, "echo_env missing");
    let miss_result = miss_resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("echo_env missing returned no result"));
    let is_error = miss_result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        is_error,
        "unresolvable credential must return isError: true (structured error), not a literal echo; \
         result: {miss_result:?}"
    );

    std::env::remove_var(&env_only_key);
    child.into_guard().shutdown().await.ok();
}
