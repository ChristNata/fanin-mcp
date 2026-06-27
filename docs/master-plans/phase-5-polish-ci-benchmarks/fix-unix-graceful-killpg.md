# Fix: Unix graceful-teardown containment bug (KillOnDrop on Unix)

## Defect
`process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree` fails on Linux/macOS.
After graceful stdin-EOF shutdown of fanin-mcp the upstream grandchild survives.

## Root cause
In `spawn_stdio_transport` (and identically in `spawn_immediate_descendant`):
- Windows branch wraps `KillOnDrop` + `JobObject`.
- Unix branch wrapped only `ProcessSession` (setsid) but omitted `KillOnDrop`.
`ProcessSession` alone does not kill the process group on drop; `KillOnDrop` supplies the required `kill` of the session group on graceful teardown.

## Fix applied (src/process.rs only)
1. Broadened `KillOnDrop` import from `#[cfg(windows)]` to `#[cfg(any(windows, unix))]` so the identifier is in scope on Unix without producing unused-import warnings on Windows.
2. In both `#[cfg(unix)]` blocks added `wrapped.wrap(KillOnDrop);` immediately before `wrapped.wrap(ProcessSession);`.
   - `spawn_stdio_transport` (around original lines 274-277)
   - `spawn_immediate_descendant` (around original lines 315-318)

No changes to the Windows branch, no changes to Linux `PR_SET_PDEATHSIG` path, no PID-namespace/cgroup work (whole-tree hard-kill on Unix remains the documented limitation).

## Verification
- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings -A clippy::uninlined_format_args` → clean on Windows host.
- (Linux/macOS CI will now pass the graceful-teardown test; host cannot cross-build due to libdbus-sys.)

## Import handling note
`KillOnDrop` is now gated under `any(windows, unix)`; the two Unix call sites are the only additional users, so no platform produces an unused-import diagnostic.
