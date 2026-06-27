---
Feature: remediation-s1-d1
Scope: flat
Stack: rust
Tier: thorough
Status: draft
Created: 2026-06-27
Target: src/registry.rs
Dependencies: docs/master-plans/full-codebase-review/review-SYNTHESIS.md
---

# Master Plan: Remediation S-1 And D-1

## What

Ship a focused remediation cycle for the two full-codebase review findings in
scope: bound every upstream connect, discovery, and dirty-refetch await by the
server's effective timeout, and implement the documented per-server `cwd` field
for stdio upstreams without changing unrelated behavior.

## Why

The full-codebase synthesis in
`docs/master-plans/full-codebase-review/review-SYNTHESIS.md` names S-1 as the
production blocker: `src/registry.rs` currently wraps only `call_tool` in
`timeout_secs` at lines 188-190, while cold connect, initial `list_all_tools`,
and `ensure_fresh` dirty-refetch await without a timeout at lines 132-133,
378/387, 399-402, and 281. That violates D-012 in `docs/DECISIONS.md`, which
says every upstream call must fail informatively first and free resources.

The same synthesis names D-1 as documented-but-unimplemented drift. `docs/PRD.md`
line 41 lists `cwd`; `docs/ARCHITECTURE.md` lines 65-70 and 95-97 specify
`${VAR}` interpolation, default inheritance of the aggregator CWD, and HTTP
ignore semantics; `docs/GOTCHA.md` #30 and D-019 in `docs/DECISIONS.md` explain
why directory-scoped upstreams such as Morph need it. The current
`src/config.rs` `ServerConfig` has no `cwd` field, and `src/process.rs`
`spawn_stdio_transport` never calls `current_dir`.

Corrected drift from the inputs:

- The prompt says "if `config.rs` has a `ConfigBuilder`/`to_toml` writer." It
  does not. The actual builders and `to_toml` helpers live in
  `tests/common/fixtures.rs` lines 76-88 and 166-215, with additional builders
  later in the same file. The test-creator may update test fixtures; production
  code work must not invent a writer in `src/config.rs`.
- The review says `grep cwd|current_dir src/` has no hits. The source still has
  no `cwd` field or `current_dir` call in `src/config.rs`, `src/registry.rs`, or
  `src/process.rs`; this plan therefore treats D-1 as active drift, not already
  fixed.
- `Cargo.toml` lines 31-38 confirm `rmcp = "=1.8.0"` with the streamable-HTTP
  client features already present. This remediation must not bump the rmcp pin.

## Dependencies

This plan depends on the current post-Phase-5 tree and the synthesized review.
It is sequenced, not parallelizable. S-1 runs first because it changes the
connect await envelope in `src/registry.rs`; D-1 then plugs resolved `cwd` into
that same connect/spawn path.

Verified source anchors:

- `src/registry.rs`: `get_or_connect` holds only the per-server init guard
  across cold connect; the entries map lock is dropped before await. `connect`
  stores a local `ContainmentGuard`, awaits `handler.serve(...)`, then awaits
  initial `list_all_tools`. `ensure_fresh` re-marks dirty on service error.
- `src/config.rs`: `ServerConfig` deserializes transport, command, endpoint,
  args, env, headers, log_file, and timeout_secs, but no `cwd`; validation
  already has transport-specific branches for stdio and streamable HTTP.
- `src/process.rs`: `resolve_env_value` is the existing `${VAR}` resolver for
  env and headers; `spawn_stdio_transport` owns the `tokio::process::Command`
  and is the single correct place to set `current_dir` for stdio children.
- `src/error.rs`: `ToolError::UpstreamTimeout` already serializes public code
  `upstream_timeout`; `ToolError::UpstreamConnect` serializes
  `upstream_connect_failed`. D-005 makes these code strings public API.

## Scope

In scope:

- Wrap cold connect initialize, initial discovery, and dirty-refetch discovery
  in each server's effective timeout.
- Preserve D-009 process containment on timeout by ensuring the stdio
  `ContainmentGuard` stays local across the connect await and is dropped on the
  timeout/cancel path before any entry is inserted.
- Preserve the D-007 lock model: never hold the registry entries map lock across
  connect, discovery, refetch, or tool-call awaits.
