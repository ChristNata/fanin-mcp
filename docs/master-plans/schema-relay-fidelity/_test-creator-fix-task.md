# Test-creator task — close review blocker B1 (schema-path forbidden-control coverage)

The adversarial review (`docs/master-plans/schema-relay-fidelity/review.md`)
found ONE blocker: the `get_tool_schema` display-safety acceptance evidence is
incomplete. The schema-annotation tests only assert `assert_no_control_chars`
(C0 + DEL), and the `poison_schema`/`poison_validation` annotation fixtures
carry only C0-style controls. So master SC 2 — "`get_tool_schema` annotation
strings are single-line and free of the EXISTING forbidden display controls" —
is NOT independently proven on the schema path for the BROADER set (C1, Unicode
line/paragraph separators U+2028/U+2029, bidi controls, BOM, zero-width). The
`list_tools` path already proves it (the `f1_*` test using
`F1_FORBIDDEN_CODEPOINTS`); the schema path does not.

The CODE IS ALREADY CORRECT — both paths share `neutralize_upstream_display`,
which neutralizes the full set via `should_neutralize_upstream_char`. So this is
a TEST-EVIDENCE gap: add coverage that GREEN-passes today but FAILS if the
`get_tool_schema` annotation path ever regresses to a C0-only strip. You are the
sole authority over test files.

## Read first
- `docs/master-plans/schema-relay-fidelity/review.md` (the blocker, with its
  Fix guidance).
- `tests/integration/sanitization.rs` — note `F1_FORBIDDEN_CODEPOINTS`
  (~line 520) and the existing `get_tool_schema_sanitizes_poisoned_metadata_
  preserves_shape` and `f3_*` tests.
- `tests/probe-server/main.rs` — `poison_schema_tool()` and
  `poison_validation_tool()` annotation fixtures.
- `src/server.rs` `should_neutralize_upstream_char` (the authoritative
  forbidden set) — your assertion's forbidden set must match it (same set as
  `F1_FORBIDDEN_CODEPOINTS`).

## What to do
1. **Fixture:** embed the broad forbidden codepoints — C1 (e.g. U+0080, U+0085),
   Unicode separators (U+2028, U+2029), a bidi control (U+202E), a zero-width
   (U+200B), and BOM (U+FEFF) — into at least one `get_tool_schema` annotation
   key (`title`, `description`, `$comment`, or `markdownDescription`) of an
   existing schema fixture (`poison_schema` or `poison_validation`). Keep the
   fixture's validation strings (`enum`/`const`/`default`) and structure
   unchanged. Do NOT disturb the `long_clean` full-length fixture or the
   existing exact-equality bite test.
2. **Assertion:** in the matching `get_tool_schema` test, add a check that NONE
   of the broad forbidden codepoints survive in the returned annotation
   string(s). Reuse `F1_FORBIDDEN_CODEPOINTS` (or a shared helper that checks
   the same set as `should_neutralize_upstream_char`) so list-row and schema
   paths assert the identical forbidden set. Keep the existing single-line +
   `assert_no_control_chars` checks.
3. Do NOT add any assertion that would require sanitizing validation strings or
   invoke args/results — those stay verbatim (D-004).

## Bite check (state it in your return)
Confirm the new assertion genuinely bites: it must be the case that if
`get_tool_schema`'s annotation path used a C0-only strip, your new assertion
would FAIL (because C1/separator/bidi/zero-width/BOM would survive). You do not
change production code to prove this — reason it explicitly and confirm the
fixture actually carries those codepoints in a get_tool_schema annotation key.

## Hard constraints
- Touch ONLY `tests/integration/sanitization.rs` and (if needed)
  `tests/probe-server/main.rs`. Do NOT touch `src/`, docs, or `state.json`.

## Gate before returning (MANDATORY)
- `cargo fmt --all --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test --all` — ALL green (the new assertion passes, since the code is
  correct). Report counts. If `multi_upstream::alpha_slow...` flakes under load,
  re-run it in isolation and report.

## Return
Exact test/fixture names changed, the codepoints added and where, the bite-check
reasoning, and the final gate counts. You do NOT advance `state.json`.
