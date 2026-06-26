# Fix: review-targeted

## Defect

- `tests/probe-server/main.rs`: `needs_sampling` spawned a detached task that
  awaited `peer.send_request(...)` without a bound. If a client never answered
  the sampling request, the fixture could retain a pending request future for
  the lifetime of the probe process.
- `Cargo.toml`: `rmcp` enabled the `macros` feature even though Phase 0 uses
  manual tool definitions and no `#[tool]` macro.
- Production rustfmt drift existed in `src/*.rs` and the probe fixture.

## Root cause

- The probe fixture intentionally avoided blocking `tools/call`, but the
  detached request future still had no cancellation bound.
- The `rmcp/macros` feature stayed in the dependency list from the initial
  feature set comment, not from a concrete Phase 0 macro use.
- Source formatting had not been applied after implementation edits.

## Fix applied

- Wrapped the detached `peer.send_request(request)` in
  `tokio::time::timeout(Duration::from_secs(3), ...)` and logged a timeout with
  `tracing::warn!`. The request is still emitted by `send_request`; only the
  indefinite response wait is bounded.
- Removed `macros` from the `rmcp` feature list and let `Cargo.lock` drop the
  unused macro dependency chain.
- Ran `rustfmt --edition 2021 src/*.rs tests/probe-server/main.rs` only. No
  files under `tests/integration/` or `tests/common/` were edited.

## Verification

- `cargo build`: pass. This verifies `rmcp/macros` is not required by Phase 0.
- `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo nextest run`: pass, 14 passed and 2 skipped. The
  `probe_needs_sampling_sends_sampling_create_message_on_wire` test still
  passes.
- `rustfmt --edition 2021 --check src/*.rs tests/probe-server/main.rs`: pass.
- `cargo fmt --all -- --check`: still reports drift only in read-only test
  files under `tests/common/` and `tests/integration/`. Per routing, those are
  left untouched for `test-creator`.

## Suggested-fix divergence

None. The timeout and feature-removal suggestions held after verification.

## Surfaced

- trivial: `cargo fmt --all -- --check` remains red on test-owned files in
  `tests/common/` and `tests/integration/`. Route that formatting correction to
  `test-creator`; debugger scope forbids editing those files.
