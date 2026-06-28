FIX a gamed test — oss-readiness A2 (structural). TEST FILES ONLY.

You are the test-creator. A prior fix to `tests/integration/literal_header_redaction.rs`
GAMED the contract: to make it pass, it DROPPED the `assert!(logs.contains("[REDACTED]"))`
positive assertion and kept only `assert!(!logs.contains(&secret))`. The adversarial
review proved that negative assertion passes TRIVIALLY — NO production code path
ever writes a resolved HTTP header value to a log line, so the secret is never
present whether or not the H-3 fix exists. The test would still pass if
`src/registry.rs:126`'s `register_secret(&resolved)` were deleted. It does not
verify H-3 at all. This is exactly the "weaken the assertion to pass" failure mode
— do NOT do that again.

## Context
- H-3 (the impl) is correct: `registry.rs:124-128` registers EVERY resolved header
  value for redaction unconditionally. This is defense-in-depth — `redact()`
  (`process.rs`) replaces a registered secret with `[REDACTED]` IF it ever appears
  in a log line / stderr. The catch: nothing in production currently logs header
  values, so there is no natural path that emits the value to observe redaction.
- The redaction layer DOES run on the log-file and stderr writers, and on upstream
  notifications routed through `forward.rs` (`append_log_line` / the logging-message
  handler redact before writing).

## Required outcome — pick ONE, honestly (no gaming)

**Option A (preferred) — make the test genuinely bite.** Drive the resolved
Authorization header value into a redacted log path and assert it is `[REDACTED]`.
The realistic route: extend the loopback HTTP probe (`start_http_probe` in
`tests/integration/http_upstream.rs`) so it emits an MCP server→client logging
notification (or otherwise causes fanin-mcp to log) that ECHOES the Authorization
header value it received. That value then flows through `forward.rs`'s redacted
log path into the per-server log file. Then assert BOTH `!logs.contains(&secret)`
AND `logs.contains("[REDACTED]")`. With the H-3 fix the literal value is
`[REDACTED]`; WITHOUT it (guarded registration) the literal value would appear raw
→ the test fails → it genuinely bites. Probe/helper code under `tests/` is yours
to extend.

**Option B (only if A is genuinely infeasible without editing `src/`)** — make the
test HONEST instead of fake. Keep `!logs.contains(&secret)` as a leak-regression
guard, BUT rename the test and add a doc-comment stating plainly: this is a
defense-in-depth guard that no production path currently logs header values, so
`[REDACTED]` cannot be asserted behaviorally; the registration→redaction wiring for
literal header values is verified by inspection (cite `registry.rs:126`). Do NOT
present it as a behavioral redaction proof. Report clearly that you took Option B
and exactly why A was infeasible.

## Rules
- Edit ONLY `tests/**` (the test + `http_upstream.rs` probe / `tests/common` if
  needed). Do NOT touch `src/**` — the H-3 impl is correct.
- `cargo test --all` must be 100% GREEN; `cargo fmt --check` + `clippy -D warnings`
  clean. If you take Option A, run it and confirm the new positive assertion
  actually passes (and would fail if registration were guarded — reason about it).
- Return: which option you took and why; what you changed; the final gate numbers.
