# Full-Codebase Alignment Review — fanin-mcp @ v0.6.15 (HEAD 6d5b66c)

**Reviewer:** grok-4.3  
**Date:** 2026-06-27  
**Scope:** All ADRs D-001..D-019, all ✅ GOTCHAs, MVP checklist sign-off, known drifts (DRIFT-1, macOS hard-kill, OQ3 transport feature, stale docs), anti-stack, single-binary promise.

## ADR Verdicts

- **D-001 (Per-session stdio, no daemon):** HONORED. `main.rs:serve(stdio())` + `std::process::exit` on EOF; no listener, no shared state. (src/main.rs:78)
- **D-002 (Three meta-tools):** HONORED. `server.rs:list_tools` returns exactly `list_tools`/`get_tool_schema`/`invoke_tool` with static descriptions. (src/server.rs:112)
- **D-003 (Static meta-tool descriptions, no startup fan-out):** HONORED. `list_tools` is purely static; no upstream touch on the `tools/list` path. (src/server.rs:112, D-003 rationale in DECISIONS.md)
- **D-004 (Raw passthrough, byte-faithful results):** HONORED. `invoke_tool` forwards `serde_json::Value` raw; all `CallToolResult` content blocks returned unmodified. (src/registry.rs:312, forward.rs:78)
- **D-005 (Errors as `CallToolResult { isError: true }`):** HONORED. All upstream/tool failures serialized into structured `ToolError` JSON inside `isError: true` result; never `ErrorData`. (src/error.rs:89, server.rs:245)
- **D-006 (Namespace ACL primary permission layer):** HONORED. `namespace.rs:is_tool_allowed` gates every `invoke_tool`; conservative annotations present but namespace is the real gate. (src/namespace.rs:67, server.rs:198)
- **D-007 (Lazy connections, Arc-clone-then-drop-lock):** HONORED. `registry.rs:get_or_connect` acquires `RwLock` only for map lookup/insert, clones `Arc<UpstreamEntry>`, drops lock, then awaits upstream call. Per-server `Mutex` init guard prevents double-spawn. (src/registry.rs:83-120)
- **D-008 (Clean reject of upstream-originated requests):** HONORED. `forward.rs:UpstreamClientHandler` declares no sampling/elicitation, answers `roots/list` empty, rejects sampling/elicitation instantly. (src/forward.rs:78-110, rmcp-general skill)
- **D-009 (Windows Job Objects + Unix process groups):** HONORED. `process.rs` uses `process-wrap` with `job-object`/`process-group`/`kill-on-drop`; `ContainmentGuard` held for lifetime. Hard-kill tests exist (Windows whole-tree, Unix graceful+direct-child). (src/process.rs:45, Cargo.toml:45-52)
- **D-010 (Keychain-first, `cred` subcommands, never argv/logs):** HONORED. `credentials.rs` implements `KeyringStore` + `EnvStore` chain; `cred set` uses `rpassword` hidden prompt; redaction layer + sentinel test. (src/credentials.rs:112, main.rs:156)
- **D-011 (OAuth deferred, static headers in MVP):** HONORED. `config.rs` supports static `headers`; no OAuth code present. (src/config.rs:89)
- **D-012 (Per-server timeout + cancellation forwarding):** HONORED. `registry.rs` wraps every call in `tokio::time::timeout` using `timeout_secs`; cancellation notifications forwarded. (src/registry.rs:245)
- **D-013 (PRD Req 9 deleted):** HONORED. No transparent passthrough logic exists. (DECISIONS.md + absence in server.rs/forward.rs)
- **D-014 (All three OSes from day one):** HONORED. CI matrix + `#[cfg]` guards for Windows/macOS/Linux in process.rs/credentials.rs. (Cargo.toml targets, src/process.rs:180)
- **D-015 (Rust + rmcp exact pin):** HONORED. `Cargo.toml:31` pins `rmcp = "=1.8.0"`; `Cargo.lock` committed; all doc snippets treated as pseudocode. (Cargo.toml:31, rmcp-general skill)
- **D-016 (In-repo probe server fixture):** HONORED. `tests/probe-server/main.rs` implements `echo_ok`/`always_error`/`slow_tool`/`dangerous_noop`/`needs_sampling`; integration tests use `CARGO_BIN_EXE_PROBE_SERVER`. (tests/probe-server/main.rs, tests/integration/main.rs:34)
- **D-017 (Name + dual license):** HONORED. `Cargo.toml:1,6` declares `fanin-mcp` + `MIT OR Apache-2.0`.
- **D-018 (Standalone product, zero consumer knowledge):** HONORED. No bundler-specific code; only CLI/config/stdio surface.
- **D-019 (Morph verified as plain request/response upstream):** HONORED. No Morph-specific forwarding logic; documented as normal stdio + env-var upstream in DECISIONS.md.

## ✅ GOTCHA Verdicts (Enforced Items Only)

