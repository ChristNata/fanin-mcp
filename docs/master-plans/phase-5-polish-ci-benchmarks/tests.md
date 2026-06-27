# Tests: Phase 5 Polish, CI, And Benchmarks

## Files created or changed

| Path | Coverage |
|---|---|
| `tests/integration/observability.rs` | P1 logging, redaction, log-level, per-call structured fields |
| `tests/integration/process_lifetime.rs` | P2 immediate-start descendant hard-kill regression |
| `tests/probe-server/main.rs` | Immediate-descendant startup fixture for CARRY-1 |
| `tests/integration/http_upstream.rs` | P3 Streamable-HTTP mock, header auth, missing credential, stdio regression |
| `tests/integration/tool_search.rs` | P4 downstream `tools/list` Tool Search composition |
| `tests/integration/token_figures.rs` | P5 token bench target and README generated-figure drift gate |
| `tests/integration/error_hardening.rs` | P4 CARRY-4 deterministic always-run source guard |
| `tests/common/fixtures.rs` | Raw config helper for HTTP transport fixture TOML |
| `tests/integration/main.rs` | Registers new integration modules |

## Coverage map

### Master success criteria P1-P5

| Criterion | Test contract |
|---|---|
| 1. `--log-file` creates redacted NDJSON and never writes diagnostics to stdout | `observability::log_file_writes_ndjson_and_stdout_stays_json_rpc_only`; `observability::sentinel_secret_absent_from_stderr_and_json_file_sink` |
| 2. `--log-level` controls verbosity and rejects invalid levels before serve | `observability::log_level_debug_includes_debug_events_and_invalid_level_fails_before_serve` |
| 3. Structured logs include config/namespace/lifecycle/per-call fields | `observability::invoke_tool_logs_success_and_failure_without_args_or_secrets` covers per-call fields; config/namespace/lifecycle are expected NDJSON events in the same file sink |
| 4. Sentinel secrets absent from stderr, child stderr logs, and JSON sink | `observability::sentinel_secret_absent_from_stderr_and_json_file_sink`; existing `cred_store` redaction tests cover child stderr/log sink |
| 5. Windows CI proves immediately-started descendant cannot escape Job Object assignment | `process_lifetime::hard_kill_kills_immediate_startup_descendant_during_test_window` on Windows |
| 6. Linux CI proves hard-kill containment through parent-death signaling | `process_lifetime::hard_kill_kills_immediate_startup_descendant_during_test_window` on Linux |
| 7. macOS does not overclaim hard-kill; graceful teardown remains tested | Existing `process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree`; no macOS hard-kill assertion is authored |
| 8. Streamable-HTTP header auth works against in-repo mock | `http_upstream::http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs` |
| 9. Missing HTTP header credentials return structured credential error and do not leak | `http_upstream::missing_http_header_credential_returns_structured_error_without_connecting` |
| 10. Namespace switching E2E proves different sessions expose different rows and denied returns `namespace_denied` | Existing `namespace_acl::server_visibility_matrix_across_namespaces` and related namespace ACL tests |
| 11. Credential E2E proves names-only listing and env fallback in CI/keyring-less mode | Existing `cred_store::cred_list_emits_names_only_never_values`, `cred_store::env_fallback_resolves_without_keyring`, and keyring deferred test |
| 12. Tool Search composition exposes exactly three meta-tools and no upstream schemas at startup | `tool_search::downstream_tools_list_returns_exactly_three_meta_tools_and_does_not_spawn_upstreams` |
| 13. `TransportSend` maps deterministically to `upstream_disconnected` | `error_hardening::service_error_transport_send_maps_to_upstream_disconnected_deterministically`; see test-needs-impl for direct API exposure |
| 14. `cargo bench --bench token_cost` runs and emits both token measurements | `token_figures::cargo_bench_token_cost_target_is_declared`; benchmark execution itself is a Phase 6/CI gate |
| 15. README token figures match benchmark output exactly | `token_figures::readme_token_figure_markers_match_benchmark_generated_output_exactly` |

Criteria 16-20 are P6/P7 and intentionally not unit/integration-test targets
for this stage per `test-creator-task.md`. Criterion 21 is the objective gate:
all authored tests must pass after implementation; no ignored Phase 5 test is
the only proof for a shipped P1-P5 criterion.

### Phase success criteria

