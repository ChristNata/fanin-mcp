---
Feature: phase-1-config-upstream
Scope: flat
Stack: rust
Tier: THOROUGH
Status: draft
Created: 2026-06-26
Target: src/server.rs, src/config.rs, src/registry.rs, src/forward.rs, src/process.rs
Dependencies: docs/master-plans/phase-0-*/master.md; docs/MVP.md Phase 1
---

# Master Plan: Phase 1 Config + Single Upstream + Reverse Traffic

## What

Ship the first real proxy path: load a TOML config for one stdio upstream,
connect lazily on first targeted meta-tool use, cache the upstream tool
inventory, expose that inventory through `list_tools` / `get_tool_schema`, and
forward `invoke_tool` calls byte-faithfully. The upstream client side also
answers reverse traffic from day one: empty roots, clean rejection for
sampling/elicitation, logging notifications to the log file, and progress
notifications tolerated.

## Why

Phase 0 proved the downstream stdio server surface. Phase 1 is where fanin-mcp
becomes a proxy instead of a static MCP server. The binding anchor is
`docs/MVP.md` Phase 1, which lists seven items for config, registry, end-to-end
invoke, live discovery, clean reverse-traffic handling, stderr capture, and the
probe `needs_sampling` test.

The plan is also constrained by accepted decisions and gotchas read for this
plan: `docs/DECISIONS.md` D-004, D-005, D-007, D-008, and D-010;
`docs/GOTCHA.md` #1, #2, #3, #4, and #16; `docs/ARCHITECTURE.md` module
contracts for `config.rs`, `server.rs`, `registry.rs`, `forward.rs`, and
`process.rs`; and `docs/AGG-MCP.md`, whose code snippets are explicitly
pseudocode until checked against the pinned rmcp version.

Verified current tree state:

| Claim | Verified file | Result |
|---|---|---|
| rmcp is exactly pinned to 1.8.0 with server/client/stdio-child features | `Cargo.toml` | Matches: `=1.8.0` with `server`, `client`, `transport-io`, `transport-child-process`. |
| Phase 0 downstream server has three static meta-tools and call stubs | `src/server.rs` | Matches: static `list_tools`, `get_tool_schema`, `invoke_tool`; `call_tool` returns `CallToolResult::error`. |
| Config is only a CLI flag carrier | `src/config.rs` | Matches: `CliConfig { namespace, config_path }`, no TOML model yet. |
| Registry, forward, process, namespace, credentials are stubs | `src/registry.rs`, `src/forward.rs`, `src/process.rs`, `src/namespace.rs`, `src/credentials.rs` | Matches, with comments preserving the relevant gotchas. |
| Probe fixture exists with `needs_sampling` | `tests/probe-server/main.rs` | Matches: five probe tools; `needs_sampling` emits an upstream-originated sampling request. |
| Existing harness is raw JSON-RPC over stdio | `tests/common/mod.rs`, `tests/integration/aggregator.rs` | Matches; Phase 0 tests explicitly mark proxying and reverse traffic out of scope. |

Context7 was used against `/websites/rs_rmcp_rmcp` for rmcp 1.8.0 signatures.
It confirms the current `ServerHandler` shape used by `src/server.rs`,
`ClientHandler` methods for `create_message`, `create_elicitation`,
`list_roots`, `on_logging_message`, and `on_progress`, and `serve_client` /
`RunningService<RoleClient, S>` availability under the `client` feature. The
implementer must still compile against the pin; docs are a guide, not a lockfile.

No blocking drift found. One scope note: `docs/ARCHITECTURE.md` describes HTTP
upstreams and credential/header interpolation, but `docs/MVP.md` Phase 1 and the
task limit this phase to a single stdio upstream. HTTP and credentials stay out.

## Dependencies

- Phase 0 is prerequisite and appears landed in the current tree: the binary,
  static meta-tools, probe fixture, and integration harness are present.
