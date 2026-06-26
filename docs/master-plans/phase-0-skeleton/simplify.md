# simplify: phase-0-skeleton

## Files simplified

- `src/main.rs` — Trimmed duplicated Phase 0 CLI narrative while keeping the
  stdout transport warning and credential secret boundary comments.
- `src/server.rs` — Trimmed repeated phase-history prose around the aggregator
  skeleton while keeping static-description, no-upstream-fanout, and D-005
  tool-level-error comments.
- `src/registry.rs` — Reworded the stub header to keep only the future D-007 /
  GOTCHA #16 lock-discipline invariant.
- `src/forward.rs` — Reworded the stub header to keep only the D-004 /
  GOTCHA #4 byte-faithful pass-through invariant.
- `src/process.rs` — Reworded the stub header to keep only the D-009 process-tree
  lifetime invariant.
- `src/namespace.rs` — Reworded the stub header to keep only the D-006 namespace
  security-boundary invariant.
- `src/error.rs` — Trimmed Phase 0 variant narrative while keeping the D-005
  tool-level-error rationale.
- `src/config.rs` — Trimmed repeated config-deferral prose while keeping the
  stdout diagnostic boundary.
- `tests/probe-server/main.rs` — Trimmed fixture phase-history and redundant
  helper comments while keeping stdout transport and `needs_sampling` reverse-
  traffic warnings.

## Files reverted

(none)

Recovery point: `46e9872d9aee3f9717a4b16db67f6a2b75bc4f71`.

## Files unchanged

- `src/credentials.rs` — Left unchanged. Its short comment block is mostly the
  D-010 / GOTCHA #18/#19/#22 secret-handling boundary, so trimming it would save
  little and risk weakening the warning.
- Test contract files under `tests/integration/` and `tests/common/` — Unchanged;
  tests are read-only for simplify.
- `Cargo.toml`, `Cargo.lock`, and `docs/master-plans/phase-0-skeleton/tests.md`
  — Unchanged by this pass; dependency and test-contract findings are outside
  this simplification scope.

## Issues spotted for later routing

(none)

Carry note: JSON-schema duplication between the aggregator and probe remains
intentionally untouched. `review.md` says to carry it until the pattern grows.

## Verification

- Baseline before edits: `cargo fmt --all -- --check` passed.
- Baseline before edits: `cargo clippy --all-targets -- -D warnings` passed.
- Baseline before edits: `cargo nextest run` passed: 14 passed, 2 skipped.
- After trim: `cargo fmt --all -- --check` passed.
- After trim: `cargo clippy --all-targets -- -D warnings` passed.
- After trim: `cargo nextest run` passed: 14 passed, 2 skipped.
