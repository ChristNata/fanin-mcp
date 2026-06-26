# General Review: phase-1-config-upstream

Found 0 blocker, 0 structural, 4 targeted, 0 trivial.

## Verification

- `cargo test` passed: 61 passed, 0 failed, 2 ignored/deferred.
- `cargo clippy --all-targets --all-features -- -D warnings` passed.
- `cargo fmt --all -- --check` failed on formatting drift in source and test
  files; source locations are called out below.
- Context7 checked `/websites/rs_rmcp_rmcp` for rmcp client/server handler and
  `Peer::list_tools` pagination signatures.

## Done Well

- The module split is coherent: config owns startup validation, registry owns
  lazy connection/cache state, forward owns the upstream `ClientHandler`, and
  server owns meta-tool dispatch.
- The registry uses `Arc<RunningService<...>>`, drops registry map guards before
  upstream calls, and documents the lock discipline where future maintainers
  need it.
- rmcp handler signatures match the pinned API shape, and tool-level failures
  stay as `CallToolResult::error` rather than leaking JSON-RPC errors.
- The code avoids stdout diagnostics and has no production `unwrap`/`expect`,
  `unsafe`, or anti-stack dependency creep.

## Findings

- File: src/error.rs:37; src/registry.rs:69
  Severity: targeted
  Pass:     general
  What:     The source tree is not rustfmt-clean; `cargo fmt --all -- --check`
            reports diffs in `src/error.rs` and `src/registry.rs`.
  Why:      The Rust review baseline requires a formatting-clean tree. This is
            contained and mechanical, but it makes the standard formatting gate
            red.
  Cite:     rust-review §Stack & versions / lint baseline.
  Fix:      Run `cargo fmt --all` and keep the generated formatting changes.

- File: src/registry.rs:138
  Severity: targeted
  Pass:     general
  What:     Upstream inventory discovery uses `peer().list_tools(None)` once and
            caches only the first page.
  Why:      rmcp exposes `list_tools` as a paginated request. A future upstream
            with more tools than one page will silently lose tools from the
            session cache, making discovery and invoke behavior inconsistent.
  Cite:     rmcp-general §RunningService and the lock discipline; Context7
            `Peer::list_tools` pagination docs.
  Fix:      Use the pagination-safe helper if available for the pin, or loop on
            `next_cursor` until all pages are collected before caching.

- File: src/server.rs:299
  Severity: targeted
  Pass:     general
  What:     `invoke_tool` treats a non-object `arguments` value as `None` and
            still calls the upstream.
  Why:      The meta-tool schema requires an object. Silently dropping malformed
            arguments turns a caller error into an upstream call with empty
            input, which is hard to diagnose and can execute the wrong action.
  Cite:     rust-review §General pass / ordinary correctness faults; D-004 raw
            argument passthrough.
  Fix:      Require `args.get("arguments")` to exist and be an object. Return a
            structured `invalid_request` tool result when it is missing or has
            the wrong type.

- File: src/process.rs:60
  Severity: targeted
  Pass:     general
  What:     Log writes are fire-and-forget tasks, and `write_all` / `flush`
            errors are discarded.
  Why:      Child stderr and upstream log notifications are part of the
            observability contract. Dropping the join handle and swallowing
            write failures makes log delivery nondeterministic under shutdown
            or filesystem errors, and future phases will be hard to debug when
            required evidence disappears.
  Cite:     rust-review §Concurrency correctness / spawned task whose
            `JoinHandle` is dropped; rust-review §Error handling.
  Fix:      Route log lines through one owned async logger task per sink, or make
            `append_log_line` return a future/result that callers await where
            ordering and durability matter. At minimum, warn on write and flush
            failures.

## Verdict

PASS-WITH-ISSUES
