# fanin-mcp — Architecture

## System Context

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Claude Code or OpenCode session                                         │
│                                                                         │
│   LLM ←→ MCP Client                                                    │
│              │                                                          │
│              │ stdin/stdout (JSON-RPC)                                   │
│              ▼                                                          │
│   ┌──────────────────┐                                                  │
│   │    fanin-mcp       │ (spawned as child process by CC/OC)             │
│   │                    │                                                │
│   │  ┌──────────────┐ │   stdin/stdout    ┌─────────────────────────┐  │
│   │  │  ServerHandler │─│────────────────→ │ postgres-mcp (child)     │ │
│   │  │  (3 meta-tools)│ │                  └─────────────────────────┘  │
│   │  │  Forwarder     │ │   stdin/stdout    ┌─────────────────────────┐  │
│   │  │  Registry      │─│────────────────→ │ obsidian-mcp (child)     │ │
│   │  │  Namespace ACL │ │   HTTP            ┌─────────────────────────┐  │
│   │  │  Credentials   │─│────────────────→ │ context7 (remote)         │ │
│   │  └──────────────┘ │                  └─────────────────────────┘  │
│   └──────────────────┘                                                  │
└─────────────────────────────────────────────────────────────────────────┘
```

Each CC/OC session spawns its own `fanin-mcp` process. No shared state between sessions. Session ends → CC/OC closes stdin → EOF → tear down all upstream connections (killing full process trees) → exit.

Traffic is bidirectional: downstream requests (tool calls) flow up to upstreams; upstream-originated requests (sampling, elicitation, roots) are answered locally in MVP — clean rejection / empty roots, never a silent hang (see `forward.rs`). Forwarding them to capable clients is v1.1.

## Module Structure

### `main.rs` — Entry point + subcommands
- Subcommands: *(default: serve)*, `cred set|list|rm`, future: `warm`, `auth`, `install`
- Parse CLI args (`--config`, `--namespace`, `--log-level`, `--log-file`, `--credential-store`, `--passthrough-stderr`)
- Load and validate TOML config (including server-name validation)
- Initialize credential store
- Build `Registry` with namespace-filtered server list
- Create `AggServer`, call `agg_server.serve(stdio()).await`

### `config.rs` — Configuration

```toml
# Example config.toml

[servers.postgres]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres@1.2.3"]  # pin versions
env = { DATABASE_URL = "${POSTGRES_URL}" }   # resolved from keyring/env, never literal secrets
timeout_secs = 120                            # per-server; default 60
description = "Postgres queries & schema inspection"  # optional, enriches list_tools output

[servers.obsidian]
transport = "stdio"
command = "obsidian-mcp"
args = ["--vault", "/path/to/vault"]

[servers.context7]
transport = "http"
url = "https://mcp.context7.com/mcp"
headers = { Authorization = "Bearer ${CONTEXT7_TOKEN}" }  # static header injection (OAuth: v1.1)

[servers.morph]
transport = "stdio"
command = "npx"
args = ["-y", "@morphllm/morphmcp@1.2.3"]   # pin versions
env = { MORPH_API_KEY = "${MORPH_API_KEY}", ALL_TOOLS = "false" }  # edit-only; "true" = full filesystem
cwd = "${PROJECT_ROOT}"                       # spawn CWD = session project root, or Morph edits the wrong tree (GOTCHA #30)
timeout_secs = 120                            # large applies can be slow

[namespaces.default]
servers = ["*"]

[namespaces.web-project]
servers = ["context7", "morph"]

[namespaces.research]
servers = ["obsidian", "postgres"]
tools.postgres = ["query", "list_tables"]  # tool-level filter — the real permission layer

