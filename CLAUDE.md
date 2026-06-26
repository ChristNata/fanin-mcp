# fanin-mcp — Project Preamble

Both CLIs read this file (CC natively; OC via the `AGENTS.md > CLAUDE.md`
fallback). It is the project frame on top of Covenant's global doctrine.

## Purpose

A standalone, stdio-native MCP **proxy** that federates many upstream MCP
servers behind a single endpoint. Configure servers once, store credentials
once in the OS keychain, and scope each session's visible tools with a
namespace. Clients see 3 meta-tools (`list_tools`, `get_tool_schema`,
`invoke_tool`) instead of every upstream schema; upstreams spawn lazily on
first use, not at startup.

## Stack

- **Language:** Rust 1.80+, edition 2021. Single static binary, no runtime
  deps — a product promise, not an implementation detail.
- **Protocol:** `rmcp`, the official MCP SDK, **server and client roles in
  one process**. Pinned to an exact version (`=x.y.z`); `Cargo.lock`
  committed. The rmcp API moves fast — see the `rmcp-general` skill.
- **Core crates:** `tokio` (full), `serde`/`serde_json`, `toml`, `clap`
  (derive), `keyring`, `rpassword`, `tracing`/`tracing-subscriber`,
  `process-wrap`/`command-group`, `dirs`, `schemars`.
- **Anti-stack (deliberately absent):** no web framework, no HTTP server,
  no database/ORM, no plugin loader, no Node/Docker/system services at
  runtime. A PR adding one contradicts ROADMAP non-goals — flag it in review.

## Project shape

Flat single-stack Rust project. One binary crate; flat `src/` module layout
(`main.rs`, `server.rs`, `registry.rs`, `forward.rs`, `process.rs`,
`namespace.rs`, `credentials.rs`, `error.rs`, `config.rs`) plus an in-repo
probe-server fixture under `tests/probe-server/`. No workspace, no sub-crates.

Plan scope: flat

## Design canon — the `docs/` vault

`docs/` is an **Obsidian vault**, read and written by the `glasswing` MCP
server. It is the project's knowledge store: the design corpus, the Covenant
plan workspace (`docs/master-plans/`), and all future research / debugging /
RCA notes live here. Prefer the `glasswing` tools to read and write vault
notes; plain file reads still work. Standard markdown and `[[wikilinks]]` are
both fine.

The design docs are binding. Read the relevant one before touching a
subsystem — they encode decisions already made, not suggestions:

- **`docs/DECISIONS.md`** — accepted ADRs (D-001..D-019). The *why* behind
  every non-obvious choice. Diverging from one is a spec conflict to surface,
  not a silent rewrite.
- **`docs/GOTCHA.md`** — the trap list (symptom → cause → fix). Items marked ✅
  are enforced by design/tests; do not "simplify" them away.
- **`docs/MVP.md`** — the phased implementation plan and verification checklist.
- **`docs/PRD.md` / `docs/ARCHITECTURE.md` / `docs/AGG-MCP.md`** — requirements
  and internal architecture. AGG-MCP snippets are **pseudocode** until verified
  against the rmcp pin.
- **`STACK.md` / `ROADMAP.md` / `SECURITY.md`** (repo root, GitHub-facing) —
  stack rationale, what is in scope per version, and the threat model.

## Binding project rules

These are the sharp edges that bite first. The full set lives in `docs/GOTCHA.md`.

1. **stdout is the MCP transport.** Once `serve(stdio())` runs, any `println!`
   to stdout corrupts the JSON-RPC stream. All output goes to stderr or the
   log file via `tracing`. (GOTCHA #1)
2. **Never hold a lock across an upstream call.** Lock the registry map only to
   get/insert `Arc<RunningService>`, clone the Arc, drop the lock, then await.
   A lock held across `call_tool` serializes the whole session. (D-007, GOTCHA #16)
3. **Answer bidirectional traffic from Phase 1.** Declare no sampling/elicitation
   capabilities upstream; reject strays instantly; return an empty `roots/list`.
   An unanswered upstream request hangs that server forever. (D-008, GOTCHA #2)
4. **Errors are `CallToolResult { isError: true }`, never JSON-RPC errors.** The
   structured-error JSON shape is public API. (D-005, GOTCHA #3)
5. **Secrets never on argv, never in logs.** `cred set` reads from a hidden stdin
   prompt; the redaction layer + sentinel test guard logs; each upstream gets
   only its own vars. (D-010, GOTCHA #18/#19/#22)
6. **Process-tree lifetime is non-negotiable on Windows.** Every upstream lives
   in a Job Object / Unix process group; hard-kill leaves zero orphans. (D-009,
   GOTCHA #11/#14)
7. **Results pass byte-faithfully.** Never `to_string()` a content array — it
   corrupts images/resources. (D-004, GOTCHA #4)
8. **Tests are a read-only contract.** Only `test-creator` writes them. 100% pass,
   no thresholds.

## Project skill

- **`rmcp-general`** (`.claude/skills/rmcp-general/SKILL.md`) — rmcp idioms,
  the exact-pin discipline, and the implementation traps. Loaded by the
  implementer/simplifier/debugger, test-creator, and reviewer children
  alongside the seeded `rust-*` triplet.

## Project command

- **`/gate`** — runs the security/release gate: `cargo audit`, `cargo deny`,
  and the token benchmark. Dual-written for both CLIs.

## MCP

- **Context7** (remote, credential-free) — current rmcp/crate docs on demand.
  The rmcp API shifts across versions; prefer Context7 over memory for rmcp
  signatures. Add `CONTEXT7_API_KEY` via `/update` for higher rate limits.
- **glasswing** (local, credential-free) — Obsidian-vault read/write over the
  `docs/` vault. Zero network calls; vault never leaves the machine. The vault
  path is passed as a relative arg (`docs`), so it resolves against the session
  CWD — the repo root in a normal session, the worktree root under
  orchestration. Requires `npm install -g glasswing-mcp` on the host.
