# Implementer task — feature `schema-relay-fidelity`, Phases 2 + 3

Implement **Phase 2 (src decouple)** and **Phase 3 (docs lock)** of
`docs/master-plans/schema-relay-fidelity/master.md`. The tests are written and
form a binding read-only contract — make them green by changing PRODUCTION
code and docs, never by editing tests.

## Read first
- `docs/master-plans/schema-relay-fidelity/master.md` (Phases 2 + 3, Success
  Criteria, Constraints) and `state.json` decisions (`I2-truncation-fix`,
  `I1-sanitization-stance`).
- `src/server.rs` lines ~150-253 (handlers) and ~359-463 (sanitize_* helpers).
- `tests/integration/sanitization.rs` — the contract you must satisfy. Note the
  RED test `get_tool_schema_preserves_full_length_annotations_without_row_cap`
  asserts EXACT full-string equality (235 chars) — the ONLY way to pass it is to
  actually relay the full annotation; do not attempt to special-case it.
- `SECURITY.md` §Threat Model (the prompt-injection paragraph) and
  `docs/GOTCHA.md` #20.

## Phase 2 — decouple control-neutralization from length-capping (`src/server.rs`)

Root cause: `sanitize_upstream_text` (server.rs:369) bundles TWO concerns —
control-neutralization (strip C0/C1/DEL/separators/bidi/zero-width/BOM → space,
trim, single-line) AND a `CAP = 100` length cap. Both `list_tools` (line 173,
description) and `get_tool_schema` (line 251 → `sanitize_metadata_value` →
`sanitize_upstream_text`) call it, so the row cap wrongly truncates full schema
annotations.

**Required behavior after the change:**
- Factor out a control-neutralization helper that strips the forbidden chars +
  trims to a single line but does **NOT** length-cap. Keep the EXACT existing
  neutralization set and the `should_neutralize_upstream_char` predicate
  unchanged.
- `list_tools` row descriptions: neutralize **and** cap at 100 (unchanged
  behavior — keep the row summary cap).
- `get_tool_schema` annotation strings (`title`/`description`/`$comment`/
  `markdownDescription`, including nested `properties.*.description`):
  neutralize **only**, FULL-LENGTH, no cap.
- Leave `sanitize_upstream_identifier` (the name path, CAP 200) untouched.
- Leave schema VALIDATION values (`enum`/`const`/`default`/`pattern`/`examples`)
  and the `invoke_tool` arg + result paths COMPLETELY untouched (D-004).
- Update the stale doc-comment on the sanitize helpers (server.rs ~359-368) so
  it accurately describes the new split: neutralization is display-wide; the
  100-char cap is `list_tools` rows ONLY. (The planner flagged this comment as
  implementation drift.)

Pick the cleanest factoring (e.g. a `neutralize_upstream_display(s)` that both
paths share, with the list-row path appending the cap). Keep it minimal and
idiomatic.

## Phase 3 — lock the docs (`SECURITY.md`, `docs/GOTCHA.md`)

Edit precisely; keep `capital-style` voice; do not restructure the docs.
- **SECURITY.md**, the "Prompt injection via upstream-provided text" paragraph:
  (a) state that the length cap applies to `list_tools` description ROWS, and
  that `get_tool_schema` annotation strings are control-neutralized but relayed
  FULL-LENGTH (no cap); (b) add an explicit sentence that `invoke_tool`
  arguments AND result content pass through VERBATIM by design (D-004) and are
  the residual, bounded, documented injection channel. Keep the honest
  "bounds, cannot eliminate" framing.
- **docs/GOTCHA.md #20:** make the cap statement precise — control-neutralization
  applies to all LLM-visible display annotations (`list_tools` rows +
  `get_tool_schema` `title`/`description`/`$comment`); the ~100-char LENGTH cap
  is `list_tools` description ROWS ONLY, NOT `get_tool_schema` annotations;
  `invoke_tool` args/results are verbatim pass-through. Keep the ✅ marker.

## Hard constraints
- Do NOT edit any file under `tests/`. If you believe a test is wrong, STOP and
  surface it as a test-issue in your returned result — do not edit it.
- Do NOT sanitize/parse/transform `invoke_tool` arguments or result content, and
  do NOT touch validation strings — that violates D-004 and the locked stance.
- stdout stays the MCP transport — no `println!`/stdout writes.
- Scope discipline: touch ONLY `src/server.rs`, `SECURITY.md`, `docs/GOTCHA.md`.
  Surface anything adjacent in your returned result; do not silently change it.

## Gate before returning (MANDATORY)
Run and report results:
- `cargo fmt --all`  then `cargo fmt --all --check` (clean).
- `cargo clippy --all-targets -- -D warnings` (clean).
- `cargo test --all` — ALL must pass now, including
  `get_tool_schema_preserves_full_length_annotations_without_row_cap` (the
  former bite, now green) and every existing sanitization / validation /
  byte-faithful test. Report the full pass/fail counts. If the
  `multi_upstream::alpha_slow_tool_does_not_block_concurrent_beta_echo` timing
  test flakes under load, re-run it in isolation and report.
- `cargo build --release` (clean, 0 warnings).

## Return
A short summary: the helper factoring you chose, the exact doc sentences you
added/changed, the final gate counts, and any out-of-scope items you noticed
(surface, do not fix). You do NOT write or advance `state.json`.
