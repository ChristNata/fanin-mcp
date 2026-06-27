# Simplify pass — Phase 5

**Recovery anchor:** `b25d2804295810d7dc65b7813e3ce93b973e943e`

## Files simplified (2)

- `src/process.rs` — tightened doc comments on `ProcessTreeGuard` and `contain_current_process_tree` (F1) and added explicit one-line rationale comment above the `#[allow(dead_code)]` on the self-Job guard (F2). Both changes are non-behavioral comment hygiene.

## Files reverted

(none)

## Files unchanged (6 + benches)

- `src/main.rs`, `src/registry.rs`, `src/config.rs`, `src/error.rs`, `src/server.rs`, `benches/token_cost.rs` — no candidates met the bounded simplification criteria (no unnecessary indirection, dead paths, over-generic types, or clone/alloc churn that preserved behavior across the full domain). The implementation was already minimal on these surfaces.
- `src/credentials.rs` — explicitly out of scope per task.

## Issues spotted

(none)

## Verification

- `cargo fmt --all` — clean
- `cargo clippy --all-targets -- -D warnings` — zero warnings
- `cargo test --test integration` — 115 passed, 0 failed, 4 ignored (exactly the green baseline handed to the pass)

All hard constraints observed: tests read-only, rmcp `=1.8.0`, no-lock-across-await, stdout-clean serve, redaction intact, behavior-preserving only. No invented churn beyond the two required review findings.
