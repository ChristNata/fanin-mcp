---
Feature: phase-2-multi-namespace
Scope: flat
Stack: rust
Tier: THOROUGH
Status: draft
Created: 2026-06-26
Target: src/namespace.rs, src/config.rs, src/server.rs, src/registry.rs
Dependencies: docs/master-plans/phase-1-config-upstream/master.md; docs/MVP.md Phase 2
---

# Master Plan: Phase 2 Multi-Upstream + Namespace ACLs

## What

Ship the Phase 2 proof and hardening layer for fanin-mcp's already-landed
multi-upstream and namespace scaffolding: verify that one slow upstream call
does not serialize a sibling upstream, complete name-level namespace ACL
coverage if approved, document the read-only namespace pattern, and add the
2-3-upstream / namespace-switching test contract.

## Why

The binding scope anchor is `docs/MVP.md` Phase 2: N lazy upstreams; prove a
slow `slow_tool` call does not block a different upstream; namespace ACLs with
`is_server_allowed()` / `is_tool_allowed()` wired through `list_tools` and
`invoke_tool`; `default` namespace behavior; `namespace_denied`; documentation
for read-only namespaces; and tests covering 2-3 upstreams, lazy spawn, and
namespace switching.

The plan is grounded in `docs/DECISIONS.md` D-006 and D-007: namespaces are the
primary permission layer, server + tool allow-lists are intended, no
parameter-level firewall belongs in the proxy, and registry locks must be held
only for map access. It is also constrained by `docs/GOTCHA.md` #9, #10, #16,
#17, and #31: annotations are not a permission layer, OpenCode ignores them,
a lock across upstream `call_tool` freezes the whole session, racing first
calls must spawn once, and a read-only namespace is only as safe as its
name-level tool exposure. `docs/ARCHITECTURE.md` specifies the relevant module
contracts for `namespace.rs`, `registry.rs`, and `server.rs`, including the
intended namespace shape with `tools.<server> = [...]` filters.

Verified current tree state:

| Surface | Verified file | Done | Phase 2 still adds |
|---|---|---|---|
| Active namespace shell | `src/namespace.rs` | `ActiveNamespace` exists with `name`, `is_server_allowed`, `is_tool_allowed`, and deterministic `allowed_servers`; omitted `--namespace` resolves to `default` through `DEFAULT_NAMESPACE`. | Tool-level ACL storage and checks if the schema decision is approved; current `is_tool_allowed` delegates to server-level only. |
| Meta-tool namespace wiring | `src/server.rs` | `list_tools`, `get_tool_schema`, and `invoke_tool` all check the active namespace and return `ToolError::NamespaceDenied` as a tool result. | Verify server visibility matrix across namespaces; ensure `list_tools` hides tool-denied rows once tool filters exist. |
| N-upstream registry | `src/registry.rs` | Configured servers live in a `HashMap`; `get_or_connect` is lazy; per-server init guards exist; the registry clones an `Arc<UpstreamEntry>` before upstream awaits; `call_tool` awaits on the service outside the map lock. | Add the cross-upstream non-serialization proof. No code change is expected for lock discipline unless tests expose a missed lock or harness issue. |
| Structured namespace error | `src/error.rs` | `ToolError::NamespaceDenied` renders JSON with `code: "namespace_denied"`, `server`, optional `tool`, `message`, and `recoverable`, inside `CallToolResult::error`. | Test the public error shape on denied server/tool calls and document the pattern. |
| Config namespace schema | `src/config.rs` | `[namespaces.<name>] servers = [...]` exists; default namespace startup validation exists. | Current schema intentionally ignores `tools.<server>` per Phase 1 comments. This is the only real implementation question for Phase 2. |
| Probe fixture | `tests/probe-server/main.rs` | One probe binary exposes eight tools, including configurable-delay `slow_tool`, `echo_ok`, `always_error`, reverse-traffic tools, and `echo_image`. | Reuse the same probe binary under distinct server names for multi-upstream tests; no second fixture identity is needed. |
| Predecessor scope boundary | `docs/master-plans/phase-1-config-upstream/master.md` | Phase 1 explicitly left multi-upstream proof and namespace-switching matrix out while allowing minimal namespace checking to land early. | Phase 2 owns those omitted proofs; it must not re-plan Phase 1 proxy basics. |

