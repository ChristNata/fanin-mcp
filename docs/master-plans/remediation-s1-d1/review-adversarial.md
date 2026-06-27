# Adversarial Lens Review — remediation-s1-d1

Scope: only the remediation-s1-d1 change set (git diff HEAD~1 -- src/). Full suite green (134/0/4). Binding canon: D-004/D-005/D-007/D-009/D-012, GOTCHA #16/#30/#11/#14, ARCHITECTURE:97, rmcp=1.8.0.

## Test-gaming / Generality of Timeout

- No server-name, probe, or test-only special casing. `effective_timeout(server)` reads the real `ServerConfig::timeout_secs`; `timeout(effective, future)` is applied uniformly to any upstream.
- A brand-new hung upstream (never seen by any test) follows the identical `get_or_connect` → `connect` → `ensure_fresh` paths and receives the same `UpstreamTimeout` treatment.
- Evidence: registry.rs:150 (effective), 158 (connect), 321 (ensure_fresh); no `if server == "probe"` or feature-flag guards.

**Finding**: none. Timeout mechanism is general.

## Containment on Timeout (D-009)

- `connect(...)` owns a local `ContainmentGuard` (Inert default, replaced by real guard only on successful stdio spawn at line 436).
- The entire `connect(...)` future is wrapped by the outer `timeout(...)` in `get_or_connect`.
- On expiry the future is dropped before any `UpstreamEntry` is constructed or inserted; the local guard drops and kills the process tree.
- The post-serve `list_all_tools` inside `connect` (458-468) is also inside the timed envelope.
- No path moves the guard out of the future, stores it elsewhere, or leaks it past cancellation.
- `serve` task ownership is inside the dropped future; no dangling task left running.

**Finding**: none. Containment invariant preserved on all timeout paths.

## Race / Lock Discipline (D-007, GOTCHA #16)

- Per-server `init_guard` serializes concurrent `get_or_connect` for the same name; only the winner performs the timed connect.
- Registry `entries` write-lock is acquired only after the timed connect succeeds (173-176); never held across any await.
- `ensure_fresh` holds only the cloned `Arc<Entry>` + per-entry `tools` lock briefly after the await (344).
- Timeout path returns error without inserting; subsequent caller retries cleanly. No double-insert or cache poisoning possible.
- No new lock-across-await introduced.

**Finding**: none. Lock discipline unchanged and correct.

## cwd Abuse / TOCTOU / Secret Leak

- Resolution uses the exact same `resolve_env_value` path as env/headers (135); result is passed only to `Command::current_dir` (process.rs:249) and never logged.
- Post-resolution empty check (136-140) rejects whitespace (including tabs/newlines via `trim()`); surfaces as structured `UpstreamConnect` (tool surface).
- Load-time validation (config.rs:210) catches literal empty `cwd`; resolved-blank case is the connect-time path (plan-intended).
- TOCTOU between resolution and spawn exists but is accepted (non-existent dir already fails at spawn as `upstream_connect_failed` per plan).
- No attacker-controlled path can bypass the trim rejection or inject a secret into logs/errors.

**Finding**: none. cwd handling matches D-019/GOTCHA #30 and D-005.

## Panics / DoS Surface

- New paths use `match`/`?` only; no `unwrap`/`expect`/`index`/`as` truncation on runtime data.
- `unreachable!` for transport kind was pre-existing.
- Empty-cwd and timeout paths are explicit error returns.

**Finding**: none. No new panic vectors.

## Error Shape (D-005)

- `StartupError::EmptyCwd` is a config-load / CLI error; never reaches JSON-RPC tool surface.
- All timeout cases reuse the existing `ToolError::UpstreamTimeout` with public code `upstream_timeout` (no new code introduced).
- `resolved cwd empty` surfaces as `UpstreamConnect` (existing code), matching the plan's intent for blank-${VAR} cases.

**Finding**: none. Public error surface unchanged.

## Lens Verdict

PASS
