# Fix: process-tree-containment

## Defect

`process_lifetime::hard_kill_orphan_test_no_surviving_descendants` failed on
Windows because the specific descendant PID stayed alive for the full bounded
poll window after fanin-mcp was force-killed.

## Root cause

rmcp `=1.8.0` does spawn through `process_wrap::tokio::CommandWrap`; it does not
strip the wrapper. process-wrap `9.1.0` also configures
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` when `JobObject` sees the sibling
`KillOnDrop` wrapper, and it does not request breakaway.

The failure was the hard-kill carrier: relying on the wrapped child transport
left the kernel Job Object lifetime tied to rmcp/process-wrap internals. On
`taskkill /F`, Rust `Drop` does not run, so `KillOnDrop` is irrelevant. The
proxy must own a Job Object handle that remains open for the exact upstream
lifetime and closes only when fanin-mcp exits or is killed.

## Fix applied

Fix avenue B. `src/process.rs` now creates an explicit Windows Job Object after
rmcp spawns the upstream, sets only `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, opens
the upstream process by PID, assigns it to the job, and returns the retained
containment guard with the transport. The job does not set breakaway flags.

`src/registry.rs` stores the containment guard inside `UpstreamEntry`, alongside
the `RunningService`, so the Job Object handle lives for the upstream service
lifetime. Unix still uses the existing process-wrap `ProcessSession` path and
stores a no-op guard.

## Verification

- `cargo build`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test --test integration process_lifetime::hard_kill_orphan_test_no_surviving_descendants`
  looped three times: passed 3/3.
- `cargo test --test integration`: 84 passed, 0 failed, 3 ignored.
- `grep -rn "marker\|grandchild\|spawn_grandchild\|temp_dir" src/`: only a
  legitimate containment comment in `src/process.rs`.

One full-suite run produced a transient failure in
`timeout_cancellation::cancellation_frees_local_resources_without_waiting_full_upstream`;
rerunning that test passed, and rerunning the full integration suite passed.

## Suggested-fix divergence

The requested order listed process-wrap integration as avenue A first. The API
checks showed rmcp uses the wrapper and process-wrap sets kill-on-close without
breakaway, but the test still failed. The applied fix therefore used avenue B:
an explicit Win32 Job Object retained by the registry.

## Anything surfaced

- targeted: The Unix `ProcessSession` path is unchanged and still needs Linux CI
  verification for the same descendant-liveness oracle.
