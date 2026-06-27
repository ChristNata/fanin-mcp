# simplify.md — remediation-s1-d1

**Snapshot commit:** `1f0cf0d`  
**Baseline gate:** 129 passed / 0 failed / 4 ignored; fmt + clippy clean.

## Files simplified
(none)

## Files reverted
(none)

## Files unchanged
- `src/registry.rs` — cwd-resolution branch (lines 131–148) and three distinct timeout arms already minimal; no behavior-preserving simplification found that preserves per-site invariants (no-insert-on-error, restore-dirty, log-tool+latency).
- `src/process.rs` — no candidates inside remediation scope.
- `src/config.rs` — declarative model; nothing to cut.
- `src/error.rs` — error shapes are public API (D-005); untouched.

## Issues spotted
(none)

## Final gate
134/0/4 green (same as baseline). No behavior change. No churn.
