# Tests: schema-relay-fidelity

## Files Created

| Path | Contract Covered |
|---|---|
| `tests/integration/sanitization.rs` | Update existing wire-level sanitization contract for Success Criteria 1-6. |
| `tests/probe-server/**` if needed | Minimal fixture support for a long clean schema annotation and invoke echo of control-bearing arguments. |

No separate test file is required unless the existing sanitization integration
module becomes too large to keep readable. Prefer extending the existing module
because it already owns the helpers, probe config, and stale assertions that
must be flipped.

## Coverage Map

| Master Criterion | Test Contract |
|---|---|
| SC 1 — long `get_tool_schema` annotation is full-length | New test: `get_tool_schema_preserves_full_length_annotations_without_row_cap`. It must request a probe schema whose annotation string is longer than `DESC_CAP` after neutralization and assert the returned annotation equals the expected full string or has the exact expected full character count and suffix. A reintroduced 100/120-char cap must fail loudly. |
| SC 2 — schema annotations remain display-safe | Existing `get_tool_schema_sanitizes_poisoned_metadata_preserves_shape` plus F3 annotation assertions, after the flip, still assert no newline/carriage return/control display chars in annotation strings. |
| SC 3 — `list_tools` rows remain capped | Existing `list_tools_sanitizes_poisoned_description_strips_control_and_caps_length` and F1 list-row coverage remain unchanged for the row description cap. |
| SC 4 — validation strings remain verbatim | Existing `f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations` remains unchanged for `enum`, `default`, and `const`; add `pattern` only if the fixture already exposes it cheaply. |
| SC 5 — invoke argument BEL round-trips | New test: `invoke_tool_arguments_with_control_chars_round_trip_verbatim`. It sends a message containing BEL U+0007 through `invoke_tool` to an echo-like probe tool and asserts the response text contains the same semantic string, including BEL. |
| SC 6 — invoke result byte-faithfulness | Existing `invoke_tool_result_content_not_sanitized_passes_byte_faithfully` remains green and unchanged in behavior. |
| SC 7 — SECURITY.md documents invoke pass-through | Documentation review assertion: Threat Model text names `invoke_tool` arguments and results as intentional verbatim pass-through. |
| SC 8 — GOTCHA #20 documents row-only cap | Documentation review assertion: GOTCHA #20 distinguishes row description cap from full schema annotation neutralization. |

## Required Test Edits

### Flip stale get_tool_schema cap assertions

In `tests/integration/sanitization.rs`, update these stale assertions:

- `get_tool_schema_sanitizes_poisoned_metadata_preserves_shape`, around the
  annotation checks for `title`, `description`, and `$comment`.
- `f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations`,
  around the annotation checks for `title` and `description`.

The flipped contract is:

- still assert annotation strings are single-line;
- still assert annotation strings are free of the forbidden display controls;
- do not assert `chars().count() <= DESC_CAP` for `get_tool_schema` annotation
  strings;
- assert full expected content when the fixture provides a known long clean or
  post-neutralized annotation.

Do not change the `DESC_CAP` constant just to make these pass. The constant
still belongs to `list_tools` row description tests.

### Add full-length schema annotation regression

Add a wire-level test that calls `get_tool_schema` for a probe tool with an
annotation longer than the old cap. Preferred fixture shape:

- a clean annotation string with no controls and a distinctive suffix after
  character 120; or
- reuse an existing poison fixture only if its post-neutralization string is
  deterministic, still longer than `DESC_CAP`, and has a distinctive suffix.

The assertion must prove the cap is gone. Acceptable observables:

- exact equality to the expected full annotation string;
- exact character count plus assertion that a suffix beyond the old cap is
  present;
- exact equality after applying the expected control-neutralization transform
  in the test helper.

Weak assertion to avoid: only checking `len() > DESC_CAP` without checking the
tail content. That can miss mid-string corruption.

### Add invoke argument control-character round-trip

Add a wire-level test that calls `invoke_tool` through the aggregator with an
argument string containing BEL U+0007.

The test must assert:

- the tool call succeeds as a tool result, not a JSON-RPC error;
- the response content includes the exact argument value, including BEL;
- no sanitizer has replaced BEL with a space, removed it, or otherwise changed
  the semantic string.

Use an existing echo-like probe tool if it already returns the input message
verbatim. Add fixture support only if the existing echo path normalizes text.

## Deferred Tests

(none)

## Side-Effect Assertions

- Existing `list_tools` row cap tests remain green and continue to fail if row
  descriptions are emitted uncapped.
- Existing validation-string tests remain green and continue to fail if
  validation data is sanitized.
- Existing byte-faithful result tests remain green and continue to fail if
  content blocks are stringified or sanitized.
- No test should require natural-language prompt-injection scrubbing.
- No test should require an untrusted-content envelope around upstream text.

## Fixture Notes

- Prefer a new probe schema tool with a long clean `description` or `title` and
  a distinctive suffix. This makes the truncation regression direct and avoids
  coupling the expected value to a poison-string cleanup algorithm.
- Reusing `poison_schema` or `poison_validation` is acceptable only if the
  expected post-neutralization output can be asserted exactly.
- Keep fixture additions minimal. Do not add routing, concurrency, namespace,
  or process-lifetime behavior to support this plan.
