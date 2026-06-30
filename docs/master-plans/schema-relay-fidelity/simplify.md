# Simplify — feature `schema-relay-fidelity`

## Status

**0 edits — change set already minimal.** The decouple is cleanly factored and
the residual code carries no over-engineering worth touching. This is the
expected clean outcome the task foresees; no churn applied.

## Scope

This cycle's production change only — `src/server.rs`:

- `neutralize_upstream_display` (new; control-neutralize, trim, no cap).
- `sanitize_list_row_description` (new; neutralize + CAP 100 row summary).
- Call-site reroutes: `handle_list_tools` row description, and
  `sanitize_metadata_value` for `get_tool_schema` annotation strings.

## Candidate assessment

Checked against the task's specific checklist; none yielded a real
improvement.

- **Redundant allocations / collect→trim chains.** `neutralize_upstream_display`
  is `collect::<String>()` → `.trim()` → `.to_string()`. That is one allocation
  (the collected `String`) plus the trim-borrow and the owned copy back — the
  idiomatic minimal form for "collect chars, trim, return owned." There is no
  spare allocation to cut without restructuring into a hand-rolled trim-aware
  collector, which would be more code, not less. No churn.
- **`sanitize_list_row_description` intermediate `String`.**
  `neutralize_upstream_display(s).chars().take(CAP).collect()` drops the
  neutralized `String` once `.chars()` is drained. The prior single-function
  form had the identical `stripped.trim().chars().take(CAP).collect()` shape.
  The one extra short-lived allocation is the *cost of the decouple itself*,
  and the task explicitly protects the split ("Do not undo the decouple").
  Not actionable.
- **Dead / duplicated helper.** `sanitize_upstream_text` was fully removed;
  no leftover. No duplicate logic between the two new helpers — one owns
  neutralize-only, the other layers the row cap on top.
- **Comment accuracy.** Doc on `neutralize_upstream_display` correctly scopes
  control-neutralization as display-wide, names the annotation keys, and
  excludes `invoke_tool` args/results (D-004) and schema validation values.
  Doc on `sanitize_list_row_description` correctly attributes the cap as a
  row-only control and points back to the neutralize path. Inline
  `// Cap AFTER strip; char iterator never splits multibyte.` is accurate.
  No drift.
- **`sanitize_metadata_value`.** One-line match routing strings to
  `neutralize_upstream_display` and recursing on anything else. Not
  over-engineered; the recurse is structural, not a trivial wrapper.

## Files changed

None.

## Files reverted

None.

## Files unchanged

- `src/server.rs` — assessed against every candidate in the task checklist;
  the implementation is already minimal. The two-helper split is the intended
  design and is protected by the task; the residual allocations are the
  idiomatic floor, not removable overhead.

## Issues spotted

None.

## Gate

No edits → no re-gate required. Baseline confirmed green before the pass:

- `cargo fmt --all --check` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test --all` — 5 unit + 131 integration / 0 failed / 5 ignored.
  Matches the task's stated gate exactly.

## Self-check

- Suite green — yes (baseline, unchanged).
- Every file touched is in scope — yes (none touched).
- Every change behavior-preserving across the full domain — N/A (no changes).
- Lint and type-check pass — yes (baseline, clean).
- `simplify.md` records every in-scope file's status — yes.