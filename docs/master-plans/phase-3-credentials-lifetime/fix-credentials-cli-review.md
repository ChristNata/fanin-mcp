# Fix: phase-3-credentials-lifetime credential CLI review

## Defect

- Blocker: `src/registry.rs` resolved missing `${VAR}` credentials through a
  side channel keyed to the probe `echo_env` tool instead of failing the server
  connection generally.
- Targeted: `src/main.rs` returned success when `cred set` could not persist the
  secret, and the read-only env backend could no-op writes/removals.
- Targeted: `Cargo.toml` enabled all `keyring` platform backends globally.
- Trivial: `src/main.rs` still described `cred` as a stub.

## Root cause

The implementation optimized around the probe fixture shape instead of the
server credential contract. The CLI also preserved an old phase contract that
treated rejected credential writes as non-fatal.

## Fix applied

- Removed the registry `bad_env` side channel and the `echo_env` branch.
- Resolve the full server env before spawning an upstream. Any unresolved
  placeholder now returns `ToolError::CredentialResolution` from connection and
  therefore reaches callers as `CallToolResult { isError: true }`.
- Made `cred set` fail when the selected mutable store rejects the write.
- Rejected `cred set` and `cred rm` with `--credential-store env` before any
  write path; messages name the backend and point to env fallback without
  printing secrets.
- Moved `keyring` backend features into target-specific dependency sections for
  Windows, macOS, and Linux. `rmcp` remains pinned to `=1.8.0`.
- Updated the stale `cred` doc comment.

## Verification

- `cargo build`: PASS.
- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --all-targets -- -D warnings`: PASS.
- `grep` over `src/*.rs` for `echo_env`, `bad_env`, and the stale stub comment:
  PASS, no matches.
- `cargo test --test integration`: FAIL — 83 passed, 1 failed, 3 ignored.

The failing test is
`cred_store::credential_resolution_order_env_fallback_then_structured_error`.
It asserts that a server with one resolved credential and one missing credential
still spawns and serves the resolved value. That contradicts the routed blocker
fix, which requires any unresolved configured placeholder to fail the server
connection so every `invoke_tool` targeting that server returns
`credential_resolution_failed`.

## Suggested-fix divergence

None. The registry fix follows the routed finding, but the existing integration
test contract is stale against that finding.

## Surfaced

- Targeted test contract issue: `tests/integration/cred_store.rs:763` still
  expects mixed good/bad env resolution on one server. It should be routed to the
  test creator, not edited by the debugger.
