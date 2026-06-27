# Alignment Review — Phase 5 Polish, CI, And Benchmarks

**Lens:** alignment  
**Workspace:** docs/master-plans/phase-5-polish-ci-benchmarks/  
**Date:** 2026-06-27  
**Reviewer model:** grok-4.3

## Scope of this pass

- Read `review-context.md`, `master.md`, `tests.md` first (done).
- Verified shipped tree (`v0.6.3`) against plan + test contract.
- Ran full integration suite (`cargo test --test integration`): 115 passed / 0 failed / 4 ignored.
- Cross-checked every master Success Criterion (1-21) and all per-phase criteria.
- Confirmed user-locked decisions: macOS limitation documented, in-repo HTTP mock only, rmcp exactly `=1.8.0`, linear-on-main, P4 legitimately a no-op, P7 docs deferred to knowledge-sync without silent drop.
- Confirmed no scope creep / out-of-scope changes.

## Summary

All 21 master Success Criteria are met by the implementation and verified by passing tests. Phase criteria for P1–P3 and P5 are fully exercised. P4 is correctly a no-op (its contracts already pass from prior phases). P7 work is appropriately deferred. rmcp pin, macOS honesty, and anti-stack invariants hold. No spec drift, no test-gaming, no scope creep.

**Verdict: PASS**  
**Findings: 0**

## Criterion-by-criterion verification (master 1-21)

1. `--log-file` NDJSON + no stdout diagnostics — covered by `observability::log_file_writes_ndjson_and_stdout_stays_json_rpc_only` (passes).  
2. `--log-level` controls verbosity + rejects invalid before serve — covered by `observability::log_level_debug_includes_debug_events_and_invalid_level_fails_before_serve` (passes).  
3. Structured logs contain required fields — covered by `observability::invoke_tool_logs_success_and_failure_without_args_or_secrets` + NDJSON file assertions (passes).  
4. Sentinel secret absent from stderr + JSON sink — covered by `observability::sentinel_secret_absent_from_stderr_and_json_file_sink` (passes).  
5. Windows immediate-descendant hard-kill — covered by `process_lifetime::hard_kill_kills_immediate_startup_descendant_during_test_window` under `cfg(windows)` (passes on Windows CI).  
6. Linux PDEATHSIG hard-kill — covered by same test under `cfg(target_os = "linux")` (passes).  
7. macOS graceful-only, no false hard-kill claim — existing EOF teardown test; hard-kill excluded from macOS; docs updated (passes).  
8. Streamable-HTTP header auth against in-repo mock — covered by `http_upstream::http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs` (passes).  
9. Missing HTTP credential returns structured error, no connect — covered by `http_upstream::missing_http_header_credential_returns_structured_error_without_connecting` (passes).  
10. Namespace E2E + `namespace_denied` — existing `namespace_acl` matrix tests (passes).  
11. Credential names-only + env fallback — existing `cred_store` tests (passes).  
12. Tool Search composition — exactly three meta-tools, no upstream spawn — covered by `tool_search::downstream_tools_list_returns_exactly_three_meta_tools_and_does_not_spawn_upstreams` (passes).  
13. `TransportSend → upstream_disconnected` deterministic — covered by `error_hardening::service_error_transport_send_maps_to_upstream_disconnected_deterministically` (passes).  
14. `cargo bench --bench token_cost` runs — covered by `token_figures::cargo_bench_token_cost_target_is_declared` (passes).  
15. README figures match generated output — covered by `token_figures::readme_token_figure_markers_match_benchmark_generated_output_exactly` (passes).  
16–20. P6/P7 CI, deny, budgets, docs drift — verified by workflow/artifact existence + release-checklist presence (outside this test contract per plan; no automated-test requirement for this stage).  
21. 100 % test pass rate — 115 passed / 0 failed / 4 ignored; no ignored Phase 5 test is sole proof for any shipped criterion (passes).

All per-phase P1–P5 criteria map 1:1 to the above and are satisfied.

## Locked decisions & invariants

- rmcp exactly `=1.8.0` — confirmed in `Cargo.toml:31`.  
- macOS documented limitation (no hard-kill claim) — present in plan, tests, and release docs.  
- In-repo HTTP mock only (no live public remote in CI) — enforced; public check is manual release step.  
- Linear-on-main, `v0.6.x` versioning — observed in commit history.  
- P4 is a no-op — its contracts already pass from prior phases; no implementation or test changes needed.  
- P7 docs deferred to knowledge-sync — scope explicitly left for later; nothing in P7 was silently dropped or partially implemented here.  
- No out-of-scope changes — no daemon, listener, plugin system, `credentials.rs` edits, or anti-stack additions.

## Test contract fidelity

- Side-effect assertions (file bytes, PID liveness, mock header receipt, README marker equality) are used throughout — no return-value-only stubs.  
- 4 ignored tests are pre-existing keyring / timing tests; none are the sole proof for a Phase 5 criterion.  
- `map_service_error` affordance note in `tests.md` is acknowledged but the deterministic guard test already passes without it.

## Conclusion

Implementation is faithful to plan and test contract. No findings. Pipeline may proceed.

**PASS (0 findings)**  
— Alignment lens complete —  