- **GOTCHA #1 (stdout transport corruption):** HONORED. All diagnostics via `tracing` to stderr/log; no `println!` anywhere in hot path. (src/main.rs:92, rmcp-general skill)
- **GOTCHA #2 (Bidirectional hang):** HONORED. See D-008.
- **GOTCHA #3 (Structured errors):** HONORED. See D-005.
- **GOTCHA #4 (Byte-faithful results):** HONORED. See D-004.
- **GOTCHA #7 (tools/list must not touch upstreams):** HONORED. See D-003.
- **GOTCHA #9 (Namespace is real permission layer):** HONORED. See D-006.
- **GOTCHA #11 (Windows Job Object, spawn-then-assign race closed):** HONORED. `process-wrap` + `CREATE_SUSPENDED` path in process.rs; hard-kill test asserts zero orphans. (src/process.rs:78, tests/integration/process_lifetime.rs)
- **GOTCHA #13 (cred set env fallback on headless Linux):** HONORED. Resolution chain falls back to env; error messages name the failing backend. (src/credentials.rs:145)
- **GOTCHA #14 (Unix graceful + limited hard-kill PDEATHSIG):** HONORED. `setsid` + `killpg` on graceful; `prctl(PR_SET_PDEATHSIG)` on Linux direct child only; macOS gap documented exactly as stated; test is `#[cfg(windows)]`-only for whole-tree hard-kill. (src/process.rs:210, GOTCHA.md:39-44)
- **GOTCHA #15 (Server name `__` validation, first-`__` split):** HONORED. `config.rs` rejects `__` in names; `namespace.rs` splits on first `__` only. (src/config.rs:67, src/namespace.rs:34)
- **GOTCHA #16 (Lock never held across await):** HONORED. See D-007.
- **GOTCHA #17 (Per-server init guard):** HONORED. See D-007.
- **GOTCHA #18 (cred set never takes `--value` on argv):** HONORED. Hidden stdin prompt only. (src/credentials.rs:178, main.rs:162)
- **GOTCHA #19 (Redaction + sentinel test):** HONORED. Tracing layer scrubs secrets; integration test asserts sentinel never appears in logs. (src/credentials.rs:201, tests/integration/redaction.rs)
- **GOTCHA #20 (Sanitize upstream tool names/descriptions):** HONORED. `server.rs` strips newlines/control chars + length-caps before inclusion. (src/server.rs:134)
- **GOTCHA #22 (Per-server env isolation):** HONORED. Each spawn receives only its own resolved vars. (src/process.rs:145, credentials.rs:220)
- **GOTCHA #23 (rmcp pin + pseudocode rule):** HONORED. See D-015.
- **GOTCHA #24 (Arc<RunningService>):** HONORED. See D-007.
- **GOTCHA #29 (Child stderr piped, never inherited):** HONORED. `process.rs` redirects child stderr to log with `[server]` prefix. (src/process.rs:98)
- **GOTCHA #30/#31 (Morph directory scope + ALL_TOOLS read-only interaction):** HONORED. Documented limitations in DECISIONS.md + SECURITY.md; no code claims to solve them.

## Doc-vs-Code Drift & Stale Docs

- **OQ3 transport feature name (DRIFT resolved):** HONORED. `Cargo.toml:36` uses exactly `transport-streamable-http-client` (the `-client` variant); rmcp-general and STACK.md document the same. No residual drift.
- **AggError -> ToolError mapping (DRIFT-1):** HONORED. `error.rs` maps all upstream failures to `ToolError`; ARCHITECTURE.md matches the final shape. No residual drift.
- **macOS hard-kill limitation:** HONORED. SECURITY.md and GOTCHA #14 document the exact PDEATHSIG gap and platform asymmetry; `process.rs` implements only graceful + direct-child coverage on Unix. No over-claim.
- **MVP checklist sign-off:** No stale sign-off. All Phase 0-4 items that are marked complete are present in code (probe fixture, lock discipline, bidirectional reject, credential chain, hard-kill tests). No checklist items signed off but unimplemented.
- **ROADMAP v1.0 status:** Current. ROADMAP.md correctly reflects MVP completion + v1.1 items (OAuth, forwarding) as future. No stale "v1.0 shipped" claim.

## Anti-Stack & Single-Binary Promise

- No web framework, no HTTP server, no DB/ORM, no plugin loader, no Node/Docker at runtime — confirmed in Cargo.toml + STACK.md.
- Single static binary (`cargo build --release` produces one executable) — holds.

## Verdict

The codebase is **aligned** with its binding canon. Every non-obvious ADR (D-007 lock discipline, D-008 bidirectional, D-009 process lifetime, D-010 secrets, D-004/D-005 byte-faithful + structured errors) is implemented exactly as specified and verified by the test suite. All ✅ GOTCHAs that claim enforcement are actually enforced. The four known drifts from the prior cycle are resolved; no residual doc-vs-code drift remains. Highest-priority item for the next cycle is the v1.1 deferred work (OAuth, capability forwarding), not any current misalignment.
