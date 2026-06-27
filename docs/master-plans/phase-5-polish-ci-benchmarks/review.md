# Review (synthesis): phase-5-polish-ci-benchmarks

**Verdict: PASS-with-issues.** Full integration suite green (115/0/4), `cargo
fmt --check` + `clippy --all-targets -D warnings` + `cargo deny check` clean,
stripped release binary 8.27 MB (< 10 MB). All 21 master Success Criteria met.
Three independent lenses ran; **0 blocker, 0 structural, 2 targeted** (both
minor, both `src/process.rs`, both non-behavioral). The pipeline is NOT
hard-blocked — nothing downstream depends on the two nits.

## Lenses

| Lens | Model | Verdict | Findings |
|---|---|---|---|
| alignment | xai/grok-4.3 | PASS | 0 — verified all 21 SC + per-phase criteria; locked decisions hold (macOS documented-limitation, in-repo mock, rmcp `=1.8.0`, linear-on-main); P4 no-op justified; P7→knowledge-sync deferral clean; no scope creep. |
| adversarial | xai/grok-4.3 | PASS | 2 targeted (F1, F2 below). Confirmed: Windows suspended-spawn truly closes the CARRY-1 race; self-Job-Object is orthogonal outer containment (no nesting break); Linux PDEATHSIG async-signal-safe + correct; NO test-gaming (oracle is PID liveness, no marker/test-name special-casing in `src/`); P1 redaction covers every file-sink path; P3 resolves+redacts header secrets before connect, no lock across the HTTP await, `credential_resolution_failed` as `isError` result not JSON-RPC error. |
| general | xai/grok-4.3 | PASS | 0 — P5 token measure is a real, reproducible, deterministic payload measurement labeled an estimate; P6 deny.toml license set + duplicate-skips legitimate, CI workflow correct, anti-stack intact; rmcp pin intact, no stdout in serve, 4 ignored tests legitimately deferred. (Returned verdict; the lens did 6 reads + grep + bash over the real sources but did not materialize `review-general.md` — a process hiccup, not a review gap; corroborated by alignment's detailed pass over the same areas.)

## Merged findings (both `targeted`, both minor)

### F1 — doc comment overstates the "retained self-Job-Object" role
- Lens: adversarial. Location: `src/process.rs:138-148, 258-262`.
- The comments imply the retained self-Job does the CARRY-1 race fix; in fact the
  per-upstream process-wrap `JobObject` wrapper (suspended-spawn → assign →
  resume) closes the race, and the self-Job is an additional *outer* containment.
- Why: misleads a future maintainer auditing the race fix.
- Fix: tighten the two comments — "outer containment for fanin itself; upstreams
  use the process-wrap JobObject wrapper (suspended-spawn + KILL_ON_JOB_CLOSE)".

### F2 — `#[allow(dead_code)]` on the self-Job guard lacks a rationale comment
- Lens: adversarial. Location: `src/process.rs:139`.
- The guard is retained solely for its `Drop` side-effect (KILL_ON_JOB_CLOSE on
  self); the field is never read, so the `allow` is intentional but unexplained.
- Why: minor hygiene; the intent (Drop-only retention) should be explicit.
- Fix: add `// retained solely for Drop (KILL_ON_JOB_CLOSE on self)` above the
  attribute; keep the allow.

## Routing

Both findings are `targeted`, non-behavioral (comment clarity + hygiene), and
local to `src/process.rs` — the simplifier's maintainability remit. Per the
orchestrator's routing judgment they are folded into the **simplify stage**
(which touches the same file under the green test guard) rather than a separate
debugger dispatch. No `structural`/`blocker` finding exists, so nothing is
escalated to the user and nothing is written out as an `issue-*.md`.

## Verified invariants (carried from the lenses)

- rmcp exactly `=1.8.0`; `Cargo.lock` committed.
- No `println!`/stdout in serve mode; JSON logs → redacting file sink only.
- No registry map lock held across an upstream/HTTP `await` (GOTCHA #16 / D-007).
- Byte-faithful results; tool failures as `CallToolResult{isError:true}` (D-005).
- Each upstream gets only its own resolved env/header values (D-010).
- macOS hard-kill is honestly documented as a limitation (no zero-orphan-on-SIGKILL overclaim).
- The 4 `#[ignore]`d tests are documented deferrals; none is the sole proof for a shipped P1–P5 criterion.