Corrected drift: the task says Phase 1 already built namespace scaffolding ahead
of schedule, which is true for server-level ACL checks. It is not true for the
full documented namespace shape in `docs/ARCHITECTURE.md` and `docs/DECISIONS.md`
D-006: the current `NamespaceConfig` has only `servers`, and the current
`is_tool_allowed` ignores the tool name. Phase 2 must either add the documented
name-level filter or get an explicit decision to defer it. That decision is an
Open Question below because it changes the current config schema.

Probe fixture decision: the existing `tests/probe-server/main.rs` binary is
sufficient for multi-upstream tests. Register it two or three times under
distinct server names such as `alpha`, `beta`, and `gamma`; the configured
server name is what the aggregator routes on. A second binary would add fixture
maintenance without proving a different behavior.

## Dependencies

- Phase 1 is prerequisite and appears landed in the current tree: config load,
  lazy registry, live discovery, invoke forwarding, reverse traffic, and
  namespace-denied scaffolding are present.
- This plan is sequenced after Phase 1 and before Phase 3 credentials/timeouts /
  process-tree lifetime and Phase 4 sanitization/error hardening.
- Test creation should wait for the Open Question on tool-level namespace schema
  if the orchestrator treats that as a human decision point. The proposed
  default is to implement the documented `tools.<server> = [...]` filter now.
- No two implementation phases below write the same file concurrently. Phase 1
  is test-only planning/coverage; Phase 2 writes namespace/config/server code;
  Phase 3 writes docs; Phase 4 is the final gate/cleanup and depends on all
  earlier phases.

## Scope

### In

- Multi-upstream integration coverage with two or three configured upstreams
  using the existing probe binary under different server names.
- Observable proof that `alpha__slow_tool` with a configurable delay does not
  block a concurrent `beta__echo_ok` or `beta` discovery call.
- Lazy-spawn proof that an untargeted second upstream remains untouched until a
  call targets it.
- Concurrent first-call proof for one server: racing calls to the same cold
  upstream spawn/connect exactly once, using the strongest observable the test
  harness can support.
- Namespace matrix proof: a server visible in one namespace is hidden/denied in
  another; `default` is selected when the flag is omitted.
- `namespace_denied` shape verification for denied server/tool paths.
- Name-level ACL completion if approved: per-namespace allowed-tool lists in the
  documented `tools.<server> = [...]` shape, wired through `is_tool_allowed`,
  `list_tools`, `get_tool_schema`, and `invoke_tool`.
- Documentation of the read-only namespace pattern in `SECURITY.md`, because
  that file already holds the user-facing namespace safety guidance.

### Out

- Phase 1 proxy basics: TOML load, one-upstream forwarding, static downstream
  `tools/list`, reverse-traffic rejection, byte-faithful result passthrough, and
  stderr capture are not reimplemented.
- Parameter-level ACLs, SQL parsing, argument inspection, or any proxy-side
  firewall. ROADMAP says name-level filtering only; D-006 says argument-level
  safety belongs upstream.
- Per-server `readonly = true` enforcement based on upstream annotations. D-006
  defers that sibling control to v1.1.
- Credentials, keyring/env fallback, `${VAR}` interpolation, auth headers,
  `timeout_secs`, cancellation forwarding, and process-tree Job Object / process
  group lifetime. Phase 3 owns them.
- Phase 4 name/description sanitization, length-capping, final public error enum
  hardening, upstream crash isolation, and `notifications/tools/list_changed`
  cache invalidation.
- HTTP upstream transport, OAuth, install/warm/auth subcommands, resource/prompt
  proxying, cache persistence, CI matrix, audit/deny, token benchmarks, binary
  size, and memory profiling.
- Changing the three public meta-tool names, static descriptions, or conservative
  `invoke_tool` annotations.
- Adding a second probe fixture binary solely to simulate a different upstream.

## Phases

### Phase 1 — Multi-upstream proof contract

**Scope:** Add the Phase 2 test contract for N upstreams, lazy spawn, same-server
first-call guard, and the headline non-serializing cross-upstream proof. This is
expected to be test coverage against existing registry code, not a production
code change.