- This plan is sequenced before Phase 2 multi-upstream namespace switching.
  Phase 1 may implement enough namespace checking to reject unknown
  `--namespace` and gate the active default namespace, but it must not expand
  into Phase 2's multi-upstream proof or namespace-switching matrix.
- No other plan can run in parallel against `src/server.rs`, `src/config.rs`,
  `src/registry.rs`, `src/forward.rs`, or `src/process.rs` without conflict.

## Scope

### In

- `config.rs`: TOML config data model, config file parsing, default config path
  resolution as needed for `--config`, server-name validation, namespace
  validation, and fail-fast startup errors for unknown `--namespace`.
- `registry.rs`: lazy one-upstream connection storage using
  `Arc<RunningService<...>>`, per-server init guard, cached session tool
  inventory, and lock discipline that never holds a registry map lock across an
  upstream await.
- `forward.rs`: upstream `ClientHandler` for MVP clean rejection, empty roots,
  log notification routing, progress tolerance, and byte-faithful call/result
  forwarding helpers.
- `process.rs`: Phase-1 stdio child spawn path and stderr capture to log file
  with `[server]`-prefixed lines. Full process-tree lifetime is deferred.
- `server.rs`: replace the not-implemented meta-tool call stub with real
  dispatch for `list_tools`, `get_tool_schema`, and `invoke_tool` while keeping
  downstream `tools/list` static and lazy.
- `main.rs`: wire config loading/validation and aggregator construction without
  printing to stdout.
- `namespace.rs`: minimal active-namespace representation/checking needed for
  Phase 1. Only the configured namespace is used; Phase 2 owns broader ACL
  switching proof.
- `error.rs`: enough structured tool-level errors for Phase 1 failures to be
  returned as `CallToolResult { isError: true }`, not JSON-RPC errors.
- Integration tests are written only by `test-creator`; implementation stages
  treat tests as read-only.

### Out

- Multi-upstream concurrency proof and namespace ACL switching matrix. Phase 2
  owns N upstreams, slow-call-does-not-block-sibling proof, and namespace
  switching behavior.
- Credentials, keyring, hidden prompt behavior, env fallback resolution,
  per-server credential scoping enforcement, timeouts, cancellation forwarding,
  and process-tree Job Objects / Unix process groups. Phase 3 owns them.
- HTTP or remote upstreams, static auth headers, OAuth, and Streamable HTTP
  transport wiring.
- Error-code hardening, final public error enum, upstream string sanitization,
  and tool-list-changed cache invalidation. Phase 4 owns them unless a minimal
  internal error is needed to satisfy Phase 1 behavior.
- CI matrix, `cargo audit`, `cargo deny`, release token benchmark, binary-size
  and memory gates. Phase 5 owns them.
- Changing the three static downstream meta-tool names/descriptions or their
  conservative annotations. Phase 0 tests pin them.
- Connecting to upstreams from downstream `tools/list`. Discovery through the
  `list_tools` meta-tool may connect; rmcp `tools/list` must remain static.
- Re-exporting upstream tools as first-class downstream tools. The public
  surface remains exactly the three meta-tools.
- Secrets in config, argv, or logs. Phase 1 should not implement secret
  resolution; it must also not add any code path that prints env maps or
  command configs containing potential secrets.

## Phases

### Phase 1 — Config model and startup validation

**Scope:** Build the Phase-1 TOML model and make `serve` load it before the MCP
stdio server starts.

**Produces:** `src/config.rs`, `src/main.rs`, minimal supporting changes in
`src/error.rs` if startup errors need typed display.

**Key Behaviors:**

- Parse the path from `--config` into a TOML config model for stdio upstreams
  and namespaces.
- Validate server names against `[a-z0-9-]+` and reject any containing `__`.
- Reject unknown `--namespace` before `serve(stdio())` starts.
- Preserve a default namespace when `--namespace` is omitted.
- Keep diagnostics on stderr/logging only; no stdout writes after stdio serving
  starts.
- Do not resolve credentials, inspect keyrings, or implement HTTP headers.

