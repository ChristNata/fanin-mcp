# fanin-mcp — Roadmap

Status legend: ✅ released · 📋 planned · 💭 exploring · 🚫 non-goal

## v1.0 — MVP (✅ released)

The shippable core. Full plan in [MVP.md](docs/MVP.md); requirements in [PRD.md](docs/PRD.md).

- Stdio MCP proxy: 3 meta-tools (`list_tools`, `get_tool_schema`, `invoke_tool`), static descriptions
- Lazy upstream connections (stdio + loopback Streamable HTTP), per-server init guards, non-serializing concurrency
- Namespace ACLs (`--namespace`) as the primary permission layer; conservative annotations on `invoke_tool`
- Credentials: OS keychain + env fallback, `cred set|list|rm` subcommands, per-server least-privilege injection, static header injection for Streamable-HTTP upstreams (loopback `http://` in v1.0)
- Clean rejection of upstream-originated requests (sampling/elicitation), empty `roots/list` — no silent hangs
- Per-server `timeout_secs` (default 60s) + cancellation forwarding
- Structured errors (`isError: true` + `recoverable`) — verified readable on CC and OC
- Process-tree lifetime: Windows Job Objects, Unix process groups; hard-kill orphan test in CI
- Upstream stderr capture to log file; secret redaction (test-enforced)
- 3-OS CI matrix, in-repo probe server fixture, `cargo audit`/`cargo deny`, token benchmark
- Dual license (MIT OR Apache-2.0)

**Exit criteria:** the full [MVP.md verification checklist](docs/MVP.md#verification-checklist).

## v1.1 — Capability & Comfort (📋)

Roughly ordered by expected impact:

1. **Cached auto-generated `list_tools` description.** Reconstructible disk cache of upstream tool inventories (keyed by command+args hash) + `warm` subcommand. Restores the "LLM knows what's connected without a round-trip" UX without violating lazy startup.
2. **Capability-mirrored forwarding.** Mirror the downstream client's declared capabilities to upstreams; forward sampling/elicitation/roots requests downstream and relay responses. Unlocks upstreams that *require* these features, adapting per client at runtime.
3. **Native remote-HTTPS Streamable-HTTP upstreams.** v1.0 ships Streamable-HTTP for loopback `http://` only; no TLS backend is linked. Remote/HTTPS upstreams are reached via stdio/npx today. Planned: enable reqwest `rustls-tls` for native remote HTTPS.
4. **OAuth 2.1 for remote upstreams.** Out-of-band `fanin-mcp auth <server>` subcommand: browser flow, PKCE, token storage in the existing credential store, refresh handling. Unlocks Linear, Notion, Atlassian, Sentry and other OAuth-only remotes.
5. **`install` subcommand.** `fanin-mcp install --client <claude|opencode> --namespace <id>` writes the correct entry into each CLI's config. The adoption lever for "every major CLI" — each new client becomes a small adapter here instead of a docs page.
6. **`list_server_status` meta-tool.** Connectivity + last-error per upstream so the LLM can self-diagnose.
7. **`tools/list_changed` push.** Re-fetch on upstream change notification, push to client (requires verifying client refetch behavior).
8. **Per-server `readonly = true`.** Reject calls to tools whose upstream annotations aren't read-only — namespace ACL's enforcement-grade sibling.
9. **Progress-based idle timeout + progress forwarding.** Reset the per-call clock on upstream `notifications/progress`; relay progress downstream.

## v1.2+ — Breadth (💭)

- **Resource and prompt proxying** (`resources/*`, `prompts/*`) with namespace ACL coverage
- **Verified client matrix:** Gemini CLI, Cursor, Zed, Cline — probe-tested, documented quirks, `install` adapters
- **Connection pooling / pre-warming** for frequently-used upstreams
- **Config hot-reload** (watch + atomic swap; restart remains the fallback)
- **Per-server `singleton` warning** for upstreams holding exclusive resources (ports, file locks) across concurrent sessions
- **Structured audit log mode** (stable JSON schema for consumption by downstream tools/dashboards)

## Non-Goals (🚫 — identity, not backlog)

These define what fanin-mcp *is* by what it refuses to become. Revisiting any of them is a fork-the-identity decision, not a feature request:

- **No daemon, no HTTP/SSE listener, no ports.** Per-session stdio process only. (The daemon/gateway niche is served by McpMux et al.)
- **No web UI / dashboard.** CLI + config file. Consuming apps may layer UX via the same CLI.
- **No plugin/middleware system.** Add features by adding code.
- **No multi-tenancy.** One process, one session, one user.
- **No parameter-level ACL.** Name-level filtering only; argument-level safety belongs to upstreams.
- **No transparent passthrough of unknown JSON-RPC methods.** Capability negotiation makes it unnecessary; "first matching upstream" routing is a trap.

## Release practice

- Published to **crates.io** (`cargo install fanin-mcp`); source distributed via tagged **GitHub Releases** (auto-generated source `.zip`/`.tar.gz`) for `cargo build --release`. No prebuilt binaries shipped.
- SemVer. The 3-meta-tool surface and the structured-error JSON shape are the public API — breaking either bumps major.
- README performance/token claims are regenerated from the in-repo benchmark per release, never hand-edited.
