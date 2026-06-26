# fanin-mcp — MVP Implementation Plan

## Scope

A single Rust binary added to any CC or OC config as a stdio MCP server, proxying tool calls to N upstream MCP servers with lazy connections, namespace filtering, credential injection, clean rejection of upstream-originated requests (forwarding: v1.1), and clean process-tree lifetime on all three OSes. No UI, no daemon, no HTTP listener, no OAuth (static header injection only).

## Phase 0: Skeleton + Stdio Echo + Probe Fixture (Day 1)

**Goal:** A binary CC can spawn that answers `initialize` and `tools/list` with the 3 static meta-tools — plus the test fixture everything else depends on.

1. `cargo init --name fanin-mcp`; pin `rmcp` to an exact version; commit `Cargo.lock`
2. Implement `ServerHandler`: `get_info()`, `list_tools()` (3 meta-tools, **static descriptions** — Option C, final design not a stub), `call_tool()` stubs
3. `main.rs`: `clap` with subcommand layout (`serve` default; `cred` stubs), `--namespace`, `--config`; `serve(stdio())`
4. **Probe server fixture** (`tests/probe-server/`): tiny rmcp binary with `echo_ok`, `always_error` (structured JSON, `isError: true`), `slow_tool` (configurable delay), `dangerous_noop` (destructive annotations), `needs_sampling` (sends a sampling request). Used by all integration tests and CI — no Node required. (Same probe design already validated manually against OpenCode.)
5. **Test:** raw JSON-RPC over stdin; `claude mcp add` → CC sees 3 tools

## Phase 1: Config + Single Upstream + Reverse Traffic (Days 2–4)

**Goal:** TOML config, one stdio upstream, full proxy path — including the bidirectional handling that bites on the very first real server.

1. `config.rs`: structs + validation (server names `[a-z0-9-]+`, reject `__`; fail-fast on unknown namespace)
2. `Registry` with `get_or_connect()`: `Arc<RunningService>` per connection, **lock only for map access — clone Arc, drop lock, then call**; per-server init guard against double-spawn
3. `invoke_tool` end-to-end: parse on first `__` → ACL → connect → forward **raw arguments** → return result **byte-faithfully** (all content block types)
4. `list_tools` / `get_tool_schema` handlers against the live upstream's cached `tools/list`
5. **`forward.rs` (clean reject):** declare no sampling/elicitation capabilities to upstreams; `UpstreamClientHandler` answers `roots/list` with an empty list, immediately rejects any sampling/elicitation request with a structured error (never a hang), routes upstream log notifications to the log file, tolerates progress notifications
6. **stderr capture:** pipe child stderr → `[server]`-prefixed lines → log file
7. **Test:** probe server invocations through CC; probe's `needs_sampling` gets a clean rejection, not a hang

## Phase 2: Multi-Upstream + Namespace ACLs (Days 5–6)

1. N lazy upstreams; verify a slow call on one (probe `slow_tool`) doesn't block another (concurrency test for the locking discipline)
2. `Namespace` with `is_server_allowed()` / `is_tool_allowed()`; wire into `list_tools` + `invoke_tool`; `default` namespace
3. `namespace_denied` structured error; document the read-only-namespace pattern
4. **Test:** 2–3 upstreams; lazy spawn verified (second upstream untouched until targeted); namespace switching

## Phase 3: Credentials + Timeouts + Process Lifetime (Days 7–9)

1. `CredentialStore` trait; `KeyringStore` + `EnvStore`; chain = preferred backend → env → error
2. **`cred set|list|rm` subcommands** — `set` reads value from hidden stdin prompt (never argv); `list` prints names only
3. `${VAR}` interpolation at spawn; inject **only that server's** vars (least privilege); static `headers` injection for HTTP upstreams
4. **Log redaction layer** + automated test: sentinel secret must never appear in log output
5. **Per-server `timeout_secs`** (default 60) wrapping every upstream call → `upstream_timeout`; forward client cancellation notifications to in-flight calls
6. **`process.rs`:** Windows Job Objects + Unix process groups (evaluate `process-wrap`/`command-group`; thin custom child transport if rmcp's `TokioChildProcess` can't be wrapped)
7. **Test:** keyring round-trip via `cred set`; env fallback; **hard-kill test** — `kill -9` the aggregator, assert zero surviving upstream processes (Windows: no orphaned `node.exe`)

## Phase 4: Error Hardening + Sanitization (Day 10)

1. `AggError`/`ErrorCode` finalized; all upstream communication wrapped (spawn failure, timeout, call error, mid-session upstream death)
2. **Sanitize upstream-provided tool names/descriptions** before inclusion in anything the LLM reads: strip newlines/control chars, length-cap (~100 chars in `list_tools` rows) — bounds description-based prompt injection
3. Cache invalidation on upstream `notifications/tools/list_changed`
4. **Test:** kill an upstream mid-session → structured error, siblings unaffected; probe `always_error` round-trips intact; probe `needs_sampling` gets a clean rejection, not a hang

