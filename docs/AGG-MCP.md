# AGG-MCP — Working with the rmcp SDK for MCP Aggregation

> **⚠️ Treat all code snippets in this document as pseudocode.** rmcp's API surface has moved fast across versions (trait signatures, capability builders, transport construction). Before implementation: pin an **exact** rmcp version in Cargo.toml, commit `Cargo.lock`, and verify every snippet against the pinned version's docs/source. Do not fight the compiler with signatures from this document.

Practical knowledge for building an MCP aggregator/proxy in Rust using the official `rmcp` SDK: API surface, transport mechanics, the dual server+client pattern, bidirectional traffic, and proxy-specific gotchas.

## SDK Overview

**Crate:** `rmcp` (crates.io, MIT) · **Repo:** `github.com/modelcontextprotocol/rust-sdk` · **Version line:** 1.x (migration from 0.x: repo discussions #716) · **Spec:** MCP 2025-11-25

Workspace crates: `rmcp` (protocol, transports, handler traits), `rmcp-macros` (`#[tool]`, `#[tool_box]`, `#[prompt]`).

### Feature Flags

```toml
[dependencies]
rmcp = { version = "=1.x.y", features = [   # exact pin
    "server",                       # ServerHandler, serve() for server role
    "client",                       # ClientHandler, serve() for client role
    "transport-child-process",      # TokioChildProcess
    "transport-streamable-http",    # StreamableHttpClientTransport
] }
```

A proxy needs BOTH `server` and `client` — it is a server to CC/OC and a client to each upstream.

## Core Concepts

### ServiceExt and serve()

```rust
use rmcp::ServiceExt;

// Server side (downstream — CC/OC talks to us):
let server = my_handler.serve(stdio()).await?;

// Client side (upstream — we talk to an MCP server):
let client = my_client_handler.serve(TokioChildProcess::new(command)?).await?;
```

`.serve()` performs the initialize handshake and returns a `RunningService<Role>`.

### ServerHandler — the Downstream Server

```rust
impl ServerHandler for AggServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            name: "fanin-mcp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_tools_list_changed()
                .build(),
            ..Default::default()
        }
    }

    async fn list_tools(&self, _req: Option<PaginatedRequestParams>, _ctx: RequestContext<RoleServer>)
        -> Result<ListToolsResult, ErrorData>
    {
        // 3 meta-tools with STATIC descriptions (Option C — no upstream fan-out at startup)
        Ok(ListToolsResult { tools: vec![
            self.make_list_tools_tool(),
            self.make_get_tool_schema_tool(),
            self.make_invoke_tool_tool(),
        ], next_cursor: None })
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>)
        -> Result<CallToolResult, ErrorData>
    {
        match request.name.as_str() {
            "list_tools" => self.handle_list_tools(request.arguments).await,
            "get_tool_schema" => self.handle_get_tool_schema(request.arguments).await,
            "invoke_tool" => self.handle_invoke_tool(request.arguments).await,
            _ => Err(ErrorData::method_not_found()),
        }
    }
}
```

**Client capabilities at initialize (v1.1).** The `initialize` request carries the client's declared capabilities (sampling/elicitation/roots). MVP doesn't need them (clean-reject only); the v1.1 forwarding feature will — note where your pinned rmcp version exposes them (initialize handler / peer info accessor).

### Meta-Tool Construction (manual, not macros)

Manual `Tool` structs are preferred over `#[tool]` macros here: `invoke_tool`'s arguments are fully dynamic (`serde_json::Value`), and you want full control over error formatting.

```rust
fn make_invoke_tool_tool(&self) -> Tool {
    Tool {
        name: "invoke_tool".into(),
        description: Some("Call a tool by server__tool name with arguments (e.g. postgres__query).".into()),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["name", "arguments"],
            "properties": {
                "name": { "type": "string", "description": "server__tool format" },
                "arguments": { "type": "object" }
            }
        }).as_object().unwrap().clone().into(),
        // Deliberately conservative — annotation-aware clients (CC) should prompt,
        // never auto-allow. (OpenCode ignores annotations entirely — verified.)
        annotations: Some(ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(true),
            idempotent_hint: Some(false),
            open_world_hint: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}
```

## The Proxy Pattern: Server + Client in One Process

One process holds one `RunningService<RoleServer>` (CC/OC over stdio) and N lazily-created `RunningService<RoleClient>` (one per upstream).

### Registry — CORRECT locking pattern

**Never hold the registry lock across an upstream call.** The original draft of this doc locked the whole registry for the duration of `invoke` — that serializes every tool call in the process behind one mutex (a 60s postgres query blocks a 100ms docs lookup). The correct pattern:

```rust
struct Registry {
    configs: HashMap<String, UpstreamConfig>,
    connections: tokio::sync::RwLock<HashMap<String, Arc<RunningService<RoleClient>>>>,
    init_guards: HashMap<String, Arc<tokio::sync::Mutex<()>>>,  // one per configured server
    tool_cache: tokio::sync::RwLock<HashMap<String, Vec<Tool>>>,
}

impl Registry {
    async fn get_or_connect(&self, name: &str) -> Result<Arc<RunningService<RoleClient>>, AggError> {
        // Fast path: brief read lock, clone the Arc, drop the lock.
        if let Some(c) = self.connections.read().await.get(name) {
            return Ok(c.clone());
        }
        // Slow path: per-SERVER guard (not the global map lock) prevents double-spawn
        // when two first-calls race; other servers stay unaffected.
        let guard = self.init_guards.get(name)
            .ok_or_else(|| AggError::tool_not_found(name, ""))?.clone();
        let _g = guard.lock().await;
        if let Some(c) = self.connections.read().await.get(name) {  // re-check
            return Ok(c.clone());
        }
        let client = Arc::new(self.spawn_and_init(name).await?);    // creds, transport, tools/list cache
        self.connections.write().await.insert(name.into(), client.clone());
        Ok(client)
    }

    async fn invoke(&self, server: &str, tool: &str, arguments: Option<serde_json::Value>, timeout: Duration)
        -> Result<CallToolResult, AggError>
    {
        let client = self.get_or_connect(server).await?;   // NO lock held beyond this line
        tokio::time::timeout(timeout, client.call_tool(CallToolRequestParams {
            name: tool.into(),
            arguments: arguments.and_then(|v| v.as_object().cloned()).map(Into::into),
            meta: None,
        }))
        .await
        .map_err(|_| AggError::upstream_timeout(server, tool))?
        .map_err(|e| AggError::upstream_unavailable(server, &e.to_string()))
    }
}
```

Notes: `timeout` comes from the per-server `timeout_secs` config (default 60s). Pass `arguments` through as raw JSON — never parse/validate/transform upstream tool arguments. Return upstream results byte-faithfully (text, images, embedded resources, structuredContent).

## Transport Layer

### Stdio (downstream)

```rust
let server = my_handler.serve(rmcp::transport::stdio()).await?;
```

**Once `serve(stdio())` runs, stdout belongs to the protocol.** All logging → stderr or file.

### TokioChildProcess (upstream stdio) + process-tree lifetime

```rust
let transport = TokioChildProcess::new(Command::new("npx").configure(|cmd| {
    cmd.arg("-y").arg("@modelcontextprotocol/server-filesystem@1.2.3")  // pin versions
       .env("MY_SECRET", resolved_value);   // ONLY this server's secrets
}))?;
```

**Windows:** npm servers need `Command::new("cmd").args(["/c", "npx", ...])` — and `cmd /c` creates a tree where killing `cmd.exe` orphans `node.exe`. **Required mitigation:** assign the child to a Job Object with kill-on-close (`#[cfg(windows)]`), so dropping the handle — even on aggregator crash — kills the whole tree. **Unix:** spawn in a fresh process group (`setsid`) and kill the group. The `process-wrap` / `command-group` crates (watchexec) abstract both behind one API; since `TokioChildProcess` constructs the command internally, integrating may require a thin custom child-process transport — keep it isolated in `process.rs`.

**stderr:** request a piped stderr, read it line-by-line, prefix `[server]`, write to the log file. Never let upstream stderr mix into the aggregator's own stderr (CC surfaces it as noise).

### StreamableHttpClientTransport (remote upstreams)

```rust
let transport = StreamableHttpClientTransport::new("https://mcp.context7.com/mcp")?;
// Inject STATIC auth headers resolved from the credential store (Authorization = "Bearer ${TOKEN}").
// OAuth 2.1 flows are deliberately out of MVP scope (v1.1 `auth` subcommand).
```

SSE transport is deprecated by the spec; prefer Streamable HTTP.

## Bidirectional Traffic: Upstream-Originated Requests (MVP: clean reject)

MCP upstreams send requests **to their client** — which is the aggregator: `sampling/createMessage`, `elicitation/create`, `roots/list`, plus `notifications/message` (logging) and progress notifications. **An unanswered request hangs the upstream forever** and surfaces to you as a mysterious tool-call timeout. Handle from the first upstream connection (Phase 1), not in polish.

**MVP design — clean reject:**
1. Declare **no** sampling/elicitation capabilities in the aggregator's client info when connecting upstreams — spec-compliant servers then never send those requests at all.
2. Reject any that arrive anyway with an immediate structured error; answer `roots/list` with an empty list; route logging notifications to the log file; tolerate progress notifications.
3. Documented limitation: upstreams that *require* sampling/elicitation are unsupported until v1.1.

**v1.1 design — capability mirroring (reference for later):** record the downstream client's capabilities at `initialize`, declare the same to upstreams, forward declared-capability requests downstream and relay the response back. The reject arm below stays as the fallback; the forwarding arm slots in beside it:

```rust
// Pseudocode — exact handler method names vary by rmcp version; verify against the pin.
impl ClientHandler for UpstreamClientHandler {
    async fn create_message(&self, params: CreateMessageRequestParams, _ctx: ...)
        -> Result<CreateMessageResult, ErrorData>
    {
        match &self.downstream {
            Some(peer) if self.client_caps.sampling => {
                // rmcp correlates requests per-connection — just await and relay.
                peer.create_message(params).await
            }
            _ => Err(ErrorData::new(/* clean structured rejection */
                "client does not support sampling; tool cannot be served through this proxy")),
        }
    }

    async fn list_roots(&self, _ctx: ...) -> Result<ListRootsResult, ErrorData> {
        match &self.downstream {
            Some(peer) if self.client_caps.roots => peer.list_roots().await,
            _ => Ok(ListRootsResult { roots: vec![] }),   // empty, never hang
        }
    }

    async fn on_logging_message(&self, msg: ..., _ctx: ...) {
        tracing::info!(server = %self.server_name, "upstream log: {:?}", msg); // → log file
    }

    async fn on_tool_list_changed(&self, _ctx: NotificationContext<RoleClient>) {
        let _ = self.tool_change_tx.send(self.server_name.clone()).await;      // invalidate cache
    }
    // Accept progress notifications without crashing; forwarding is v1.1.
}
```

MVP consequence: the `peer.create_message(...)` forwarding branch is v1.1 — in MVP every sampling/elicitation request takes the rejection branch, and `roots/list` always returns empty. Either way, no request goes unanswered.

### Sending notifications to the client

```rust
context.peer.notify_tool_list_changed().await?;   // from within a ServerHandler method
```

## Lifecycle and Shutdown

1. CC/OC closes stdin → rmcp stdio transport sees EOF → `server.waiting().await` returns
2. Drop all upstream `RunningService<RoleClient>` handles
3. Process module guarantees full-tree teardown: close child stdin (polite), brief grace, then Job-Object/process-group kill (forceful)
4. Exit

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let running = agg_server.serve(stdio()).await?;
    let _reason = running.waiting().await?;
    registry.shutdown_all().await;   // explicit, tree-killing teardown
    Ok(())
}
```

## Error Handling Patterns

**Upstream errors never crash the aggregator, and never become JSON-RPC errors.** Return structured JSON in `CallToolResult { is_error: true }` so the error stays in the conversation where the LLM can read `code`/`recoverable` and decide what to do. *Empirically verified on both CC and OC:* such results reach the model as readable content with the JSON intact and parseable.

```rust
async fn handle_invoke_tool(&self, args: serde_json::Value) -> Result<CallToolResult, ErrorData> {
    let name = args.get("name").and_then(|v| v.as_str())
        .ok_or_else(|| ErrorData::invalid_params("missing 'name'", None))?;
    let (server, tool) = name.split_once("__")            // FIRST "__" only —
        .ok_or_else(|| ErrorData::invalid_params("expected server__tool", None))?;  // tool names may contain __

    if !self.namespace.is_tool_allowed(server, tool) {
        return Ok(agg_error_result(AggError::namespace_denied(server, name, &self.namespace.name)));
    }
    match self.registry.invoke(server, tool, args.get("arguments").cloned(),
                               self.registry.timeout_for(server)).await {
        Ok(result) => Ok(result),                          // byte-faithful passthrough
        Err(e) => Ok(agg_error_result(e)),                  // is_error: true, JSON body
    }
}
```

(Server names are pre-validated at config load to `[a-z0-9-]+` with `__` rejected, so first-`__` splitting is unambiguous.)

## Sanitization of Upstream Strings

Anything an upstream provides that ends up in text the LLM reads (tool names, descriptions in `list_tools` rows, schema annotation strings in `get_tool_schema`) is a prompt-injection channel. Control-neutralization (strip C0/C1/DEL/Unicode separators/bidi/zero-width/BOM → space, single-line) is **display-wide** — applied to both `list_tools` rows and `get_tool_schema` annotation strings (`title`/`description`/`$comment`/`markdownDescription`). The ~100-char **length-cap is a `list_tools` row control ONLY**; `get_tool_schema` annotations are relayed **full-length** after neutralization (so real argument docs are not silently truncated). Tool-name identifiers are length-capped at 200 (defense-in-depth). Schema *validation* strings (`enum`/`const`/`default`/`pattern`) and `invoke_tool` arguments + result content pass through **verbatim** by design (D-004) — the residual, bounded channel. This bounds — not eliminates — prompt injection; LLMs read tool descriptions by design. Documented in SECURITY.md and GOTCHA #20.

## Key rmcp Types Reference

| Type | Use |
|------|-----|
| `ServerHandler` / `ClientHandler` | Downstream server / upstream client traits |
| `RunningService<RoleServer/RoleClient>` | Live connection handles (`Arc`-wrap to share; not `Clone`) |
| `ServiceExt::serve()` | Handshake + handle |
| `ServerInfo`, `ServerCapabilities` | Identity + capability flags (no `tools` capability → clients never call you) |
| `Tool`, `ToolAnnotations` | Tool defs; annotations matter on CC, ignored by OC |
| `CallToolRequestParams`, `CallToolResult`, `Content` | Tool call I/O — pass content through byte-faithfully |
| `ErrorData` | JSON-RPC-level errors only (method not found, bad params); tool-level failures → `CallToolResult{is_error}` |
| `RequestContext` / `NotificationContext` | Peer handles for notifications/forwarding |
| `TokioChildProcess`, `StreamableHttpClientTransport` | Upstream transports |
| `list_all_tools()` | Pagination-handling tool fetch (use for upstream discovery, not `list_tools()`) |

## Common Pitfalls

1. **stdout is the protocol.** All logging to stderr/file after `serve(stdio())`.
2. **Windows npm = `cmd /c` = orphan risk.** Job Objects are mandatory, not optional.
3. **`RunningService` is not `Clone`.** Store `Arc<RunningService>`; lock maps briefly, never across calls (see Registry).
4. **Capability negotiation gates everything** — both directions: declare `tools` downstream or clients never call you; declare **no** sampling/elicitation upstream (MVP) so servers don't send requests you won't serve.
5. **Unanswered upstream requests hang silently.** Wire the `ClientHandler` dispatch before connecting any real server.
6. **`ErrorData` vs `is_error`.** Protocol errors vs tool errors — keep tool errors in the conversation.
7. **rmcp drift.** Exact pin, committed lockfile, snippets-are-pseudocode.
8. **`list_all_tools()` for discovery** (handles pagination), not single-page `list_tools()`.

## Reference & Competitor Projects

- **`postrv/forgemax`** — Rust/rmcp aggregator, stdio, multi-backend, search/execute meta-tools. Closest architectural reference.
- **`stephenlacy/rmcp-proxy`** — minimal server+client-in-one-process pattern.
- **`modelcontextprotocol/rust-sdk/examples/`** — official examples for every MCP feature.
- **`mcpmux` (npm)** — TypeScript meta-tool aggregator (discover/call pattern, ~97% claimed token reduction). Direct conceptual competitor; differentiate on native binary, namespaces, keychain creds, structured errors.
- **McpMux (desktop app, mcpmux.com)** — daemon/gateway + GUI + registry. Different architecture (localhost service vs per-session stdio); overlapping configure-once pitch.
- **`metatool-ai/mcp-server-metamcp`** — TS aggregator UX reference.