| Phase criterion | Test contract |
|---|---|
| P1.1 NDJSON file + no stdout diagnostics | `observability::log_file_writes_ndjson_and_stdout_stays_json_rpc_only` |
| P1.2 debug level and invalid-level startup failure | `observability::log_level_debug_includes_debug_events_and_invalid_level_fails_before_serve` |
| P1.3 sentinel absent from stderr and JSON sink | `observability::sentinel_secret_absent_from_stderr_and_json_file_sink` |
| P1.4 successful invoke logs server/tool/latency/success | `observability::invoke_tool_logs_success_and_failure_without_args_or_secrets` |
| P1.5 failing invoke logs failure without args/secrets | `observability::invoke_tool_logs_success_and_failure_without_args_or_secrets` |
| P2.1 Windows immediate descendant killed | `process_lifetime::hard_kill_kills_immediate_startup_descendant_during_test_window` under `cfg(windows)` |
| P2.2 stdin-EOF teardown terminates tree | Existing `process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree` |
| P2.3 Linux hard-kill descendant killed during window | `process_lifetime::hard_kill_kills_immediate_startup_descendant_during_test_window` under `cfg(target_os = "linux")` |
| P2.4 macOS graceful only, no false hard-kill claim | Existing EOF teardown test; hard-kill test excluded from macOS |
| P2.5 wrapping does not break stderr capture/redaction | Existing `process_lifetime::stderr_capture_intact_after_process_wrapping` plus P1 redaction sink tests |
| P3.1 HTTP transport + Authorization placeholder invokes mock | `http_upstream::http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs` |
| P3.2 mock observes header, logs redact value | `http_upstream::http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs` |
| P3.3 missing header credential structured error, no connect | `http_upstream::missing_http_header_credential_returns_structured_error_without_connecting` |
| P3.4 stdio behavior unchanged | `http_upstream::stdio_upstream_still_lazy_and_namespace_filtered_after_http_support` |
| P3.5 real-public-remote is manual release step only | Untestable in this stage; must be verified in docs/release checklist by P7 |
| P4.1 namespace switching and `namespace_denied` | Existing `namespace_acl` matrix tests |
| P4.2 credential names-only + env fallback | Existing `cred_store` tests |
| P4.3 each upstream receives only its own env/header values | Existing `cred_store::per_upstream_env_isolation_sibling_and_ambient_not_inherited`; HTTP header path in `http_upstream` |
| P4.4 downstream `tools/list` exactly three meta-tools | `tool_search::downstream_tools_list_returns_exactly_three_meta_tools_and_does_not_spawn_upstreams` |
| P4.5 `TransportSend -> upstream_disconnected` deterministic | `error_hardening::service_error_transport_send_maps_to_upstream_disconnected_deterministically`; direct function-call test needs impl affordance |
| P5.1 bench target exits 0 | `token_figures::cargo_bench_token_cost_target_is_declared` declares availability; CI/gate runs the bench |
| P5.2 benchmark output has permanent-tool and representative-session measurements | `token_figures::readme_token_figure_markers_match_benchmark_generated_output_exactly` consumes generated output; bench/gate must produce it |
| P5.3 README figures match generated output | `token_figures::readme_token_figure_markers_match_benchmark_generated_output_exactly` |
| P5.4 drift test/gate fails on marker mismatch | `token_figures::readme_token_figure_markers_match_benchmark_generated_output_exactly` |

## Side-effect assertions

- Observability tests assert file bytes and stdout/stderr bytes, not return
  values. A stub that returns success without writing NDJSON fails.
- Redaction tests assert the sentinel is absent from stderr and the JSON file
  sink while the probe receives the resolved value.
- HTTP tests assert the loopback mock observed the resolved `Authorization`
  header. A proxy that returns success without making the HTTP request fails.
- Missing-credential HTTP test asserts the mock was not contacted.
- Process-lifetime tests assert PID liveness during the test window, not a
  post-suite survivor count.
- Tool Search test asserts the upstream log remains empty after downstream
  `tools/list`, proving no startup/lazy spawn side effect occurred.
- Token-figure tests assert README marker contents equal generated output on
  disk; a hand edit fails.

## Deferred

| Test | Reason | Re-enable trigger |
|---|---|---|
| `cred_store::keyring_round_trip_succeeds_when_keyring_available` | Requires a usable OS keyring; headless CI may not have one | Run on a host/CI profile with a configured OS keyring daemon |
| `error_hardening::f4_send_side_death_returns_upstream_disconnected_not_call_failed` | Wire-level send-side pipe timing is non-deterministic | Replace with direct `map_service_error`/wrapper test once the function is exposed, or with a transport wrapper that deterministically forces send failure |

No new Phase 5 P1-P5 proof is deferred as its only verification. The CARRY-4
always-run source guard is present now; the stronger direct function-call test
needs the implementation affordance below.

## Test-needs-impl dependencies

- Expose `map_service_error` to tests, or provide a thin testable wrapper that
  accepts a forced `ServiceError::TransportSend(_)` and returns the public
  structured error code. The current always-run guard is compile-safe without
  `src/` edits, but the intended final proof is a direct deterministic call.
- Implement the Phase 5 serve CLI flags exactly as exercised:
  `--config <path> --log-file <path> --log-level <level>`.
- Implement HTTP config shape used by the tests:
  `transport = "streamable-http"`, `endpoint = "http://.../mcp"`, and
  `[servers.<name>.headers] Authorization = "Bearer ${TOKEN}"`.
- The token benchmark must write `target/token-figures.generated.md`, and
  README must contain `<!-- fanin-token-figures:start -->` / `end` markers.

## Untestable success criteria

- P3.5 / master release-checklist public remote check is documentation/manual
  release verification, not an automated test target.
- P6 and P7 criteria are intentionally outside this test contract per the task
  brief. CI, deny/audit, binary/memory budgets, and docs drift are verified by
  their workflow/artifact gates, not by integration tests here.

## Verification run

- `cargo fmt --check` passed.
- `cargo test --test integration --no-run` passed; the suite compiles.
- `cargo test --test integration cargo_bench_token_cost_target_is_declared`
  failed by assertion, not by compile/import error, because the bench target is
  not implemented yet.