**Depends On:** Phase 0 CLI and `CliConfig` flag carrier.

**Skills Needed:** `rust-general`, `rmcp-general` for stdio transport
constraints, `tool-use`.

**Phase Success Criteria:**

1. A valid Phase-1 TOML config with one stdio server and a default namespace
   starts the aggregator successfully.
2. A server name outside `[a-z0-9-]+` fails startup before serving.
3. A server name containing `__` fails startup before serving.
4. An unknown `--namespace` fails startup before serving.
5. No config failure is emitted to stdout.

### Phase 2 — Upstream client handler and stderr/log plumbing

**Scope:** Create the client-side handler used for every upstream connection and
the Phase-1 child-process spawn/log path.

**Produces:** `src/forward.rs`, `src/process.rs`, supporting `src/config.rs`
fields for command/args/log file if needed.

**Key Behaviors:**

- Implement the rmcp 1.8.0 `ClientHandler` methods used by the reverse path:
  `create_message`, `create_elicitation`, `list_roots`, `on_logging_message`,
  and `on_progress`.
- Declare no sampling or elicitation capability to upstreams through the client
  info used at connection time.
- Return an empty roots list.
- Immediately reject any sampling or elicitation request with a structured
  error, never by ignoring the request.
- Route upstream log notifications and child stderr to the configured log sink
  with `[server]` context.
- Tolerate progress notifications without forwarding and without failing the
  active tool call.
- Spawn only stdio children for Phase 1. Do not implement Job Object / process
  group kill guarantees yet.

**Depends On:** Phase 1 config model for server command/args and log settings.

**Skills Needed:** `rust-general`, `rmcp-general`, Context7-verified rmcp 1.8.0
signature checking.

**Phase Success Criteria:**

1. The upstream client advertises no sampling/elicitation capabilities.
2. `roots/list` from an upstream receives an empty list response.
3. A sampling request from an upstream receives a bounded rejection response,
   not a hang.
4. An elicitation request from an upstream receives a bounded rejection
   response, not a hang.
5. Upstream logging notifications and child stderr lines appear in the log sink
   with the originating server name.
6. No child stderr is inherited into fanin-mcp stdout.

### Phase 3 — Lazy registry and inventory cache

**Scope:** Connect one configured stdio upstream lazily and cache its tools/list
inventory for the session.

**Produces:** `src/registry.rs`, `src/forward.rs`, `src/process.rs`, minimal
call sites in `src/server.rs` / `src/main.rs` to hold the registry.

**Key Behaviors:**

- Store each live upstream as `Arc<RunningService<...>>`; do not try to clone
  `RunningService` directly.
- `get_or_connect(server)` uses a brief map lock for lookup/insert only,
  clones the `Arc`, drops the map lock, then returns the handle.
- Use a per-server async init guard so concurrent first calls to the same
  server spawn exactly once.
- Re-check the map after acquiring the init guard.
- Fetch and cache the upstream inventory after connecting, using the
  pagination-safe rmcp helper when available for the pin.
- Keep downstream rmcp `tools/list` static and disconnected.

**Depends On:** Phase 1 config; Phase 2 upstream handler/spawn path.

**Skills Needed:** `rust-general`, `rmcp-general`, Context7/lockfile signature
verification.

**Phase Success Criteria:**

1. Starting fanin-mcp and calling downstream `tools/list` opens zero upstream
   processes.
2. The first meta-tool call that needs a named server spawns exactly one
   upstream process.
3. Two concurrent first calls to the same server produce exactly one upstream
   spawn.
4. The registry map lock is not held across an upstream `call_tool` or
   `tools/list` await; this is visible in code structure and covered by the
   test-creator's concurrency/guard assertions where possible.
5. The upstream tool inventory is cached and reused for the session.

### Phase 4 — Live `list_tools` and `get_tool_schema` meta-tools

**Scope:** Replace the Phase 0 not-implemented behavior for discovery
meta-tools with live inventory reads from the selected upstream cache.

