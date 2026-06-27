REVIEW the remediation-s1-d1 change set. Tier thorough, stack rust.

You are the reviewer. The change is already implemented and the full suite is
green (134 passed / 0 failed / 4 ignored, fmt + clippy -D warnings clean). Your
job is to find what the green gate does NOT prove. Review ONLY this change set —
do not re-review the whole codebase.

## What changed (read these)

- The plan: `docs/master-plans/remediation-s1-d1/master.md`.
- The contract: `docs/master-plans/remediation-s1-d1/tests.md` and
  `tests/integration/remediation_s1_d1.rs` (read-only; do not propose editing
  tests — flag a wrong test as a finding routed to test-creator).
- The implementation diff — run `git diff HEAD~1 -- src/` (the implement commit is
  HEAD). Four files: `src/registry.rs` (timeout envelope around connect +
  ensure_fresh; stdio cwd resolution), `src/process.rs` (`spawn_stdio_transport`
  gains `resolved_cwd`, applies `current_dir`), `src/config.rs`
  (`ServerConfig::cwd` + empty-cwd validation), `src/error.rs`
  (`StartupError::EmptyCwd`).
- Binding canon: `docs/DECISIONS.md` (D-004 byte-faithful, D-005 public error
  shape, D-007 lock discipline, D-009 containment, D-012 timeout), `docs/GOTCHA.md`
  (#16 lock-across-await, #30 cwd/Morph, #11/#14 containment),
  `docs/ARCHITECTURE.md:97` (cwd spec). The `rmcp-general` skill for rmcp =1.8.0.

## Output

Write `docs/master-plans/remediation-s1-d1/review-<LENS>.md` (LENS given below).
Each finding: severity (blocker | structural | targeted | trivial), exact
`file:line`, the issue, a suggested fix, and routing. If the lens is clean in an
area, say so with evidence — do not pad. End with a one-line lens verdict
(PASS | PASS-with-issues | FAIL). Your returned result is data for the
orchestrator, not chat.
