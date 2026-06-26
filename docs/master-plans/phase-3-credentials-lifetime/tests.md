# test-creator: phase-3-credentials-lifetime

Phase 3 test contract — credentials, timeouts, cancellation, process-tree
lifetime. The implementer codes against this suite; the objective gate runs
it. Test files are read-only to every later stage.

## Stack & runner

- **Runner:** `cargo test --test integration` (main suite). `cargo nextest
  run --workspace` works equivalently. Inherits the Phase 0/1/2 harness
  unchanged — wire-level JSON-RPC-over-stdio, no `src/` stubs.
- **Async:** `#[tokio::test]` single-threaded default; concurrency tests
  (`timeout_cancellation::cancellation_frees_local_resources_without_waiting_full_upstream`,
  `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling`)
  use `flavor = "multi_thread", worker_threads = 2`.
- **Wire-level (D-015).** Tests spawn the built `fanin-mcp` binary and speak
  raw JSON-RPC over stdio, and invoke the `fanin-mcp` CLI subcommands
  (`cred set|list|rm`) as child processes. Tests reference NO `src/` symbols;
  they depend only on `tokio`, `serde_json`, `tempfile` (dev-deps), and
  `CARGO_BIN_EXE_fanin-mcp` / `CARGO_BIN_EXE_probe-server`. The suite
  compiles clean against the current tree (where `credentials.rs` is a stub,
  `config.rs` has no `timeout_secs`, `process.rs` has no Job Object) and
  fails RED on absent behavior, not on missing symbols or compile errors.
- **Build order.** `cargo test` builds the `fanin-mcp` and `probe-server`
  `[[bin]]` targets before the test binary, so `CARGO_BIN_EXE_fanin-mcp` and
  `CARGO_BIN_EXE_probe-server` resolve. Env var names use the bin names
  EXACTLY as-declared (dashes/case preserved).

## Files created / extended

| Path | Criteria covered |
|---|---|
| `tests/probe-server/main.rs` (extended) | Adds `echo_env` tool (env isolation proof) and `spawn_grandchild` tool + grandchild sentinel branch (hard-kill orphan proof). Probe now exposes 10 tools. |
| `tests/common/fixtures.rs` (extended) | `Phase3ServerEntry`, `Phase3ConfigBuilder` (timeout_secs + env), `grandchild_marker_path`, `phase3_env_var_name`, `phase3_sentinel_value`, `phase3_unique_seq`. |
| `tests/common/mod.rs` (extended) | `run_fanin_cli` (CLI subcommand spawn with stdin pipe + deadline), `CliOutput`, `ChildGuard::kill_and_wait` (force-kill for hard-kill proof), `kill_process_by_id`. |
| `tests/integration/cred_store.rs` (new) | Master SC 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11. |
| `tests/integration/timeout_cancellation.rs` (new) | Master SC 13, 14, 15, 16, 18; P3.SC1–4 (SC 12 / 17 are coverage boundaries). |
| `tests/integration/process_lifetime.rs` (new) | Master SC 19, 20, 21, 22, 23. |
| `tests/integration/regression_guard.rs` (new) | Master SC 24. |
| `tests/integration/main.rs` (extended) | `mod cred_store;`, `mod timeout_cancellation;`, `mod process_lifetime;`, `mod regression_guard;` declarations. |
| `tests/integration/probe.rs`, `discovery.rs`, `multi_upstream.rs`, `namespace_acl.rs` (extended) | `PROBE_TOOL_NAMES` constant updated 8 → 10 to reflect the extended probe; prose comments updated. Behavioral assertions unchanged. |

Phase 0/1/2 behavioral guarantees are preserved. The only change to
existing tests is the probe tool-count constant (8 → 10), which is a factual
correction matching the extended probe fixture — not a weakening of any
behavioral assertion. The static-3-meta-tools, byte-faithful,
reverse-traffic, lazy-startup, and namespace-ACL invariants are unchanged.

## Config schema (Phase 3 extension — binding)

The Phase 3 config is a strict superset of the Phase 2 shape. The
implementer's Phase 2 parser must already accept it; Phase 3 adds parsing
of `timeout_secs` and the interpolation-aware `env` values.

```toml
[servers.<name>]
transport = "stdio"
command = '<path>'
args = []
timeout_secs = 1            # optional; default 60 (SC 12)
log_file = '<path>'         # optional

[servers.<name>.env]        # optional; values may carry ${VAR} (SC 8)
KEY = "literal-value"       # literal non-secret (SC 10)
TOKEN = "${SECRET_KEY}"     # interpolated at spawn (SC 8)

[namespaces.<name>]
servers = ["<name>"]
[namespaces.<name>.tools]   # optional, Phase 2 shape preserved
<server> = ["<tool>", ...]
```

### Choices recorded (Open Questions resolved by the orchestrator)

- **OQ1 — HTTP/`headers`: DEFERRED to Phase 5.** No test asserts a working
  HTTP transport or `headers` injection. The credential
  interpolation/redaction plumbing is tested via the stdio `env` path only.
  `transport = "http"` still fails startup as a later-phase transport (P2.SC7
  / Phase 2 config validation, unchanged).
- **OQ2 — process-wrap vs custom transport: implementer's choice.** The
  hard-kill orphan test asserts the OUTCOME (zero surviving descendants
  after `fanin-mcp` is force-killed), not the mechanism. It must pass
  regardless of which crate or custom transport the implementer picks.