**Produces:** `src/server.rs`, `src/registry.rs`, `src/namespace.rs`,
`src/error.rs`.

**Key Behaviors:**

- `call_tool` dispatches only the three meta-tool names; unknown meta-tool names
  remain protocol/tool-level errors according to the existing contract chosen by
  the code/tests.
- `list_tools` meta-tool optionally filters by server and returns rows for the
  active namespace from the live cached inventory.
- `get_tool_schema` parses `server__tool` on the first `__`, checks the active
  namespace, then returns the upstream tool input schema from the cache.
- Failures are returned as tool-level `CallToolResult { isError: true }` with
  structured JSON content.
- Do not connect to upstreams from downstream rmcp `tools/list`; only the
  `list_tools` meta-tool may trigger or use the registry.

**Depends On:** Phase 3 lazy registry and inventory cache.

**Skills Needed:** `rust-general`, `rmcp-general`.

**Phase Success Criteria:**

1. Calling the `list_tools` meta-tool returns the probe server's tool rows for
   the active namespace.
2. Calling `list_tools` with a specific configured server returns only that
   server's rows.
3. Calling `get_tool_schema` for `probe__echo_ok` returns the schema advertised
   by the probe server.
4. Calling `get_tool_schema` for an unknown server/tool returns
   `CallToolResult { isError: true }`, not a JSON-RPC error.
5. Downstream rmcp `tools/list` still returns exactly the three static
   meta-tools.

### Phase 5 — End-to-end `invoke_tool` forwarding

**Scope:** Replace the Phase 0 not-implemented behavior for `invoke_tool` with
real forwarding to the cached/lazy upstream connection.

**Produces:** `src/server.rs`, `src/forward.rs`, `src/registry.rs`,
`src/namespace.rs`, `src/error.rs`.

**Key Behaviors:**

- Parse tool names on the first `__`; server names are unambiguous because
  config validation rejects `__`.
- Check the active namespace before connecting/calling.
- Forward raw JSON arguments unchanged to the upstream tool call. Do not parse,
  validate, normalize, or stringify them proxy-side.
- Return the upstream `CallToolResult` byte-faithfully across all content block
  types. Never `to_string()` a content array.
- Convert spawn, connect, namespace, tool lookup, and upstream call failures
  into `CallToolResult { isError: true }` structured JSON content.
- Use JSON-RPC errors only for protocol-level malformed meta-tool requests where
  rmcp requires them; tool-level failures stay in the conversation.

**Depends On:** Phase 3 registry; Phase 4 namespace/cache helpers.

**Skills Needed:** `rust-general`, `rmcp-general`.

**Phase Success Criteria:**

1. `invoke_tool` against `probe__echo_ok` returns the probe success result.
2. The exact raw arguments sent to `probe__echo_ok` are visible in the echoed
   result without proxy-side transformation.
3. `invoke_tool` against a probe error tool returns `isError: true` content from
   the upstream, not a JSON-RPC error.
4. Tool names containing additional `__` after the server delimiter are treated
   as part of the upstream tool name.
5. A denied or unknown namespace/server/tool failure returns structured
   `isError: true` content.
6. Non-text content blocks returned by an upstream fixture are preserved as
   structured content blocks, not stringified.

### Phase 6 — Phase-1 integration gate and cleanup

**Scope:** Close the Phase-1 test contract and remove Phase-0-only stubbing
where it conflicts with real proxying.

**Produces:** Updates only in implementation files needed to satisfy the tests;
no test edits by implementer/simplifier/debugger.

**Key Behaviors:**

- Exercise the full path with the in-repo probe server: config load, lazy spawn,
  discovery, schema lookup, invoke, reverse traffic, and stderr capture.
- Preserve Phase 0 guarantees: exactly three static downstream meta-tools,
  conservative `invoke_tool` annotations, fast initialize, and no stdout
  diagnostics.
- Ensure `needs_sampling` receives a clean rejection through the upstream
  client handler and the proxy call does not hang.
