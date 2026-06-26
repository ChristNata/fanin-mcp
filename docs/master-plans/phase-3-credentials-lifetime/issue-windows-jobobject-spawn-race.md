# Issue: Windows Job Object spawn-then-assign race (deferred to Phase 5)

**Severity:** blocker (adversarial lens) — **carried to Phase 5 by user decision.**
**Surfaced during:** Phase 3 review (adversarial lens), `src/process.rs`.

## What

The Windows containment assigns the upstream child to the kill-on-close Job Object
**after** `TokioChildProcess::builder(...).spawn()` returns — i.e. after the child is
already created and running (`ContainmentGuard::for_transport` opens the process and calls
`AssignProcessToJobObject` post-spawn). There is a small window between the child starting
and being placed in the job. A child that spawns a detached descendant *within that window*
creates the grandchild before the parent is in the job, so the grandchild can escape
`KILL_ON_JOB_CLOSE` containment (D-009).

The current hard-kill orphan test does not catch this because the probe spawns its
grandchild well after initialization, outside the race window. Windows happy-path
containment (grandchild spawned after init) is verified working (test green, 3/3 loop).

## Correct fix (for Phase 5)

Install containment before any child code runs:
- Create the child **suspended** (`CREATE_SUSPENDED` — process-wrap's `creation-flags`
  feature is enabled), `AssignProcessToJobObject` while suspended, then `ResumeThread`.
  The obstacle: rmcp `=1.8.0`'s `TokioChildProcess` spawns running and may not expose the
  main thread handle needed for `ResumeThread`. If so, a thin custom child transport (own
  the `CreateProcess` call) is the fallback — D-009 sanctions Windows Job Objects.
- Add a regression test that spawns the descendant immediately at child startup to exercise
  the race window.

## Why deferred

User decision (Phase 3 review): defer to Phase 5, the cross-platform CI + process-hardening
phase. The practical risk is low (real upstreams rarely fork a daemon in the first
milliseconds), the proper fix may require working around an rmcp transport limitation, and
Phase 5 is where process-tree behavior is hardened and CI-verified across OSes.
