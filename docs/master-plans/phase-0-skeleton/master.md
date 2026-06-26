---
Feature: phase-0-skeleton
Scope: flat
Stack: rust
Tier: THOROUGH
Status: draft
Created: 2026-06-26
Target: src/main.rs; src/server.rs; tests/probe-server/
Dependencies: none
---

# Phase 0 Skeleton + Stdio Echo + Probe Fixture

## What

Ship the first buildable `fanin-mcp` Rust binary: a flat cargo scaffold, the
stdio MCP server skeleton, the three final static meta-tools, a structured
not-implemented `call_tool` stub, CLI flag/subcommand plumbing, and the in-repo
probe server fixture used by later integration tests.

## Why

Phase 0 anchors the MVP on a real stdio MCP process before any upstream proxying
exists. `docs/MVP.md` defines this phase as the binary that answers
`initialize` and `tools/list` with the 3 static meta-tools, plus the no-Node
probe fixture. `docs/DECISIONS.md` D-002 fixes the three-tool surface, D-003
makes the static descriptions final rather than placeholder startup text, D-006
requires conservative `invoke_tool` annotations, D-015 requires an exact `rmcp`
pin and committed `Cargo.lock`, and D-016 requires the five-tool probe fixture.
`docs/GOTCHA.md` #1 is the primary implementation trap: stdout is the MCP
transport once `serve(stdio())` starts, so diagnostics must never use stdout.

Verification against the tree: this worktree currently has the design docs and
plan workspace but no `Cargo.toml` or `src/` tree. The scaffold is therefore a
creation task, not an extension of existing Rust code. `CLAUDE.md` confirms the
flat project scope and canonical module names; `STACK.md` confirms Rust 1.80+,
edition 2021, exact-pinned `rmcp`, committed `Cargo.lock`, and a no-runtime-deps
product promise. `docs/ARCHITECTURE.md` provides the exact static descriptions
and annotation intent used below. Context7 was checked for current `rmcp`
`ServerHandler` shapes; implementers must still verify the exact signatures
against the version they pin because D-015 treats all snippets as pseudocode
until compiled against the pin.

No blocking drift found.

## Dependencies

No prerequisite master plans. This is the first implementation plan for the
MVP. P0.1 is the serial scaffold dependency. After P0.1 lands, the aggregator
skeleton and the probe fixture can dispatch concurrently because their primary
code paths are disjoint (`src/` versus `tests/probe-server/`). Any shared root
`Cargo.toml` edits must be kept minimal and coordinated by phase order.

## Scope

### In

- `cargo init --name fanin-mcp`-equivalent scaffold for a Rust 1.80+, edition
  2021 binary crate.
- `rmcp` pinned to an exact version with `Cargo.lock` committed.
- Flat module stubs for `main.rs`, `server.rs`, `registry.rs`, `forward.rs`,
  `process.rs`, `namespace.rs`, `credentials.rs`, `error.rs`, and `config.rs`.
- Downstream `ServerHandler` skeleton with `get_info()`, `list_tools()` returning
  exactly the three meta-tools, and `call_tool()` returning a structured
  not-implemented tool result.
- Final static meta-tool descriptions, not temporary filler.
- `invoke_tool` annotations: `destructiveHint=true`, `readOnlyHint=false`, and
  `openWorldHint=true`.
- `main.rs` CLI plumbing: `serve` as the default behavior, `cred` subcommand
  stubs, global `--namespace`, global `--config`, and `serve(stdio())`.
- In-repo probe fixture under `tests/probe-server/`, buildable without Node or
  `npx`, exposing `echo_ok`, `always_error`, `slow_tool`, `dangerous_noop`, and
  `needs_sampling` over stdio.

### Out

- No real upstream connections.
- No `Registry::get_or_connect()` implementation beyond a stub module.
- No config file parsing or validation beyond CLI flag plumbing.
- No namespace ACL enforcement beyond carrying the selected namespace value.
- No reverse-traffic handling in the aggregator: no `UpstreamClientHandler`, no
  sampling rejection, no `roots/list` response, and no upstream log/progress
  routing. That starts in Phase 1.
- No credentials logic, no keyring calls, no hidden prompt implementation, and no
  credential persistence. `cred` is a CLI stub only.
- No process-tree or Windows Job Object work.
- No upstream timeouts, cancellation forwarding, or child stderr capture.
- No proxying from `call_tool`; every meta-tool call returns a structured
  not-implemented result in Phase 0.
