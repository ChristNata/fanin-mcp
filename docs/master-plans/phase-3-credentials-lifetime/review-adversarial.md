# Review adversarial: phase-3-credentials-lifetime

Found 3 blocker, 1 targeted, 0 structural, 0 trivial.

Lens verdict: FAIL.

## Test run

- `cargo test --test integration process_lifetime::hard_kill_orphan_test_no_surviving_descendants -- --nocapture`: PASS (1 passed).
- `cargo test --test integration`: PASS (84 passed, 0 failed, 3 ignored).

## Findings

- File: `src/registry.rs:122`
  Severity: blocker
  Pass: adversarial
  What: Missing `${VAR}` credentials are recorded and the upstream is spawned
        anyway; the structured error is returned only for the test-shaped
        `echo_env` call at `src/registry.rs:166`.
  Why: A config with `env.TOKEN = "${MISSING}"` followed by
       `invoke_tool` on `server__echo_ok` succeeds with the credential silently
       omitted. The plan requires missing credentials to produce a structured
       tool-level error, not a best-effort spawn. The `tool == "echo_env"`
       branch handles the probe test input instead of the production failure
       mode.
  Cite: D-010; master SC 2 / P2.SC2; fakery checklist — branches handling only
        test inputs.
  Fix: Treat unresolved configured secret placeholders as a server spawn/call
       failure for every tool on that server, or associate the failure with the
       server connection state and return `CredentialResolution` before any
       upstream call, not only when `echo_env` asks for the same LHS.

- File: `src/process.rs:193`
  Severity: blocker
  Pass: adversarial
  What: Windows Job Object assignment happens after the child process is already
        spawned and running.
  Why: `builder.spawn()` returns at line 193, then `ContainmentGuard::for_transport`
       opens the process and calls `AssignProcessToJobObject` later. A hostile or
       fast upstream can spawn a detached descendant during that window; that
       descendant is created before the parent is placed in the job and can escape
       D-009 containment. The current hard-kill test spawns the grandchild after
       initialization, so it misses this spawn-time race.
  Cite: D-009; GOTCHA #11/#14; rmcp-general §Process and transport.
  Fix: Install containment before user code can run. Use a wrapper that assigns
       the process to the Job Object at creation time, spawn suspended then assign
       before resume, or use a custom child transport that creates the job and
       process atomically enough to close the race.

- File: `src/process.rs:183`
  Severity: blocker
  Pass: adversarial
  What: The Unix hard-kill path relies on a process session without any mechanism
        that runs when `fanin-mcp` is killed forcefully.
  Why: `ProcessSession` can create a fresh session/group, but a process group is
       only killed if some surviving code sends the group a signal. On `kill -9`
       of `fanin-mcp`, Rust `Drop` and any process-wrap cleanup do not run, and
       this code retains no parent-death signal or external supervisor. A Unix
       upstream descendant can therefore survive the aggregator hard-kill even
       though SC 20/21 require zero orphans.
  Cite: D-009; GOTCHA #14; master SC 20/21.
  Fix: Add a Unix crash-safe mechanism, not just a clean-teardown mechanism: for
       Linux, use a pre-exec `prctl(PR_SET_PDEATHSIG, SIGTERM/SIGKILL)` plus
       process-group cleanup; for macOS and other Unix targets, add a supported
       supervisor/contract or re-plan the claimed hard-kill guarantee.

- File: `src/main.rs:197`
  Severity: targeted
  Pass: adversarial
  What: `cred set` reports success when the selected credential store rejects the
        write.
  Why: The error branch logs a warning and returns `ExitCode::SUCCESS`, so a
       locked/unavailable keyring or `--credential-store env` path can pass the
       CLI test without storing anything. That is a stub-shaped side effect:
       the command says "credential stored" by exit status, but no later lookup
       can resolve the value.
  Cite: D-010; master SC 3; fakery checklist — side effects that never happen.
  Fix: Return failure for a failed mutable-store write with an actionable message
       naming the backend and env fallback. If `env` is intentionally read-only,
       reject `cred set --credential-store env` rather than succeeding as a
       no-op.
