# fanin-mcp

> *fan-in (n.): the number of inputs a single gate can handle.* Many MCP servers, one endpoint.

A standalone, stdio-native MCP proxy that federates multiple MCP servers behind a single endpoint. Configure your MCP servers once, store credentials once in the OS keychain, and every Claude Code or OpenCode session gets exactly the tools it needs — on demand, not upfront.

## Why

If you use more than 2–3 MCP servers across more than one project or more than one coding CLI, you are maintaining the same server configs and the same plaintext credentials in many places, and every session pays the full startup and context cost of every server whether it uses them or not.

`fanin-mcp` fixes the three problems in order of durability:

1. **Configure once, run anywhere.** One TOML file defines all servers. Any stdio-capable MCP client (Claude Code, OpenCode, and others) points at one binary.
2. **Credentials once, in the OS keychain.** Secrets live in DPAPI / macOS Keychain / Secret Service — never in config files, never duplicated per project. Each upstream child process receives *only its own* secrets (least-privilege injection).
3. **Per-session scoping via namespaces.** `--namespace web-project` makes a session see only the servers and tools that project needs. On clients that ignore tool annotations (e.g. OpenCode), the namespace is your only permission layer — it is designed as one.
4. **Context savings as a bonus.** Clients see 3 meta-tools (~600 tokens) instead of every upstream schema (30–60K tokens). This matters most on clients without native deferred tool loading; Claude Code's Tool Search defers schemas but still spawns every configured server process — `fanin-mcp` spawns nothing until a tool call needs it.

```
claude session                       claude session
├── postgres-mcp                     └── fanin-mcp --namespace my-project
├── obsidian-mcp                           ├── (lazily spawns postgres-mcp)
├── morph-mcp                              ├── (lazily spawns obsidian-mcp)
├── context7-mcp                           ├── (lazily spawns morph-mcp)
└── filesystem-mcp                         └── (lazily spawns context7 via npx)

BEFORE: 5 processes spawned eagerly     AFTER: 1 process, lazy on demand
30–60K tokens of tool schemas upfront   ~600 tokens (3 meta-tools)
Creds duplicated per project/CLI        Creds once, in the OS keychain
```

## Core concepts

- **One aggregator, many backends.** CC/OC spawn `fanin-mcp` as their single MCP server. The aggregator owns all upstream connections internally. Per-session process: no daemon, no ports, no shared state. Session ends → stdin EOF → all upstreams torn down.
- **Progressive disclosure.** The LLM sees 3 meta-tools (`list_tools`, `get_tool_schema`, `invoke_tool`). Tool inventories and full schemas are fetched on demand, entering context as compactable tool *results*, not permanent tool definitions.
- **Namespace-based access control.** `--namespace <id>` scopes which servers (and optionally which tools per server) are visible. This is the primary permission layer.
- **Capability advertisement.** `initialize.instructions` provides a compact
  configured-server table of contents, with the `list_tools` description as a
  fallback. It uses configured descriptions and never spawns an upstream.
- **Lazy connections.** Upstreams are spawned/connected on the first tool call that targets them — never at startup.
- **Streamable HTTP is loopback-only in v1.0.0.** `transport = "streamable-http"` supports loopback `http://` upstreams only; no TLS backend is linked. Remote servers are reached exclusively via stdio/npx upstreams (e.g. `npx -y @upstash/context7-mcp`).
- **No silent hangs on bidirectional traffic.** Upstream `elicitation/create` forwards to the downstream client **only when that client declared elicitation capability** (capability-gated, D-020); on timeout/disconnect/cancel the outcome resolves to a safe non-accept, never accept. `sampling/createMessage` is still rejected and `roots/list` returns empty — sampling and roots forwarding are planned for a later v1.1 slice. Upstreams never hang waiting on the proxy.
- **Structured errors.** Upstream failures return as readable JSON inside the tool result (`isError: true`) with a `recoverable` flag — verified to reach the model as conversational content on both CC and OC.

