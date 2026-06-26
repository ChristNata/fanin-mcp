# Review: phase-3-credentials-lifetime

Verdict: FAIL.

Found 3 blocker, 0 structural, 2 targeted, 1 trivial.

## Gate evidence

- `cargo test --test integration`: PASS — 84 passed, 0 failed, 3 ignored.
- `cargo test`: PASS — unit tests and integration suite passed; 3 ignored.
- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --all-targets -- -D warnings`: PASS.

The green suite did not catch the two converged blockers: the
credential-resolution test only bites the `echo_env` probe shape, and the Unix
hard-kill guarantee was not tested on this Windows host.

## Merged findings

- Severity: blocker
  Type: test-shaped production contract breach
  Lenses: alignment, adversarial, general
  Location: `src/registry.rs:122` and `src/registry.rs:166`
  Issue: Missing `${VAR}` credential resolution is stored as a side channel and
    only returned when the later tool call is exactly the probe tool `echo_env`
    with a matching `key`. Production tools spawn with the secret silently
    omitted instead of returning the structured `credential_resolution_failed`
    tool error required by D-010 and master SC 2/8.
  Suggested fix: Treat unresolved configured placeholders as a server-wide
    connect/call failure for every tool on that server, surfaced through the
    D-005 `CallToolResult { isError: true }` path. Remove the `echo_env` special
    case.
  Routing: issue-to-user. This is a blocker gate failure; route to a debugger
    only after the user accepts the fix direction.

- Severity: blocker
  Type: process-lifetime contract breach
  Lenses: alignment, adversarial, general
  Location: `src/process.rs:183`
  Issue: The Unix hard-kill path relies on `ProcessSession`/process-group
    behavior but has no parent-death mechanism. If `fanin-mcp` is killed with
    `SIGKILL`, Rust `Drop` does not run and no surviving code sends a group
    signal, so upstream descendants can survive. This violates D-009 and master
    SC 20/21. The Windows Job Object path is accepted by alignment/general and
    passed on this host; the failure is Unix containment.
  Suggested fix: Add a Unix hard-kill-safe mechanism that survives parent death
    and verify it on Linux/macOS CI, or re-plan the Unix hard-kill guarantee if
    the product contract changes.
  Routing: issue-to-user. This needs a design decision for Unix parent-death
    containment before debugger work.

- Severity: blocker
  Type: process-containment race
  Lenses: adversarial
  Location: `src/process.rs:193`
  Issue: The adversarial lens reported that Windows Job Object assignment occurs
    after `builder.spawn()`, leaving a window where a hostile or very fast
    upstream could spawn a detached descendant before the parent joins the job.
    Alignment and general did not raise this and treated the Windows path as
    fine, so this is an adversarial-only disagreement, not a converged finding.
  Suggested fix: If the project keeps the strict Windows zero-orphan guarantee,
    prove the post-spawn assignment cannot leak descendants or move containment
    earlier with a creation-time/suspended-process job assignment strategy.
  Routing: issue-to-user. The lenses disagree on whether this is a real shipping
    blocker; the orchestrator/user should decide whether to re-plan or dispatch
    a focused debugger investigation.

- Severity: targeted
  Type: CLI side-effect failure
  Lenses: alignment, adversarial, general
  Location: `src/main.rs:197` and `src/credentials.rs:146`
  Issue: `cred set` returns success when the selected store rejects the write;
    `EnvStore::set` is a no-op success. The CLI can tell the user a secret was
    stored when later resolution cannot retrieve it.
  Suggested fix: Return failure with a redacted, actionable diagnostic when the
    mutable store cannot persist the secret. If `env` is read-only fallback,
    reject `cred set/rm/list --credential-store env` instead of no-oping.
  Routing: debugger fix.

- Severity: targeted
  Type: dependency/platform configuration
  Lenses: general
  Location: `Cargo.toml:71`
  Issue: `keyring` enables `apple-native`, `windows-native`, and
    `sync-secret-service` globally instead of target-gating platform backends.
    This leaves Linux/macOS dependency behavior unverified and may pull platform
    backends outside their target.
  Suggested fix: Move keyring backend features into target-specific dependency
    sections, or add proof/CI that the global feature set builds cleanly on
    Linux and macOS.
  Routing: debugger fix.

- Severity: trivial
  Type: stale documentation
  Lenses: general
  Location: `src/main.rs:62`
  Issue: The `cred` subcommand doc comment still says "Credential management
    stub" after the implementation landed.
  Suggested fix: Update the comment to describe the implemented credential
    commands.
  Routing: debugger fix.

## Lens agreement and disagreement

- Strong agreement: all three lenses raised the missing-credential
  special-case as a blocker.
- Strong agreement: all three lenses raised Unix hard-kill containment as a
  blocker, and all noted the Windows-host test does not verify Unix.
- Strong agreement: all three lenses raised `cred set` false success as a
  targeted defect.
- General-only: target-gated `keyring` features and the stale `cred` comment.
- Disagreement: adversarial alone raised a Windows post-spawn Job Object race as
  a blocker. Alignment and general treated the Windows Job Object path as fine;
  the synthesis keeps the finding because a lens flagged it, and marks the
  disagreement explicitly.
