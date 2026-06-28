# fanin-mcp — Product Requirements Document

## Problem

AI coding CLIs (Claude Code, OpenCode, Gemini CLI) connect to MCP servers for external tool access — databases, knowledge bases, code-apply engines, documentation lookups, etc. As the number of MCP servers grows beyond 2–3, three compounding problems emerge:

1. **Setup repetition.** Each MCP server must be configured (command, args, credentials, env vars) in each CLI's config, for each project. A user running CC + OC across 3 projects configures each server up to 6 times. Credentials are duplicated in plaintext across config files.

2. **No cross-CLI consistency.** CC has lazy schema loading (Tool Search) but coarse per-project/global scoping. OC has per-agent MCP config but eager loading. There is no unified way to say "these 4 servers with these credentials are available everywhere, but this session only sees these 2."

3. **Context and process bloat.** Each MCP server's tool schemas load into the LLM's context (30–60K tokens for 5–8 servers; Anthropic has documented setups exceeding 67K) and each configured server is spawned as a process at session start whether used or not. CC's Tool Search mitigates the *schema* cost on CC specifically; it does not mitigate process spawn cost, and other clients lack it entirely.

(Problems are ordered by durability: #1 and #2 are the lead value proposition; #3 matters most on clients without deferred loading.)

## Solution

A standalone stdio MCP proxy — `fanin-mcp` — that sits between the LLM client and N upstream MCP servers. It presents itself as a single MCP server exposing 3 meta-tools. Upstream servers are spawned/connected lazily and scoped via namespace-based ACLs. Credentials are stored once in the OS keychain and injected per-process at spawn time.

## Users

- **Primary:** Solo developers and small teams using CC and/or OC with 3+ MCP servers across multiple projects.
- **Secondary:** Any stdio-compatible MCP client (Gemini CLI, Cursor, future tools).
- Downstream products may bundle the binary as a sidecar; the aggregator has zero knowledge of any consuming application — all integration is via CLI args, config files, and stdio.

## Requirements

### Must Have (MVP)

1. **Stdio transport.** The aggregator is spawned by CC/OC via `command` + `args` over stdin/stdout. No HTTP listener, no background process, no port management.

2. **Three meta-tools with progressive disclosure.**
   - `list_tools` — Returns server names, tool names, and one-line descriptions (truncated to ~100 chars each) for all tools visible in the current namespace. Accepts an optional `server` filter so the LLM can fetch one server's inventory.
   - `get_tool_schema` — Returns the full JSON input schema for a single tool by `server__tool` name.
   - `invoke_tool` — Proxies a tool call to the correct upstream, returns the result byte-faithfully (text, images, embedded resources, structured content — all content block types passed through unmodified).
   - `invoke_tool` carries conservative annotations (`destructiveHint: true`, `openWorldHint: true`) so annotation-aware clients (CC) prompt rather than silently auto-approve. Note: OpenCode does not surface annotations (empirically verified) — see Req 4.

3. **Lazy upstream connections.** Upstream MCP servers are not spawned or connected until the first tool call that targets them. Idle sessions consume only the aggregator process (~10–15MB RSS). Concurrent first-calls to the same cold upstream must not double-spawn (per-server init guard).

4. **Namespace ACLs — the primary permission layer.** A `--namespace <id>` CLI flag selects which upstream servers (and optionally which tools per server) are visible. Namespace definitions live in the TOML config. A `default` namespace includes all configured servers. Because the meta-tool indirection collapses client-side per-tool permission prompts (everything is `invoke_tool`), and because some clients ignore annotations entirely, the namespace tool-filter is the real access control and is documented and designed as such (e.g., read-only namespaces are a first-class documented pattern).

5. **TOML-based configuration.** A single config file defines all upstream servers (command, args, env, transport type, optional `timeout_secs`, optional `description`, optional `cwd` working-directory override, optional HTTP `headers`) and all namespaces. Located at a platform-appropriate path or overridden via `--config`. Server names are validated at load (`[a-z0-9-]+`; `__` rejected) to keep `server__tool` parsing unambiguous.

6. **Credential injection + management subcommands.** Secrets read from the OS keychain (`keyring` crate — DPAPI on Windows, Keychain on macOS, Secret Service on Linux) and injected as env vars into upstream child processes, or as static headers for remote upstreams. Resolution chain: keychain → process environment → error (the `--credential-store` flag selects the *preferred* backend; env always remains the fallback). The binary provides `cred set <server> <KEY>` (value read from hidden stdin prompt — never argv), `cred list` (names only, never values), and `cred rm`. Secrets are never written to config files, logs, or argv.

7. **Structured error responses.** When an upstream fails, `invoke_tool` returns structured JSON inside a tool result with `isError: true`: `server`, `tool`, `code` (`upstream_unavailable`, `upstream_timeout`, `tool_not_found`, `invalid_arguments`, `namespace_denied`, `spawn_failed`), `message`, `recoverable`. Verified empirically that both CC and OC surface such results to the model as readable conversational content.

8. **Static meta-tool descriptions (no startup fan-out).** The 3 meta-tools ship with static, generic descriptions; the LLM calls `list_tools` once per session to discover what's connected (costing one round-trip, ~1–2K tokens as a compactable tool result). The aggregator never connects to upstreams just to build descriptions — this preserves lazy connections (Req 3) and the startup metric. A per-server `description` config field optionally enriches `list_tools` output without any spawn. (Auto-generated descriptions from a reconstructible disk cache: v1.1, see Should Have #14.)

9. **Safe handling of upstream-originated requests (no silent hangs).** MCP is bidirectional: upstreams may send `sampling/createMessage`, `elicitation/create`, and `roots/list` to their client (the aggregator). The aggregator declares **no** sampling/elicitation capabilities to upstreams (so spec-compliant servers never send those requests) and answers any that arrive anyway with an immediate clean structured rejection — never a silent hang; `roots/list` receives an empty list. Upstream logging notifications are written to the aggregator's log file; progress notifications are accepted without crashing. Wired in from the first upstream connection (Phase 1), because an unanswered request hangs that upstream forever. Documented limitation: upstreams that *require* sampling/elicitation are unsupported in MVP. (Capability-mirrored forwarding to capable clients: v1.1, Should Have #15.)

10. **Per-server timeouts and cancellation.** Each upstream call is wrapped in a timeout (`timeout_secs` per server in config, default 60s) returning a structured `upstream_timeout` error. Client-initiated cancellation notifications are forwarded to the in-flight upstream call. (Note: clients have their own MCP timeouts, e.g. CC's `MCP_TOOL_TIMEOUT` — document the interplay.)

11. **Process-tree cleanup on all platforms.** Spawned upstream process trees must not orphan children on session end or aggregator crash: Windows Job Objects (kill-on-close) and Unix process groups, e.g. via the `process-wrap`/`command-group` approach. Verified by a hard-kill test (kill aggregator, assert zero surviving upstream processes).

12. **Upstream stderr capture.** Child stderr is piped, prefixed `[server-name]`, and written to the aggregator's log file (never mixed into the aggregator's own stderr).

13. **Security hardening (see SECURITY.md).** Log redaction of secret values (enforced by an automated test); sanitization of upstream-provided tool names/descriptions before inclusion in any text the LLM reads (strip newlines/control chars, length-cap) to bound description-based prompt injection; `Cargo.lock` committed; `cargo audit` + `cargo deny` in CI.

### Should Have (v1.1)

14. **Cached auto-generated `list_tools` description.** A reconstructible disk cache of upstream tool lists (keyed by hash of command+args) at a platform cache path, optionally pre-populated by a `warm` subcommand, used to enrich the `list_tools` meta-tool description without spawning anything at session start. (Amends the state principle: no *authoritative* persistent state; a reconstructible cache is permitted.)
15. **Capability-mirrored forwarding.** Record the downstream client's declared capabilities at `initialize`, mirror them when connecting upstreams, forward sampling/elicitation/roots requests downstream with responses relayed back (rmcp correlates per connection); clean rejection remains the fallback for undeclared capabilities.
16. **`list_changed` push notification.** Re-fetch on upstream `notifications/tools/list_changed`, push to client.
17. **Health-check meta-tool.** `list_server_status` returning connectivity and last-error state per upstream.
18. **Resource and prompt proxying.** `resources/*`, `prompts/*` with namespace ACL coverage.
19. **Per-server `readonly = true` enforcement flag.** Reject calls to tools whose upstream annotations are not read-only.
20. **Progress-based idle timeout.** Reset the per-call timeout clock on upstream `notifications/progress`; forward progress downstream.
21. **OAuth 2.1 for remote upstreams.** Out-of-band `auth <server>` subcommand (browser flow, token storage in the existing credential store, refresh handling). MVP supports static header injection only.
22. **Connection pooling / pre-warming.**
23. **Client-config installer.** `install --client <claude|opencode> --namespace <id>` writes the correct entry into each CLI's config — the adoption lever for "support all major CLIs."

### Won't Have (MVP)

- HTTP/SSE/Streamable HTTP **listener** (stdio only; remote *upstreams* over Streamable HTTP are supported)
- OAuth flows (v1.1 — static `Authorization` header injection from the credential store covers API keys and PATs)
- Web UI or dashboard
- Plugin/middleware system
- OpenTelemetry tracing (structured file logging only)
- Multi-user / multi-tenant support
- Parameter-level ACL filtering
- Hot-reload of config (restart to reload)
- Auto-discovery of installed MCP servers
- Transparent passthrough of unknown JSON-RPC methods (removed: underspecified routing, and capability negotiation means clients don't send undeclared methods)

## Success Metrics

- **Context reduction:** `tools/list` response to CC/OC ≤ 1,000 tokens (3 meta-tools) vs. 30–60K baseline; full one-query session ≤ ~3K tokens of tool-related context. All token figures verified by an in-repo benchmark before being stated in the README.
- **Startup overhead:** `initialize` handshake < 500ms. Zero upstream connections at startup.
- **Per-call overhead:** < 20ms routing latency for `invoke_tool` (excluding upstream execution). Concurrent calls to different upstreams must not serialize on each other.
- **First-call cold start:** Upstream spawn + initialize ≤ 3s local stdio, ≤ 1s remote HTTP.
- **Binary size:** < 10MB stripped release.
- **Memory:** < 15MB RSS idle, < 50MB with 5 active upstreams.
- **Cleanliness:** Zero orphaned upstream processes after session end or aggregator hard-kill (all platforms).

## Constraints

- Windows 10+, macOS 12+, Linux supported and CI-tested from day one (3-OS matrix).
- No runtime dependencies beyond the binary.
- Must not conflict with existing CC/OC MCP configurations — additive, not a replacement.
- Must compose with CC's Tool Search.
- Credentials never on disk outside the OS keychain, never on argv, never in logs.
- Known limitation (documented): concurrent sessions each spawn their own upstream instances; upstreams holding exclusive resources (ports, file locks) may conflict.
- Known limitation (documented): client-side per-tool permission prompts collapse to a single `invoke_tool` entry; use namespaces for access control.
