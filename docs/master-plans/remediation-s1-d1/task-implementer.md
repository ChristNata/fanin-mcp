IMPLEMENT remediation-s1-d1 — make the S-1/D-1 contract green. BOTH phases.

You are the implementer. Write production code in `src/**` ONLY, to satisfy the
already-written test contract. Read, in order:
- `docs/master-plans/remediation-s1-d1/master.md` — the plan. Phase 1 (S-1) then
  Phase 2 (D-1), in that order; they share `src/registry.rs::connect`.
- `docs/master-plans/remediation-s1-d1/tests.md` — the contract + coverage map.
- `tests/integration/remediation_s1_d1.rs` — the 12 failing tests you must turn
  green (read to understand intent; NEVER edit).
- `rmcp-general` skill for the exact rmcp =1.8.0 client API; `rust-general`.

Current gate state (orchestrator-verified): `cargo test --all` → 117 passed / 12
failed / 4 ignored. The 12 failures are the new `remediation_s1_d1::*` tests.
Your done-condition: `cargo test --all` 100% green (0 failed), `cargo fmt --all
-- --check` clean, `cargo clippy --all-targets -- -D warnings` clean. No existing
test may regress.

## ABSOLUTE RULES

1. **Tests are a read-only contract.** Do NOT edit ANY file under `tests/**` —
   not the integration tests, not `tests/common/fixtures.rs`, not
   `tests/probe-server/main.rs`. If you become convinced a test is genuinely
   WRONG (not merely demanding), STOP, do not edit it, and write
   `docs/master-plans/remediation-s1-d1/test-issue-<slug>.md` describing which
   test, what is wrong, and why — then continue with the rest. The orchestrator
   routes test issues to the test-creator.
2. **NO TEST-GAMING. This is the whole point of S-1.** The timeout MUST be a real
   `tokio::time::timeout(effective, <future>)` wrapping the ACTUAL connect /
   serve-handshake / `list_all_tools` / refetch futures, generalizing to ANY hung
   upstream. FORBIDDEN: detecting test/probe mode, special-casing the probe
   binary or a server name, a sleep/race shortcut, returning a canned timeout
   without awaiting the real future, or threading a flag that only the test sets.
   A reviewer WILL check that a brand-new hung upstream not seen by any test would
   also time out and be reaped. Implement the general mechanism, not a test-pass.
3. **Scope is S-1 + D-1 only.** Touch only the `src/` files in the plan's Produces
   (`registry.rs`, `error.rs` only if needed, `config.rs`, `process.rs`). Do not
   fix unrelated review findings (O-*/D-2/H-*). Surface anything else you notice
   in your returned result; do not edit it.
4. Do NOT edit `src/credentials.rs` (managed-deny); you do not need to — the
   `${VAR}` resolver you reuse is `process::resolve_env_value`.

## Phase 1 — S-1 (do this first)

Per master.md Key Behaviors. The load-bearing points:
- Wrap the WHOLE cold-connect future (stdio spawn → `handler.serve(...)`
  initialize → initial `peer().list_all_tools()`) in the server's
  `effective_timeout` inside `get_or_connect`, while only the per-server init
  guard is held (NO entries map lock across the await — D-007/GOTCHA #16).
- **Containment on timeout (D-009):** the `ContainmentGuard` from
  `spawn_stdio_transport` must live as a local INSIDE the timed future until the
  `UpstreamEntry` is constructed, so that a timeout cancels/drops the future,
  drops the guard, and KILLS/REAPS the half-connected child tree. Verify the
  guard is not moved out or `mem::forget`-ed before the timeout boundary.
- On timeout: insert NO entry, let the init guard release on return, map expiry
  to `ToolError::UpstreamTimeout` (public code `upstream_timeout` — do NOT add a
  new code; an internal optional tool/operation label is fine if it keeps the
  public `code` string unchanged).
- `ensure_fresh`: wrap the dirty-refetch `list_all_tools` in the same timeout; on
  expiry re-set `dirty = true`, return structured `upstream_timeout`, and do NOT
  overwrite the cached inventory. Hold only the cloned Arc (no `tools` lock across
  await).

## Phase 2 — D-1 (after Phase 1 compiles & its tests pass)

- Add `pub cwd: Option<String>` to `ServerConfig` (`config.rs`), serde-optional,
  documented to match `docs/ARCHITECTURE.md:97`.
- Config validation rejects empty/whitespace-only `cwd` at load (config-shape
  error before serving), for the field when present.
- At connect time (stdio only), resolve `cwd` via
  `process::resolve_env_value(&*store, cred_choice, server, raw)` — the SAME path
  as env/headers. A `${VAR}` resolving to blank/whitespace is rejected BEFORE
  spawn with a structured tool-level error (not a hang/panic).
- Pass the resolved cwd into `spawn_stdio_transport`, which applies
  `Command::current_dir(resolved)`. Absent `cwd` → no `current_dir` call (inherit
  aggregator CWD — no regression).
- Streamable-HTTP: accept `cwd` in the schema but do NOT resolve or apply it.
- Non-existent dir: do not preflight; let the OS spawn error surface as
  `ToolError::UpstreamConnect` (public code `upstream_connect_failed`).

## Finish

Run `cargo fmt --all`, then confirm `cargo clippy --all-targets -- -D warnings`
and `cargo test --all` are clean/green locally before returning. (Your fmt sweep
touches only `src/`; the tests are already fmt-clean — do not let fmt rewrite a
test file. If fmt wants to change a test file, that is a signal you edited one —
revert it.)

Return as data for the orchestrator: which src files you changed and the core of
each change; how you implemented the timeout envelope and the containment-drop on
timeout (name the exact scope where the guard lives); confirmation the public
error `code` set is unchanged; the final gate numbers; and any out-of-scope issue
you spotted but did not touch.
