# Simplifier task — Phase 5 (THOROUGH simplify pass)

Refactor the Phase 5 code for clarity, reuse, and simplicity UNDER THE GREEN TEST
GUARD. The full integration suite is the binding contract — it must stay 115/0/4
and you must NOT edit any test file. Behavior-preserving only.

## Read first

- `docs/master-plans/phase-5-polish-ci-benchmarks/review.md` — the synthesized
  review. **Address its 2 targeted findings (F1, F2) as part of this pass** (see
  below). No blocker/structural findings exist.
- `master.md` (scope), `carry-over.md` (the invariants), `docs/GOTCHA.md`
  (#1 stdout, #16 lock, #19 redaction), `docs/DECISIONS.md` (D-004/005/007/009/010).
- Skills: `rust-general`, `rmcp-general`.

## Scope (the Phase 5 src surface)

`src/main.rs`, `src/process.rs`, `src/registry.rs`, `src/config.rs`,
`src/error.rs`, `src/server.rs`, `benches/token_cost.rs`. Look for: duplicated
logic across the new logging/HTTP/process paths, over-long functions, awkward
error plumbing, needless clones/allocations, anything that reads harder than it
should. Prefer the smallest change that improves clarity; do not gold-plate and
do not expand scope.

## Required: the 2 review findings

- **F1** (`src/process.rs:138-148, 258-262`): the doc comments overstate the
  "retained self-Job-Object" role. Tighten them: the per-upstream process-wrap
  `JobObject` wrapper (suspended-spawn → assign → resume) closes the CARRY-1
  race; the self-Job is an *additional outer* containment for fanin itself.
- **F2** (`src/process.rs:139`): add a one-line rationale above the
  `#[allow(dead_code)]` on the self-Job guard — e.g.
  `// retained solely for Drop (KILL_ON_JOB_CLOSE on self)`; keep the allow.

## Hard constraints

- **Tests are read-only.** Never edit `tests/**`. If a simplification would
  require a test change, STOP and surface it — do not touch the test.
- **Behavior-preserving.** No functional change; the suite proves it. Do NOT
  edit `src/credentials.rs` (CARRY-3). Keep rmcp `=1.8.0`, the lock discipline
  (no map lock across await), stdout-clean serve, redaction on every log sink,
  and byte-faithful results.
- End state: `cargo fmt --all` clean, `cargo clippy --all-targets -- -D warnings`
  zero warnings, `cargo test --test integration` 115/0/4 green.
- If you find nothing worth changing beyond F1/F2, that is a valid outcome — say
  so; do not invent churn.

## Return

Write `simplify.md` into the workspace: what you changed and why (incl. F1/F2),
what you deliberately LEFT alone, confirmation the suite stays green, and any
issue you spotted but did not fix (surface, don't fix — structural items route
to the orchestrator).
