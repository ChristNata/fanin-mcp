# fanin-mcp — Decision Log

Lightweight ADRs. Each entry: what was decided, why, what was rejected, and when to revisit. New decisions are appended, never edited — superseded entries get a status change and a pointer.

All entries below: **Status: Accepted · Date: 2026-06** (initial design review).

---

## D-001 — Per-session stdio process; no daemon, no listener

**Decision:** fanin-mcp is spawned by the client per session over stdin/stdout and dies on EOF. No background process, no ports, no shared state between sessions.
**Why:** Kills the complexity class that drowns gateway-style aggregators: listener auth, lifecycle management, multi-tenancy, port conflicts. Teardown is free (EOF).
**Rejected:** Localhost daemon/gateway (McpMux's model) — better for GUI products, wrong for a composable CLI primitive.
**Consequence (accepted cost):** Concurrent sessions each spawn their own upstream instances; upstreams holding exclusive resources may conflict (documented limitation; `singleton` warning in v1.2).

## D-002 — Three meta-tools with progressive disclosure

**Decision:** Expose `list_tools` / `get_tool_schema` / `invoke_tool` instead of re-exporting upstream tools.
**Why:** Bounded, client-agnostic context cost (~600 tokens of permanent definitions); inventories and schemas enter context on demand as compactable tool *results*.
**Rejected:** 1:1 tool re-export (re-creates the bloat problem and requires schema synchronization); search/execute style discovery (npm `mcpmux`, forgemax) — heavier, and ranking adds a failure mode.
**Consequence (accepted cost):** One extra LLM round-trip per session for discovery; client per-tool permission prompts collapse (see D-006).

## D-003 — Static meta-tool descriptions; no startup fan-out

**Decision:** Meta-tool descriptions are static. The aggregator never connects to upstreams just to build description text. Optional per-server `description` config field enriches `list_tools` *results*. Auto-generated descriptions return in v1.1 via a reconstructible disk cache.
**Why:** The original spec (PRD Req 8) required connecting to every upstream on the client's first `tools/list` — which CC sends at **every session start**, silently destroying lazy connections, the <500ms init metric, and the idle-memory target. The three docs contradicted each other; this resolves it in favor of laziness.
**Rejected:** Eager fan-out (the contradiction); hand-written summaries as the *only* mechanism (kept as optional field); cache-in-MVP (deferred — not needed to ship).
**Revisit:** v1.1 cache design. Amended principle: no *authoritative* persistent state; reconstructible caches are permitted.

## D-004 — Raw argument passthrough; byte-faithful results

**Decision:** `invoke_tool` arguments pass to upstreams as raw `serde_json::Value` — never parsed, validated, or transformed. Results (all content block types) return unmodified.
**Why:** Protocol-agnostic, zero maintenance when upstream schemas change, no corruption of non-text content.
**Rejected:** Proxy-side schema validation ("helpful" errors aren't worth the drift risk and maintenance).

## D-005 — Errors as `CallToolResult { isError: true }`, never JSON-RPC errors

**Decision:** All upstream failures return structured JSON (`server`, `tool`, `code`, `message`, `recoverable`) inside a tool result.
**Why:** Keeps the error in the conversation where the LLM can read it, reason, and retry. JSON-RPC errors surface as opaque failures. **Empirically verified on both CC and OC** that such results reach the model readable and parseable.
**Consequence:** The error JSON shape is public API (SemVer-major to break).

## D-006 — Namespace ACL is the primary permission layer

**Decision:** Namespaces (server + tool allow-lists, `--namespace` per session) are the real access control. `invoke_tool` carries deliberately conservative annotations (`destructiveHint: true`, `openWorldHint: true`).
**Why:** The meta-tool indirection collapses client per-tool prompts — approving `invoke_tool` once approves the whole namespace. Worse, OC ignores annotations entirely (verified by probe), so client-side gating can't be relied on at all there. Argument-level safety (e.g. SELECT vs DROP) was always the upstream's job — no client permission model sees inside arguments.
**Rejected:** Pretending client prompts still work; building a proxy-side argument firewall (parameter-level ACL is a permanent non-goal).
**Follow-up:** `readonly = true` per-server enforcement in v1.1.

## D-007 — Lazy connections with per-server init guards; Arc-clone-then-drop-lock

**Decision:** Upstreams spawn on first targeting call. Connections stored as `Arc<RunningService>`; registry locks held only for map access — never across an upstream call. Per-server async guard prevents double-spawn on racing first-calls.
**Why:** The original reference pattern (`registry.lock().await.invoke(...)`) held one mutex across the entire upstream call, serializing every tool call in the process — a 60s query would block a 100ms lookup.
**Consequence:** Concurrency test in MVP checklist (slow upstream must not block siblings).

## D-008 — Upstream-originated requests: clean reject in MVP, mirror-forward in v1.1

**Decision:** MVP declares **no** sampling/elicitation capabilities to upstreams (spec-compliant servers then never send those requests), instantly rejects any that arrive, answers `roots/list` with an empty list, and logs upstream notifications. Capability-mirrored forwarding is v1.1.
**Why:** An unanswered upstream request hangs that server forever — so *some* handler is mandatory from the first real connection (Phase 1, not polish). Forwarding adds ~3–5 days plus a testing matrix the primary client (CC, no sampling support) can't exercise; deferred.
**Rejected:** Doing nothing (hangs); unconditional forwarding (ignores capability negotiation — the client may not support the request).
**Consequence:** Upstreams *requiring* sampling/elicitation are unsupported until v1.1 (documented).

## D-009 — Windows Job Objects + Unix process groups for process-tree lifetime

**Decision:** Every spawned upstream lives in a Windows Job Object (kill-on-close) / Unix process group; teardown and crashes kill the whole tree. Prefer the `process-wrap`/`command-group` abstraction; a thin custom child transport is acceptable if rmcp's `TokioChildProcess` can't be wrapped.
**Why:** `cmd /c npx` on Windows creates a tree where killing `cmd.exe` orphans `node.exe` — the classic MCP-on-Windows zombie complaint, on our primary platform. Polite stdin-close alone doesn't survive aggregator crashes.
**Rejected:** Graceful-teardown-only (crash-unsafe); resolving `npx.cmd` to dodge the wrapper (fragile path-guessing).
**Consequence:** Hard-kill orphan test is a CI release gate on all OSes.

## D-010 — Credentials: keychain-first with env fallback; `cred` subcommands; never argv, never logs

**Decision:** Secrets live in the OS keychain; `config.toml` holds only `${PLACEHOLDERS}`. Resolution: preferred backend (`--credential-store`, default keyring) → process env → error. `cred set` reads values from a hidden stdin prompt; `cred list` prints names only. A redaction layer plus an automated sentinel test guarantee secrets never reach logs. Each upstream receives only its own vars.
**Why:** Without `cred set`, no standard cross-platform keyring CLI exists — users literally couldn't use the default backend. Argv is visible in process listings and shell history. Env fallback covers headless Linux/CI where Secret Service is absent.
**Rejected:** "Use a keyring CLI" (doesn't exist usably); secrets-in-config (the status quo this project exists to kill).
**Honesty clause:** The keychain does not protect against same-user malware; SECURITY.md says so explicitly.

## D-011 — OAuth deferred to v1.1; static header injection in MVP

**Decision:** Remote upstream auth in MVP = static headers from the credential store (`Authorization = "Bearer ${TOKEN}"`). OAuth 2.1 (browser flow, PKCE, refresh, mid-session 401 recovery) ships as an out-of-band `auth` subcommand in v1.1.
**Why:** OAuth is 2–4 weeks with per-provider quirks (Linear, Notion, Atlassian), and the flow can't run mid-session under a headless stdio process anyway. Static headers cover API keys, PATs, and context7 today. The credential store already has the right shape for tokens — deferral costs no redesign.

## D-012 — Per-server `timeout_secs` (default 60s) + cancellation forwarding

**Decision:** Every *blocking upstream await* wrapped in a per-server-configurable timeout returning structured `upstream_timeout` — the connect/`initialize` handshake, the initial `list_all_tools` discovery, the `list_changed` dirty-refetch, and the `tools/call` itself (not only tool calls). A connect-time expiry caches no entry, releases the per-server init guard, and drops the containment guard to reap the half-connected child. Client cancellation notifications forwarded to in-flight calls. Progress-based idle timeout is v1.1.
**Why:** A flat global timeout is wrong in both directions (big DB queries vs fast doc lookups). Clients run their own MCP timeouts (e.g. CC's `MCP_TOOL_TIMEOUT`); ours exists to fail *informatively* first and free resources — documented interplay.
**Rejected:** No timeout (hung upstreams hold resources silently); dynamic-only (many servers never emit progress).

## D-013 — PRD Req 9 (transparent passthrough of unknown methods) deleted

**Decision:** Removed entirely.
**Why:** "Forward to the first upstream with a matching capability" is arbitrary routing; capability negotiation means clients don't send undeclared methods anyway. It was forward-compat theater and absent from the architecture and plan regardless.

## D-014 — All three OSes from day one

**Decision:** Windows 10+, macOS 12+, Linux are release targets with a CI matrix from Phase 5 onward.
**Why:** Rust makes it ~1–2 days of marginal cost (keyring/dirs/tokio abstract the platforms; the hard platform work — Job Objects — is Windows, which we do anyway). Retrofitting costs more.

## D-015 — Rust + rmcp, exact-pinned

**Decision:** Official `rmcp` SDK, pinned to an exact version with `Cargo.lock` committed; all doc snippets treated as pseudocode until verified against the pin.
**Why:** Single static binary (no Node runtime — a differentiator vs the TS aggregators), server+client roles in one crate, official spec tracking. rmcp's API has moved fast; the pin policy prevents implementing agents from fighting the compiler with stale signatures.
**Rejected:** TypeScript SDK (runtime dependency contradicts "no runtime dependencies"); hand-rolled JSON-RPC (protocol-tracking burden).

## D-016 — In-repo probe server fixture

**Decision:** A tiny rmcp test server (`echo_ok`, `always_error`, `slow_tool`, `dangerous_noop`, `needs_sampling`) lives in the workspace and backs all integration tests.
**Why:** CI on three OSes can't depend on Node/npx/real databases. The same probe design was field-tested manually against OpenCode before any aggregator code existed — it's already paid for itself (produced D-005's and D-006's empirical findings).

## D-017 — Name: `fanin-mcp`; license: MIT OR Apache-2.0

**Decision:** Public name `fanin-mcp` (fan-in: many inputs, one gate). Dual permissive license, Rust-ecosystem standard.
**Why:** `mcpmux` is taken twice (a desktop gateway *and* an npm aggregator with a near-identical meta-tool concept); `plexus` and `backplane` are squatted crates. Dual licensing adds Apache's patent grant to MIT's compatibility at zero cost.
**Resolved (oss-readiness cycle):** `[package]` metadata in place (`repository`, `homepage`, `readme`, `keywords`, `categories`) with `publish = true`; SECURITY.md contact is GitHub Security Advisories (private reporting — no email). **Open:** a final `cargo publish --dry-run` smoke remains a release-checklist step before the first publish.

## D-018 — Standalone product, zero knowledge of consumers

**Decision:** fanin-mcp knows nothing about any application that may bundle it. All integration surface is CLI args, the config file, stdio, and log files.
**Why:** Keeps the OSS product honest and reviewable; downstream products (sidecars, GUIs) layer on top of the same public surface every user gets.

## D-019 — Morph fast-apply verified as a plain request/response upstream (no forwarding dependency)

**Decision:** The Morph MCP server (`@morphllm/morphmcp`, a launch-list upstream) is confirmed compatible with MVP fanin-mcp with **no new logic**. Verified against Morph's official docs (2026-06).
**Findings:**
- Transport/auth: stdio, npx-launched, `MORPH_API_KEY` env var. The most standard upstream shape — env-var injection (D-010), child-process spawn (D-009), no OAuth.
- Tools: `edit_file` (Morph Apply), `codebase_search` (Warp-Grep), `github_codebase_search`; gated by `ALL_TOOLS` / `DISABLED_TOOLS` env flags.
- Interaction shape: **request → response only.** The original premise — "Morph returns edited content for the caller to write back" — is slightly off: Morph **reads and writes files itself on disk** (it's launched with a directory scope), and returns a result summary. Either way it is a normal tool call. It does **not** send upstream-originated requests (`sampling`/`elicitation`/`roots`) back up the chain; the fast-apply merge runs on Morph's *own* hosted model, not the caller's. So it rides the existing `invoke_tool` raw-passthrough (D-004) + byte-faithful result path with nothing added, and has **no dependency on the deferred capability-mirrored forwarding (D-008)**.
**Consequences (design notes, not blockers):**
- Morph operates on a **directory**, and as a spawned child it inherits *fanin-mcp's* working directory. The child's `current_dir` must be set to the session's project root (or the path passed explicitly in `args`), or Morph's workspace auto-detection can target the wrong tree. See GOTCHA #30.
- With `ALL_TOOLS: "true"`, Morph is a general read/write filesystem server, not just `edit_file`. This interacts with the namespace permission model (D-006): a "read-only" namespace containing Morph-with-ALL_TOOLS is not actually read-only. Scope via the per-server tool-filter (expose only `edit_file`) or run it in `ALL_TOOLS: "false"` edit-only mode. Noted in SECURITY.md.
**Rejected:** Pulling forwarding into MVP for Morph's sake (unnecessary — Morph never calls back up).
