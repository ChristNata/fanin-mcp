# test-creator: phase-0-skeleton

Phase 0 test contract. The implementer codes against this suite; the
objective gate runs it. Test files are read-only to every later stage.

## Stack & runner

- **Runner:** `cargo nextest run --workspace` (main suite) +
  `cargo test --workspace --doc` (doc-tests, separately).
- **Async:** `#[tokio::test]` single-threaded default; concurrency tests use
  `flavor = "multi_thread"` (none in Phase 0).
- **Wire-level by default.** Tests spawn the built binary and speak raw
  JSON-RPC over stdio, asserting on the wire JSON — decoupling the contract
  from rmcp's fast-moving Rust API (D-015). The only non-wire test is the
  pinning gate (file-content regex check on `Cargo.toml`).

## Files created

| Path | Criteria covered |
|---|---|
| `tests/common/mod.rs` | Shared JSON-RPC-over-stdio harness (spawn, request/read, initialize, timeout bounds, ChildGuard kill-on-drop) |
| `tests/common/expectations.rs` | Canonical static-description + meta-tool-name expectations (single source of truth) |
| `tests/integration/main.rs` | Integration test binary entry; `mod` declarations only |
| `tests/integration/aggregator.rs` | Criteria 2, 3, 4, 5 |
| `tests/integration/probe.rs` | Criteria 6, 7, 8 |
| `tests/integration/pinning.rs` | Criterion 9 |

## Coverage map

