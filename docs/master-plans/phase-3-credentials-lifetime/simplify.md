# Simplify: phase-3-credentials-lifetime

**Baseline:** 8cc6c85 — `cargo test --test integration` = 84 passed / 0 failed / 3 ignored (green).

All changes are behavior-preserving. Tests are a read-only contract; no test files or probe fixture were touched.

## Process
- Snapshot taken before any edit (commit 8cc6c85).
- After every edit: `cargo test --test integration` re-run; must stay 84/0/3 or immediate revert.
- Final gates: `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` clean.
- Only files in the Phase 3 Produces sets were considered.
- High-risk / verified-correct surfaces left verbatim (see "Files unchanged").

## Files simplified
- `src/main.rs`: hoisted `credential_store` out of the parsed `Cli` once; removed two needless clones on the hot path before `run_cred`/`run_serve`.
- `src/error.rs`: removed the always-`true` `recoverable: bool` parameter from the private `structured_error` helper. All call sites now implicitly produce `recoverable: true` (matches D-005 contract and all current tests).
- `src/forward.rs`: extracted a small private `append_redacted` helper to deduplicate the "redact then append" pattern shared by `on_logging_message` and `on_progress`.
- `src/registry.rs`: (1) elided the fully-qualified `std::collections::HashMap` type in `get_or_connect`; (2) removed the `ServerConfig` clone by borrowing the entry from the `Arc<TomlConfig>` (the reference is valid for the resolution + connect work; lock discipline unchanged); (3) satisfied clippy `needless_borrow` on the subsequent `connect` call.
- `src/server.rs`: unified the two near-duplicate schema builders (`optional_string_object_schema` / `required_string_object_schema`) behind a single `string_object_schema(props, required)` helper. Callers unchanged.

## Files reverted
- `src/namespace.rs`: attempted a one-line clone simplification (`ns.servers.clone()` instead of `ns.servers.iter().cloned().collect()`). Compile error (type mismatch: `Vec<String>` vs the `HashSet` path). Reverted immediately via `git checkout -- src/namespace.rs`. No test impact; recovery point = 8cc6c85.

## Files left unchanged
- `src/process.rs`: contains the D-009 Windows Job Object unsafe block, `KILL_ON_JOB_CLOSE` logic, handle lifetime, `register_secret` call sites, `${VAR}` resolution chain, `env_clear()` + least-privilege injection, and redaction wiring. All left verbatim per explicit instructions.
- `src/credentials.rs`: edit-denied by managed rule (see `issue-credentials-edit-deny-rule.md`).
- `src/config.rs`: no low-risk, behavior-preserving cleanups identified that justified touching a load-bearing Phase 2 surface.
- Registry map lock discipline (D-007), per-call `timeout` wrapper, and downstream cancellation `select!` paths left exactly as shipped.
- `ContainmentGuard` / `WindowsJobGuard` retention and Drop semantics untouched.

## Issues spotted for routing
- `src/main.rs:222` — `cred list` uses `eprintln!` (to stderr) rather than `tracing`. This was the deliberate choice so integration tests can observe names without stdout corruption (GOTCHA #1). Trivial cleanup candidate only; values remain names-only either way.
- All structured tool errors now hard-wire `recoverable: true`. This matches the current public contract and every test, but if a future variant must be non-recoverable the helper (or the enum) will need to carry the flag again. Not a defect today; surfaced for awareness.

## Final gate
- `cargo test --test integration`: 84 passed / 0 failed / 3 ignored.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --all-targets -- -D warnings`: clean.

A perfectly good outcome is "few or zero risky changes on verified security code." The pass removed duplication and indirection only where the test guard and manual review confirmed behavior preservation.
