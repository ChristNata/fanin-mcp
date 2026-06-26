# review-adversarial: phase-0-skeleton

Found 2 blocker, 0 structural, 1 targeted, 0 trivial.

## Verification run

- `cargo nextest run --workspace`: PASS — 14 run, 14 passed, 2 skipped.
- `cargo test --workspace --doc`: FAIL — cargo exits non-zero with
  `error: no library targets found in package fanin-mcp`.
- `cargo clippy -- -D warnings`: PASS.

## Findings

- File: `src/server.rs:213`, `src/error.rs:20`,
  `tests/integration/aggregator.rs:203`
  Severity: blocker
  Pass: adversarial
  What: The aggregator's not-implemented tool error is plain text, not the
        structured JSON error shape required by D-005.
  Why: The test claims the content is "readable structured JSON" but only
       checks for any text block. A client expecting the public API shape
       (`server`, `tool`, `code`, `message`, `recoverable`) receives
       `tool `name` is not implemented...` and cannot parse or route the
       error. Trigger: call any meta-tool or unknown tool; the response has
       `isError: true` but non-JSON text content.
  Cite: D-005 / GOTCHA #3; fakery checklist: tests assert return shape instead
        of the documented effect.
  Fix: Build the error content as structured JSON now, including at least
       `server`, `tool`, `code`, `message`, and `recoverable`; update the
       test contract through `test-creator` to parse and assert those fields.

- File: `docs/master-plans/phase-0-skeleton/tests.md:8`, `Cargo.toml:18`
  Severity: blocker
  Pass: adversarial
  What: The documented verification command is red for this binary-only crate.
  Why: `cargo test --workspace --doc` exits non-zero with `no library targets
       found`, so the suite described by `tests.md` cannot reach 100% pass rate.
       `autobins = false` and explicit `[[bin]]` targets are fine for Phase 0,
       but there is no library target for doc tests to run.
  Cite: Covenant invariant: 100% test pass rate; plan-format objective gate.
  Fix: Route to `test-creator` to correct the doc-test gate for a binary-only
       crate, or intentionally add a library target if the product shape changes.

- File: `tests/probe-server/main.rs:287`
  Severity: targeted
  Pass: adversarial
  What: `needs_sampling` spawns an unbounded detached task that can wait forever
        for a client response.
  Why: The handler returns immediately, so the current test observes the
       outbound request and passes. But every call leaves a pending
       `peer.send_request(...).await` if the client never answers. A later
       long-lived probe session or repeated adversarial calls can accumulate
       leaked tasks and request state until disconnect.
  Cite: GOTCHA #2; rust-review concurrency/resource-leak sweep.
  Fix: Bound the detached send with a short timeout and log the timeout to
       stderr/tracing, or emit the reverse request through an API that does not
       retain a pending response future after the frame is written.

## Attacks attempted

- Searched source and probe for stdout writes after `serve(stdio())`: no
  `println!`, `print!`, or `dbg!` implementation path found; tracing is
  explicitly configured with `with_writer(std::io::stderr)` in both binaries.
- Checked downstream capability construction: both servers use
  `ServerCapabilities::builder().enable_tools().build()` and do not advertise
  sampling or elicitation.
- Scanned server request paths for production `unwrap` / `expect` / indexing:
  none found in `src/`; panics are confined to test harness code.
- Reviewed malformed/edge tool-call handling by shape: valid `tools/call`
  requests always reach the stub and return `CallToolResult::error`; malformed
  JSON-RPC params are rejected by rmcp before `call_tool`, which is protocol
  error territory.
