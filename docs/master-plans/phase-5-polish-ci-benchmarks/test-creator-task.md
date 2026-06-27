# Test-creator task — Phase 5 test contract (P1–P5)

**Tier: THOROUGH.** You are the SOLE authority over test files. The tests you
write are the binding, read-only contract every later agent (implementer,
simplifier, debugger) must satisfy at 100% green — no thresholds. Author depth,
not stubs.

## Read first (binding)

1. `docs/master-plans/phase-5-polish-ci-benchmarks/master.md` — the 7 phases,
   their Key Behaviors, and per-phase Success Criteria. **Your contract targets
   the Phase Success Criteria of phases 1–5** (see scope note below).
2. `docs/master-plans/phase-5-polish-ci-benchmarks/carry-over.md` — deferred
   issues + the gotchas that bind this phase.
3. `docs/master-plans/phase-5-polish-ci-benchmarks/oq3-http-transport-findings.md`
   — the verified rmcp Streamable-HTTP **client** API + feature names for the
   Phase 3 HTTP mock. Use the real API shape; the in-repo mock is loopback plain
   HTTP, no TLS.
4. `docs/GOTCHA.md` (#1, #11, #14, #19, #26, #30, #31), `docs/DECISIONS.md`
   (D-004, D-005, D-009, D-010, D-019), and the existing test suite under
   `tests/` to match conventions exactly (`tests/integration/main.rs` mod list,
   `tests/common/`, the probe-server fixture pattern).
5. Skills: `rust-test`, `rmcp-general`, `rust-general`.

## Scope of THIS test stage

Author contracts for the testable phases **P1–P5**. Do NOT author "tests" for:
- **P6 (CI/audit/deny/budgets)** — its verification IS the CI workflow + gate
  commands; the implementer writes `.github/workflows/ci.yml` + `deny.toml`.
  Not a unit-test target. (Exception: if a binary-size/memory budget can be a
  cheap repo-local assertion, you may note it in tests.md, but CI is the gate.)
- **P7 (docs)** — no tests; knowledge-sync/reviewer handle doc accuracy.

## What to produce (test files + fixtures only)

Per master.md Produces, author at least:
- **P1** `tests/integration/observability.rs` — `--log-file` writes NDJSON & nothing
  to stdout; `--log-level debug` includes debug events; invalid level exits
  non-zero BEFORE serve; **redaction sentinel absent from BOTH stderr AND the JSON
  file sink** (GOTCHA #19 — the key new assertion); a successful `invoke_tool` log
  entry carries `server`/`tool`/numeric latency/outcome; a failing call logs a
  failure outcome WITHOUT leaking args/secrets.
- **P2** `tests/integration/process_lifetime.rs` + a probe addition in
  `tests/probe-server/main.rs` that forks a descendant **immediately at child
  startup** (to hit the Windows spawn-then-assign race window, CARRY-1).
  Hard-kill ⇒ zero survivors. **Check PID liveness DURING the test window — a
  post-`cargo test` survivor count is masked by the runner's own job**
  (process-containment-verification-masking). macOS: assert GRACEFUL
  (stdin-EOF / group) teardown only; **do NOT assert zero-orphan-on-SIGKILL for
  macOS** — that is a documented limitation, not a contract. Gate Windows + Linux
  crash-safe paths via `#[cfg(...)]`/CI; if a path can't run on this Windows host,
  mark it clearly for the CI runner rather than faking a pass.
- **P3** an in-repo Streamable-HTTP mock fixture (e.g. `tests/http-probe-server/`
  or equivalent) + `tests/integration/http_upstream.rs` — config with
  `Authorization`/static header connects to the loopback mock and invokes a tool;
  the mock ASSERTS it received the resolved header value; that value is ABSENT
  from stderr + JSON log; missing header credential ⇒ the existing structured
  credential error, no spawn/connect. Stdio upstream behavior unchanged.
- **P4** `tests/integration/namespace_acl.rs` (switching E2E — one namespace hides
  what another exposes; denied ⇒ `namespace_denied`), `tests/integration/cred_store.rs`
  (keyring round-trip where available + env fallback in keyring-less CI; names-only
  listing; per-upstream least-privilege env — **without editing src/credentials.rs**,
  CARRY-3), `tests/integration/tool_search.rs` (downstream `tools/list` returns
  EXACTLY the 3 meta-tools, no upstream schemas at startup), and the CARRY-4
  deterministic test in `tests/integration/error_hardening.rs`:
  `ServiceError::TransportSend → upstream_disconnected`, **un-`#[ignore]`d**.
- **P5** `tests/integration/token_figures.rs` — a test that FAILS if the README
  token-figure markers drift from the benchmark-generated output (GOTCHA #26).

Update `tests/integration/main.rs` to register the new modules (add your `mod`
lines; do not remove existing ones). Use `tests/common/fixtures.rs` /
`expectations.rs` for shared helpers.

## CARRY-4 note (test needs an impl affordance)

The deterministic `TransportSend` test requires `map_service_error` to be
callable from tests (it is currently private in `src/registry.rs`). You CANNOT
edit `src/`. Write the test against the intended surface (e.g.
`pub(crate) fn map_service_error` or a thin testable wrapper) AND record in your
returned result + `tests.md` that the implementer must expose it. This is a
test-needs-impl dependency, not a test you can satisfy alone.

## Hard constraints

- Touch ONLY: `tests/**`, the probe fixtures, and `tests.md`. NEVER `src/**`
  (that is the implementer's; you only consume its planned API).
- The test code must be **`cargo fmt --check` clean and `clippy` clean** at the
  checkpoint (fmt-check-at-test-checkpoint) — so it must COMPILE. Write against
  the planned signatures in master.md; where a planned `src` API doesn't exist
  yet, the suite must still compile (the test stage gate is fmt+clippy; green
  pass-rate comes after implement). If a contract genuinely cannot compile
  without a src signature the implementer hasn't written, surface that as a
  test-needs-impl note rather than stubbing src yourself.
- No hardcoded fixture-shaped literals that let an implementer pattern-match the
  test (anti-gaming) — assert behavior, not magic strings.
- stdout is the MCP transport: any test that inspects logs must look at the file
  sink / stderr, never assume stdout carries diagnostics.

## Deliverable

Write `tests.md` (the contract doc, per `plan-format`) into the workspace
covering all P1–P5 contracts. Return a concise result: the test files authored,
the test-needs-impl dependencies (esp. CARRY-4 `map_service_error` exposure and
any planned src signatures the suite compiles against), and any master.md
success criterion you found untestable as written (with a proposed fix).
