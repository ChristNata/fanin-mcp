# Review context — Phase 5 (THOROUGH, 3-lens)

Shared context for the alignment / adversarial / general review lenses. Each lens
writes its own `review-<lens>.md` into this workspace and returns a concise
summary. The orchestrator synthesizes into `review.md` and routes findings.

## What shipped (the diff to review)

Commits on `main`: `v0.6.1` (plan) → `v0.6.2` (tests) → `v0.6.3` (implement).
Review the `v0.6.3` implementation against the plan + the test contract. Source
of truth for intent: `master.md`, `tests.md`, `carry-over.md`,
`oq3-http-transport-findings.md` in this workspace; `docs/DECISIONS.md`,
`docs/GOTCHA.md`, `docs/MVP.md` §Phase 5.

Plan-phase → files:
- **P1 observability** — `src/main.rs` (CLI `--log-file`/`--log-level`, tracing
  init, pre-serve level validation), `src/registry.rs`/`src/server.rs` (per-call
  + lifecycle logging). Contract: `tests/integration/observability.rs`.
- **P2 containment (CARRY-1/CARRY-2)** — `src/process.rs` (Windows suspended-spawn
  via process-wrap + retained self-Job-Object; Linux `prctl(PR_SET_PDEATHSIG)`
  pre_exec; macOS graceful-only), `src/main.rs` (`--spawn-immediate-descendant`
  hook), `Cargo.toml` (target-gated `libc`). Contract:
  `tests/integration/process_lifetime.rs`. See `impl-p2-result.md`.
- **P3 streamable-http** — `src/config.rs` (transport/endpoint/headers),
  `src/registry.rs` (HTTP connect + header resolve/redaction), `src/error.rs`,
  `Cargo.toml` (rmcp `transport-streamable-http-client[-reqwest]`, `http`).
  Contract: `tests/integration/http_upstream.rs`. See `impl-p3-result.md`.
- **P4** — NO-OP (contracts already pass; nothing implemented). Verify that claim.
- **P5 token bench** — `benches/token_cost.rs`, `Cargo.toml` `[[bench]]`,
  `README.md` markers. Contract: `tests/integration/token_figures.rs`. See
  `impl-p5-result.md`.
- **P6 CI/deny/budgets** — `.github/workflows/ci.yml`, `deny.toml`,
  `docs/release-checklist.md`. See `impl-p6-result.md`.

Gate state at review: full suite **115 passed / 0 failed / 4 ignored**; `cargo
fmt --check` + `clippy --all-targets -D warnings` + `cargo deny check` clean;
stripped binary 8.27MB (<10MB).

## High-priority scrutiny (all lenses, weighted to your angle)

1. **P2 is the security-critical carried blocker. Hammer it (adversarial esp.):**
   - Does the Windows suspended-spawn ACTUALLY close the CARRY-1 race (child
     created suspended, assigned to the kill-on-close Job Object, resumed only
     after)? Or is the "retained self-Job-Object" doing the real work while the
     per-upstream assignment is still post-spawn? Confirm against `process.rs`.
   - Job Object nesting: does fanin-mcp putting ITSELF in a job break per-upstream
     jobs or the CC-spawns-fanin scenario? Any handle-leak that defeats
     KILL_ON_JOB_CLOSE?
   - Linux `PR_SET_PDEATHSIG` in `pre_exec`: correct signal/order, `unsafe`
     soundness, and is it set on the right child? (CI-verified only — read for
     correctness.)
   - Test-gaming check: is the green from REAL containment or a test-shaped
     shortcut? The oracle is PID liveness, but verify `src/` has no
     test-name/marker-path special-casing.
2. **P1 redaction on the new file sink (GOTCHA #19):** does the JSON file layer
   truly pass through the redaction writer? Any path where a resolved secret or a
   `{:?}` env map reaches the file/stderr unredacted? Are call arguments excluded
   from logs?
3. **P3 secret + protocol fidelity:** header `${VAR}` values registered for
   redaction BEFORE any log; no registry lock held across the HTTP await
   (GOTCHA #16/D-007); `credential_resolution_failed` returned as
   `CallToolResult{isError:true}` (D-005), not a JSON-RPC error, WITHOUT
   contacting the endpoint; results still byte-faithful (D-004).
4. **P5 token measure honesty (GOTCHA #26):** the `(bytes+3)/4` estimate — is it
   a real, reproducible measurement of the actual payloads and clearly labeled an
   estimate, or a hand-waved constant? Deterministic across runs/OSes?
5. **P6 supply-chain:** are the `deny.toml` duplicate-version SKIPS legitimate
   (truly unavoidable transitive dups) or do they paper over bloat? Is the
   license allow-list correct + tight? Is the CI workflow actually correct
   (matrix, gates, the <10MB check, tool install)? Any anti-stack regression?
6. **Cross-cutting:** rmcp pin still EXACTLY `=1.8.0`; no `println!`/stdout in
   serve (GOTCHA #1); scope discipline (no out-of-scope rewrites); macOS
   documented-limitation honesty (no zero-orphan overclaim); the 4 `#[ignore]`d
   tests are legitimately deferred, not hiding a shipped criterion.

## Output

Severity-tier every finding per `plan-format` (trivial / targeted / structural /
blocker). For each: file:line, what, why it matters, and a concrete fix. State
your lens verdict (PASS / PASS-with-issues / FAIL). Do NOT edit code — review only.