[namespaces.research-readonly]            # documented pattern: read-only namespace
servers = ["postgres"]
tools.postgres = ["query", "list_tables"]
```

**Validation at load (fail-fast):**
- Server names must match `[a-z0-9-]+` and must not contain `__` (keeps `server__tool` parsing unambiguous; parsing splits on the *first* `__` since upstream tool names may contain it).
- Unknown `--namespace` → startup error.
- Literal-looking secrets in `env`/`headers` values (heuristic) → warning.

**Config resolution:** `--config` → `$FANIN_MCP_CONFIG` → platform default (`%APPDATA%\fanin-mcp\config.toml` / `~/.config/fanin-mcp/config.toml`).

**Env var interpolation:** `${VAR}` resolved at spawn time: preferred credential backend → process environment → error. `--credential-store` selects the *preferred* backend; env is always the fallback (covers headless Linux without Secret Service).

**Child working directory:** an optional per-server `cwd` field sets the spawned child's working directory (supports `${VAR}` interpolation). Defaults to the aggregator's own CWD. Required for directory-scoped upstreams like Morph, which auto-detect a workspace root and otherwise inherit fanin-mcp's CWD rather than the session's project root (GOTCHA #30). Ignored for HTTP upstreams.

### `server.rs` — ServerHandler (downstream, facing CC/OC)

Implements `rmcp::ServerHandler`.

**`get_info()`**: name, version, `tools` + `tools.listChanged` capabilities.

**`list_tools()`** returns exactly 3 meta-tools with **static descriptions** (no upstream contact — see "Description strategy" below):

1. **`list_tools`** — "Lists the tools available through this aggregator, grouped by server, with one-line descriptions. Call this once to see what's connected; pass `server` to fetch a single server's tools." Input: `{ server?: string }`. Output rows: `{ server, tool, description }` (descriptions sanitized + truncated ~100 chars).
2. **`get_tool_schema`** — "Get the full input schema for a tool. Format: server__tool (e.g. postgres__query)." Input: `{ name: string }`.
3. **`invoke_tool`** — "Call a tool by server__tool name with arguments." Input: `{ name: string, arguments: object }`. **Annotations: `destructiveHint: true`, `readOnlyHint: false`, `openWorldHint: true`** — deliberately conservative so annotation-aware clients (CC) prompt instead of auto-allowing. (OpenCode ignores annotations — verified empirically — which is why namespaces are the primary permission layer.)

**`call_tool()`** dispatches on meta-tool name. For `invoke_tool`:
1. Parse `server__tool` (split on first `__`)
2. Namespace ACL check
3. `registry.get_or_connect(server)` (lazy)
4. Forward `tools/call` with **raw `serde_json::Value` arguments — no parsing, validation, or transformation**
5. Return the upstream result **byte-faithfully**: all content block types (text, image, embedded resource, resource link, structuredContent) pass through unmodified

**Description strategy (MVP vs v1.1):** MVP ships static meta-tool descriptions; the per-server `description` config field optionally enriches `list_tools` *results*. This preserves lazy connections — the aggregator never fans out to upstreams at session start. v1.1 adds a reconstructible disk cache of upstream tool lists (keyed by hash of command+args, at the platform cache dir, optionally pre-populated via `fanin-mcp warm`) used to auto-enrich the description. The cache is not authoritative state — deleting it only costs one re-fetch.

### `registry.rs` — Upstream Server Registry

```rust
struct Registry {
    configs: HashMap<String, UpstreamConfig>,
    connections: RwLock<HashMap<String, Arc<RunningService<RoleClient>>>>,
    init_guards: HashMap<String, Arc<tokio::sync::Mutex<()>>>,  // one per configured server
    tool_cache: RwLock<HashMap<String, Vec<ToolInfo>>>,         // session-lifetime cache
    credentials: Box<dyn CredentialStore>,
}
```

**Locking discipline (critical):** locks are held only to look up or insert the `Arc<RunningService>` — **never across an upstream call**. The calling code clones the `Arc`, drops the lock, then awaits `call_tool` on the clone. A slow postgres query must not block a context7 lookup. Cold-start races are handled by the per-server `init_guard`: two concurrent first-calls to the same server result in exactly one spawn.

**Lazy connection lifecycle:**
1. `get_or_connect(name)` — fast path: read-lock, clone Arc, return. Slow path: take that server's init guard, re-check, then spawn/connect.
2. Stdio upstreams: resolve credential placeholders, spawn via the process module (Job Object / process group wrapped), with **only that server's** env vars injected (least privilege).
3. HTTP upstreams: `StreamableHttpClientTransport` with resolved static headers.
4. Each call is wrapped in `tokio::time::timeout(server.timeout_secs)` → `upstream_timeout` structured error on expiry. Client cancellation notifications are forwarded to in-flight upstream calls.
5. `tools/list` fetched and cached per session; invalidated on upstream `notifications/tools/list_changed`.

**Error isolation:** each upstream connection runs in its own task; a panic or hang in one upstream never affects others or crashes the aggregator. All failures become structured `AggError` results.

**Teardown:** stdin EOF → drop all handles → process module guarantees full-tree kill (see `process.rs`).

### `forward.rs` — Upstream-Originated Request Handling

MCP upstreams can send requests *to* the aggregator (their client). Ignoring them hangs the upstream forever and surfaces as a mysterious tool-call timeout — so this module is MVP and wired in from the first upstream connection (Phase 1).

**MVP behavior (clean reject):**
1. The aggregator declares **no** sampling/elicitation capabilities when connecting upstreams — spec-compliant servers therefore never send those requests in the first place.
2. Any `sampling/createMessage` or `elicitation/create` that arrives anyway → immediate structured rejection ("capability not available through this proxy") — never a silent hang.
3. `roots/list` → empty list.
4. Upstream `notifications/message` (logging) → aggregator log file with `[server]` prefix. Progress notifications accepted, not forwarded.
5. Documented limitation: upstreams that *require* sampling/elicitation are unsupported in MVP.

**v1.1 (capability mirroring):** record the downstream client's declared capabilities at `initialize`, mirror them to upstreams, forward declared-capability requests downstream and relay responses back (rmcp correlates requests per connection — no manual ID bookkeeping). The module is shaped so the forwarding arm slots in beside the reject arm without restructuring.

### `process.rs` — Platform Process Management

- **Windows:** spawn `cmd /c npx ...` children inside a Job Object with kill-on-close, so dropping the handle (or aggregator crash) kills the **entire tree** — no orphaned `node.exe`. (`#[cfg(windows)]`.)
- **Unix:** new process group per child (`setsid`); teardown kills the group. (`#[cfg(unix)]`.)
- Prefer a unified wrapper (the `process-wrap` / `command-group` approach) behind one API; note rmcp's `TokioChildProcess` constructs the command itself, so this may require a thin custom child-process transport — contained, isolated in this module.
- **stderr:** piped, line-buffered, `[server]`-prefixed, written to the log file. `--passthrough-stderr` mirrors to the aggregator's stderr for debugging.
- Polite-then-forceful teardown: close child stdin, brief grace period, then kill tree.