| # | Master Success Criterion | Test(s) |
|---|---|---|
| 1 | Build gate (`cargo build` + `cargo clippy -- -D warnings`) | **gate-enforced** — the implement-stage cargo build/clippy gate runs this; a test file cannot assert its own compilation. Noted in P0.1/P0.2/P0.3 Phase Success Criteria. |
| 2 | Static discovery — `initialize` + `tools/list` returns exactly 3 meta-tools with exact static descriptions | `static_discovery_returns_three_meta_tools_with_exact_descriptions` (aggregator.rs) |
| 3 | Annotation gate — `invoke_tool` carries `destructiveHint=true`, `readOnlyHint=false`, `openWorldHint=true` | `invoke_tool_carries_conservative_annotations` (aggregator.rs) |
| 4 | Startup laziness — `initialize` < 500ms; zero upstream connections | `initialize_returns_under_500ms_and_no_upstream_connections` (aggregator.rs) — asserts `< 500ms` on the initialize round-trip; zero-upstream is observable because Phase 0 ships no upstream config/registry/children and `tools/list` returns only the static meta-tools (no fan-out on the discovery path, D-003/GOTCHA #7) |
| 5 | Stub call gate — each meta-tool returns structured not-implemented `CallToolResult` with `isError:true`; no panic, no hang | `calling_each_meta_tool_returns_structured_not_implemented` + `calling_unknown_tool_returns_structured_result_not_rpc_error` (aggregator.rs) — both bounded by `RPC_DEADLINE` (5s) inside the harness `request()` |
| 6 | Probe build/run — fixture spawns standalone over stdio, no Node/npx | `probe_builds_and_runs_over_stdio_without_node` (probe.rs) — build enforced by the cargo gate; the test proves the binary answers `initialize` over stdio |
| 7 | Probe inventory — `tools/list` returns exactly `echo_ok`, `always_error`, `slow_tool`, `dangerous_noop`, `needs_sampling` | `probe_exposes_exactly_five_named_tools` (probe.rs) |
| 8 | Probe behavior — echo_ok echoes; always_error `isError:true`; slow_tool honors delay; dangerous_noop destructive annotations; needs_sampling sends `sampling/createMessage` on the wire | `probe_echo_ok_returns_supplied_input`, `probe_always_error_returns_structured_is_error_result`, `probe_slow_tool_honors_requested_delay`, `probe_dangerous_noop_advertises_destructive_annotations`, `probe_needs_sampling_sends_sampling_create_message_on_wire`, `probe_all_five_tools_reachable_over_stdio` (probe.rs) |
| 9 | Pinning gate — `Cargo.lock` exists; `Cargo.toml` pins `rmcp` with exact `=x.y.z` | `cargo_toml_pins_rmcp_exactly_and_lockfile_exists` (pinning.rs) |

### Phase Success Criteria coverage

| Phase | Criterion | Test |
|---|---|---|
| P0.1 | `cargo build` succeeds | gate-enforced (criterion 1) |
| P0.1 | `cargo clippy -- -D warnings` succeeds | gate-enforced (criterion 1) |
| P0.1 | `Cargo.lock` exists | `cargo_toml_pins_rmcp_exactly_and_lockfile_exists` |
| P0.1 | `Cargo.toml` pins `rmcp` exactly | `cargo_toml_pins_rmcp_exactly_and_lockfile_exists` |
| P0.1 | All canonical module files exist under flat `src/` | gate-enforced — cargo build fails if a declared module is missing; `src/main.rs` is the build root. Not asserted as a separate test to avoid duplicating the build gate. |
| P0.2 | Spawned over stdio, `initialize` < 500ms | `initialize_returns_under_500ms_and_no_upstream_connections` |
| P0.2 | `tools/list` returns exactly 3 meta-tools + Required Pattern descriptions | `static_discovery_returns_three_meta_tools_with_exact_descriptions` |
| P0.2 | `invoke_tool` annotations | `invoke_tool_carries_conservative_annotations` |
| P0.2 | Calling any meta-tool returns structured not-implemented with `isError:true`, no panic/hang | `calling_each_meta_tool_returns_structured_not_implemented`, `calling_unknown_tool_returns_structured_result_not_rpc_error` |
| P0.2 | `initialize` opens zero upstream connections | `initialize_returns_under_500ms_and_no_upstream_connections` |
| P0.2 | No stdout diagnostics corrupt stdio JSON-RPC traffic | side-effect: every wire test implicitly asserts clean JSON on stdout; a stray `println!` produces unparseable lines the harness rejects (see Side-effect assertions) |
| P0.3 | Probe builds as part of the Rust project | gate-enforced (criterion 1) + `probe_builds_and_runs_over_stdio_without_node` |
| P0.3 | Probe spawned over stdio independently of the aggregator | `probe_builds_and_runs_over_stdio_without_node` |
| P0.3 | `tools/list` exposes exactly the five tools | `probe_exposes_exactly_five_named_tools` |
| P0.3 | Each of the five probe tools reachable over stdio | `probe_all_five_tools_reachable_over_stdio` + the per-tool behavior tests |
| P0.3 | `always_error` returns structured `isError:true` | `probe_always_error_returns_structured_is_error_result` |
| P0.3 | `dangerous_noop` exposes destructive annotations | `probe_dangerous_noop_advertises_destructive_annotations` |
| P0.3 | `needs_sampling` attempts a sampling request; no Phase 0 reverse-handler added | `probe_needs_sampling_sends_sampling_create_message_on_wire` |

## Side-effect assertions

Every spawned-process test asserts the observable effect, not just a return
value, so a stub that returns the right shape without doing the work fails.

- **Clean process spawn/kill.** `ChildGuard` kills the child on drop (sync
  `start_kill` in `Drop`, async `shutdown()` for the graceful path). No test
  leaves an orphan behind on failure — the side effect asserted is "the
  process actually exited," enforced by `shutdown()` waiting up to 1s then
  force-killing. This is the Phase 0 analog of the D-009 orphan guarantee,
  scoped to the test harness (the full Job Object/process-group work is later
  phases).
- **No orphaned children.** `Drop` for `ChildGuard` calls `start_kill()`
  unconditionally; a test that panics mid-run still tears down its child.
- **Timeouts bound every spawn.** Every `request()` / `read_line()` /
  `read_next_message()` is wrapped in `tokio::time::timeout(RPC_DEADLINE)`
  (5s). `initialize` is wrapped in `INIT_DEADLINE` (10s) with a tighter 500ms
  assertion inside the laziness test. The needs_sampling test bounds its
  wire-scan with a 5s deadline — critical because nothing in Phase 0 answers
  the outbound sampling request (GOTCHA #2: an unanswered upstream request
  hangs forever without the bound).
- **stdout corruption is a test failure.** The harness parses each stdout
  line as JSON via `serde_json::from_str`; a stray `println!` (GOTCHA #1)
  produces a non-JSON line and the test panics with the offending line. This
  makes "no stdout diagnostics corrupt stdio" (P0.2) an observable side effect,
  not just a convention.
- **Slow_tool delay is a wall-clock effect.** `probe_slow_tool_honors_requested_delay`
  asserts `elapsed >= 100ms` on `Instant::now()` — a stub that returns
  instantly fails the lower bound.
- **Echo payload appears in content.** `probe_echo_ok_returns_supplied_input`
  searches the text content for the supplied substring — a stub returning
  empty content fails.
- **needs_sampling outbound request is observed.** The test reads raw
  messages off the wire and matches `method == "sampling/createMessage"` with
  an `id` — a probe whose `call_tool` returns without emitting that request
  fails (the deadline expires with no match).

## Deferred tests

| Test | File | `#[ignore]` reason | Unblock trigger |
|---|---|---|---|
| `live_cc_and_oc_each_discover_exactly_three_meta_tools` | `tests/integration/manual_e2e.rs` | `deferred: live CC and OC E2E — no MCP client available in CI; the wire-level static_discovery test covers the same contract programmatically` | Re-enable when a CI job with a real CC/OC MCP client is wired (manual verification gate, not a unit test). Run manually via `cargo nextest run -- --ignored` against a live CC/OC session. |

The deferred E2E test asserts the real client contract — that Claude Code
and OpenCode each discover exactly 3 meta-tools when spawned — which cannot
run in CI without an MCP client harness. The wire-level
`static_discovery_returns_three_meta_tools_with_exact_descriptions` test
covers the same observable contract from the server side; the manual E2E is
the client-side confirmation, not a substitute.

## dev-dependencies the implementer must add to Cargo.toml

The implementer owns `Cargo.toml`; this is the requirement, not the file.

```toml
[dev-dependencies]
tokio = { version = "1", features = ["process", "io-util", "macros", "rt", "rt-multi-thread", "time"] }
serde_json = "1"
```

Rationale:

- `tokio` with `process` (spawn child + `ChildStdin`/`ChildStdout`/`ChildStderr`),
  `io-util` (`AsyncBufReadExt`/`AsyncReadExt`/`AsyncWriteExt`), `macros`
  (`#[tokio::test]`), `rt` + `rt-multi-thread` (test runtime), `time`
  (`tokio::time::timeout`). The main crate already depends on `tokio` with
  `full`; the dev-dep is explicit so the test binary is self-contained.
- `serde_json` for JSON-RPC wire parsing. The main crate already depends on
  `serde_json`; repeated as a dev-dep for the test binary.
- No `mockall`, `proptest`, `insta`, or `rstest` — Phase 0 has no trait mocks,
  properties, snapshots, or parameterized matrices. Adding them is scope creep.

## `[[bin]]` targets the implementer must declare

```toml
[[bin]]
name = "fanin-mcp"
path = "src/main.rs"

[[bin]]
name = "probe-server"
path = "tests/probe-server/main.rs"   # or wherever the probe fixture lives
```

The probe must be a `[[bin]]` target so cargo injects
`CARGO_BIN_EXE_PROBE_SERVER` for the integration tests' `spawn_bin("probe-server")`.
D-016 requires the probe live under `tests/probe-server/`; the exact source
layout inside that directory is the implementer's choice, but the `[[bin]]
name` must be `probe-server` (dashes) so the harness env var resolves to
`CARGO_BIN_EXE_PROBE_SERVER`. The main `fanin-mcp` bin may use the default
`src/main.rs` path; the explicit `[[bin]]` is only required if the
implementer wants to be unambiguous about the name.

The probe fixture source itself is **not** a test file and is not under the
read-only rule — it is production code the implementer writes (P0.3).
`test-creator` owns only the test files listed above.

## Notes for the implementer

- The suite is **red until the implementer lands P0.1–P0.3**. Every test
  fails cleanly (file-not-found / spawn-failure / assertion mismatch), not
  with a compile error — the harness compiles standalone against
  `tokio` + `serde_json` only.
- `CARGO_BIN_EXE_*` is only injected when cargo builds the bin targets before
  the test binary. `cargo nextest run` does this by default; a bare
  `cargo test` does too. The probe must be a declared `[[bin]]`, not a
  `[[test]]`, for the env var to resolve.
- The static descriptions in `tests/common/expectations.rs` are the exact
  contract. If the plan's Required Pattern text and the implementation
  diverge, that is a spec conflict to surface — do not silently edit the
  expectations file (it is a read-only test file once written).
- `needs_sampling` has no responder in Phase 0 (master.md §Out: no
  reverse-traffic handling). The probe test observes the outbound request
  and drops the child; the probe's own `call_tool` future for that tool may
  time out or error, which is expected and does not affect the test outcome.