- Preserve structured tool errors under D-005. Reuse public code
  `upstream_timeout` for timeout expiry; do not add a new error code.
- Add `cwd: Option<String>` to `ServerConfig`, serde-optional and documented.
- Resolve configured `cwd` through `process::resolve_env_value` at connect time
  using the same credential/env fallback as env and HTTP headers.
- Apply resolved `cwd` only to stdio children via `Command::current_dir` in
  `spawn_stdio_transport`; `None` preserves current inherited-CWD behavior.
- Reject empty or whitespace-only configured/resolved `cwd` before spawn.
- Let a non-existent directory fail at `Command` spawn and surface as the
  existing `ToolError::UpstreamConnect` / public code `upstream_connect_failed`.
- Ensure Streamable-HTTP ignores `cwd` per `docs/ARCHITECTURE.md` line 97.
- Add or adjust tests needed to prove these behaviors. Tests are authored only
  by test-creator and are read-only for implementer.

Out of scope:

- No remediation of O-1, O-2, O-3, D-2, any H-* hygiene item, or glm dispatch
  reliability from the full-codebase synthesis.
- No change to the meta-tool surface, namespace ACL model, tool-name parsing,
  read-only enforcement, resources, prompts, or capability-mirrored forwarding.
- No change to D-004 byte-faithful result passthrough.
- No change to D-005's public structured-error shape except using the existing
  `upstream_timeout` code on more timeout paths.
- No new public timeout code such as `upstream_connect_timeout` in this cycle.
- No rmcp version bump, Cargo feature churn, or new runtime dependency.
- No HTTP listener, daemon, process supervisor, database, web framework, Node
  runtime dependency, plugin loader, OAuth flow, hot reload, or prewarming.
- No production-code edits to test fixtures. Test fixture changes belong to
  test-creator's test artifact, not implementer code.

## Phases

### Phase 1 — Bounded Connect And Discovery Awaits

Scope: Make the S-1 timeout guarantee true for cold connect, initialize,
initial discovery, and dirty-refetch, while preserving containment, cache
integrity, and lock discipline.

Produces:

- `src/registry.rs`
- `src/error.rs` only if internal `ToolError::UpstreamTimeout` shape needs an
  optional tool/operation label while keeping public code `upstream_timeout`
- `tests/probe-server/main.rs`
- `tests/integration/timeout_cancellation.rs`
- `tests/integration/list_changed.rs`
- `tests/integration/process_lifetime.rs`
- `tests/integration/main.rs`

Key Behaviors:

- `get_or_connect` obtains the server config and effective timeout without any
  entries map lock, then wraps the whole cold-connect future in that timeout
  while only the per-server init guard is held.
- The wrapped connect future covers stdio spawn, rmcp `handler.serve(...)`
  initialize handshake, and initial `peer().list_all_tools()` discovery.
- On connect timeout, no `UpstreamEntry` is inserted into the entries map. The
  init guard is released when the function returns, so a later call retries
  cleanly instead of queueing forever behind a poisoned cold start.
- For stdio connect timeouts, the real `ContainmentGuard` returned by
  `spawn_stdio_transport` lives as a local inside the timed connect future until
  `UpstreamEntry` construction. If `handler.serve(...)` or initial discovery is
  cancelled by timeout, dropping that future drops the guard and kills/reaps the
  half-connected upstream process tree under D-009.
- Timeout expiry maps to `ToolError::UpstreamTimeout` and public code
  `upstream_timeout`. This deliberately avoids a new public error code. Connect
  and discovery timeouts may use `tool: null` or an internal operation label,
  but the code string remains unchanged and recoverable.
- `ensure_fresh` wraps dirty-refetch `list_all_tools` in the same effective
  timeout. On timeout it restores `dirty = true`, returns a structured
  `upstream_timeout`, and does not overwrite the cached inventory.
- The timeout envelope does not introduce an entries map lock across await.
  `ensure_fresh` still holds only the cloned `Arc<UpstreamEntry>` and no
  `tools` lock while awaiting.
- Logging remains stderr/file-only through `tracing`; no stdout diagnostics are
  introduced.

Depends On: existing `Registry::effective_timeout`, `tokio::time::timeout`,
`ToolError::UpstreamTimeout`, `UpstreamEntry::_containment`, and
`process::ContainmentGuard` drop semantics.