### `namespace.rs` — Namespace ACL

Unchanged in shape; elevated in role: **this is the primary permission layer** (meta-tool indirection collapses client per-tool prompts; OC ignores annotations).

```rust
struct Namespace {
    name: String,
    servers: HashSet<String>,                         // "*" = all
    tool_filter: HashMap<String, HashSet<String>>,    // server -> allowed tools (absent = all)
}
```

Selected by `--namespace` (default `"default"`); unknown namespace → fail-fast startup error.

### `credentials.rs` — Credential Store + Subcommands

```rust
trait CredentialStore: Send + Sync {
    fn get(&self, service: &str, key: &str) -> Result<String, CredError>;
    fn set(&self, service: &str, key: &str, value: &str) -> Result<(), CredError>;
    fn delete(&self, service: &str, key: &str) -> Result<(), CredError>;
}
```

- **`KeyringStore`** — `keyring` crate, service `fanin-mcp/{server}`. DPAPI / Keychain / Secret Service.
- **`EnvStore`** — process environment. Always the fallback in the resolution chain (headless Linux, CI, containers).
- **Subcommands:** `cred set <server> <KEY>` reads the value from a **hidden stdin prompt** (never argv — process listings and shell history are world-readable), `cred list` prints names only, `cred rm` deletes.
- **Redaction:** a tracing layer redacts all resolved secret values from log output; an automated test asserts a known sentinel secret never appears in logs.

### `error.rs` — Structured Errors

