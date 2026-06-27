# OSS-Readiness Test Contract (tests.md)

**Cycle:** oss-readiness  
**Scope:** narrow — only genuinely-new behavioral tests.  
**Tier:** thorough, stack = rust.  
**Date:** 2026-06-28

## Files Created

- `tests/integration/literal_header_redaction.rs`  
  New integration test exercising H-3 (literal secret header redaction).  
  Mirrors the structure of the existing HTTP redaction test but supplies a literal `Authorization` value with no `${VAR}` template.

## Coverage Map

| Criterion | Verification Approach | Test(s) |
|-----------|-----------------------|---------|
| H-3 (literal header redaction) | New behavioral test asserting sentinel never appears un-redacted | `literal_secret_header_value_is_registered_for_redaction` |
| H-2 (identifier length cap) | **Defense-in-depth, code-inspection only.** Probe-server already emits a 120-char tool name (`LONG_TOOL_NAME`). rmcp server API prevents registering a name ≥ 200 chars, so the cap is unreachable via any real upstream. No test written. | — |
| H-1 (mutex poison) | Code inspection (grep: no `.expect()` on the two global `Mutex`es). Poisoning from an integration test is not worth a fixture. | — |
| H-4 / H-5 / H-6 | Existing tests must stay green (meta-tools, startup/observability, `cred list` raw output preserved). | — |
| D-2, O-1/O-2/O-3, H-8 | Docs/metadata only — no code, no tests. | — |

## Side-Effect Assertions

The single new test asserts on the **observable effect** (sentinel secret absent from the log file produced by the child process). A stub that merely returns success without performing unconditional registration would fail.

## Deferred Tests

None in this cycle.

## Notes for Orchestrator / Implementer

- The suite must compile and be `cargo fmt` + `clippy -D warnings` clean.
- The new test is expected **RED** until the implementer lands the H-3 change.
- All 134 existing tests must remain **GREEN**.
- H-2 is intentionally left as inspection-only; attempting to force an over-cap name from the probe would require changing rmcp’s server API, which is out of scope.