Skills Needed: `rust-general`, `rust-test`, `rmcp-general`.

Phase Success Criteria:

1. A stdio upstream that hangs during initialize makes the first `list_tools` or
   `invoke_tool` return `CallToolResult { isError: true }` with code
   `upstream_timeout` within approximately the configured `timeout_secs` bound.
2. A stdio upstream that hangs during initial `list_all_tools` returns the same
   structured timeout within the configured bound.
3. A Streamable-HTTP upstream that stalls during connect/initialize returns the
   same structured timeout within the configured bound.
4. A `notifications/tools/list_changed` dirty-refetch that hangs times out within
   the configured bound, re-marks the entry dirty, and leaves the prior cache
   untouched.
5. A connect timeout inserts no entry into the registry cache, releases the init
   guard, and allows a later call to attempt a fresh connect.
6. A connect timeout for a stdio upstream leaves zero surviving child processes
   from the timed-out spawn, verified while the timeout path is under test rather
   than by a later post-run process count only.
7. Concurrent calls to a different already-connected server are not blocked by a
   hung cold connect on another server.

### Phase 2 — Per-Server Stdio Working Directory

Scope: Implement the documented `cwd` config field for stdio upstreams and keep
HTTP behavior unchanged.

Produces:

- `src/config.rs`
- `src/registry.rs`
- `src/process.rs`
- `src/error.rs` only if a new startup validation variant is needed for empty
  or whitespace-only `cwd`
- `tests/common/fixtures.rs`
- `tests/integration/config.rs`
- `tests/integration/registry.rs` or a new cwd-focused integration test module
- `tests/integration/http_upstream.rs`
- `tests/integration/main.rs`

Key Behaviors:

- `ServerConfig` gains `cwd: Option<String>` with serde default/optional
  behavior and documentation matching `docs/ARCHITECTURE.md` line 97.
- Config validation rejects `cwd = ""` and whitespace-only values when present.
  This is a config-shape error, not a late spawn failure.
- At connect time, stdio servers resolve `cwd` through
  `process::resolve_env_value(&*store, cred_choice, server, raw)` exactly like
  env and headers. Missing `${VAR}` credentials surface as the existing
  structured credential-resolution error.
- After resolution, an empty or whitespace-only `cwd` is rejected before spawn.
  This catches `${VAR}` resolving to blank.
- The resolved cwd is passed to `spawn_stdio_transport`; that function applies
  it to the `Command` with `current_dir`. If `cwd` is absent, no `current_dir`
  call is made and the child inherits the aggregator CWD, preserving current
  behavior.
- A non-existent resolved directory is not preflighted. The OS spawn error is
  allowed to surface through the existing `ToolError::UpstreamConnect` path with
  public code `upstream_connect_failed`.
- Streamable-HTTP upstreams ignore `cwd`: validation accepts the field for
  HTTP for schema consistency, but `registry.rs` does not resolve it and the HTTP
  transport builder does not consume it.
- Test builders and `to_toml` helpers in `tests/common/fixtures.rs` render `cwd`
  where needed so test round-trips do not drop the field. Production code does
  not gain a TOML writer.

Depends On: Phase 1's connect envelope in `src/registry.rs`, existing
`process::resolve_env_value`, and `spawn_stdio_transport` owning the child
`Command` construction.

Skills Needed: `rust-general`, `rust-test`, `rmcp-general`.

Phase Success Criteria:

1. A stdio server configured with `cwd` observes that directory as its actual
   child working directory.
2. A stdio server configured with `cwd = "${PROJECT_ROOT}"` resolves the value
   through the same credential/env resolver used for env and headers.
3. A stdio server with no `cwd` preserves today's behavior by inheriting the
   aggregator process CWD.
4. A configured empty or whitespace-only `cwd` fails config validation before
   MCP serving starts.
5. A `${VAR}` cwd that resolves to an empty or whitespace-only string fails
   before spawn and returns a structured tool-level error when reached lazily.
6. A non-existent stdio `cwd` fails through the existing upstream-connect error
   path with public code `upstream_connect_failed`.
7. A Streamable-HTTP server with `cwd` configured connects normally and does not
   attempt to resolve or apply that field.

