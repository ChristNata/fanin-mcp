# Implementer result: phase-5-polish-ci-benchmarks P2

## Status

PASS. Process lifetime tests are green on the Windows host.

## Per-file changes

- `src/process.rs`
  - Added Windows process-tree containment for `fanin-mcp` itself via a retained
    kill-on-close Job Object.
  - Switched upstream Windows spawning to `process-wrap` `JobObject` +
    `KillOnDrop`; this uses suspended spawn, assigns to the Job Object, then
    resumes.
  - Added Linux `pre_exec` `prctl(PR_SET_PDEATHSIG, SIGKILL)` for upstream and
    immediate descendant children.
  - Added the contained immediate-descendant spawn helper.
- `src/main.rs`
  - Added the hidden `--spawn-immediate-descendant <marker>` hook and private
    sentinel child mode that writes its PID and sleeps.
  - Installed the Windows self Job Object for `serve` before serving.
  - Also honors the existing process-lifetime fixture shape where the hidden
    immediate flag appears in the configured probe args.
- `Cargo.toml` / `Cargo.lock`
  - Added target-gated Linux `libc = "0.2"` for `prctl`.

## CARRY-1 path and API evidence

Path taken: process-wrap suspended-spawn path, not a custom transport.

Evidence checked:

- Context7 for rmcp `=1.8.0` shows `TokioChildProcessBuilder::spawn()` returns
  `(TokioChildProcess, Option<ChildStderr>)` and exposes only `id()` /
  `into_inner()`, not a main-thread handle for manual `ResumeThread`.
- rmcp `1.8.0` source shows the builder stores a `process_wrap::tokio::CommandWrap`
  and calls `self.cmd.spawn()`, so process-wrap wrappers are applied.
- process-wrap docs/source show `JobObject` sets `CREATE_SUSPENDED`, assigns the
  process to the Job Object, and resumes threads unless the user explicitly
  requested `CREATE_SUSPENDED`.

I also added a retained self Job Object for `fanin-mcp` on Windows. That closes
containment for descendants spawned by fanin itself and makes the force-kill
oracle pass under the Windows test runner.

## Linux approach

Linux uses `tokio::process::Command` `pre_exec` to call:

```rust
libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL)
```

This is `#[cfg(target_os = "linux")]`; macOS stays graceful-only. Local Linux
execution was not possible on this Windows host. `cargo check --target
x86_64-unknown-linux-gnu --all-targets` was attempted but the target is not
installed (`can't find crate for core`). CI remains the Linux runtime verifier.

## Verification

- `cargo test --test integration process_lifetime` — PASS, 4 passed.
- `cargo clippy --all-targets -- -D warnings` — PASS.
- `cargo fmt -- --check` — PASS.

## Surfaced issues

- The integration fixture passes `--spawn-immediate-descendant` through the
  configured probe server args, while the task brief describes a fanin CLI flag.
  I supported both paths to keep the binding test green without editing tests.
