# Implementer task — Phase 5, plan **Phase 2: Process Containment Hardening**

**This is the highest-risk phase of the cycle: the carried D-009 process-tree
blockers (CARRY-1, CARRY-2).** Implement ONLY plan Phase 2. The success oracle
is real OS PID liveness (`tasklist` / `kill -0`) — it CANNOT be gamed by marker
files or test-name matching. Do not attempt to. If a test seems impossible,
STOP and surface a test-issue; do not contort `src/` to fake a pass.

## Read first

- `master.md` §"Phase 2 — Process Containment Hardening" (Key Behaviors + SC).
- **`tests/integration/process_lifetime.rs`** — your binding READ-ONLY contract.
  Study the oracle docs at the top: the grandchild/descendant PID is the
  observable; the marker file persists even on a contained (hard) kill, so
  marker-absence is NOT the test — the DEAD PROCESS is.
- `tests/probe-server/main.rs` — the test-creator added the immediate-descendant
  fixture; read what it expects.
- `src/process.rs` — the current containment: post-spawn
  `AssignProcessToJobObject` (Windows, the CARRY-1 race) and `ProcessSession`
  (Unix, the CARRY-2 hard-kill gap). `Cargo.toml` process-wrap features:
  `job-object`, `kill-on-drop`, `creation-flags`, `process-group`,
  `process-session` are already enabled.
- `docs/master-plans/phase-5-polish-ci-benchmarks/carry-over.md` CARRY-1/CARRY-2
  (the exact fixes + the rmcp suspended-spawn obstacle + fallback).
- Skills: `rmcp-general` (process & transport section), `rust-general`.

## What to build

### 1. The test hook: `--spawn-immediate-descendant <marker_path>`
The test `hard_kill_kills_immediate_startup_descendant_during_test_window`
launches fanin-mcp via `ConfigBuilder::new().args(["--spawn-immediate-descendant",
<marker>])` with NO upstream server. So **fanin-mcp itself**, at startup (as
early as possible, to exercise the Windows race window), must:
- spawn a long-lived descendant process **into the same containment primitive**
  used for upstreams (kill-on-close Job Object on Windows / process group +
  PDEATHSIG on Linux),
- write that descendant's PID (decimal, as `Child::id()`) to `<marker_path>`,
- keep serving normally afterward.
When fanin-mcp is hard-killed, that descendant MUST die within ~5s. This is a
legitimate test affordance, not gaming — it deterministically exercises the
race the real fix must close. Gate it so it is harmless in normal operation
(only active when the flag is passed).

### 2. CARRY-1 (Windows): close the spawn-then-assign race
Create the child **suspended**, assign it to the kill-on-close Job Object, THEN
resume — so there is no window where a descendant can escape `KILL_ON_JOB_CLOSE`.
- Preferred: drive process-wrap's `creation-flags` (`CREATE_SUSPENDED`) +
  `AssignProcessToJobObject` while suspended + `ResumeThread`.
- **Obstacle (OQ1):** rmcp `=1.8.0` `TokioChildProcess::builder(...).spawn()`
  spawns running and may not expose the main-thread handle needed for
  `ResumeThread`. If it cannot, the sanctioned fallback is a **thin custom child
  transport** owning the `CreateProcess` call, isolated in `process.rs` (D-009
  sanctions Job Objects; rmcp-general blesses a custom transport when
  `TokioChildProcess` can't be wrapped). Verify the real rmcp/process-wrap API
  at the pin via Context7 — do NOT guess. Document which path you took and why.

### 3. CARRY-2 (Linux): crash-safe parent-death
Add a `pre_exec` hook setting `prctl(PR_SET_PDEATHSIG, SIGKILL)` on the child
(`#[cfg(target_os = "linux")]`) so the kernel kills the child when fanin-mcp
dies — crash-safe on `kill -9`. Keep the existing process-group/session
teardown for the graceful path. (Needs a `libc` dependency for `prctl`/
`PR_SET_PDEATHSIG`, or process-wrap's hook surface — pick the smallest addition;
if you add `libc`, target-gate it to unix and note it for the Phase 6 deny/budget.)

### 4. macOS: documented limitation (NO code heroics)
macOS keeps graceful process-group teardown only. Do NOT add a supervisor
process, kqueue watcher, or launchd agent (rejected by user decision — honors
the "no daemon" non-goal). The test contract already excludes macOS from the
zero-orphan-on-hard-kill claim (`#[cfg(any(windows, linux))]`). The macOS
SIGKILL-orphan gap is documented in Phase 7 docs, not engineered around here.

## Verification reality (Windows-only dev host)

- The Windows immediate-descendant test + the existing
  `hard_kill_orphan_test_no_surviving_descendants` + the EOF teardown +
  stderr-capture tests run **locally** — they MUST pass here.
- The **Linux** PDEATHSIG path cannot run on this host; it is verified on CI
  (Phase 6). Write it correctly and `#[cfg]`-gated; do not fake a local Linux
  pass. Confirm it at least COMPILES for the linux target if feasible
  (`cargo check`), and clearly note in your result that Linux is CI-verified.

## Constraints

- Scope: Phase 2 only. Surface (don't fix) anything outside it.
- Tests read-only. Never `--no-verify`. stdout stays the MCP transport.
- Lock discipline intact (no registry lock across awaits). Child stderr capture
  + redaction must keep working (SC P2.5 — `stderr_capture_intact_after_process_wrapping`).
- rmcp stays `=1.8.0`. Any new dep (`libc`) is minimal, target-gated, and noted
  for Phase 6 `cargo deny` + the <10MB binary budget.
- End: `cargo fmt` clean, `cargo clippy --all-targets` zero warnings, and on
  THIS host `cargo test --test integration process_lifetime` green (Linux-gated
  test compiles but is inactive on Windows).

## Return

Per-file changes; **which CARRY-1 path you took (suspended-spawn via process-wrap
vs. custom transport) and the exact rmcp/process-wrap API evidence**; how the
immediate-descendant hook is wired; the Linux PDEATHSIG approach + that it is
CI-verified not local; any new dependency added; and any surfaced issue or
test-issue. Be explicit if you could NOT close the Windows race and why.