- **OQ3 — cancellation: per the plan default.** The test asserts the
  observable: a cancelled downstream request frees fanin-mcp's local call
  without waiting the full upstream duration. If rmcp `=1.8.0` cannot expose
  the upstream request identity for forwarding `notify_cancelled`, that is a
  documented coverage boundary (§Coverage gaps) — no test asserts forwarding
  the API can't support.

## Coverage map — master Success Criteria

| # | Master Success Criterion | Test(s) |
|---|---|---|
| 1 | `credentials.rs` defines a server-scoped credential abstraction with keyring and env backends | `cred_store::credential_resolution_order_env_fallback_then_server_wide_fail_closed` (wire-level resolution-order + fail-closed proof); `cred_store::env_fallback_resolves_without_keyring` |
| 2 | Credential resolution order: preferred backend → env fallback → structured error | `cred_store::credential_resolution_order_env_fallback_then_server_wide_fail_closed`; `cred_store::missing_credential_returns_structured_error_not_rpc_error` |
| 3 | `cred set <server> <KEY>` stores a hidden-prompt value, no CLI arg carries the secret | `cred_store::cred_set_reads_secret_from_hidden_stdin_not_argv` (stdin pipe, no echo, exit 0) |
| 4 | `cred list <server>` returns names only, never values | `cred_store::cred_list_emits_names_only_never_values` |
| 5 | `cred rm <server> <KEY>` removes the key so later lookup cannot resolve it | `cred_store::cred_rm_makes_later_resolution_not_return_value` |
| 6 | Keyring-backed round trip succeeds on hosts with an available keyring | `cred_store::keyring_round_trip_succeeds_when_keyring_available` (#[ignore] on keyring-less hosts) |
| 7 | Env fallback works in a keyring-less/headless case | `cred_store::env_fallback_resolves_without_keyring`; `cred_store::credential_resolution_order_env_fallback_then_server_wide_fail_closed` |
| 8 | `${VAR}` interpolation resolves at spawn (keyring + env sources) | `cred_store::dollar_brace_interpolation_resolves_and_literal_values_pass_through` |
| 9 | Each spawned upstream receives ONLY its own env vars; sibling + ambient not inherited | `cred_store::per_upstream_env_isolation_sibling_and_ambient_not_inherited` (D-010 least-privilege proof) |
| 10 | Literal non-secret env values still reach the upstream unchanged | `cred_store::dollar_brace_interpolation_resolves_and_literal_values_pass_through` (literal-var assertion) |
| 11 | Sentinel secret never appears in tracing, child stderr logs, or upstream notification logs | `cred_store::sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs` (mandatory D-010 log-leak guard) |
| 12 | `timeout_secs` parses per server, defaults to 60 when omitted | **Config/unit coverage boundary — see §Coverage gaps.** Wire-level: `timeout_cancellation::fast_call_within_timeout_passes_through_byte_faithfully` (no timeout field, fast call succeeds); the default-60 value is config-observable without a 60s wait. |
| 13 | Every upstream tool call is wrapped in the effective per-server timeout | `timeout_cancellation::timeout_secs_wraps_upstream_call_and_returns_structured_error`; `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling` |
| 14 | Timed-out call returns `CallToolResult { isError: true }` with JSON `code: "upstream_timeout"`, server, tool, message, `recoverable: true` | `timeout_cancellation::timeout_secs_wraps_upstream_call_and_returns_structured_error`; `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling` |
| 15 | Timeout failures are NOT JSON-RPC errors | `timeout_cancellation::timeout_secs_wraps_upstream_call_and_returns_structured_error` (`assert_no_rpc_error`); `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling` |
| 16 | Downstream cancellation frees local resources without waiting the full upstream duration | `timeout_cancellation::cancellation_frees_local_resources_without_waiting_full_upstream` |
| 17 | When rmcp exposes the request identity, cancellation sends a forwarded notification upstream | **Coverage boundary — OQ3.** See §Coverage gaps. The test asserts the LOCAL observable (SC 16); forwarded cancellation is not asserted. |
| 18 | Registry locks never held across spawn/init/list/call/timeout/cancellation awaits | `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling` (cross-upstream non-serialization under timeout, D-007 / GOTCHA #16) |
| 19 | Windows Job Object containment kills descendants on hard-kill | `process_lifetime::hard_kill_orphan_test_no_surviving_descendants` (platform-conditional; outcome-asserted, not mechanism) |
| 20 | Unix process group containment kills descendants on hard-kill | `process_lifetime::hard_kill_orphan_test_no_surviving_descendants` (same test, platform-conditional) |
| 21 | Mandatory hard-kill orphan test: zero surviving descendants after force-killing fanin-mcp | `process_lifetime::hard_kill_orphan_test_no_surviving_descendants` (MANDATORY, D-009) |
| 22 | Normal stdin-EOF teardown terminates the full upstream tree | `process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree` |
| 23 | Child stderr capture writes `[server]`-prefixed redacted lines to the log file after process wrapping | `process_lifetime::stderr_capture_intact_after_process_wrapping` (capture half); `cred_store::sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs` (redaction half) |
| 24 | Public downstream MCP surface remains exactly three meta-tools | `regression_guard::phase3_config_preserves_phase012_guarantees`; Phase 0/1/2 tests unchanged |
| 25 | All required gates pass at 100% | The full suite (Phase 0 + 1 + 2 + 3) is the gate. Phase 5 of the plan runs it. |

## Coverage map — Phase sub-criteria

| Phase | Criterion | Test |
|---|---|---|
| P1.1 | `cred set` hidden-prompt, no argv secret | `cred_store::cred_set_reads_secret_from_hidden_stdin_not_argv` |
| P1.2 | `cred list` names only | `cred_store::cred_list_emits_names_only_never_values` |
| P1.3 | `cred rm` removes the key | `cred_store::cred_rm_makes_later_resolution_not_return_value` |
| P1.4 | Keyring round trip on hosts with keyring; env fallback on headless | `cred_store::keyring_round_trip_succeeds_when_keyring_available` (ignored on headless); `cred_store::env_fallback_resolves_without_keyring` |
| P1.5 | Cargo.toml keeps rmcp pinned, adds only Phase 3 deps | Structural — review verifies. No test asserts on Cargo.toml content. |
| P2.1 | `${VAR}` resolves from keyring then env | `cred_store::dollar_brace_interpolation_resolves_and_literal_values_pass_through` |
| P2.2 | Missing credentials → structured error naming server + variable, no secret (server-wide fail-closed) | `cred_store::missing_credential_returns_structured_error_not_rpc_error` |
| P2.3 | Spawned upstream receives exactly its configured env keys; no sibling credentials | `cred_store::per_upstream_env_isolation_sibling_and_ambient_not_inherited` |
| P2.4 | Literal non-secret env values reach the upstream | `cred_store::dollar_brace_interpolation_resolves_and_literal_values_pass_through` |
| P2.5 | Sentinel-redaction test proves sentinel absent from tracing, child stderr, upstream logs | `cred_store::sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs` |
| P2.6 | `timeout_secs` parses with default 60 | **Coverage boundary — §Coverage gaps.** |
| P2.7 | `transport = "http"` still fails startup | Phase 2 config validation test (`config.rs` startup-failure tests, unchanged). No Phase 3 test weakens this. |
| P3.1 | `timeout_secs = 1` + slow probe → `upstream_timeout` structured error | `timeout_cancellation::timeout_secs_wraps_upstream_call_and_returns_structured_error` |
| P3.2 | Default 60s timeout observable without 60s wait | **Coverage boundary — §Coverage gaps.** |
| P3.3 | Fast successful call passes through byte-faithfully, not wrapped as error | `timeout_cancellation::fast_call_within_timeout_passes_through_byte_faithfully` |
| P3.4 | Cancelled downstream request frees local call without full upstream wait | `timeout_cancellation::cancellation_frees_local_resources_without_waiting_full_upstream` |
| P3.5 | Where rmcp exposes request identity, forward cancellation upstream | **Coverage boundary (OQ3) — §Coverage gaps.** |
| P3.6 | Concurrency: slow timed/cancelled call on one upstream does not block sibling | `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling` |
| P4.1 | Hard-kill orphan test: zero surviving descendants | `process_lifetime::hard_kill_orphan_test_no_surviving_descendants` |
| P4.2 | Windows `cmd /c` descendant shape caught | `process_lifetime::hard_kill_orphan_test_no_surviving_descendants` (platform-conditional; the grandchild is a detached descendant) |
| P4.3 | Unix process-group child killed on fanin-mcp force-kill | `process_lifetime::hard_kill_orphan_test_no_surviving_descendants` (platform-conditional) |
| P4.4 | Stdin-EOF teardown terminates full tree | `process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree` |
| P4.5 | Stderr capture + `[server]` prefix + redaction intact after wrapping | `process_lifetime::stderr_capture_intact_after_process_wrapping`; `cred_store::sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs` |
| P4.6 | Process wrapper does not change downstream stdout transport | `regression_guard::phase3_config_preserves_phase012_guarantees` (implicit — harness panics on non-JSON stdout); `process_lifetime::stderr_capture_intact_after_process_wrapping` |
| P5.1–P5.5 | Gate + scope cleanup | Phase 5 runs the full suite; no new tests. |

## Side-effect assertions

Every Phase 3 test asserts the observable effect, not just a return value,
so a stub that returns the right shape without doing the work fails.

- **`cred set` hidden-stdin is a side-effect assertion on the secret's
  location.** `cred_set_reads_secret_from_hidden_stdin_not_argv` feeds the
  secret through the child's stdin pipe and asserts the value does NOT
  appear on stdout or stderr. A stub that accepts the secret on argv would
  still fail the no-echo assertion if it printed it; a stub that echoes the
  secret to stderr fails the stderr assertion. The exit-0 check catches the
  current stub (exits FAILURE).
- **`cred list` names-only is a side-effect assertion on the value's
  absence.** `cred_list_emits_names_only_never_values` asserts the secret
  VALUE never appears in the combined stdout+stderr, regardless of whether
  the KEY name appears. A stub that prints the value fails; a stub that
  prints nothing passes the value-absence but the keyring round-trip test
  (when un-ignored) catches the missing name.
- **`cred rm` is a side-effect assertion on the post-rm resolution.**
  `cred_rm_makes_later_resolution_not_return_value` stores a secret, removes
  it, then spawns an upstream that references `${KEY}` and asserts the
  secret value does NOT reach the probe. A stub that does not actually
  remove the key would let the value resolve and fail the assertion.
- **Per-upstream env isolation is a side-effect assertion on the probe's
  visible environment.** `per_upstream_env_isolation_sibling_and_ambient_not_inherited`
  invokes `echo_env` on the probe for sibling and ambient keys and asserts
  the probe reports `<absent>`. A non-isolated impl that inherits the
  aggregator's full env or shares sibling env would let the probe see the
  value — failing the assertion. The probe's `echo_env` tool is the
  observable: it reads the actual env of the spawned child.
- **Resolution order + fail-closed is a side-effect assertion on the
  SERVER-WIDE failure, not the per-call echo_env shape (Option A).**
  `credential_resolution_order_env_fallback_then_server_wide_fail_closed`
  splits into two independent servers: (1) a server whose `${VAR}` resolves
  ONLY via process-env fallback — `echo_env` for that var returns the
  resolved value byte-faithfully (preferred-backend → env-fallback ordering
  delivers); (2) a SEPARATE server with a guaranteed-unresolvable
  `${DEFINITELY_MISSING}` var — a GENERIC `echo_ok` call (NOT `echo_env`,
  no matching `key` arg) returns the structured `credential_resolution_failed`
  error (`isError: true`, names the server + the missing variable, carries
  `code: "credential_resolution_failed"`, NO `${...}` literal, NO sibling
  value). The generic-tool call is the load-bearing observable: the proxy is
  name-level only and cannot know which tool needs which credential, so an
  unresolvable configured placeholder must fail the WHOLE server, not just
  the one tool that happens to read the env var. The old per-call-via-
  `echo_env(key=<bad-lhs>)` shape (Option B / partial-resolve) is replaced —
  a stub that records the bad LHS and only short-circuits `echo_env` lets a
  generic `echo_ok` sail through to the probe and fails the `isError: true`
  assertion.
- **Missing-credential fail-closed is a side-effect assertion on the
  generic-tool call, not the echo_env key match (Option A).**
  `missing_credential_returns_structured_error_not_rpc_error` invokes
  `echo_ok` (NOT `echo_env`, no matching `key` arg) on a server with an
  unresolvable `${VAR}` and asserts the structured `credential_resolution_failed`
  error (`isError: true`, names server + missing variable, `code:
  "credential_resolution_failed"`, NO `${...}` literal, NOT a JSON-RPC
  error). Generalized from the old `echo_env(key=env_name)` shape, which the
  current Option B code satisfies via the per-key short-circuit; the
  generic-tool call exposes the partial-resolve gap and forces the
  server-wide fail-closed implementation.
- **Sentinel-redaction is a side-effect assertion on the log sinks.**
  `sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs` stores a
  sentinel, configures it as a resolved env value, triggers a lazy spawn
  (which resolves and injects the sentinel), and asserts the sentinel string
  appears in NONE of: the aggregator's tracing stderr, the child stderr log
  file. A stub with no redaction layer that logs the resolved env value
  leaks the sentinel into one or both sinks — failing the assertion.
- **Timeout wrapping is a side-effect assertion on the call duration.**
  `timeout_secs_wraps_upstream_call_and_returns_structured_error` issues a
  3s slow_tool call under a 1s timeout and asserts the call returns within
  5s with the `upstream_timeout` structured error. A stub with no timeout
  wrapping completes at 3s with `isError: false` — failing the
  `assert_is_error_result` check. The error JSON shape (`code`, `server`,
  `tool`, `message`, `recoverable: true`) is then asserted; a stub that
  returns a generic error fails the `code: "upstream_timeout"` check.
- **Cancellation is a wall-clock side-effect assertion.**
  `cancellation_frees_local_resources_without_waiting_full_upstream` issues
  a 3s slow call, sends `notifications/cancelled`, then issues a fast call
  on the SAME server and asserts it completes within 500ms — strictly
  shorter than the slow delay. If the cancellation did not free local
  resources, the fast call would block behind the slow one.
- **Cross-upstream non-serialization under timeout (D-007 / GOTCHA #16).**
  `slow_timed_out_call_does_not_block_concurrent_sibling` issues a slow
  alpha call (under a 1s timeout) and a concurrent beta echo, and asserts
  the beta echo completes within 400ms — strictly shorter than the 1s
  timeout. A registry lock held across the timeout await would serialize the
  session; the beta echo would block until alpha's timeout fired (>= 1s).
- **Hard-kill orphan is a side-effect assertion on the process LIVENESS,
  not the marker file.** `hard_kill_orphan_test_no_surviving_descendants`
  spawns a grandchild via the probe (the probe writes the grandchild's PID
  as the marker content), force-kills fanin-mcp, waits a cleanup interval,
  and asserts the grandchild PROCESS is DEAD by probing its PID. The
  marker file may PERSIST — Job Object `KILL_ON_JOB_CLOSE` / process-group
  kill is a hard `TerminateProcess` that kills the grandchild WITHOUT
  running its cleanup (the grandchild removes the marker only on a clean
  exit after its 30s sleep; a force-kill of fanin-mcp also skips Rust
  `Drop`). So marker-absence cannot distinguish "contained/killed" from
  "survived" — both leave the marker. The dead PROCESS is the oracle: a
  contained tree leaves the PID dead at the check; an uncontained tree
  leaves the orphan alive and the PID still resolves — the failure the
  test catches. Cross-platform liveness probe: `kill -0 <pid>` on Unix,
  `tasklist /FI "PID eq <pid>"` on Windows (no `libc`/`windows-sys` test
  dep — shell-out only). This is the MANDATORY D-009 proof.
- **Stdin-EOF teardown is a side-effect assertion on the full tree's
  LIVENESS, not the marker file.**
  `stdin_eof_teardown_terminates_full_upstream_tree` spawns a grandchild,
  captures its PID from the marker, closes stdin (EOF => fanin-mcp exits),
  waits, and asserts the grandchild PROCESS is DEAD by probing its PID. A
  teardown that killed only the aggregator (not the tree) would leave the
  grandchild alive and the PID still resolving. The marker may persist
  (kill does not run cleanup); the dead PID is the oracle, mirroring the
  hard-kill test.
- **Stderr capture intact is a side-effect assertion on the log file.**
  `stderr_capture_intact_after_process_wrapping` invokes a tool that makes
  the probe write to its stderr, then reads the log file and asserts at
  least one `[server]`-prefixed line. A process wrapper that broke stderr
  capture would leave the log empty.
- **No stdout diagnostics (GOTCHA #1).** Every wire test implicitly asserts
  clean JSON on stdout — the harness panics on a non-JSON line. Phase 3
  adds no stdout-writing path.

## Deferred tests

| Test | Reason | Unblock trigger |
|---|---|---|
| `cred_store::keyring_round_trip_succeeds_when_keyring_available` | `#[ignore = "requires a usable OS keyring; re-enable on a host with a keyring daemon"]` | Re-enable on a host with a working OS keyring service (D-Bus Secret Service on Linux, Keychain on macOS, Credential Manager on Windows). The always-run path (`env_fallback_resolves_without_keyring`, `credential_resolution_order_env_fallback_then_server_wide_fail_closed`) covers the env-fallback half of SC 6/7 on every host. |

## Coverage gaps & boundaries

These are criteria the suite does NOT fully prove at the wire level, with
the reason and the proxy/boundary that does cover them:

- **Master SC 12 / P2.6 / P3.2 (default-60 timeout).** The wire-level suite
  does NOT wait 60 seconds to prove the default. The default-60 value is
  config/unit-observable: a server with NO `timeout_secs` field uses 60.
  The wire-level proxy is `fast_call_within_timeout_passes_through_byte_faithfully`
  (no timeout field, fast call succeeds within the default). The
  implementer SHOULD add a unit test in `src/config.rs` asserting the
  default is 60; the wire-level suite does not duplicate that. Review
  verifies the default is 60 and the field parses.
- **Master SC 17 / P3.5 (forwarded cancellation).** rmcp `=1.8.0` exposes
  `Peer::notify_cancelled(CancelledNotificationParam)`, but the plan's OQ3
  names the open question: the pinned API may not expose the upstream
  request identity for a typed `peer().call_tool(...)` so the aggregator
  can correlate a downstream cancellation with the hidden upstream request.
  The test asserts the LOCAL observable (SC 16: the cancelled call frees
  local resources without waiting the full upstream duration). If the
  implementer surfaces a structural finding that rmcp `=1.8.0` cannot
  forward the cancellation, that is accepted per OQ3 — the test does NOT
  assert forwarding the API can't support. When a future rmcp pin exposes
  the request identity, add a forwarded-cancellation test (observe the
  upstream receives `notifications/cancelled`).
- **Master SC 19 / SC 20 (platform-specific containment mechanism).** The
  hard-kill orphan test asserts the OUTCOME (zero surviving descendants),
  not the mechanism (Job Object vs process group vs custom transport). It
  runs on both Windows and Unix; the platform-conditional behavior is
  inside the implementer's `process.rs`. The test does not pin to
  `process-wrap` or `command-group` internals (OQ2 resolved: implementer's
  choice, gate decides).
- **Master SC 1 (credential abstraction exists).** The wire-level suite
  proves the resolution ORDER (preferred backend → env fallback →
  structured error) and the env-fallback path. The existence of a
  `CredentialStore` trait with `KeyringStore` and `EnvStore` implementations
  is a structural property review verifies against `src/credentials.rs`;
  the wire-level observable is the resolution behavior.
- **P1.5 (Cargo.toml keeps rmcp pinned, adds only Phase 3 deps).**
  Structural — review verifies. No test asserts on Cargo.toml content. The
  rmcp pin is enforced by the Phase 0 `pinning.rs` test (unchanged).
- **P2.7 (`transport = "http"` still fails startup).** Owned by the Phase 2
  config-validation tests (`tests/integration/config.rs`, unchanged). No
  Phase 3 test weakens this; the orchestrator's OQ1 resolution defers HTTP
  to Phase 5, so Phase 3 does not add an HTTP-transport test.
- **`cred set` exit-0 on a keyring-less host.** The `cred set` test asserts
  exit 0; on a keyring-less host, the implementer's env-fallback backend
  must still exit 0 (or the test fails RED, surfacing the gap). The
  keyring-specific round-trip test is `#[ignore]`-gated; the env-fallback
  `cred set` path is the always-run proof.
- **Grandchild marker race.** The hard-kill test waits 200ms after
  `spawn_grandchild` for the grandchild to write the marker (whose content
  is the grandchild PID) before force-killing fanin-mcp. A slower host
  might need a longer window; the 200ms is tuned for CI. If the marker is
  not present before the kill, the test fails with a clear "marker must be
  present before fanin-mcp is killed" message — a setup failure, not a
  containment failure. The PID is parsed from the marker content; the
  liveness probe (`kill -0` / `tasklist`) is the containment oracle.

## Run-and-fail confirmation

The suite compiles clean (`cargo build --tests` — zero warnings) and runs
with the expected test-first state against the current tree (Phase 2 code
landed, Phase 3 NOT yet built):

- **18 Phase 3 tests total** (10 in `cred_store.rs`, 4 in
  `timeout_cancellation.rs`, 3 in `process_lifetime.rs`, 1 in
  `regression_guard.rs`). 1 ignored (keyring round-trip on headless hosts).
- **`cargo fmt --all -- --check`: CLEAN.** `cargo clippy --all-targets`:
  CLEAN (zero warnings).
- **Against the current tree:** 74 passed, 10 failed RED, 3 ignored (2
  Phase 0 manual E2E + 1 Phase 3 keyring).
  - The 10 RED failures are all Phase 3 behavior assertions:
    - `cred_set_reads_secret_from_hidden_stdin_not_argv` — the `cred set`
      stub exits FAILURE (no real subcommand args wired).
    - `cred_rm_makes_later_resolution_not_return_value` — the `cred rm`
      stub exits FAILURE.
    - `dollar_brace_interpolation_resolves_and_literal_values_pass_through`
      — `process.rs` injects env literally with `${KEY}` unresolved; the
      probe sees the literal `${KEY}` string, not the sentinel.
    - `missing_credential_returns_structured_error_not_rpc_error` — no
      interpolation, so the probe echoes the literal `${...}` instead of a
      structured missing-credential error.
    - `per_upstream_env_isolation_sibling_and_ambient_not_inherited` — the
      current `process.rs` inherits the aggregator's full env (no
      least-privilege filtering), so the probe sees the ambient var.
    - `credential_resolution_order_env_fallback_then_server_wide_fail_closed`
      — no interpolation; the env-fallback value does not resolve.
    - `timeout_secs_wraps_upstream_call_and_returns_structured_error` — no
      timeout wrapping; the slow call completes at 3s with `isError: false`.
    - `slow_timed_out_call_does_not_block_concurrent_sibling` — no timeout
      wrapping; the slow alpha call returns success instead of
      `upstream_timeout`.
    - `hard_kill_orphan_test_no_surviving_descendants` — no containment;
      the grandchild survives fanin-mcp's force-kill and its PID is still
      alive at the check (the marker may persist, but the PID liveness is
      the oracle).
    - `stdin_eof_teardown_terminates_full_upstream_tree` — no containment;
      the grandchild survives the clean shutdown and its PID is still
      alive (the marker may persist, but the PID liveness is the oracle).
  - The 8 Phase 3 tests that PASS are: `cred_list_emits_names_only_never_values`
    (value-absence holds against any stub output), `env_fallback_resolves_without_keyring`
    (no-echo holds), `sentinel_secret_redacted_from_tracing_stderr_and_upstream_logs`
    (the sentinel never reaches the log because interpolation is absent —
    the test still catches a leak once interpolation lands),
    `fast_call_within_timeout_passes_through_byte_faithfully` (fast call
    succeeds without timeout wrapping), `cancellation_frees_local_resources_without_waiting_full_upstream`
    (the fast-after-cancel call completes — the current registry lock
    discipline already supports concurrency),
    `stderr_capture_intact_after_process_wrapping` (the current
    `TokioChildProcess` path already captures stderr), and
    `phase3_config_preserves_phase012_guarantees` (Phase 0/1/2 guarantees
    hold under a Phase 3 config).
- **Full suite (Phase 0 + 1 + 2 + 3):** 74 passed, 10 failed (the
  expected-red Phase 3 behavior tests), 3 ignored. No regressions to
  Phase 0/1/2 tests — the probe tool-count correction (8 → 10) is a factual
  update matching the extended probe fixture, not a weakening.
- **The red is meaningful:** every RED test fails on a Phase 3 behavior
  assertion (cred stub, no interpolation, no isolation, no timeout, no
  containment), not on a compile error, a missing symbol, or a malformed
  harness. The implementer turns each RED green by building the Phase 3
  logic in `src/credentials.rs`, `src/config.rs`, `src/registry.rs`,
  `src/process.rs`, and `src/main.rs`.

### Resolution-model rewrite — Option A fail-closed (post-Phase-3-landed)

The credential resolution model was decided: **fail-closed (Option A)**.
The old `credential_resolution_order_env_fallback_then_structured_error`
encoded the partial-resolve contract (Option B): one server with BOTH a
resolvable var and a missing var, expecting that single server to spawn and
deliver the resolvable var while the missing var returned a per-call
structured error only when `echo_env(key=<bad-lhs>)` was invoked. That
contradicts the decided fail-closed model — the proxy is name-level only and
cannot know which tool needs which credential, so any unresolvable configured
`${VAR}` must fail the WHOLE server.

Two tests were rewritten to specify the new contract:

- **`credential_resolution_order_env_fallback_then_structured_error`** →
  renamed **`credential_resolution_order_env_fallback_then_server_wide_fail_closed`**.
  Split into two INDEPENDENT servers: (1) env-fallback delivery (a server
  whose `${VAR}` resolves ONLY via process-env fallback — `echo_env` returns
  the value byte-faithfully); (2) fail-closed (a SEPARATE server with an
  unresolvable `${DEFINITELY_MISSING}` var — a GENERIC `echo_ok` call returns
  the structured `credential_resolution_failed` error).
- **`missing_credential_returns_structured_error_not_rpc_error`** generalized
  from `echo_env(key=env_name)` to a generic `echo_ok` call (no matching key
  arg), so the failure must be server-wide, not gated on the `echo_env` tool
  shape.

**Expected RED against the current (Option B) code:** the current
`registry.rs` records bad env LHS names at spawn and only short-circuits
`echo_env(key=<bad-lhs>)` in `call_tool`. A generic `echo_ok` call sails
through to the probe and returns a successful result (`isError: false`), so
both rewritten tests fail the `isError: true` assertion — the clean,
meaningful RED the implementer turns green by failing the whole server when
any configured `${VAR}` is unresolvable. The env-fallback delivery half (1)
still passes against the current code (env fallback already delivers).

### Oracle correction (post-implementation)

The original `process_lifetime` oracle (marker-absence) was incorrect:
Job Object `KILL_ON_JOB_CLOSE` / process-group kill hard-terminates the
grandchild WITHOUT running its cleanup, so the marker PERSISTS even though
the process is dead. The oracle was corrected to grandchild-PID LIVENESS:
the marker's CONTENT is the grandchild PID (the probe writes
`std::process::id().to_string()`); the test captures that PID before
teardown and polls for the PID's death within a bounded window
(`CLEANUP_INTERVAL`, 5s — far shorter than the 30s grandchild lifetime).
The marker may remain as the PID-communication channel; the test owns its
temp-file cleanup.

**Evidence on the current tree (Windows host):**

- `stdin_eof_teardown_terminates_full_upstream_tree` (SC 22): the grandchild
  dies in ~400ms after stdin-EOF. Clean shutdown runs Rust `Drop`, which
  fires `process_wrap`'s `KillOnDrop`, killing the job members. Containment
  works on this path. **PASS** when run in isolation.
- `hard_kill_orphan_test_no_surviving_descendants` (SC 21): the grandchild
  is STILL ALIVE 5s after `guard.kill_and_wait()`. A force-kill of fanin-mcp
  skips Rust `Drop`, so `KillOnDrop` does not fire; the only kill mechanism
  is `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` when the job HANDLE closes. The
  grandchild lives its full 30s natural lifetime and exits cleanly (which
  removes the marker — the original broken oracle would have called this
  "contained"). The detached `probe-server.exe` grandchild IS confirmed
  alive at PID-from-marker cross-check (`tasklist` lists it). **FAIL — this
  is a production containment defect, not a test oracle error.**

The likely root cause (for the implementer, not the test): the probe (the
direct child in the job) inherits a clone of the Job Object handle, so the
job does not close when fanin-mcp is hard-killed — it stays open until the
probe (and its grandchild) die on their own. `KILL_ON_JOB_CLOSE` only fires
when the LAST handle to the job closes. The fix is in `src/process.rs`:
ensure the Job Object handle is non-inheritable (or use a dedicated
non-inherited handle) so the child does not hold the job open. This is
exactly the classic Windows Job Object pitfall D-009 / GOTCHA #11/#14 warn
against.

- **`cargo test --test integration` (current tree, Windows): 82 passed /
  2 failed / 3 ignored** when run in parallel (the stdin-EOF test flakes
  under parallel load at the 5s window; in isolation it passes at ~400ms).
  The 2 failures are both `process_lifetime` — the hard-kill path is a
  real defect; the stdin-EOF path works in isolation.
- `gate.rs::credential_store_flag_is_accepted` (renamed from
  `phase1_does_not_accept_phase3_credential_store_flag`) passes for the
  right reason: `--credential-store keyring` is an ACCEPTED global flag
  (clap does not reject it; the server starts and answers `initialize`),
  not the stale "unknown flag rejected" premise.

## Per-module inventory

| Module | Tests |
|---|---|
| `cred_store` (Phase 3) | 10 (1 ignored) |
| `timeout_cancellation` (Phase 3) | 4 |
| `process_lifetime` (Phase 3) | 3 |
| `regression_guard` (Phase 3) | 1 |
| **Phase 3 total** | **18** (1 ignored) |
| Phase 0/1/2 modules (behavior unchanged; probe count corrected 8 → 10) | 67 (2 ignored) |
| **Grand total** | **85** (3 ignored) |

## Probe-fixture additions

The probe fixture (`tests/probe-server/main.rs`) is extended with two new
tools and a grandchild-mode branch, all owned by `test-creator`:

- **`echo_env`** — reads the env var named `key` from the probe's visible
  environment and echoes its value (or `<absent>`). Used by the per-upstream
  env isolation proof (SC 9). The probe never invents a value; a missing
  key is reported honestly.
- **`spawn_grandchild`** — spawns a long-lived descendant process (re-exec
  of the probe binary in grandchild mode) that writes a presence marker at
  `marker_path` whose CONTENT is the grandchild's PID, then sleeps for 30s
  and removes the marker only on a CLEAN exit. Used by the hard-kill orphan
  proof (SC 21) and the stdin-EOF teardown proof (SC 22). The grandchild is
  spawned with stdin/stdout/stderr null so it never touches the probe's MCP
  stdio stream (GOTCHA #1). On Windows it is detached
  (`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`) so it can survive a plain
  kill of the probe — the containment layer must catch it instead. The
  test oracle is the grandchild PID LIVENESS (parsed from the marker), not
  the marker's absence — Job Object / process-group kill hard-terminates
  the grandchild without running its cleanup, so the marker persists even
  when containment succeeds; the dead PID is the oracle.
- **Grandchild sentinel branch** — `__grandchild__ <marker_path> <secs>`
  argv selects a branch of `main` that writes the marker, sleeps, and
  removes the marker on a clean exit. Runs BEFORE tracing/serve setup so it
  never touches the MCP stdio stream.

The probe now exposes 10 tools. The Phase 0/1/2 `PROBE_TOOL_NAMES`
constant is updated 8 → 10 in `probe.rs`, `discovery.rs`, `multi_upstream.rs`,
and `namespace_acl.rs` — a factual correction, not a weakening.

## Notes for the implementer

- The `cred set|list|rm` CLI subcommands are currently stubs in `src/main.rs`
  with NO positional operands. The implementer MUST add `<server> <KEY>`
  positionals to `CredAction::Set` and `CredAction::Rm`, and `<server>` to
  `CredAction::List`. The tests invoke `fanin-mcp cred set <server> <KEY>`
  with the secret on stdin (never argv). A `--value` flag or positional
  secret MUST NOT be added (SC 3, D-010, GOTCHA #18).
- `cred set` reads the secret through a hidden stdin prompt (`rpassword` or
  equivalent). The test feeds `"<secret>\n"` on the child's stdin pipe. The
  prompt must NOT echo the value to stdout or stderr (SC 3).
- `cred list` emits ONLY credential names. The test asserts the secret
  VALUE never appears in stdout or stderr. A `cred list` that prints
  nothing passes the value-absence assertion, but the keyring round-trip
  test (when un-ignored) catches the missing name.
- `${VAR}` interpolation lands in `src/process.rs` (or `src/registry.rs`)
  at spawn time: resolve each configured env value through the preferred
  credential backend, then process env fallback, then a structured
  missing-credential error. The test configures `env.TOKEN = "${SECRET_KEY}"`
  and asserts the probe's `echo_env` sees the resolved sentinel, not the
  literal `${SECRET_KEY}`.
- Per-upstream env isolation (SC 9, D-010): the spawned child must receive
  ONLY its own configured env keys. The current `process.rs` iterates
  `config.env` and calls `cmd.env(key, value)` — but `tokio::process::Command`
  inherits the parent's full env by default. The implementer MUST call
  `cmd.env_clear()` before injecting the configured vars, OR construct the
  env map explicitly. The test asserts the probe does NOT see a sibling
  server's var or an aggregator-ambient var.
- The sentinel-redaction layer (SC 11) must register resolved secret values
  with a redaction component BEFORE any value can be logged. The test
  configures a sentinel as a resolved env value and asserts the sentinel
  string appears in NONE of: the aggregator's tracing stderr, the child
  stderr log file. The current tree has no redaction; the test passes today
  only because interpolation is absent (the sentinel never reaches the
  resolved env). Once interpolation lands, the redaction layer must be in
  place or the test fails RED.
- `timeout_secs` is added to `ServerConfig` in `src/config.rs` with default
  60. The wire-level test uses `timeout_secs = 1`. The implementer wraps
  every upstream `call_tool` in `tokio::time::timeout(effective_timeout, ...)`
  in `src/registry.rs`. On timeout, return `ToolError::UpstreamTimeout`
  rendering JSON with `code: "upstream_timeout"`, server, tool, message,
  `recoverable: true` inside `CallToolResult::error(...)`. NOT a JSON-RPC
  error (D-005, GOTCHA #3).
- Cancellation (SC 16): the implementer tracks in-flight calls by the
  smallest stable key rmcp exposes in `RequestContext<RoleServer>`. On
  `notifications/cancelled`, abort the local future. The test sends
  `notifications/cancelled` with the request id and asserts a subsequent
  fast call on the SAME server completes within 500ms. If rmcp `=1.8.0`
  cannot forward the cancellation upstream (OQ3), surface that as a
  structural finding — the test does not assert forwarding.
- Process-tree containment (SC 19/20/21): the implementer evaluates
  `process-wrap` first (D-009 prefers it). If it cannot wrap rmcp's
  `TokioChildProcess` safely, implement a thin custom child transport in
  `src/process.rs`. The hard-kill test asserts the OUTCOME (zero surviving
  descendants after force-killing fanin-mcp), not the mechanism. The
  grandchild is a detached descendant of the probe; the containment layer
  must kill the whole tree.
- `ChildGuard::kill_and_wait` is the force-kill path used by the hard-kill
  test. It kills the aggregator child immediately (no stdin EOF) and waits
  for it to be reaped. This simulates `taskkill /F` / `kill -9`.
- Phase 0/1/2 tests are read-only and unchanged in behavior. The probe
  tool-count correction (8 → 10) is the only edit — it is a factual update
  matching the extended probe fixture. Do not weaken the static-3-meta-tools,
  byte-faithful, reverse-traffic, lazy-startup, or namespace-ACL assertions.