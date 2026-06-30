# Simplifier task — feature `schema-relay-fidelity`

Behavior-preserving simplification pass over THIS cycle's change set only, under
test guard. The change is small and was already adversarially reviewed as
cleanly factored — if nothing is genuinely worth changing, return **0 edits**.
Do NOT churn for its own sake.

## Scope (only this cycle's production code)
- `src/server.rs` — the new `neutralize_upstream_display` /
  `sanitize_list_row_description` split and their call sites
  (`handle_list_tools`, `sanitize_metadata_value`).

## Hard constraints
- **Tests are a read-only contract** — do NOT edit anything under `tests/`. They
  must stay green unchanged.
- Do NOT touch docs, `state.json`, or anything outside `src/server.rs`.
- Behavior-preserving ONLY: no change to the sanitization behavior, the cap
  semantics, D-004 byte-faithfulness, or the forbidden-char set.
- Do not undo the decouple — the two-helper split is the intended design.

## What to look for (only if it genuinely improves clarity/cost)
- Redundant allocations or collect/trim chains in the two helpers.
- Any dead/duplicated helper left by the refactor.
- Comment accuracy vs the final code.
If none of these yields a real improvement, that is the expected outcome — say so.

## Gate before returning (if you made ANY edit)
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --all` — all green (5 unit + 131 integration / 0 failed /
  5 ignored). Report counts.

## Return
Write `simplify.md` to `docs/master-plans/schema-relay-fidelity/` with: what you
changed (or "0 edits — change set already minimal"), why, and any issues spotted
but not fixed (surface, do not fix). You do NOT advance `state.json`.
