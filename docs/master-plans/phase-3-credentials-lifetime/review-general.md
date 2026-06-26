# Review General: phase-3-credentials-lifetime

Found 2 blocker, 0 structural, 2 targeted, 1 trivial.

Lens verdict: FAIL

## Gate evidence

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --all-targets -- -D warnings`: PASS.
- `cargo test --test integration`: PASS — 84 passed, 3 ignored.
- `cargo test`: PASS — unit tests and integration suite passed; 3 ignored.

## Findings

- File: `src/registry.rs:122`
  Severity: blocker
  Pass:     general
  What:     Missing `${VAR}` credentials are recorded and ignored during spawn;
            only later `echo_env` calls are special-cased into an error.
  Why:      A real upstream with a missing configured secret starts without that
            env var and can fail later with an arbitrary upstream error, instead
            of the promised structured credential-resolution error. The
            `tool == "echo_env"` branch at `src/registry.rs:166` makes the
            behavior depend on the probe tool shape, not the production
            contract.
  Cite:     CLAUDE.md binding rule #5; rmcp-general §Errors stay in the
            conversation; rust-review §General pass / ordinary correctness.
  Fix:      Treat unresolved configured credentials as a call/connect failure
            for that server and return `ToolError::CredentialResolution` through
            `CallToolResult::error(...)`. Remove the `echo_env`-specific
            `bad_env` side channel.

- File: `src/process.rs:183`
  Severity: blocker
  Pass:     general
  What:     The Unix containment path uses `ProcessSession` only; a parent
            hard-kill does not kill a separate Unix session or process group.
  Why:      `setsid`/new-session isolation prevents terminal signal coupling,
            but it does not attach child lifetime to the fanin-mcp process.
            If fanin-mcp receives `SIGKILL`, Rust `Drop` does not run and no
            code sends a group kill, so the upstream tree can survive on Unix.
  Cite:     CLAUDE.md binding rule #6; rmcp-general §Process and transport;
            rust-review §General pass / concurrency and resource lifetime.
  Fix:      Add a Unix hard-kill-safe mechanism, not only a drop-time wrapper:
            use a supervisor/parent-death strategy that survives `SIGKILL`, or
            document that true hard-kill containment is not supportable and
            re-plan the Unix contract before shipping. Verify on Linux/macOS CI.

- File: `src/main.rs:197`
  Severity: targeted
  Pass:     general
  What:     `cred set` exits success after the selected store rejects the write.
  Why:      The command reports success while discarding the secret. A user on a
            headless or broken keyring host will believe the credential was
            stored, but later resolution can only succeed if an unrelated env
            var happens to exist.
  Cite:     rust-review §General pass / ordinary correctness; CLAUDE.md binding
            rule #5.
  Fix:      Return failure when the mutable selected store cannot persist the
            secret. Keep env as read fallback for resolution, but do not turn a
            failed write into success.

- File: `Cargo.toml:71`
  Severity: targeted
  Pass:     general
  What:     `keyring` enables `apple-native`, `windows-native`, and
            `sync-secret-service` globally instead of gating platform features
            per target.
  Why:      This may compile on the current Windows host, but it leaves
            Linux/macOS dependency behavior unverified and pulls platform
            backends outside their target. The review task explicitly calls out
            cfg-gated keyring features as a cross-platform soundness surface.
  Cite:     rust-review §Dependencies and supply chain; CLAUDE.md stack and
            cross-platform product promise.
  Fix:      Move keyring backend features into target-specific dependency
            sections or otherwise prove the global feature set builds cleanly on
            Linux and macOS. Add a CI note/gate for the remaining platform
            caveat.

- File: `src/main.rs:62`
  Severity: trivial
  Pass:     general
  What:     The `cred` subcommand doc comment still says "Credential management
            stub" after the implementation landed.
  Why:      It is stale API/help text and can mislead future maintainers, though
            it does not change runtime behavior.
  Cite:     rust-review §General pass / code health.
  Fix:      Update the comment to describe the implemented credential commands.

## Notes

- `rmcp` remains exactly pinned as `=1.8.0`.
- The Windows process-lifetime tests passed on this host, but Linux/macOS
  process containment still needs native CI verification.