- No HTTP server, web framework, database, plugin loader, Node runtime, Docker
  dependency, or system service.
- No tests written by implementer phases. The executable test contract is owned
  by `test-creator`.

## Required Pattern

The static downstream tool surface is exact for Phase 0. Implementations may
adapt type names to the pinned `rmcp` API, but must preserve the public shape.

| Tool | Static description | Input shape | Annotations |
|---|---|---|---|
| `list_tools` | `Lists the tools available through this aggregator, grouped by server, with one-line descriptions. Call this once to see what's connected; pass server to fetch a single server's tools.` | Optional `server` string. | None required. |
| `get_tool_schema` | `Get the full input schema for a tool. Format: server__tool (e.g. postgres__query).` | Required `name` string. | None required. |
| `invoke_tool` | `Call a tool by server__tool name with arguments.` | Required `name` string and required `arguments` object. | `destructiveHint=true`, `readOnlyHint=false`, `openWorldHint=true`. |

`list_tools()` must return exactly these three tools and no upstream-derived
content. `call_tool()` must not proxy any request in this phase; it returns a
structured not-implemented `CallToolResult` with `isError: true`, no panic, and
no hang.

## Phases

### P0.1 — Cargo scaffold and flat module skeleton

**Scope:** Create the binary crate foundation and the canonical flat module
layout. Pin `rmcp` exactly and make the lockfile part of the deliverable.

**Produces:**

- `Cargo.toml`
- `Cargo.lock`
- `src/main.rs`
- `src/server.rs`
- `src/registry.rs`
- `src/forward.rs`
- `src/process.rs`
- `src/namespace.rs`
- `src/credentials.rs`
- `src/error.rs`
- `src/config.rs`

**Key Behaviors:**

- Root package is named `fanin-mcp`.
- Edition is 2021 and MSRV is Rust 1.80+.
- `rmcp` dependency uses exact-pin syntax, not a range.
- Dependencies stay inside the project stack: `tokio`, `serde`, `serde_json`,
  `clap`, `tracing`, `tracing-subscriber`, and `schemars` are acceptable here;
  later-phase crates may be stubbed or deferred if unused.
- Stub modules compile without implementing later-phase behavior.

**Depends On:** none.

**Skills Needed:** `rust-general`; `rmcp-general`.

**Phase Success Criteria:**

- `cargo build` succeeds on the scaffold.
- `cargo clippy -- -D warnings` succeeds for scaffolded code.
- `Cargo.lock` exists and is intended to be committed.
- `Cargo.toml` pins `rmcp` with exact `=x.y.z` syntax.
- All canonical module files exist under flat `src/`.

### P0.2 — Aggregator stdio server skeleton

**Scope:** Implement the downstream MCP server surface and CLI plumbing without
connecting to any upstream.

**Produces:**

- `src/main.rs`
- `src/server.rs`
- `src/config.rs`
- Potential supporting edits in `src/error.rs`

**Key Behaviors:**

- `main.rs` defines global `--namespace` and `--config` flags.
- `serve` is the default command path and starts `serve(stdio())`.
- `cred` subcommands exist only as stubs; they must not touch keyring or secrets.
- `get_info()` advertises the server name/version and tools capability required
  for clients to call `tools/list`.
- `list_tools()` returns exactly `list_tools`, `get_tool_schema`, and
  `invoke_tool`, using the final static descriptions in the Required Pattern.
- `invoke_tool` carries conservative annotations from D-006.
- `call_tool()` returns a structured not-implemented tool result for any
  meta-tool call and does not use JSON-RPC errors for this tool-level condition.
- No stdout writes occur after stdio serving begins; diagnostics use stderr or
  tracing sinks only.
- Startup and `initialize` perform no upstream work because no upstream registry
  exists in Phase 0.

**Depends On:** P0.1.

**Skills Needed:** `rust-general`; `rmcp-general`.

**Phase Success Criteria:**

- Spawned over stdio, the binary answers `initialize` within 500ms.
- `tools/list` returns exactly the three meta-tools and the Required Pattern
  descriptions.
- The returned `invoke_tool` definition includes `destructiveHint=true`,
  `readOnlyHint=false`, and `openWorldHint=true`.
- Calling any of the three meta-tools returns a structured not-implemented
  result with `isError: true` and does not panic or hang.
- `initialize` opens zero upstream connections; this is observable because no
  upstream process, registry connection, or config-loaded server exists yet.
- No stdout diagnostics corrupt stdio JSON-RPC traffic.

