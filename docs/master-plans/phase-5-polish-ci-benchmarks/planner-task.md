# Planner task — MVP Phase 5: Polish + Cross-Platform CI + Benchmarks

**Tier: THOROUGH.** **Feature slug: `phase-5-polish-ci-benchmarks`.** This is
the final MVP phase. Produce `master.md` (and the seed inputs for `state.json`,
which the orchestrator owns) per the `plan-format` skill.

## What to read first (binding context)

1. `docs/master-plans/phase-5-polish-ci-benchmarks/carry-over.md` — the
   orchestrator's pre-flight audit: the deferred issues (CARRY-1..4), doc drift
   (DRIFT-1), the gotchas that bind this phase, the real gap-vs-done table, and
   the fixed environment facts. **This is your primary input — fold every CARRY
   item into a plan phase.**
2. `docs/MVP.md` §"Phase 5" (the 7 numbered items) + the "Verification Checklist".
3. `ROADMAP.md` v1.0 MVP section + Non-Goals (do not plan anything in v1.1+/non-goals).
4. `docs/DECISIONS.md` — especially D-009 (process-tree lifetime), D-005 (error
   shape), D-010 (per-server least-privilege env), D-004 (byte-faithful results).
5. `docs/GOTCHA.md` — #1, #11, #14, #19, #26, #30, #31 bind this phase.
6. Current code state to plan against: `src/process.rs` (containment — CARRY-1/2),
   `src/main.rs` `init_tracing()` (~:274, hardcoded INFO→stderr — MVP item 1),
   `Cargo.toml` (bin/bench targets), `src/registry.rs::map_service_error` (CARRY-4).

## Resolved decisions (already settled by the user — plan to these, do NOT re-open)

- **macOS hard-kill (CARRY-2): DOCUMENTED LIMITATION.** Linux gets crash-safe
  containment via `pre_exec` `prctl(PR_SET_PDEATHSIG, SIGKILL)` (cfg linux);
  Windows gets the suspended-spawn fix (CARRY-1). macOS gets process-group
  teardown for the GRACEFUL path only; the SIGKILL-orphan gap is explicitly
  DOCUMENTED in SECURITY.md + GOTCHA.md, not engineered around. No supervisor
  process, no kqueue watcher (both rejected — honor the "no daemon" non-goal).
- **Remote HTTP test (MVP item 3): IN-REPO MOCK.** Add a tiny in-repo
  Streamable-HTTP probe (mirrors `tests/probe-server/`) exercising header-auth
  injection deterministically in CI on all OSes — no network, no real creds. The
  one-time manual check against a real public remote becomes a documented
  release-checklist step, NOT part of the automated suite.

## Deliverables to decompose into phases (MVP §Phase 5)

1. **Observability:** `tracing` JSON file output — `--log-file <path>` +
   `--log-level <level>` flags; structured per-call log (server, tool, latency,
   outcome), connect/disconnect, config load. **Redaction MUST apply to the file
   sink** (GOTCHA #19 — the sentinel-secret test must stay green against the file,
   not just stderr; do NOT route any log to stdout — GOTCHA #1).
2. **CI matrix:** `.github/workflows/` — Windows + macOS + Linux; build, fmt,
   clippy, full test suite, `cargo audit`, `cargo deny` (add `deny.toml`:
   advisories/bans/licenses/sources; dual MIT-OR-Apache + the deliberately small
   anti-stack tree are policy). Hard-kill orphan test runs on all three OSes.
3. **Process hardening (CARRY-1 + CARRY-2):** Windows suspended-spawn-then-assign
   (CREATE_SUSPENDED → AssignProcessToJobObject → ResumeThread; fallback to a thin
   custom child transport if rmcp `=1.8.0` `TokioChildProcess` can't expose the
   suspended spawn / main-thread handle — flag this as an Open Question with the
   fallback as the default). Linux PDEATHSIG. macOS documented-limitation path.
   Regression test that forks a descendant IMMEDIATELY at child startup to hit the
   Windows race window.
4. **Integration coverage:** in-repo mock Streamable-HTTP upstream w/ header auth;
   namespace switching E2E; credential E2E (keyring round-trip + env fallback —
   note: if any of this requires EDITING `src/credentials.rs`, flag it, because
   the managed OC edit-deny on `**/credentials*` blocks that — CARRY-3, a user
   config decision; prefer keeping cred work in `tests/`); CC Tool Search
   composition check (3 meta-tools, no conflict).
5. **Token benchmark:** `benches/token_cost.rs` + `[[bench]]` target so
   `cargo bench --bench token_cost` runs (the `/gate` command already expects this
   exact target). Measures `tools/list` + typical-session token cost; README
   numbers are GENERATED from it, never hand-edited (GOTCHA #26).
6. **Budgets:** binary <10MB stripped; idle <15MB RSS, <50MB @ 5 upstreams —
   plan how these are measured/asserted (CI step or documented manual check).
7. **Release docs:** SECURITY.md already finalized in P4 (verify the macOS gap is
   now documented there); LICENSE-MIT/APACHE present; `license` field set —
   confirm, don't redo. Fold DRIFT-1 (docs say `AggError`/`ErrorCode`; code is
   `ToolError`) into a doc-fixup item or hand to knowledge-sync.
8. **CARRY-4 (low):** optionally expose/unit-test `map_service_error` to de-`ignore`
   the F4 send-side classification test. Small; include only if cheap.

## Phasing guidance

- Decompose into dependency- and file-disjoint phases where possible so the
  implement stage can fan out in parallel (cap 3). E.g. observability (`main.rs`
  + a log module), process hardening (`process.rs`), benchmark (`benches/`), and
  CI/`deny.toml`/docs are largely file-disjoint. Process hardening (`process.rs`)
  is the riskiest and most security-sensitive — call that out.
- Each phase needs: Goal, Depends On, Produces (exact file paths), Success
  Criteria (objective + testable), and the tests-contract notes test-creator needs.
- **Surface Open Questions explicitly** (esp. the rmcp suspended-spawn capability
  for CARRY-1, and the binary-size/memory measurement method). Propose a default
  for each.

## Environment facts (do not re-derive)

- Cycle model: **linear-on-`main`** — no worktree, no PR, no merge; commit each
  stage to `main`, one end-of-cycle push to `origin/main` (CI triggers on it).
- Version stamp: **`v0.6.x`** (MVP Phase 5 ⇒ B=6; C resets to 1).
- This is a **Windows-only dev host** — Unix/macOS process paths are verified only
  on CI runners, never locally. Plan CARRY-2 verification accordingly.

Write `master.md` into this workspace
(`docs/master-plans/phase-5-polish-ci-benchmarks/`). Return a concise result:
the phase list with dependencies, the Open Questions with your proposed defaults,
and anything in the inputs you found underspecified or conflicting.
