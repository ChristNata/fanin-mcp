# Fix: phase-1-config-upstream review findings

## Defects

- Pagination: `src/registry.rs` cached only the first upstream `tools/list` page.
- Invoke validation: `src/server.rs` forwarded missing or non-object
  `arguments` despite the meta-tool schema requiring an object.
- Transport validation: `src/config.rs` accepted unsupported transports and
  later treated them as stdio.
- Log write robustness: `src/process.rs` used unbounded line buffering and
  spawned one file-open/write/flush task per stderr line.
- Rustfmt: source formatting drift existed in `src/error.rs` and
  `src/registry.rs`.

## Root cause

The review findings were confirmed. The implementation used the first-page
rmcp `list_tools(None)` API, deferred schema validation to permissive JSON
accessors, documented `transport` as accepted-but-ignored, and treated log
writes as fire-and-forget per-line tasks.

## Fix applied

- `src/registry.rs`: replaced `peer().list_tools(None).await?.tools` with
  `peer().list_all_tools().await`, preserving the existing
  `ToolError::UpstreamConnect` mapping. Context7 confirmed rmcp exposes
  `async fn list_all_tools(&self) -> Result<Vec<Tool>, ServiceError>` for
  `Peer<RoleClient>` and that it pages through `list_tools` until complete.
- `src/server.rs`: require `arguments` to be present and an object before
  namespace lookup or upstream dispatch. Invalid requests now return
  `ToolError::InvalidRequest` as `CallToolResult { isError: true }`. Valid
  objects still pass through unchanged as raw JSON maps.
- `src/config.rs` / `src/error.rs`: added fail-fast startup validation for
  `transport`, accepting only absent or literal `stdio`, with typed
  `StartupError::UnsupportedTransport` rendered by the existing startup error
  path.
- `src/process.rs` / `src/forward.rs`: replaced `BufReader::lines()` with
  bounded chunk reading capped at 8 KiB per line plus a truncation marker.
  Replaced per-line spawned file writes with one bounded-channel writer task
  per `(log_file, server)` that holds the file open, applies backpressure, and
  warns through `tracing` on enqueue, open, write, or flush failure.
- `cargo fmt --all`: applied rustfmt after the code edits.

## Verification

- `cargo fmt --all -- --check`: exited 0.
- `cargo build 2>&1 | tail -3`:

```text
   Compiling fanin-mcp v0.1.0 (C:\Users\Chrisyian\RustroverProjects\fanin-mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.89s
```

- `cargo clippy --all-targets 2>&1 | grep -c "warning:"`:

```text
0
```

- `cargo test --test integration 2>&1 | grep "test result:"` run 1:

```text
test result: ok. 56 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 1.14s
```

- `cargo test --test integration 2>&1 | grep "test result:"` run 2:

```text
test result: ok. 56 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 1.19s
```

## Suggested-fix divergence

None. The targeted suggestions held after independent verification. The log
writer fix implements the requested bounded reader and single per-server writer
shape without changing stdout behavior.

## Surfaced

- targeted: `cargo fmt --all` also formatted existing test files. No test
  assertions or logic were changed, but this conflicts with the usual
  tests-read-only invariant. The requested rustfmt gate cannot pass without
  those formatting changes because the test tree had rustfmt drift.
