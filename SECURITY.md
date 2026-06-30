# Security Model — fanin-mcp

This document states honestly what fanin-mcp protects against, what it cannot, and the practices enforced in the codebase. It exists because credential-handling tools that overclaim lose trust the first time someone reads the source.

## Threat Model

### What the OS keychain (and this design) protects against

- **Plaintext secrets in config files.** `config.toml` contains only `${PLACEHOLDERS}`; it is safe to commit, share, and sync. Secrets live in DPAPI (Windows), Keychain (macOS), or Secret Service (Linux).
- **Accidental git commits / dotfile leaks** of credentials.
- **Disk theft / unencrypted backups** (keychain entries are encrypted at rest by the OS).
- **Other users on the same machine** (keychain entries are per-user).
- **Cross-server credential exposure.** Each upstream child process receives *only its own* resolved env vars. A compromised or malicious obsidian server never sees your database URL. This is strictly better than the common `.env`-file or shared-shell-environment status quo.

### What it does NOT protect against

- **Malware already running as your user.** On Windows and Linux, any same-user process can read your keychain entries; on macOS the prompt-gating is stronger but not absolute. No local secret store solves this — your browser's password manager has the same property. We will not pretend otherwise.
- **A malicious upstream server you chose to configure.** Adding a server means running its code as you, with its own credentials. The aggregator limits the *blast radius* (per-server injection, namespace scoping) but cannot make untrusted code trustworthy.
- **Prompt injection via upstream-provided text.** Tool names, descriptions, and results from upstreams are read by the LLM by design. The aggregator sanitizes upstream-authored tool names and descriptions in `list_tools` rows and the annotation strings (`title`/`description`/`$comment`) in `get_tool_schema` — neutralizing C0/C1 control characters, DEL, Unicode line/paragraph separators, bidi-override and zero-width characters — and length-caps each `list_tools` description row. The length cap applies to `list_tools` description rows ONLY; `get_tool_schema` annotation strings are control-neutralized and relayed FULL-LENGTH (no cap), so real argument documentation is not silently dropped. This *bounds* the channel; it cannot eliminate it. By design, schema *validation* strings (`enum`, `const`, `default`, `pattern`, …) pass through unmodified to stay byte-faithful, and `invoke_tool` arguments AND result content pass through VERBATIM (D-004) — these are the residual, bounded, documented injection channel. Treat upstream servers with the same trust you'd give a dependency.

## Enforced Practices (codebase guarantees)

1. **No secrets on argv.** `cred set <server> <KEY>` reads the value from a hidden stdin prompt. Process listings and shell history never contain secret values. `cred list` prints key names only.
2. **No secrets in logs.** A tracing redaction layer scrubs all resolved secret values from log output. An automated test injects a sentinel secret and asserts it never appears in any log line. This test is a release gate.
    Log redaction is exact-substring matching of registered secret values — whole-secret appearances are caught and replaced with `[REDACTED]`; a secret that appears perturbed/partial (e.g. truncated by an upstream) is out of scope.
    Since H-3, every value resolved from a server's `[headers]` section (literal strings included, not only `${VAR}` expansions) is registered for redaction; choose header values distinct from any operational text your tracing layer may emit.
3. **Least-privilege injection.** Credential env vars are injected per-child at spawn time; no upstream inherits the aggregator's full environment or another server's secrets.
4. **No secrets on disk outside the keychain.** Ever. The v1.1 tool-list disk cache contains tool metadata only, never credentials, and is fully reconstructible.
5. **Credential backend chain.** `--credential-store` selects the *preferred* backend (default: keyring); the process environment is always the fallback (headless Linux, CI, containers). Failures state which backend failed and why.
6. **Process-tree containment.** Every upstream is spawned into a container so tearing down `fanin-mcp` reaps the upstream and (where the platform allows) its descendants — e.g. the `cmd /c npx ...` → `node.exe` / `npx → node` grandchild shape. The guarantee differs by platform, and we state it honestly:
   - **Windows — full whole-tree, crash-safe.** The child is created suspended, assigned to a `KILL_ON_JOB_CLOSE` Job Object, then resumed (closing the spawn-then-assign race). A hard kill of `fanin-mcp` — even `taskkill /F` — makes the kernel reap the *entire* tree, grandchildren included. CI-tested.
   - **Linux — whole-tree on graceful shutdown; direct child crash-safe.** Graceful teardown (stdin-EOF) kills the whole process group, grandchildren included. For a hard kill (`kill -9`) of `fanin-mcp`, each direct child sets `PR_SET_PDEATHSIG = SIGKILL` so the kernel kills it — but `PDEATHSIG` only covers *one* process level, so a **grandchild forked by the upstream may orphan on a hard kill**. True whole-tree hard-kill on Linux would require PID namespaces or cgroups (privileged, heavier) — deliberately not done (minimal / no-daemon identity).
   - **macOS — whole-tree on graceful shutdown only.** No `PDEATHSIG` equivalent; graceful teardown (process-group) reaps the tree, but a **hard kill of `fanin-mcp` may leave upstreams (and their descendants) orphaned**.

   Net: a hard `SIGKILL` of `fanin-mcp` is fully contained only on **Windows**. On **Unix**, prefer a graceful shutdown (close stdin); a hard kill may orphan upstream grandchildren. This is a documented MVP limitation — fanin-mcp runs no supervisor/daemon to close it (the "no daemon" non-goal).

