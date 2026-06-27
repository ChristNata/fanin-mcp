# Tests: remediation-s1-d1

## Files Created

| Path | Criteria Covered |
|---|---|
| `tests/integration/remediation_s1_d1.rs` | Master SC 1-17; Phase 1 SC 1-7; Phase 2 SC 1-7 |
| `tests/probe-server/main.rs` | Probe modes for S-1 hangs; `report_cwd` fixture for D-1 |
| `tests/common/fixtures.rs` | `cwd` rendering in config builders / TOML helpers |
| `tests/integration/main.rs` | Registers the remediation integration module |

## Coverage Map

| Criterion | Test |
|---|---|
| Master SC 1 / P1.SC1: hung during initialize returns `upstream_timeout` within bound | `s1_hang_during_initialize_returns_structured_timeout_within_bound` |
| Master SC 2 / P1.SC2: hung initial `list_all_tools` returns `upstream_timeout` within bound | `s1_hang_during_initial_list_tools_returns_structured_timeout_within_bound` |
| Master SC 3 / P1.SC3: stalled Streamable-HTTP connect / initialize returns `upstream_timeout` within bound | `s1_http_stall_during_initialize_returns_structured_timeout_within_bound` |
| Master SC 4 / P1.SC4: hung dirty-refetch returns `upstream_timeout` within bound | `s1_hang_during_dirty_refetch_times_out_keeps_dirty_and_preserves_cache` |
| Master SC 5 / P1.SC4: dirty-refetch timeout leaves dirty state for retry and preserves prior cache | `s1_hang_during_dirty_refetch_times_out_keeps_dirty_and_preserves_cache` |
| Master SC 6 / P1.SC5: cold-connect timeout leaves no cached entry and later call fresh-connects | `s1_cold_connect_timeout_retries_and_releases_init_guard` |
| Master SC 7 / P1.SC5: cold-connect timeout releases init guard for later same-server call | `s1_cold_connect_timeout_retries_and_releases_init_guard` |
| Master SC 8 / P1.SC6: timed-out stdio connect kills spawned descendant during the test window | `s1_timed_out_connect_kills_spawned_descendant_during_test_window` |
| Master SC 9 / P1.SC7: hung cold connect on A does not block connected server B | `s1_hung_cold_connect_does_not_block_already_connected_sibling` |
| Master SC 10 / P2.SC4: optional `cwd` accepted; empty / whitespace configured `cwd` rejected at config load | `d1_empty_or_whitespace_cwd_fails_config_validation_before_serving`; positive acceptance is exercised by all `cwd` success configs |
| Master SC 11 / P2.SC1: literal stdio `cwd` becomes child working directory | `d1_literal_cwd_is_child_working_directory` |
| Master SC 12 / P2.SC2: `${VAR}` stdio `cwd` resolves through existing resolver | `d1_cwd_resolves_env_placeholder_through_existing_resolver` |
| Master SC 13 / P2.SC3: absent `cwd` inherits aggregator CWD | `d1_unset_cwd_inherits_aggregator_working_directory` |
| Master SC 14 / P2.SC5: `${VAR}` resolving blank fails before spawn with structured tool error | `d1_resolved_blank_cwd_fails_before_spawn_with_structured_tool_error` |
| Master SC 15 / P2.SC6: non-existent stdio `cwd` surfaces `upstream_connect_failed` | `d1_nonexistent_cwd_surfaces_upstream_connect_failed` |
| Master SC 16 / P2.SC7: Streamable-HTTP `cwd` is ignored, not resolved or applied | `d1_http_cwd_is_ignored_and_not_resolved_or_applied` |
| Master SC 17: `cargo test --all`, clippy, and fmt are clean once implementation lands | Verification commands below; current red tests are the intended S-1/D-1 contract failures |

## Side-Effect Assertions

- Timeout tests assert elapsed wall-clock duration as well as structured error
  shape. The probe hangs indefinitely or stalls its HTTP response, so a quick
  return without a real timeout cannot satisfy the contract.
- Dirty-refetch asserts the subsequent inventory read retries and the previous
  cache is not treated as overwritten-empty.
- Process containment asserts liveness of the specific descendant PID during
  the test, after the timeout response, rather than relying on a post-run sweep.
- `d1_empty_or_whitespace_cwd_fails_config_validation_before_serving` asserts
  the startup side effect: no MCP serving starts and stdout remains empty.
- `report_cwd` asserts the child process's actual working directory, not a
  returned config value.

## Deferred Tests

None for this plan. No new S-1/D-1 test is ignored.

Existing ignored tests remain outside this plan's authorship:

- `cred_store::keyring_round_trip_succeeds_when_keyring_available` — requires a
  usable OS keyring.
- `error_hardening::f4_send_side_death_returns_upstream_disconnected_not_call_failed`
  — blocked on deterministic send-side failure fixture.
- `manual_e2e::live_cc_discovers_exactly_three_meta_tools` and
  `manual_e2e::live_oc_discovers_exactly_three_meta_tools` — blocked on live MCP
  client CI jobs.

## Verification

- `cargo fmt --check` — pass.
- `cargo clippy --all-targets -- -D warnings` — pass.
- `cargo test --all --no-fail-fast` — expected red before implementation:
  117 passed, 12 failed, 4 ignored. The failing tests are the new S-1/D-1
  contract tests. Existing tests stayed green after gating `report_cwd` behind
  `--enable-report-cwd`.

## Coverage Gaps / Ambiguities

- No criterion is uncovered.
- Master SC 13 and SC 16 are already true in the current tree by inheritance and
  by unknown-field ignore behavior. Their tests may pass before implementation;
  they remain necessary regression guards once `cwd` becomes a real field.
- The public error code for blank resolved `cwd` is not named in the plan. The
  test asserts structured tool-level failure and prompt return, not a specific
  code, so the implementer cannot satisfy it with a panic or hang.