**Produces:** Test files only, selected by `test-creator` from the existing
integration layout: likely `tests/integration/registry.rs`,
`tests/integration/discovery.rs`, `tests/integration/invoke.rs`, and
`tests/common/fixtures.rs`. No implementation files.

**Key Behaviors:**

- Configure the same `probe-server` binary under at least two distinct server
  names, with an optional third for wider inventory/switching coverage.
- Prove downstream rmcp `tools/list` still opens zero upstreams even when N
  servers are configured.
- Prove targeting `alpha` leaves `beta` untouched until a call targets `beta`.
- Prove racing first calls to the same cold server result in one initialized
  upstream from the observable available to the harness: log sentinel, spawn
  marker, or strict consistent-success proxy if process counting is not stable.
- Prove `alpha__slow_tool` with a delay does not prevent a concurrent
  `beta__echo_ok` from completing inside a deadline shorter than the slow
  delay. This is the D-007 / GOTCHA #16 objective proof.
- Keep the proof cross-upstream. A same-upstream concurrency check is useful but
  not the Phase 2 headline criterion.

**Depends On:** `tests/probe-server/main.rs`, `tests/common/fixtures.rs`,
`tests/common/mod.rs`, current `src/registry.rs` lock discipline, current
`src/server.rs` invoke/list dispatch.

**Skills Needed:** `rust-test`, `rust-general` for async test shape,
`rmcp-general` for wire-level MCP behavior, `tool-use`.

**Phase Success Criteria:**

1. A config with two or three probe-backed upstreams starts fanin-mcp and
   downstream rmcp `tools/list` still returns only the three meta-tools without
   contacting any upstream.
2. Calling a meta-tool against `alpha` leaves `beta` unspawned/uncontacted until
   a later call targets `beta`.
3. Two concurrent first calls targeting the same cold server complete through a
   single initialized upstream according to the harness's strongest observable.
4. While `alpha__slow_tool` is awaiting a configured delay, a concurrent call to
   `beta__echo_ok` completes successfully within a deadline shorter than the
   slow delay.
5. The proof uses the existing probe binary registered under distinct names; no
   second fixture identity is introduced.

### Phase 2 — Namespace ACL completeness

**Scope:** Complete namespace ACL behavior beyond server-level filtering if the
Open Question is approved: parse the documented per-server tool allow-list,
store it in `ActiveNamespace`, and make `is_tool_allowed` enforce name-level
tool visibility for discovery, schema, and invocation.

**Produces:** `src/config.rs`, `src/namespace.rs`, `src/server.rs`; if needed,
minimal supporting updates to `src/error.rs` only to preserve the existing
structured `namespace_denied` shape. Tests are produced by `test-creator`, not
by implementer.

**Key Behaviors:**

- Preserve `servers = [...]` as the server allow-list and keep omitted
  `--namespace` selecting `default`.
- Add only name-level tool filters, not argument filters. The intended config
  shape is the architecture's `tools.<server> = ["tool", ...]` under a
  namespace table, if approved.
- Treat absent `tools.<server>` for an allowed server as all tools on that
  server visible. Treat a present list as exact tool-name allow-list.
- `list_tools` must omit rows denied by `is_tool_allowed`, not list then fail at
  invocation time.
- `get_tool_schema` and `invoke_tool` must return structured
  `namespace_denied` for a tool denied by name.
- A server denied by the namespace remains denied before any upstream connection
  is attempted.
- Validate or reject tool filters that reference unknown servers if that is
  necessary to keep startup errors fail-fast and clear; do not validate tool
  names at startup because tools are only known after lazy discovery.

**Depends On:** `src/config.rs` current namespace schema and validation;
`src/namespace.rs` current `ActiveNamespace`; `src/server.rs` current checks in
`handle_list_tools`, `handle_get_tool_schema`, and `handle_invoke_tool`;
`docs/ARCHITECTURE.md` namespace contract; Open Question #1.

**Skills Needed:** `rust-general`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. Omitting `--namespace` selects `default`, and `default` exposes the servers
   declared in `[namespaces.default]`.
