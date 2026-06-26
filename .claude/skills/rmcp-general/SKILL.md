---
name: rmcp-general
description: >-
  rmcp (official MCP Rust SDK) idioms for fanin-mcp — the exact-pin
  discipline, ServerHandler / RunningService usage, the dual server+client
  role, lazy-connection lock discipline, stdout-is-the-transport rule, and
  the bidirectional-traffic traps. Load when implementing, testing, or
  reviewing any code touching rmcp, the meta-tools, the upstream registry,
  process spawning, or the forward/reverse-traffic path.
---

# rmcp-general

fanin-mcp is an rmcp **server** (downstream, to CC/OC) and an rmcp **client**
(upstream, to N proxied servers) in one process. This skill captures the rmcp
specifics that base Rust competence does not cover. The repo's `AGG-MCP.md`
has fuller architecture; treat its code snippets as pseudocode (see below).

## The pin is law

rmcp's API has shifted across versions — trait signatures, capability
builders, transports. The repo policy (D-015, GOTCHA #23):

- `rmcp` is pinned **exactly** in `Cargo.toml` (`rmcp = "=x.y.z"`), and
  `Cargo.lock` is committed.
- Every code snippet in `AGG-MCP.md` and the other design docs is
  **pseudocode until verified against the pinned version.** Do not paste a
  doc snippet and trust it to compile.
- When a signature in your memory fights the compiler, the compiler wins —
  the pinned rmcp is the source of truth. Pull current signatures from
  Context7 (the project MCP) rather than guessing.
- Bumping the pin is a deliberate change with a changelog note, never an
  incidental `cargo update`.

Features in play: `server`, `client`, `transport-child-process`,
`transport-streamable-http`.

## stdout is the transport

Once `serve(stdio())` is running, **stdout is the JSON-RPC channel.** A single
`println!` / `dbg!` / `print!` to stdout corrupts the stream and the client
fails with garbled-JSON errors (GOTCHA #1).

- All diagnostics go to stderr or the log file via `tracing`.
- Child upstream stderr is **piped and redirected** to the log file with a
  `[server]` prefix — never inherited onto fanin-mcp's stderr, which CC
  surfaces to the user (GOTCHA #29).
- Never `{:?}`-print a resolved env map or anything that could hold a secret
  (GOTCHA #19) — the redaction layer covers `tracing` output, not stray prints.

## Meta-tools, static descriptions

The server exposes exactly three tools — `list_tools`, `get_tool_schema`,
`invoke_tool` — with **static** descriptions (D-002, D-003).

- Nothing on the `tools/list` path may connect to an upstream. CC sends
  `tools/list` at **every session start**; any upstream touch there destroys
  lazy loading and the <500ms init budget (GOTCHA #7).
- Prefer manual JSON-Schema construction (via `schemars`) for the meta-tool
  input schemas over the `#[tool]` macro — the manual path is what AGG-MCP
  specifies and keeps the three schemas under direct control.
- `invoke_tool` carries deliberately conservative annotations
  (`destructiveHint: true`, `openWorldHint: true`) so CC prompts rather than
  auto-allows — but annotations do **nothing** on OpenCode (GOTCHA #10), so
  the namespace ACL is the real permission layer, never annotations.

## RunningService and the lock discipline

Upstream connections are `Arc<RunningService<...>>` stored in the registry map.

- `RunningService` does **not** `Clone` by design — store `Arc<RunningService>`
  (GOTCHA #24).
- **Never hold the registry lock across an upstream call.** Lock only to
  get-or-insert the Arc; clone it; **drop the lock**; then `.await` the call.
  A lock held across `call_tool` serializes every tool call in the process — a
  60s upstream query would block a 100ms lookup (D-007, GOTCHA #16).
- Lazy connect: spawn/connect an upstream on the **first** targeting call, not
  at startup. Guard against double-spawn under racing first-calls with a
  per-server async init guard; re-check the map after acquiring it (GOTCHA #17).
- Use `list_all_tools()` for upstream discovery — `list_tools()` returns a
  single page and silently drops tools past it (GOTCHA #5).

## The reverse path — bidirectional traffic

MCP is bidirectional; an upstream can send requests **to** the proxy. An
unanswered one hangs that upstream forever (GOTCHA #2). This is Phase 1 work,
not polish (D-008).

- Declare **no** sampling/elicitation capabilities to upstreams. Spec-compliant
  servers then never send those requests.
- The upstream client handler must still answer everything that arrives:
  reject any sampling/elicitation request **instantly** with a structured
  error (never a hang), answer `roots/list` with an **empty list**, route
  upstream log notifications to the log file, and tolerate progress
  notifications.
- Capability-mirrored forwarding is v1.1 — clean rejection is the MVP contract.

## Errors stay in the conversation

Upstream failures return as `CallToolResult { isError: true }` with structured
JSON (`server`, `tool`, `code`, `message`, `recoverable`) — **never** as a
JSON-RPC `ErrorData` (D-005, GOTCHA #3).

- JSON-RPC errors are for protocol problems (bad params, unknown method) only.
- Tool-level failures must reach the model as readable content it can reason
  about and retry on — empirically verified on CC and OC.
- The error JSON shape is **public API**; changing it is a SemVer-major break.

## Passthrough fidelity

- `invoke_tool` arguments pass to the upstream as raw `serde_json::Value` —
  never parsed, validated, or transformed (D-004).
- Results pass back **byte-faithfully**, every content block type. Never
  `to_string()` a content array — it corrupts images/resources (GOTCHA #4).
- Tool-name parsing splits on the **first** `__` only (upstream tool names may
  contain `__`); server names are validated `[a-z0-9-]+` with `__` rejected at
  config load (GOTCHA #15).
- Sanitize upstream-provided names/descriptions (strip newlines/control chars,
  length-cap ~100 chars) before they enter any text the LLM reads — a
  prompt-injection channel by design (GOTCHA #20).

## Process and transport

- Spawn every upstream inside a Windows Job Object / Unix process group so a
  hard-kill of fanin-mcp leaves zero orphans (D-009). `cmd /c npx ...` on
  Windows orphans `node.exe` otherwise (GOTCHA #11); `Command::new("npx")`
  fails outright because `npx` is a `.cmd` (GOTCHA #12).
- Prefer the `process-wrap` / `command-group` abstraction. If rmcp's
  `TokioChildProcess` can't be wrapped, a thin custom child transport isolated
  in `process.rs` is the accepted fallback.
- Set each child's `current_dir` to the session's project root — directory-scoped
  upstreams (e.g. Morph) otherwise auto-detect the wrong tree (D-019, GOTCHA #30).

## Verifying against the pin

When unsure of a current rmcp signature: query Context7 for the pinned
version's docs, or read the rmcp source the lockfile resolves to — do not
implement against remembered or doc-snippet signatures. The fastest path out
of a compiler fight is the real API, not another guess.
