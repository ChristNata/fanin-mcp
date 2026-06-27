---
Feature: phase-4-error-sanitization
Stage: simplify
---

# Simplify: phase-4-error-sanitization

## Summary
Two targeted simplifications under strict test guard. Both preserve exact observable behavior (structured error codes/fields, sanitization output, lock discipline, dirty-flag lazy-refetch). Gate stayed green at 99 passed / 0 failed / 3 ignored. No reverts. No test edits.

## Recovery anchor
3b0837b9f90453358f1653e087d7059d1388b3fe (HEAD before first edit)

## Files simplified

- `src/registry.rs`
  - Introduced one small private helper `map_service_error(e: ServiceError, server: &str, tool: &str) -> ToolError`.
  - Replaced duplicated `if matches!(e, ServiceError::TransportClosed) { UpstreamDisconnected } else { UpstreamCall }` in `call_tool` and `ensure_fresh`.
  - Why: DRY win; eliminates identical branch logic while keeping the empty-tool convention for `ensure_fresh` and the exact string/code/fields for both call sites. Lock discipline unchanged (no map lock across await). Behavior identical across full domain.
  - Change is local to Phase 4 code; comments updated only for clarity around the helper.

- `src/server.rs`
  - Simplified `sanitize_upstream_text`: removed the convoluted `if trimmed.len() != capped.len() { re-cap } else { ... }` after `trim`.
  - New body: strip controls → `trim()` → `chars().take(CAP).collect()`.
  - Why: straightforward strip → trim → cap sequence. Produces identical single-line, control-free, ≤100-char result. Cap remains after strip; char iterator guarantees no multibyte split. No behavior change for any input (verified by gate).

## Files reverted
(none)

## Files unchanged
- `src/error.rs` — Phase 4 surface (new `UpstreamDisconnected` variant + `message` impl) is already minimal; no indirection or duplication to cut.
- `src/forward.rs` — Handler wiring, dirty flag, `on_tool_list_changed` implementation are tight and correct per rmcp pin and lock discipline. No candidates inside Phase 4 delta.
- `src/registry.rs` (the rest) — `ensure_fresh`, `UpstreamEntry` shape, `connect`, `get_or_connect`, dirty-swap logic, and lock discipline are load-bearing and already minimal. No further simplification without touching behavior or scope.

## Gate results (final)
- `cargo fmt --all` — clean
- `cargo clippy --all-targets -- -D warnings` — zero warnings
- `cargo test --test integration` — 99 passed; 0 failed; 3 ignored (identical to pre-simplify baseline)

## Issues spotted
(none — no bugs, no scope creep, no behavior drift observed)

## Notes
- Only touched the four Phase 4 files listed in master.md Target.
- All invariants preserved: D-005 shape, TransportClosed distinction, dirty-flag gate (no unconditional refetch), no map/tools lock across awaits, byte-faithful results, no stdout, rmcp pin untouched.
- Candidate 3 ("any other") yielded no additional safe wins inside Phase 4 delta; left untouched per "if not a real win, leave it" rule.