2. A server visible in namespace `open` appears in `list_tools` and is callable;
   the same server denied in namespace `restricted` is hidden from `list_tools`
   and returns `namespace_denied` from `get_tool_schema` / `invoke_tool`.
3. If tool filters are approved, an allowed server with only `echo_ok` listed
   exposes `echo_ok` in `list_tools`, hides `dangerous_noop`, returns schema for
   `echo_ok`, and returns `namespace_denied` for `dangerous_noop` invocation.
4. Namespace checks happen before lazy connection for denied servers, so a denied
   server is not spawned merely to reject it.
5. No parameter-level ACL, argument parsing, or destructive/read-only annotation
   enforcement is added.

### Phase 3 — Read-only namespace documentation

**Scope:** Document the read-only namespace pattern in the user-facing security
surface and, if useful, cross-reference the existing gotcha. This is a docs-only
phase.

**Produces:** `SECURITY.md` only. Optional cross-reference note in
`docs/GOTCHA.md` is explicitly not planned because GOTCHA #31 already names the
trap; avoid concurrent writes by keeping this phase to `SECURITY.md`.

**Key Behaviors:**

- Explain that a read-only namespace is an allow-list of servers/tools that are
  read-only by behavior, not by trust in client annotations.
- Show the pattern at the documentation level: put only read/query/list tools in
  the namespace; do not include full-filesystem or mutation-capable tools unless
  the upstream itself is restricted.
- State the boundary: fanin-mcp enforces name-level visibility only. It does not
  inspect SQL, file paths, or arguments.
- Cross-reference the existing security guidance that OpenCode ignores
  annotations and `invoke_tool` approval covers the whole namespace.

**Depends On:** `SECURITY.md` existing namespace section; `docs/GOTCHA.md` #31;
`docs/DECISIONS.md` D-006 / D-019; Phase 2 outcome if exact config syntax is
approved.

**Skills Needed:** `md-authoring`, `capital-style`, `tool-use`.

**Phase Success Criteria:**

1. `SECURITY.md` contains a concrete read-only namespace pattern that tells
   users to expose only read/query/list tools by name.
2. The documentation states that name-level filtering is the proxy boundary and
   parameter-level safety belongs to upstreams.
3. The documentation warns that full-filesystem or broad upstreams are not
   read-only just because a namespace is named read-only.
4. No docs claim that client annotations or per-server `readonly = true`
   enforcement protect MVP sessions.

### Phase 4 — Phase 2 gate and scope cleanup

**Scope:** Run the objective gate for the Phase 2 work, fix only defects inside
this plan's scope, and ensure no Phase 3/4 behavior leaked in.

**Produces:** Implementation files already owned by Phase 2 only if a gate
failure requires a scoped fix: `src/config.rs`, `src/namespace.rs`,
`src/server.rs`, and optionally `src/error.rs`. No test edits by implementer,
simplifier, debugger, or reviewer.

**Key Behaviors:**

- The full test suite required by the orchestrator passes at 100%; no thresholds
  and no ignored failures.
- Existing Phase 0 and Phase 1 contracts remain true: exactly three static
  downstream meta-tools, lazy initialization, byte-faithful invoke results,
  reverse-traffic rejection, and no stdout diagnostics.
- Multi-upstream tests prove independent upstreams do not serialize through a
  registry lock.
- Namespace tests prove default selection, server visibility, denied paths, and
  approved tool-name filtering if implemented.
- Scope audit rejects credentials, timeouts, process-tree lifetime, sanitization,
  HTTP, and parameter-level ACL changes.

**Depends On:** Phase 1 test contract, Phase 2 namespace implementation if
approved, Phase 3 docs, existing `Cargo.toml`/`Cargo.lock` rmcp pin.

**Skills Needed:** `rust-general`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. All Phase 2 tests and the pre-existing Phase 0/1 integration tests pass at
   100%.
2. `cargo fmt --all -- --check` and the normal Rust test command used by the
   orchestrator pass without modifying test files outside `test-creator`.
3. No code path introduced by Phase 2 connects to denied servers or to upstreams
   from downstream rmcp `tools/list`.
4. No Phase 3 or Phase 4 functionality is introduced beyond the explicit scope.

## Success Criteria

