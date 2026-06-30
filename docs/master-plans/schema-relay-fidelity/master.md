---
Feature: schema-relay-fidelity
Scope: flat
Stack: rust
Tier: thorough
Status: draft
Created: 2026-06-30
Target: src/server.rs
Dependencies: docs/master-plans/schema-relay-fidelity/state.json
---

# Master: schema-relay-fidelity

## What

Ship a narrow schema-relay fidelity bugfix: keep the locked transparent-ish
proxy stance for adversarial natural-language and invoke I/O, add missing
contract coverage for invoke argument control-character round-trip, and remove
the unintended `get_tool_schema` annotation length cap while preserving
control-neutralization and single-line display safety.

## Why

An external stress test found the routing and concurrency layer healthy, but
left two non-routing findings. The first finding is not a behavior bug: the
locked `I1-sanitization-stance` in
`docs/master-plans/schema-relay-fidelity/state.json` confirms that natural
language injection text, schema validation strings, and `invoke_tool` argument
and result bytes pass through verbatim by design. That stance is anchored in
`docs/DECISIONS.md` D-004 and the prompt-injection paragraph in
`SECURITY.md`.

The second finding is a genuine fidelity bug. `src/server.rs` currently routes
`get_tool_schema` through `sanitize_schema_metadata` and
`sanitize_metadata_value`, which call `sanitize_upstream_text`. That helper
both neutralizes control characters and caps strings at 100 Unicode scalars.
The cap is documented as a `list_tools` row control in `SECURITY.md` and
`docs/GOTCHA.md` #20, but it leaks into full schema detail and silently drops
real argument documentation.

Anchoring drift: `src/server.rs` lines 359-368 currently describe
`sanitize_upstream_text` as applying to descriptions and schema annotation
strings, including the cap. The locked decision in `state.json`, `SECURITY.md`,
and GOTCHA #20 narrow that cap to `list_tools` rows only. This plan follows the
locked decision and treats the server comment as implementation drift to fix.

Existing `tests/integration/sanitization.rs` already protects the list-row cap,
validation-string verbatim behavior, and byte-faithful result content. It also
contains stale assertions in the poisoned-metadata schema test and the F3
annotation block that require `get_tool_schema` annotations to be
`<= DESC_CAP`; those assertions lock the bug and must be flipped by the
test-creator.

## Dependencies

- `docs/master-plans/schema-relay-fidelity/state.json` supplies the locked
  decisions: `I1-sanitization-stance`, `I2-truncation-fix`, and
  `I2-test-contract-conflict`.
- `docs/DECISIONS.md` D-004 supplies the raw argument passthrough and
  byte-faithful result invariant.
- `SECURITY.md` Threat Model and `docs/GOTCHA.md` #20 supply the documented
  prompt-injection bounds and list-row cap framing.
- This plan is sequenced linearly. The tests contract must be updated before
  implementation, because the current test assertions encode the truncation
  bug.

## Scope

### In

- Update `tests/integration/sanitization.rs` so `get_tool_schema` annotation
  assertions require full post-neutralization strings, not `DESC_CAP` truncation.
- Add wire-level coverage that `get_tool_schema` returns a long clean
  annotation string over the old cap without truncation.
- Add wire-level coverage that `invoke_tool` arguments containing a C0 control
  character round-trip verbatim in the upstream response.
- Add or reuse probe-server fixture data only as needed to expose a long clean
  schema annotation and an echo path for control-bearing arguments.
- Refactor `src/server.rs` sanitization helpers so control-neutralization is
  shared, while length-capping is applied only to `list_tools` row
  descriptions.
- Update `SECURITY.md` and `docs/GOTCHA.md` #20 so they explicitly state that
  `invoke_tool` arguments and results are intentional verbatim pass-through
  channels.

### Out

- No natural-language prompt-injection scrubbing.
- No untrusted-content envelope or warning wrapper around upstream text.
- No sanitization, validation, parsing, or mutation of `invoke_tool` arguments.
- No sanitization, stringification, or mutation of `invoke_tool` result content.
- No changes to schema validation strings such as `enum`, `const`, `default`,
  `pattern`, or `examples`.
- No changes to routing, namespace ACLs, lazy spawning, concurrency, timeouts,
  cancellation, process containment, or credential handling.
- No changes to `state.json`.

## Phases

### Phase 1 — Test contract correction and regression coverage

**Scope:** Update only the sanitization integration contract and any minimal
probe fixture data needed for the two wire-level gaps.

**Produces:**

- `tests/integration/sanitization.rs`
- `tests/probe-server/**` only if the current fixture cannot expose the needed
  long clean annotation or echo control-bearing arguments deterministically.

**Key Behaviors:**

- Flip stale `get_tool_schema` annotation `<= DESC_CAP` assertions to require
  full post-neutralization strings.
- Add a regression that fails if any `get_tool_schema` annotation over the old
  cap is truncated.
- Add a regression that sends an `invoke_tool` argument containing BEL
  U+0007 and observes that exact value in the returned tool content.
- Keep existing list-row cap assertions unchanged.
- Keep existing validation-string verbatim and byte-faithful result tests
  unchanged except for naming/comment updates needed to align the contract.

**Depends On:** Locked decisions in `state.json`; existing
`tests/integration/sanitization.rs` helpers and probe fixture shape.

**Skills Needed:** `rust-test`, `rmcp-general`.

**Phase Success Criteria:**

1. A test named for long `get_tool_schema` annotation fidelity fails on an
   implementation that caps schema annotations at the old `DESC_CAP` boundary.
