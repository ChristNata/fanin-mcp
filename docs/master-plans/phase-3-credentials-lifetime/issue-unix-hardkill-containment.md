# Issue: Unix hard-kill containment incomplete (deferred to Phase 5)

**Severity:** blocker (all 3 review lenses) — **carried to Phase 5 by user decision.**
**Surfaced during:** Phase 3 review, `src/process.rs` (Unix path).

## What

The Unix containment uses `process-wrap`'s `ProcessSession` (new session/group via
`setsid`). That isolates the child but does NOT make the child tree die when `fanin-mcp` is
**force-killed** (`kill -9`): on SIGKILL, Rust `Drop` does not run, `process-wrap`'s
kill-on-drop cleanup does not run, and nothing else signals the group. So a Unix upstream
descendant can survive an aggregator hard-kill — violating D-009 / master SC 20-21 ("zero
orphans on hard-kill, all OSes").

This was not caught by the gate because the hard-kill orphan test ran only on this Windows
host. The Windows path (kernel `KILL_ON_JOB_CLOSE`, which IS crash-safe) is verified;
the Unix path is not, and is incomplete by design for the hard-kill case.

## Correct fix (for Phase 5)

- **Linux:** add a `pre_exec` hook setting `prctl(PR_SET_PDEATHSIG, SIGKILL)` on the child
  (cfg(target_os = "linux")), so the kernel kills the child when the parent dies — crash-
  safe. Pair with process-group teardown for the graceful path. Verify on Linux CI.
- **macOS / other Unix:** no direct `PDEATHSIG` equivalent. Options to decide in Phase 5:
  a small supervisor/monitor process that reaps the group on parent death, a kqueue
  `NOTE_EXIT` watcher, or an explicitly documented limitation. This needs a design call.
- Run the hard-kill orphan test on Linux and macOS CI runners (Phase 5's 3-OS matrix).

## Why deferred

User decision (Phase 3 review): defer to Phase 5. Verifying Unix hard-kill requires
Linux/macOS runtime, which this Windows-only dev host cannot provide; the plan already
scoped the cross-OS hard-kill CI verification to Phase 5 (MVP.md Phase 5 / verification
checklist). Adding unverifiable Unix code now would be false confidence — Phase 5 is where
it is implemented AND tested on real Unix runners.
