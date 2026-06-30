# Test-creator task — feature `schema-relay-fidelity`, Phase 1 ONLY

Implement **Phase 1** of `docs/master-plans/schema-relay-fidelity/master.md`:
the test-contract correction + regression coverage. You are the SOLE authority
over test files. Do NOT touch `src/server.rs` (that is Phase 2, the
implementer) and do NOT touch `SECURITY.md`/`docs/GOTCHA.md` (Phase 3).

## Read first
- `docs/master-plans/schema-relay-fidelity/master.md` (Phase 1 + Success
  Criteria) and `tests.md` (the full test contract — follow it exactly,
  including the anti-gaming requirements).
- `docs/master-plans/schema-relay-fidelity/state.json` decisions
  (`I1-sanitization-stance`, `I2-truncation-fix`, `I2-test-contract-conflict`).
- `tests/integration/sanitization.rs` (the existing contract — read in full).
- `tests/probe-server/main.rs` (the fixtures: `echo_ok` echoes the `message`
  arg verbatim; `poison_schema`/`poison_validation` carry control-laden
  annotations; `long_named_tool` has a clean description).

## What to do

### 1. Flip the stale `get_tool_schema` cap assertions
In `tests/integration/sanitization.rs`:
- `get_tool_schema_sanitizes_poisoned_metadata_preserves_shape` (~lines
  411-446): the `title`/`description`/`$comment` blocks currently assert
  `chars().count() <= DESC_CAP`. REMOVE those `<= DESC_CAP` assertions for
  get_tool_schema annotations. KEEP the single-line (no `\n`/`\r`) and
  control-free assertions.
- `f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations`
  (~lines 946-968): same flip for its `title`/`description` annotation block.
- Do NOT change the `DESC_CAP` constant — it still belongs to the `list_tools`
  row tests. Do NOT touch the `list_tools` row cap assertions, the F3
  validation-string-verbatim assertions, or the byte-faithful result test.

### 2. Add the full-length annotation regression (the Issue-2 BITE)
New `#[tokio::test]`
`get_tool_schema_preserves_full_length_annotations_without_row_cap`:
- Preferred fixture: ADD a probe tool (e.g. `long_desc`) with a CLEAN
  (control-free) `description` (and/or `title`) WELL over the old cap — at
  least ~180 chars — ending in a DISTINCTIVE suffix (e.g. a unique marker token
  past char 120). A clean string avoids coupling the expected value to the
  neutralization transform.
- Assert the returned `get_tool_schema` annotation equals the FULL expected
  string (exact equality), OR exact char-count PLUS presence of the distinctive
  tail-past-120 marker. The weak `len() > DESC_CAP`-only check is REJECTED by
  the contract — a re-introduced 100/120 cap MUST fail this test loudly, and
  mid-string corruption must not slip through.
- This test is RED against the current tree (cap still present) and goes GREEN
  only after Phase 2. That is correct.

### 3. Add the invoke control-char round-trip lock (Issue-1)
New `#[tokio::test]`
`invoke_tool_arguments_with_control_chars_round_trip_verbatim`:
- Call `invoke_tool` → `probe__echo_ok` with `{"message": "wei\u{0007}rd"}`
  (BEL U+0007 embedded). Assert: (a) the call is a tool RESULT, not a JSON-RPC
  error / not isError; (b) the echoed response content contains the exact value
  `wei\u{0007}rd` — BEL PRESENT and unchanged, NOT replaced by a space, deleted,
  or escaped. This LOCKS the byte-faithful invoke channel (D-004).
- NOTE: this test is expected GREEN against the current tree (invoke I/O is
  already byte-faithful) — it is a regression lock for the documented stance,
  not a RED-then-fix test. Confirm it passes as-is.

## Probe fixture rules
- Adding a clean-long-description tool is in scope (test infrastructure).
- Keep fixture additions MINIMAL — no routing/concurrency/namespace/
  process-lifetime behavior. If you add a tool to the probe's tool list, mirror
  the existing `*_tool()` builder pattern and register it in the inventory.
- If `echo_ok` already round-trips the `message` arg verbatim (it does), reuse
  it for test 3 — do not add a new echo path.

## Gate before returning (MANDATORY)
Run and report:
- `cargo fmt --check` — MUST be clean (so the implementer's later fmt sweep
  never edits these read-only test files).
- `cargo clippy --all-targets -- -D warnings` — MUST be clean.
- `cargo test --all` — report the FULL red/green split. Expected:
  test 2 (full-length) RED; the flipped get_tool_schema cap assertions RED;
  test 3 (invoke round-trip) GREEN; ALL other existing tests GREEN. If anything
  diverges from that, say so explicitly.

## Return
A short summary: the exact test names added/modified, any probe fixture added,
the red/green split with counts, and anything out-of-scope you noticed
(surface it — do not fix it). You do NOT write or advance `state.json`.
