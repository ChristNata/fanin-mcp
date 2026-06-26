# Fix: keyring-backend

## Defect

`Cargo.toml` declared `keyring = "3"` without a platform backend feature. In
keyring v3 that means no OS credential store is compiled in. The workaround was
raw Windows Credential Manager FFI in `src/process.rs`, with `CredWriteW`,
`CredReadW`, `CredentialW`, and `native_keyring_*` helpers duplicating the
keyring crate.

## Root cause

The dependency lacked the keyring v3 platform feature set. `KeyringStore` in
`src/credentials.rs` already used `keyring::Entry`; the missing backend feature
made the correct abstraction inert and pushed persistence into unsafe,
platform-specific code in the wrong module.

## Fix applied

- Enabled keyring v3 native desktop features in `Cargo.toml`:
  `apple-native`, `windows-native`, and `sync-secret-service`.
- Updated `Cargo.lock` for the new backend dependencies. `zeroize` was locked to
  `1.8.1` by `cargo update -p zeroize --precise 1.8.1` after the initial build
  selected `1.9.0` and failed against the available registry state.
- Deleted the raw Windows credential FFI and `native_keyring_*` path from
  `src/process.rs`.
- Routed `cred set`, `cred list`, and `cred rm` through
  `credentials::build_store(...)` in `src/main.rs`.
- Kept redaction and `${VAR}` resolution in `src/process.rs`; resolution is now
  `store.get(...)` then process env fallback then structured
  `ToolError::CredentialResolution`.

`src/credentials.rs` was not edited.

## Verification

- `cargo build`: passed on Windows with `windows-native` compiled.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets`: passed with zero warnings.
- `grep -rn "CredWriteW\|CredReadW\|CredentialW\|native_keyring\|temp_dir\|promotion" src/`: no matches.
- `cargo test --test integration cred_store::`: 9 passed, 1 ignored.
- `cargo test --test integration`: 80 passed, 4 failed, 3 ignored. The four red
  tests are the known later-phase timeout/process-lifetime tests:
  `process_lifetime::hard_kill_orphan_test_no_surviving_descendants`,
  `process_lifetime::stdin_eof_teardown_terminates_full_upstream_tree`,
  `timeout_cancellation::slow_timed_out_call_does_not_block_concurrent_sibling`,
  and `timeout_cancellation::timeout_secs_wraps_upstream_call_and_returns_structured_error`.

## Platform note

The keyring v3 feature names were verified from the keyring 3.6.3 Cargo feature
list. `windows-native` builds on this host. `apple-native` and
`sync-secret-service` are target-gated by their platform dependencies and need
runtime/CI verification on macOS and Linux.

## Surfaced findings

- targeted: `src/main.rs` still uses `eprintln!` for `cred list` names. This is
  outside the credential-backend defect and does not affect `serve` stdout, but
  it contradicts the nearby comment saying list output goes through tracing.