## Success Criteria

1. Phase 1: A hung-during-initialize upstream makes the first `list_tools` or
   `invoke_tool` return `CallToolResult { isError: true }` with code
   `upstream_timeout` within approximately the configured timeout bound.
2. Phase 1: A hung initial `list_all_tools` discovery returns structured code
   `upstream_timeout` within approximately the configured timeout bound.
3. Phase 1: A hung Streamable-HTTP connect/initialize path returns structured
   code `upstream_timeout` within approximately the configured timeout bound.
4. Phase 1: A hung `list_changed` dirty-refetch returns structured code
   `upstream_timeout` within approximately the configured timeout bound.
5. Phase 1: A dirty-refetch timeout leaves the dirty flag set so the next
   inventory read retries instead of treating the stale cache as fresh.
6. Phase 1: A cold-connect timeout leaves no cached `UpstreamEntry` and a later
   call attempts a fresh spawn/connect.
7. Phase 1: A cold-connect timeout releases the per-server init guard so a later
   same-server call is not queued behind the abandoned attempt.
8. Phase 1: A timed-out stdio connect leaves zero surviving child processes,
   verified during the timeout scenario rather than only after test teardown.
9. Phase 1: No registry entries map lock is held across connect, discovery,
   dirty-refetch, or tool-call awaits.
10. Phase 2: `ServerConfig` accepts optional `cwd` and rejects empty or
    whitespace-only configured values at config load.
11. Phase 2: A stdio child configured with literal `cwd` reports that directory
    as its actual working directory.
12. Phase 2: A stdio child configured with `${VAR}` in `cwd` reports the resolved
    directory as its actual working directory using the existing resolver path.
13. Phase 2: An unset `cwd` preserves current inherited-CWD behavior for stdio
    upstreams.
14. Phase 2: A `${VAR}` cwd resolving to blank fails before spawn with a
    structured tool-level error, not a hang.
15. Phase 2: A non-existent stdio `cwd` fails through public code
    `upstream_connect_failed`.
16. Phase 2: A Streamable-HTTP upstream with `cwd` configured ignores it and
    connects without resolving or applying that field.
17. Both phases: `cargo test --all`, `cargo clippy --all-targets -- -D warnings`,
    and `cargo fmt --check` pass with 100% pass rate and no ignored new failures.

## Constraints / Invariants

- Tests are a read-only contract once written by test-creator. Implementer must
  not edit tests, assertions, fixtures, or formatting; if a test is wrong,
  surface a test issue for orchestration.
- 100% test pass rate. No thresholds and no "good enough" gates.
- Scope is S-1 and D-1 only. Do not touch O-1/O-2/O-3, D-2, H-* hygiene items,
  or glm reliability notes.
- Preserve D-004 byte-faithful upstream result passthrough.
- Preserve D-005 structured errors as `CallToolResult { isError: true }`; never
  convert these failures into JSON-RPC errors.
- Preserve D-007 lock discipline: clone/drop registry map locks before awaits.
- Preserve D-009 containment: stdio process-tree guards must live until success
  entry construction or drop on timeout/error paths.
- Preserve D-012's per-server timeout semantics with `timeout_secs` defaulting
  to 60 seconds via `effective_timeout`.
- Preserve D-019 / GOTCHA #30 by making `cwd` the stdio child working directory,
  not an argument rewrite or ambient process-global directory change.
- No stdout diagnostics after MCP stdio serving begins.
- No new runtime dependency and no rmcp pin bump.

## Open Questions

None blocking. Design decisions resolved for this plan:

- Timeout code: reuse existing public code `upstream_timeout` for connect,
  initial discovery, dirty-refetch, and tool-call timeouts. Recommended default:
  keep this choice. Adding `upstream_connect_timeout` would be a public API
  extension under D-005 and is not necessary for model recovery.
- Non-existent `cwd`: let spawn fail and surface as `upstream_connect_failed`.
  Recommended default: keep this choice because it avoids racy preflight checks
  and uses the existing connect-failure envelope.
- HTTP `cwd`: accept but ignore for Streamable-HTTP. Recommended default: keep
  this choice because it matches `docs/ARCHITECTURE.md` line 97 and avoids
  transport-specific schema surprises.
