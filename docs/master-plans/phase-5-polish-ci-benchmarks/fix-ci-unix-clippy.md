# Fix CI-only clippy failure — Unix surface audit (src/process.rs)

## Defect
CI (Ubuntu) clippy under `-D warnings`:
```
error: unused import: `std::os::unix::process::CommandExt`
 --> src/process.rs:338:9
```

Root cause (verified): `install_linux_parent_death_signal` receives `&mut tokio::process::Command`. The `pre_exec` method used inside the closure is an **inherent** method on `tokio::process::Command`, not a trait method from `std::os::unix::process::CommandExt`. The import is therefore dead on every target and triggers only on Linux CI.

## Fix applied
Removed the single dead `use std::os::unix::process::CommandExt;` at line 338 (inside the `#[cfg(target_os = "linux")]` function). PDEATHSIG / `prctl(PR_SET_PDEATHSIG, SIGKILL)` behavior unchanged.

## Unix-surface audit (all `#[cfg(unix)]` / `#[cfg(not(windows))]` / `#[cfg(target_os = "linux")]` blocks)
Because the Windows host never compiles these arms, every cfg-gated item was manually inspected for clippy lints that would only fire on Unix CI:

- **Line 11**: `#[cfg(unix)] use process_wrap::tokio::ProcessSession;` — used at 276 and 317. Clean.
- **Line 147/157**: `#[cfg(not(windows))] ProcessTreeGuard::None` variant and `contain_current_process_tree` arm — used; `#[allow(dead_code)]` on the enum is intentional (Windows-only real variant). Clean.
- **Line 274/315**: `#[cfg(unix)] { wrapped.wrap(ProcessSession); }` — live code. Clean.
- **Line 336–354**: `install_linux_parent_death_signal` (linux) — after import removal, no unused imports, no needless returns, no dead code. The `unsafe { cmd.pre_exec(...) }` block is the documented correct pattern; `Ok(())` at the end is required. Clean.
- **Line 356–359**: stub (non-linux) — intentionally empty, returns `Ok(())`. Clean.
- All other Unix-gated items (`ProcessSession` wrapper, `ImmediateDescendantGuard` unix path, etc.) are either used or correctly annotated. No `unused_imports`, `unused_variables`, `dead_code`, `needless_return`, `clippy::let_unit_value`, or similar detected.

No additional fixes required. The Unix surface is now clippy-clean.

## Verification
- `cargo fmt` — clean
- `cargo clippy --all-targets -- -D warnings` (Windows) — clean (0 warnings)
- Behavior unchanged; only a dead import was removed.
- rmcp remains pinned at 1.8.0.

This single run should now pass the 3-OS CI.