- Keep all out-of-scope Phase 2/3/4/5 work out of the implementation.

**Depends On:** Phases 1–5.

**Skills Needed:** `rust-general`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. The full Phase-1 integration suite passes with 100% pass rate.
2. Existing Phase 0 tests still pass without weakening their assertions.
3. Probe `needs_sampling` through `invoke_tool` receives a clean rejection path
   and the call completes within the test deadline.
4. The configured probe server's stderr is captured in the log sink with a
   `[probe]` or configured-server-name prefix.
5. No Phase 2/3/4/5 functionality is accidentally introduced beyond what this
   plan explicitly marks in scope.

## Success Criteria

1. A valid Phase-1 TOML config with one stdio probe upstream and a default
   namespace starts fanin-mcp successfully.
2. Invalid server names fail startup: uppercase/underscore/other invalid
   characters are rejected, and names containing `__` are rejected.
3. An unknown `--namespace` fails startup before stdio serving begins.
4. Downstream rmcp `tools/list` remains static: exactly `list_tools`,
   `get_tool_schema`, and `invoke_tool`, with Phase 0 descriptions and
   annotations unchanged.
5. Startup/initialize opens zero upstream processes and stays within the Phase 0
   startup-laziness budget.
6. Calling the `list_tools` meta-tool against the configured probe upstream
   returns the probe tool rows from the live cached inventory.
7. Calling `get_tool_schema` for a probe `server__tool` returns that upstream
   tool's input schema.
8. `invoke_tool` parses on the first `__`, forwards raw arguments unchanged,
   and successfully returns `probe__echo_ok` output.
9. Upstream success and error results pass through as `CallToolResult` values;
   tool-level failures are `isError: true`, not JSON-RPC errors.
10. Content arrays are never stringified; non-text content blocks remain
    structured content blocks in the returned result.
11. Lazy connection is observable: the probe process is not spawned until the
    first meta-tool call that needs it.
12. Concurrent first calls to the same configured server spawn exactly one
    upstream process.
13. No registry map lock is held across an upstream `call_tool` await.
14. The upstream client declares no sampling/elicitation capabilities.
15. Upstream `roots/list` receives an empty list response.
16. Upstream sampling/elicitation requests receive immediate structured
    rejection responses and do not hang the upstream.
17. Upstream log notifications and child stderr are written to the log sink with
    `[server]`-prefixed context.
18. Child stderr does not corrupt stdout or get inherited into the MCP transport.
19. All diagnostics after `serve(stdio())` begins avoid stdout.
20. All required tests pass at 100%; failures are surfaced, not thresholded.

## Constraints / Invariants

- stdout is the MCP transport. No `println!`, `print!`, `dbg!`, inherited child
  stderr, or other stdout diagnostics once `serve(stdio())` runs.
- Downstream `tools/list` is static and must not connect to any upstream.
- Store upstream connections behind `Arc<RunningService<...>>`; `RunningService`
  itself is not cloneable.
- Never hold a registry map lock across an upstream await. Clone the `Arc`, drop
  the lock, then call the upstream.
- Protect cold first-connect with a per-server init guard and re-check after
  acquiring it.
- Reverse traffic is answered from Phase 1. Sampling/elicitation are rejected;
  roots/list is empty; progress/log notifications are tolerated/recorded.
- `invoke_tool` arguments pass through as raw JSON. No proxy-side schema
  validation or transformation.
- Results pass byte-faithfully. Never stringify a content array.
- Tool-level failures return `CallToolResult { isError: true }`; JSON-RPC
  errors are reserved for protocol-level failures.
- Tests are read-only except for `test-creator`. Implementer, simplifier,
  debugger, and reviewer must not edit test files.
- AGG-MCP snippets are pseudocode. rmcp 1.8.0 signatures must be verified
  against Context7, docs.rs/source, and the compiler.
- Phase 1 is single stdio upstream only. No HTTP transport, credentials,
  timeouts, cancellation, or process-tree hard-kill guarantees.

## Open Questions

(none)