## Quick start

### Install

- crates.io: `cargo install fanin-mcp`
- From source: download the source archive (`.zip`/`.tar.gz`) from [GitHub Releases](https://github.com/ChristNata/fanin-mcp/releases) — or `git clone` the repo — then `cargo build --release`.

```bash
# Build
cargo build --release

# Config-backed serve and check require --config; no default config path is resolved.
# Suggested locations you can point --config at:
# ~/.config/fanin-mcp/config.toml          (Linux/macOS)
# %APPDATA%\fanin-mcp\config.toml          (Windows)

# Store a credential (value read from a hidden stdin prompt — never argv)
fanin-mcp cred set postgres POSTGRES_URL

# Add to Claude Code (user scope — all projects)
claude mcp add --transport stdio --scope user fanin-mcp -- /path/to/fanin-mcp --config /path/to/config.toml

# Add to Claude Code (per-project with namespace)
claude mcp add --transport stdio fanin-mcp -- /path/to/fanin-mcp --config /path/to/config.toml --namespace my-project

# Add to OpenCode (in opencode.json)
# "mcp": { "fanin-mcp": { "type": "local", "command": ["/path/to/fanin-mcp", "--config", "/path/to/config.toml", "--namespace", "default"] } }
```

Stdio upstreams spawn under a cleared environment. Script runners such as
`node`, `npx`, `cmd`, and `python` need required variables passed through
explicitly in that server's `env` table.

```toml
[servers.context7]
transport = "stdio"
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
description = "Current library and API documentation"

[servers.context7.env]
PATH = "${PATH}"
PATHEXT = "${PATHEXT}"
SYSTEMROOT = "${SYSTEMROOT}"
APPDATA = "${APPDATA}"
LOCALAPPDATA = "${LOCALAPPDATA}"
TEMP = "${TEMP}"
USERPROFILE = "${USERPROFILE}"
COMSPEC = "${COMSPEC}"
```

## v1.2 capability discovery

### Capability advertisement

At session initialization, clients receive a compact table of contents of the
selected namespace's configured servers through MCP `initialize.instructions`.
The `list_tools` meta-tool description carries the same information as a
fallback for clients that do not surface instructions. Add an optional
`description` to `[servers.<name>]` to make the table useful. Neither path
contacts or spawns an upstream; the 3-meta-tool surface remains unchanged.

### Preflight with `check`

Run `check` before relying on a configuration:

```bash
fanin-mcp check --config <cfg> --namespace <ns> [--json] [--server <name>] \
  [--refresh-cache] [--no-cache-write]
```

It connects each visible upstream, verifies initialization and `tools/list`,
and confirms configured allowlisted tools exist. `--json` emits a structured
result, and any failure exits non-zero. The command cleans up every upstream
process tree. It is an operator readiness check, not a workflow gate.

### Composable namespaces

Namespaces may extend one or more parent namespaces. Effective servers are
unioned, while tool filters intersect: composition is least-privilege and can
never widen a parent's restriction. Unknown parents and cycles fail config
validation.

```toml
[namespaces.repo-read]
servers = ["repo"]
tools.repo = ["read_file", "search"]

[namespaces.db-read]
servers = ["db"]
tools.db = ["query", "list_tables"]

[namespaces.reviewer]
extends = ["repo-read", "db-read"]

[namespaces.reviewer.tools]
repo = ["read_file"]  # intersects repo-read; search stays unavailable
```

`reviewer` sees both servers, but only `repo__read_file` and the inherited
database read tools. A disjoint intersection remains an explicit no-tools
filter rather than becoming unrestricted.

### Advisory capability cache

A successful full-namespace `check` writes a reconstructible, non-secret cache
to `{cache_dir}/fanin-mcp/capabilities/<ns>.json`. Set `FANIN_MCP_CACHE_DIR` to
replace `{cache_dir}`. `serve` may read a matching cache to enrich its
advertisement with compact tool summaries. The cache is advisory display
metadata, never authorization: `invoke_tool` always checks the live namespace
ACL.

## How the LLM uses it

```
User: "Query my database for active users"

LLM → list_tools()                    # once per session, if it doesn't already know
LLM → get_tool_schema({ name: "postgres__query" })   # if it needs the schema
LLM → invoke_tool({ name: "postgres__query", arguments: { sql: "SELECT ..." } })
     ↓
fanin-mcp routes to the postgres MCP child (spawning it on first use) → returns result
```

A session that runs one query costs roughly 2.5K tokens of tool-related context vs. 30–60K with direct configs. (Token figures are verified by an in-repo benchmark — see MVP Phase 5.)

<!-- fanin-token-figures:start -->
Meta-tools (`tools/list`): 207 tokens (~828 bytes)
Representative session (list + schema + invoke + requests): 333 tokens (~1331 bytes)

Token measure: exact compact-JSON UTF-8 byte length, then (bytes + 3) / 4.
Deterministic; no external tokenizer; stable across runs and platforms.
Generated by `cargo bench --bench token_cost` (see benches/token_cost.rs).
<!-- fanin-token-figures:end -->

## Security model (summary)

See [SECURITY.md](SECURITY.md) for the full threat model. The short version:

- Secrets live only in the OS keychain; config files contain `${PLACEHOLDERS}`.
- Secrets are never accepted on argv, never written to logs (enforced by test), and each upstream receives only its own.
- The OS keychain protects against plaintext files, accidental commits, disk theft, and other users — **not** against malware already running as your user. No local secret store does.
- Adding an upstream server = running its code as you. Pin versions; don't run floating `npx -y` latest.

## Project structure

```
fanin-mcp/
├── src/
│   ├── main.rs              # CLI entry point, subcommands, config loading
│   ├── server.rs             # rmcp ServerHandler — meta-tools + routing
│   ├── registry.rs           # Upstream registry + lazy connections (Arc-per-connection)
│   ├── forward.rs            # Upstream-originated request handling (elicitation forwarded capability-gated; sampling/roots rejected/empty)
│   ├── process.rs            # Platform process management (Job Objects / process groups, stderr capture)
│   ├── namespace.rs          # Namespace ACL filtering
│   ├── credentials.rs        # CredentialStore trait + keyring/env backends + cred subcommands
│   ├── error.rs              # Structured error types
│   └── config.rs             # TOML config parsing + validation
├── tests/
│   └── probe-server/         # In-repo mock MCP server (echo / error / slow / annotated) for CI
├── Cargo.toml
├── Cargo.lock                # committed
├── docs/                     # Obsidian vault (read/written by glasswing-mcp)
│   ├── PRD.md
│   ├── ARCHITECTURE.md
│   ├── DECISIONS.md
│   ├── GOTCHA.md
│   ├── MVP.md
│   ├── AGG-MCP.md
│   └── master-plans/         # Covenant plan workspace
├── ROADMAP.md
├── STACK.md
├── SECURITY.md
├── LICENSE-MIT
├── LICENSE-APACHE
└── README.md
```

## Related projects

- **McpMux** (desktop app + localhost gateway) — same configure-once goal, but daemon/port-based with a GUI. `fanin-mcp` is a single binary, per-session, stdio-only, no daemon.
- **mcpmux** (npm) — TypeScript meta-tool aggregator with discover/call tools. `fanin-mcp` differs: native Rust binary (no Node runtime), namespace ACLs per session, OS-keychain credentials, structured recoverable errors.
- **postrv/forgemax**, **stephenlacy/rmcp-proxy** — Rust/rmcp reference implementations.

## Requirements

- Rust 1.80+
- Windows 10+, macOS 12+, or Linux (glibc 2.31+) — all three supported and CI-tested from day one
- No runtime dependencies (no Docker, no Node.js, no background services)

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE), at your option. Contributions are accepted under the same dual license.
