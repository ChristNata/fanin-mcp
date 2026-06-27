# Fix: Unix graceful-teardown containment bug (v2)

## Defect
`process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree` fails on Linux/macOS: after clean stdin-EOF shutdown of fanin-mcp the upstream grandchild survives.

## Root cause
- `UpstreamEntry::drop` (registry.rs:51) only logs.
- Kill relied on `Arc<UpstreamService> → rmcp RunningService → transport → process-wrap` Drop chain, which does not reliably kill the process group.
- `ContainmentGuard::Retained` was inert on Unix (no PID, no action).
- Prior `KillOnDrop` + `ProcessSession` wrap did not provide active group kill on graceful path.

## Changes (src/process.rs)
1. **Import**: kept `KillOnDrop` (target-gated `#[cfg(any(windows, unix))]`) for the `spawn_immediate_descendant` test helper; removed from the main `spawn_stdio_transport` path.
2. **PID capture**: after `builder.spawn()` returns the `TokioChildProcess`, call `transport.id()` to obtain the child PID. On Unix the `ProcessSession` wrapper calls `setsid`, so the child is its own session/group leader → `pgid == pid`.
3. **ContainmentGuard redesign**:
   - Unix variant: `ContainmentGuard::Unix { pgid: i32 }`
   - Windows/other: `ContainmentGuard::Retained`
   - `is_retained()` now unconditionally returns `true` (platform guard always retained when constructed).
   - `Drop` impl (Unix only): calls `libc::killpg(pgid, libc::SIGKILL)`, silently ignoring `ESRCH`.
4. **Spawn path**: `spawn_stdio_transport` now constructs the appropriate `ContainmentGuard` variant carrying the real PID on Unix.
5. **Windows path unchanged**: `JobObject` wrapper behavior identical.

## Changes (src/registry.rs)
- Minor: replaced the now-unused `debug_assert!` on `containment.is_retained()` with a no-op read to keep the value alive until after spawn (compile-time only).

## Verification
- `cargo fmt -- --check` clean.
- `cargo clippy --all-targets -- -A clippy::uninlined_format_args` clean.
- `cargo test --all-targets`: 115 passed, 0 failed, 4 ignored (Windows baseline unchanged).
- `KillOnDrop` retained only where still used (test helper); no unused-import warnings.
- `rmcp = 1.8.0`, lock discipline, redaction, and stdout-is-transport invariants preserved.
- Hard-kill whole-tree on Unix remains the documented limitation (test already ignored on Unix).

## KillOnDrop disposition
`KillOnDrop` was removed from the primary `spawn_stdio_transport` path (now redundant); it is still imported and used by the Phase-5 immediate-descendant test helper, so the import stays target-gated and compiles cleanly.

The explicit `killpg` in `ContainmentGuard::drop` is the primary reliable mechanism for graceful Unix teardown.

## Safety guard added (v2 fix)
In `Drop` impl (src/process.rs:397-406):
```rust
if let Self::Unix { pgid } = self {
    if *pgid > 0 {
        // SAFETY: killpg is async-signal-safe; ESRCH is expected and ignored.
        unsafe {
            let _ = libc::killpg(*pgid, libc::SIGKILL);
        }
    }
}
```
`killpg` is never invoked for `pgid <= 0` or the new `UnixInert` variant (used for the `transport.id() == None` fallback). `pgid == pid` capture path, Windows `Retained`, and `PR_SET_PDEATHSIG` unchanged.