```rust
enum ErrorCode {
    UpstreamUnavailable, UpstreamTimeout, ToolNotFound,
    InvalidArguments, NamespaceDenied, SpawnFailed,
}

struct AggError {
    error: bool,        // always true
    server: String,
    tool: String,
    code: ErrorCode,
    message: String,
    recoverable: bool,
}
```

Serialized as JSON in `CallToolResult { isError: true }` — never JSON-RPC errors — keeping the error in conversation where the LLM can reason about it. **Empirically verified on both CC and OC** that such results reach the model as readable content with the JSON intact.

## Data Flow: Tool Invocation

```
CC/OC → tools/call { name: "invoke_tool", arguments: { name: "postgres__query", arguments: { sql: "..." } } }
  → server.rs: dispatch "invoke_tool"; parse on first "__"
  → namespace.rs: "postgres"/"query" allowed? ✓
  → registry.rs: get_or_connect("postgres")  [lock → clone Arc → unlock]
       (first call: init guard → resolve creds → process.rs spawn (job/pgroup, stderr piped) → initialize → tools/list → cache)
  → timeout(120s): forward tools/call { name: "query", ... } with raw arguments
  → result passed through byte-faithfully
CC/OC ← tools/call response
```

## Data Flow: Discovery (Option C — one round-trip)

```
CC/OC → tools/list                       → 3 meta-tools, static descriptions (~600 tokens)
LLM   → list_tools()                     → rows of { server, tool, 1-line desc } (~1–2K tokens, compactable result)
LLM   → get_tool_schema("postgres__query") → one full schema (~100–500 tokens)
LLM   → invoke_tool(...)
```

Full schemas enter context only for tools actually used; the inventory enters as a tool *result*, not a permanent definition.

## Configuration Hierarchy

```
CLI args (--namespace, --config, --credential-store, --log-*)
    ↓ overrides
Config file (servers, namespaces, timeouts, descriptions)
    ↓ secrets resolved from
OS keychain (preferred backend) → process env (fallback)
```

No database. No authoritative persistent state beyond config + keychain. (v1.1 adds a *reconstructible* tool-list cache — deleting it is always safe.)

## Dependencies (Cargo.toml)

| Crate | Purpose | Notes |
|-------|---------|-------|
| `rmcp` (exact pin) | MCP server + client | `server`, `client`, `transport-child-process`, `transport-streamable-http`; **pin exact version, commit Cargo.lock** |
| `tokio` | Async runtime | `full` |
| `serde`, `serde_json` | Serialization | `derive` |
| `toml` | Config parsing | |
| `keyring` | OS credential store | |
| `clap` | CLI + subcommands | `derive` |
| `tracing`, `tracing-subscriber` | Logging + redaction layer | `json`, `fmt` |
| `process-wrap` (or `command-group`) | Cross-platform process-tree lifetime | Job Objects + process groups |
| `rpassword` (or similar) | Hidden stdin prompt for `cred set` | |
| `dirs` | Platform paths | |

CI: 3-OS matrix (Windows/macOS/Linux), `cargo audit`, `cargo deny`, integration tests against the in-repo probe server (no Node required).

## Platform Considerations

- **Windows (primary):** `cmd /c` wrapper for npm servers, **inside a Job Object** (kill-on-close) — the wrapper alone orphans `node.exe`. DPAPI keychain. `%APPDATA%\fanin-mcp\config.toml`.
- **macOS / Linux:** direct exec in a fresh process group. Keychain / Secret Service; env fallback covers headless Linux. `~/.config/fanin-mcp/config.toml`.
- All three OSes are release targets and CI-tested from day one.

## What This Architecture Is NOT

- **Not an HTTP gateway.** No network listener. Stdio only — a local per-session process proxy, not a service. (Contrast: McpMux runs a localhost daemon.)
- **Not a plugin system.** No middleware, hooks, or dynamic loading.
- **Not a credential manager UI.** `cred` subcommands + env vars only; consuming apps may layer UX on top via the same CLI.
- **Not multi-tenant.** One instance, one session, one user. Isolation is per-process.
- **Not a security boundary against same-user malware.** See SECURITY.md for the honest threat model.