### P0.3 — Probe server fixture

**Scope:** Add the standalone probe fixture used by later tests and CI. It must
build and run without Node, `npx`, or any external MCP server.

**Produces:**

- `tests/probe-server/` fixture source files
- Root `Cargo.toml` bin-target or fixture-build metadata as needed
- Optional fixture README if needed to document direct invocation

**Key Behaviors:**

- The probe is an rmcp stdio server binary reachable by integration tests.
- `echo_ok` returns the supplied input in a successful tool result.
- `always_error` returns structured JSON with `isError: true`.
- `slow_tool` accepts a configurable delay and waits before returning.
- `dangerous_noop` is harmless but advertises destructive annotations.
- `needs_sampling` sends a sampling request upstream to exercise later Phase 1
  reverse-traffic handling; Phase 0 aggregator is not expected to answer it.
- The fixture does not require a Node runtime, Docker, or a system service.

**Depends On:** P0.1.

**Skills Needed:** `rust-general`; `rust-test`; `rmcp-general`.

**Phase Success Criteria:**

- The probe fixture builds as part of the Rust project or by an explicitly
  documented fixture build command.
- The probe can be spawned over stdio independently of the aggregator.
- `tools/list` on the probe exposes exactly `echo_ok`, `always_error`,
  `slow_tool`, `dangerous_noop`, and `needs_sampling`.
- Each of the five probe tools is reachable over stdio.
- `always_error` returns a structured tool result with `isError: true`.
- `dangerous_noop` exposes destructive annotations.
- `needs_sampling` attempts to send a sampling request; no Phase 0 aggregator
  reverse-handler is added to satisfy it.

## Success Criteria

1. **Build gate:** `cargo build` succeeds and `cargo clippy -- -D warnings`
   exits 0. Maps to P0.1, P0.2, P0.3.
2. **Static discovery gate:** when spawned over stdio, the aggregator answers
   `initialize` and `tools/list` with exactly the three meta-tools and the
   Required Pattern descriptions. Maps to P0.2.
3. **Annotation gate:** the `invoke_tool` definition includes conservative
   annotations: `destructiveHint=true`, `readOnlyHint=false`, and
   `openWorldHint=true`. Maps to P0.2.
4. **Startup laziness gate:** `initialize` returns in under 500ms and opens zero
   upstream connections. Maps to P0.2.
5. **Stub call gate:** calling any aggregator meta-tool returns a structured
   not-implemented result with `isError: true`; it does not panic and does not
   hang. Maps to P0.2.
6. **Probe build gate:** the probe-server fixture builds and runs standalone over
   stdio with no Node or `npx`. Maps to P0.3.
7. **Probe inventory gate:** the probe exposes exactly five tools over stdio:
   `echo_ok`, `always_error`, `slow_tool`, `dangerous_noop`, and
   `needs_sampling`. Maps to P0.3.
8. **Probe behavior gate:** each probe tool is reachable; `always_error` returns
   structured JSON with `isError: true`, `slow_tool` honors a requested delay,
   `dangerous_noop` advertises destructive annotations, and `needs_sampling`
   sends a sampling request. Maps to P0.3.
9. **Pinning gate:** `Cargo.lock` exists and `Cargo.toml` pins `rmcp` exactly
   with `=x.y.z` syntax. Maps to P0.1.

## Constraints / Invariants

- Tier is **THOROUGH**. The simplify stage runs later, and review uses
  alignment, adversarial, and general lenses.
- Scope is **flat**. Artifacts live under `docs/master-plans/phase-0-skeleton/`.
- stdout is the MCP transport. No `println!`, `print!`, or `dbg!` after
  `serve(stdio())` starts.
- Static meta-tool descriptions are final design, not a temporary stub.
- `tools/list` must not connect to or inspect upstreams.
- `invoke_tool` annotations are conservative but are not a security boundary;
  namespaces become the real gate in later phases.
- Reverse-traffic clean rejection is Phase 1, not Phase 0. Do not add
  `UpstreamClientHandler` logic to the aggregator in this plan.
- Tool-level not-implemented behavior returns a structured tool result with
  `isError: true`, not a JSON-RPC error.
- rmcp snippets in docs and memory are pseudocode until verified against the
  exact pinned version.
- Tests are a read-only contract after `test-creator` writes them. Implementer,
  simplifier, and debugger phases do not edit test files.
- 100% test pass rate. No thresholds and no accepted red gates.

## Open Questions

(none)