2. The existing `get_tool_schema_sanitizes_poisoned_metadata_preserves_shape`
   and F3 annotation assertions no longer require `get_tool_schema` annotation
   strings to be `<= DESC_CAP`.
3. A test named for invoke control-character round-trip fails if an argument
   value containing U+0007 is neutralized, deleted, or escaped into a different
   semantic value before reaching the upstream response.
4. The existing `list_tools` description cap assertion remains present and
   still checks the LLM-visible row description length.

### Phase 2 — Sanitization helper decoupling

**Scope:** Change `src/server.rs` only enough to separate control
neutralization from row length-capping.

**Produces:**

- `src/server.rs`

**Key Behaviors:**

- Preserve the existing neutralization set: C0, C1, DEL, Unicode
  line/paragraph separators, bidi controls, BOM, and common zero-width format
  characters become ASCII spaces in display annotations.
- Keep `list_tools` row descriptions single-line and capped at the current
  row limit after neutralization and trim.
- Keep `list_tools` row tool/name identifiers on the existing identifier path;
  do not route them through the description cap.
- Make `get_tool_schema` annotation values control-neutralized and trimmed but
  full-length.
- Leave schema validation and structural values verbatim.
- Leave `invoke_tool` argument and result paths untouched.

**Depends On:** Phase 1 tests.

**Skills Needed:** `rust-general`, `rmcp-general`.

**Phase Success Criteria:**

1. `get_tool_schema` returns a schema annotation longer than `DESC_CAP` in full
   after control-neutralization.
2. `get_tool_schema` returned annotation strings contain no C0/C1/DEL/bidi/
   zero-width forbidden display controls and remain single-line.
3. `list_tools` row descriptions remain capped and control-neutralized.
4. `invoke_tool` argument and result payloads are not routed through any
   sanitizer.
5. Schema validation strings remain byte-for-byte semantically unchanged in the
   returned schema JSON.

### Phase 3 — Security documentation lock

**Scope:** Update only the two security-facing docs that describe the stance.

**Produces:**

- `SECURITY.md`
- `docs/GOTCHA.md`

**Key Behaviors:**

- State explicitly that `invoke_tool` arguments and result content pass through
  verbatim by design and are a residual prompt-injection channel.
- State that the length cap is for `list_tools` description rows, not full
  `get_tool_schema` annotation strings.
- Keep the existing security posture honest: the aggregator bounds the channel;
  it does not eliminate prompt injection from configured upstreams.

**Depends On:** Phase 2 implementation behavior.

**Skills Needed:** `md-authoring`, `capital-style`.

**Phase Success Criteria:**

1. `SECURITY.md` Threat Model names `invoke_tool` arguments and result content
   as intentional verbatim pass-through by design.
2. `SECURITY.md` distinguishes capped `list_tools` rows from full-length
   `get_tool_schema` annotation strings.
3. `docs/GOTCHA.md` #20 distinguishes display neutralization from row length
   caps and does not imply `get_tool_schema` annotations are capped.

### Phase 4 — Full gate and review readiness

**Scope:** Run the relevant Rust verification for the completed bugfix.

**Produces:**

- No source artifacts beyond Phase 1-3 outputs.

**Key Behaviors:**

- Verify the full sanitization integration suite.
- Verify the repository test gate required by the orchestrator for this cycle.
- Surface any unrelated failures without folding fixes into this plan.

**Depends On:** Phases 1-3.

**Skills Needed:** `rust-test`, `rmcp-general`.

**Phase Success Criteria:**

1. The sanitization integration tests pass with the new long-annotation and
   invoke-control round-trip coverage enabled.
2. No existing list-row cap, validation-string verbatim, or byte-faithful result
   coverage regresses.
3. Any failing test outside this plan's scope is reported as out-of-scope rather
   than fixed silently.

## Success Criteria

1. `get_tool_schema` returns a long clean or post-neutralized annotation string
   over the old cap in full, not truncated at `DESC_CAP` or 100 characters.
2. `get_tool_schema` annotation strings remain single-line and free of the
   existing forbidden display controls after neutralization.
3. `list_tools` row descriptions remain single-line, control-neutralized, and
   capped at the existing row limit.
4. Schema validation strings, including `enum`, `const`, `default`, and
   `pattern` values, remain verbatim in `get_tool_schema` output.
5. An `invoke_tool` argument value containing BEL U+0007 is observed verbatim in
   the upstream response.
6. `invoke_tool` result content remains byte-faithful and is not sanitized or
   stringified.
7. `SECURITY.md` explicitly documents `invoke_tool` arguments and results as
   intentional verbatim pass-through channels.
8. `docs/GOTCHA.md` #20 no longer implies that full `get_tool_schema`
   annotation strings are length-capped.

## Constraints / Invariants

- D-004 is binding: raw `invoke_tool` arguments and result content are not
  parsed, validated, sanitized, or transformed by the proxy.
- `I1-sanitization-stance` is binding: no natural-language scrubbing and no
  untrusted-content envelope.
- `I2-truncation-fix` is binding: control-neutralization applies to both
  `list_tools` rows and `get_tool_schema` annotations; length-capping applies
  only to `list_tools` rows.
- `I2-test-contract-conflict` is binding: test-creator has authority to flip
  the stale cap assertions in `tests/integration/sanitization.rs`.
- stdout remains the MCP transport; no diagnostic output may be added to
  stdout paths.
- Do not hold locks across upstream calls; this plan does not touch the
  registry concurrency path.
- Errors remain tool results, not JSON-RPC errors; this plan does not touch
  error shaping.

## Open Questions

(none)
