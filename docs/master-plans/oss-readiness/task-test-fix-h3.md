FIX a flawed test you authored — oss-readiness H-3. TEST FILE ONLY.

You are the test-creator (sole authority over tests). The H-3 implementation is
CORRECT and complete: `src/registry.rs` now registers EVERY resolved header value
for redaction unconditionally (the `if raw.contains("${")` guard is gone). But the
test you wrote, `tests/integration/literal_header_redaction.rs`, cannot pass —
its design is flawed.

## The flaw
The test points the HTTP upstream at an UNREACHABLE endpoint
(`endpoint = "http://127.0.0.1:9"`). On that path the Authorization header is never
transmitted or written to any log line, so:
- assertion 1 (raw secret absent from log) passes TRIVIALLY — the header was never
  logged at all, fixed or not, so this assertion does not actually distinguish the
  bug;
- assertion 2 (`[REDACTED]` present) FAILS — nothing was logged, so there is no
  redaction marker. This is the observed failure
  (`literal_header_redaction.rs:54: redaction marker must appear`).

## The fix
Rework the test to use the REACHABLE loopback HTTP mock that the existing passing
test uses, so the literal header value is actually transmitted and logged and
redaction is observable. Model it directly on
`tests/integration/http_upstream.rs::http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs`
(and its `start_http_probe` helper, which binds `127.0.0.1:0`). Reuse that helper
if visible across the integration crate; otherwise mirror its structure.

The reworked test must:
- configure a streamable-HTTP upstream whose Authorization header is a LITERAL
  secret (no `${VAR}` template) containing the sentinel — that is the whole point
  of H-3 (literal values, not just `${VAR}` ones, must be redacted);
- point at the reachable mock so a real `invoke_tool` round-trip occurs and the
  header value reaches the log;
- assert the sentinel does NOT appear raw in the log AND `[REDACTED]` DOES appear.

This way the test genuinely distinguishes fixed-vs-unfixed: WITHOUT the
unconditional registration the literal secret would appear raw (test fails); WITH
it the value is `[REDACTED]` (test passes).

## Rules
- Edit ONLY `tests/integration/literal_header_redaction.rs` (and, if strictly
  needed, add a shared helper in `tests/common/` — but prefer reusing
  `start_http_probe`). Do NOT touch `src/**` — the implementation is correct.
- The suite must compile, be `cargo fmt`-clean and `clippy -D warnings`-clean.
- After your change, `cargo test --all` must be 100% GREEN (this test passes, all
  others stay green). Run it and confirm.

Return as data for the orchestrator: what you changed, whether you reused
`start_http_probe`, and the final gate numbers.
