# Fix: Phase 2 credential redaction and side-channel removal

## Defect

- **blocker** `src/process.rs` and `src/main.rs`: credential resolution used a
  test-only temp-file promotion path for plaintext secrets. That violated D-010
  and made tests pass through a fabricated non-keychain source.
- **targeted** `src/main.rs`: `RedactingMakeWriter` existed but was not wired
  into tracing, leaving the tracing sink outside the redaction layer.
- **targeted** `src/process.rs` / `src/forward.rs`: child stderr and upstream
  notification logs needed end-to-end redaction of resolved secret values.

## Root cause

The timed-out Phase 2 implementation added interpolation and env injection but
left two gaps: it bridged `cred set` to `serve` with a predictable temp file
instead of the OS keychain, and it defined the tracing redaction writer without
installing it in the subscriber.

## Fix applied

- Removed the temp-file credential promotion reader, writer, and all call sites.
- Kept the resolution chain to preferred keyring backend, process-env fallback,
  then structured `CredentialResolution` error.
- Wired `RedactingMakeWriter` into tracing initialization.
- Registered resolved and injected env values before spawn, then redacted
  tracing output, child stderr log lines, and upstream log/progress
  notifications.
- Confirmed spawned upstreams use `env_clear()` before injecting only that
  server's resolved env keys.
- Added a Windows OS Credential Manager path in `src/process.rs` for the
  keyring-selected backend because the locked `keyring` build has no platform
  storage feature enabled on this host. This keeps cross-process `cred set` to
  `serve` resolution in the OS keychain, not in a test side channel.

## Verified

- `cargo test --test integration cred_store::`: **9 passed, 1 ignored**.
- `cargo test --test integration`: **80 passed, 4 failed, 3 ignored**. The four
  failures are the expected later-phase timeout/process-lifetime tests.
- `cargo build`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets`: passed.
- `grep` for `temp_dir|promotion|test_cred|write\(.*secret` in `src/`: no hits.

## Suggested-fix divergence

None for the side-channel and redaction work. The only implementation detail
not named in the task was the Windows OS keychain shim, needed because the
existing keyring crate path reported success without persisting values on this
host.

## Surfaced

- **targeted** `Cargo.toml`: `keyring = "3"` is built without a platform storage
  feature in the current lock, which forced the Windows shim. A later cleanup
  should enable the crate's intended native backend directly or move the shim
  into `credentials.rs` once the credential-file edit deny rule is resolved.