1. A config with two or three upstream entries using the existing probe binary
   starts fanin-mcp successfully and downstream rmcp `tools/list` still exposes
   exactly the three static meta-tools.
2. Starting fanin-mcp and calling downstream rmcp `tools/list` opens zero
   upstream connections when multiple upstreams are configured.
3. Targeting one upstream proves lazy isolation: an untargeted second upstream is
   untouched until a request names it.
4. Concurrent first calls to the same cold upstream initialize/spawn that
   upstream exactly once according to the strongest stable harness observable.
5. A delayed `slow_tool` call on upstream `alpha` does not block a concurrent
   successful call to upstream `beta`; `beta` completes inside a deadline shorter
   than the configured slow delay.
6. Omitting `--namespace` selects `default` and exposes exactly the servers
   listed in `[namespaces.default]`.
7. A server visible in one namespace appears in `list_tools` and can be invoked;
   the same server denied in another namespace is hidden from `list_tools` and
   returns structured `namespace_denied` from `get_tool_schema` and
   `invoke_tool`.
8. If tool filters are approved, `tools.<server> = [...]` enforces name-level
   tool filtering: allowed tools are listed/callable, denied tools are hidden
   and return `namespace_denied` when addressed directly.
9. The `namespace_denied` result is a tool-level `CallToolResult` with
   `isError: true` and JSON text containing `code: "namespace_denied"`, the
   server name, the denied tool when applicable, a message, and `recoverable`.
10. Denied server checks happen before upstream connection; a denied server is
    not spawned just to reject the request.
11. `SECURITY.md` documents the read-only namespace pattern and states the
    boundary: name-level filtering only, no parameter-level ACL.
12. The existing probe binary is reused under distinct configured server names;
    no second fixture binary is added for Phase 2.
13. Existing Phase 0/1 guarantees remain intact: static meta-tools, lazy startup,
    raw argument forwarding, byte-faithful results, reverse-traffic handling,
    and stdout discipline.
14. All required gates pass at 100%; failures are surfaced and fixed in scope or
    routed, never thresholded.

## Constraints / Invariants

- Tests are a read-only contract after `test-creator` writes them. No later
  stage edits tests.
- stdout is the MCP transport. Phase 2 must not add stdout diagnostics or child
  output inheritance.
- Downstream rmcp `tools/list` remains static and must not connect to upstreams.
- The public surface remains three meta-tools: `list_tools`, `get_tool_schema`,
  and `invoke_tool`.
- Never hold a registry map lock across upstream `call_tool`, `list_all_tools`,
  spawn, or initialize awaits. Clone the `Arc`, drop the map lock, then await.
- Per-server init guards remain required; racing cold first-calls must not
  double-spawn an upstream.
- Namespace ACLs are the permission layer. Client annotations are advisory only;
  OpenCode ignores them.
- Name-level filtering is the maximum ACL scope for MVP. No parameter-level ACL,
  argument inspection, SQL parsing, path filtering, or proxy-side policy engine.
- Tool-level failures return `CallToolResult { isError: true }`, never JSON-RPC
  errors, except for protocol-level malformed MCP traffic where rmcp requires
  JSON-RPC errors.
- Phase 3 remains out: credentials, keyring/env fallback, `timeout_secs`,
  cancellation forwarding, and process-tree lifetime.
- Phase 4 remains out: upstream-provided name/description sanitization, final
  error hardening, upstream crash handling, and list-changed invalidation.

## Open Questions

1. **Should Phase 2 implement the documented per-namespace tool allow-list
   schema now?** Current code only supports `servers = [...]`, and
   `is_tool_allowed(server, tool)` delegates to `is_server_allowed(server)`.
   That does not satisfy D-006's "server + tool allow-lists" or the ROADMAP's
   "Name-level filtering only" if read-only namespaces must hide write tools
   inside an otherwise allowed server. Proposed default: implement the existing
   `docs/ARCHITECTURE.md` shape under `[namespaces.<name>]` as
   `tools.<server> = ["tool", ...]`, where an absent tool list means all tools
   on an allowed server are visible, and a present list is an exact allow-list.
   Do not add parameter-level ACLs or `readonly = true` enforcement.