## Access Control Reality

The meta-tool indirection has a consequence users must understand: to the client, every upstream call is the single tool `invoke_tool`. **Client-side per-tool permission prompts therefore collapse** — approving `invoke_tool` once approves everything the namespace exposes. Additionally, OpenCode does not surface tool annotations at all (verified empirically), so the conservative `destructiveHint` on `invoke_tool` only helps on annotation-aware clients like Claude Code.

**The namespace tool-filter is the real permission layer.** Recommendations:

- Give each project a namespace exposing only the servers and tools it needs.
- Use read-only namespaces (allow-list only query/read tools) for sessions that shouldn't mutate anything.
- Don't put destructive-capable servers in the `default` namespace if you use `default` casually.
- Where upstreams offer their own hardening (e.g., a Postgres server's read-only mode), use it — defense in depth; the aggregator's ACL is name-level, not argument-level.
- **Beware full-filesystem upstreams.** Some servers expose broad read/write capability beyond their headline tool — e.g. the Morph fast-apply server in `ALL_TOOLS: "true"` mode is a general filesystem read/write/list server, not just `edit_file`. A namespace that looks read-only but contains such a server is *not* read-only. Scope these deliberately: expose only the intended tools via the per-server tool-filter, or run the upstream in its own restricted mode. The aggregator's namespace filter is name-level — it controls which tools are reachable, not what arguments they accept.

Concrete shape of the read-only namespace:

```toml
[namespaces.readonly]
servers = ["postgres", "obsidian"]

[namespaces.readonly.tools]
postgres = ["query", "list_tables"]  # write tools (insert, update, ...) are thereby hidden
# obsidian present in `servers` but absent from `[...tools]` => all its tools visible
```

## Operator Guidance

- **Pin upstream versions.** `npx -y some-server@1.2.3`, never floating latest. An MCP server update is a code-execution event on your machine.
- **Review what you connect.** Prefer servers with pinned, auditable releases.
- **Per-server timeouts** (`timeout_secs`) limit how long a misbehaving upstream can hold resources.
- Logs (`--log-file`) record every tool call (server, tool, latency, outcome) — useful as an audit trail; they are redacted but still treat them as sensitive (they reveal activity patterns and tool arguments are upstream-visible by necessity).

## Supply Chain (project itself)

- `Cargo.lock` committed; `rmcp` and all dependencies pinned.
- `cargo deny` (bans / licenses / sources) runs in CI on every commit. **Advisory scanning is temporarily paused:** the RustSec advisory DB now ships CVSS 4.0 entries that the current RustSec parser tooling (both `cargo audit` and `cargo deny` 0.19.x) rejects on load (`unsupported CVSS version: 4.0`). It will be re-enabled (`cargo deny check`) once the tooling supports CVSS 4.0. The deliberately small, exact-pinned dependency tree (with committed `Cargo.lock`) bounds the exposure in the interim.
- Deliberately minimal dependency tree (no web framework, no database). Remote Streamable-HTTP upstreams pull a reqwest/hyper **HTTP client** (a client, never a listener — the no-daemon/no-port non-goal holds); for the loopback test path TLS features are trimmed, and a TLS stack is only linked when a real remote upstream requires HTTPS. `cargo deny` (licenses/bans/sources) and the `< 10 MB` stripped-binary budget keep the tree honest in CI.
- Release binaries published with checksums and signatures.
- Reproducible release builds are a goal; deviations documented.

## Out of Scope (and why)

- **Memory zeroization** of secrets adds little here: the secret must live in the child process's environment for the upstream to function, so wiping the aggregator's copy doesn't change the exposure. Not advertised as protection.
- **OAuth flows** are deferred to v1.1 (out-of-band `auth` subcommand); MVP supports static `Authorization` header injection from the credential store.

## Reporting a Vulnerability

Report security issues privately via the repository **Security** tab → **Report a vulnerability** (GitHub private vulnerability reporting). No public issue. We aim to acknowledge within 72 hours.
