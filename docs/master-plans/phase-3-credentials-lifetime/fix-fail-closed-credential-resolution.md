# Fix: fail-closed credential resolution

## Defect

`src/registry.rs` still implemented the old partial-resolution model. A server
with one unresolvable configured `${VAR}` could spawn with the remaining env
values, and only a tool-name-shaped path returned `credential_resolution_failed`.
Generic calls to the same server could still reach the upstream.

## Root cause

The registry stored per-env-LHS credential failures in `bad_env` and checked
that map only for the special tool path. That made credential failure depend on
the requested tool shape instead of the server configuration.

## Fix applied

`Registry::get_or_connect` now resolves every configured env value before
spawning. If `resolve_env_value` returns `ToolError::CredentialResolution`, the
server is not cached or spawned, and the error propagates through the normal
D-005 structured tool-result path for any call targeting that server.

The per-key `bad_env` cache and the tool-name special-case were removed.

## Verified

- `cargo build` passed.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --all-targets -- -D warnings` passed.
- `cargo test --test integration` passed: 84 passed, 0 failed, 3 ignored.
- `grep -rn "echo_env" src/` returned no matches.

## Suggested-fix divergence

None. The implemented fix matches the routed Option A contract.

## Anything surfaced

None.