## Phase 5: Polish + Cross-Platform CI + Benchmarks (Days 11–12)

1. `tracing` JSON file output (`--log-file`, `--log-level`); log every call (server, tool, latency, outcome), connect/disconnect, config load — redaction verified
2. **CI matrix: Windows + macOS + Linux** from this phase onward; `cargo audit` + `cargo deny`
3. Integration tests vs CC (global + per-project), OC (`opencode.json`), probe server, and one real remote HTTP upstream with header auth
4. Namespace switching, credential E2E, Tool Search composition check on CC
5. **Token benchmark:** measure actual `tools/list` + typical-session token costs; README numbers come from this, not estimates
6. Hard-kill orphan test in CI on all OSes; binary size (<10MB stripped); memory profile (<15MB idle, <50MB @ 5 upstreams)
7. SECURITY.md finalized; dual-license files (`LICENSE-MIT`, `LICENSE-APACHE`); `license = "MIT OR Apache-2.0"`

## Timeline

| Phase | Days | Cumulative |
|-------|------|------------|
| 0: Skeleton + fixture | 1 | 1 |
| 1: Single upstream + reverse traffic | 3 | 4 |
| 2: Multi + namespaces | 2 | 6 |
| 3: Creds + timeouts + process lifetime | 3 | 9 |
| 4: Errors + sanitization | 1 | 10 |
| 5: Polish + CI + benchmarks | 2 | 12 |

**Total: ~12 working days**, assuming CC-driven implementation with rmcp familiarity. (+2 days vs the original plan: cred subcommands, process-tree work, and the reverse-traffic baseline were added; auto-generated startup descriptions and capability-mirrored forwarding moved to v1.1, partially offsetting.)

## Deferred to v1.1

- Capability-mirrored forwarding of sampling/elicitation/roots to capable clients (clean rejection ships in MVP)
- Cached auto-generated `list_tools` description (reconstructible disk cache + `warm` subcommand)
- `notifications/tools/list_changed` push to client
- `list_server_status` health-check meta-tool
- Resource and prompt proxying
- Per-server `readonly = true` enforcement
- Progress-based idle timeout + progress forwarding
- OAuth 2.1 (`auth <server>` subcommand)
- Connection pooling / pre-warming
- `install --client` config writer
- Hot-reload of config

## Verification Checklist

- [ ] CC and OC each spawn fanin-mcp and discover exactly 3 meta-tools
- [ ] `initialize` < 500ms with zero upstream connections opened
- [ ] `list_tools` returns the correct server/tool rows for the active namespace; descriptions sanitized + truncated
- [ ] `get_tool_schema` returns the correct schema for any `server__tool`
- [ ] `invoke_tool` proxies calls and returns results byte-faithfully (incl. non-text content)
- [ ] Tool name parsing splits on the first `__`; server names containing `__` are rejected at config load
- [ ] Namespace filtering hides non-namespace servers/tools; denied calls return `namespace_denied`
- [ ] Lazy connection verified: upstream not spawned until first targeting call; concurrent first-calls spawn exactly once
- [ ] A slow call on one upstream does not block calls to other upstreams
- [ ] `cred set` (hidden stdin) / `list` (names only) / `rm` work on all three OSes; env fallback works keyring-less
- [ ] Secrets never appear on argv or in logs (automated sentinel test)
- [ ] Each upstream receives only its own credential env vars
- [ ] HTTP upstream receives static auth headers from the credential store
- [ ] Per-server `timeout_secs` honored; expiry returns `upstream_timeout`; client cancellation forwarded
- [ ] No sampling/elicitation capabilities declared to upstreams; any such request that arrives is cleanly rejected (no hang); `roots/list` answered with an empty list
- [ ] Upstream crash mid-session → structured error; sibling upstreams unaffected
- [ ] Upstream stderr lands in the log file with `[server]` prefix, not the aggregator's stderr
- [ ] Hard-kill of the aggregator leaves zero orphaned upstream processes (Windows Job Object / Unix process group) — CI-tested on all OSes
- [ ] Session teardown on stdin EOF: full tree teardown, clean exit
- [ ] `cargo audit` / `cargo deny` clean; `Cargo.lock` committed; rmcp pinned exactly
- [ ] Token benchmark run; README figures match measurements
- [ ] Binary < 10MB stripped; idle < 15MB RSS; works on Windows 10+, macOS 12+, Linux
- [ ] CC Tool Search composes (3 meta-tools, no conflict)
