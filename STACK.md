# fanin-mcp — Technology Stack

## Language & Toolchain

| | Choice | Notes |
|---|---|---|
| Language | **Rust 1.80+** | Single static binary, no runtime deps — a core product promise (vs Node-based aggregators) |
| Edition | 2021 | |
| Async runtime | **tokio** (`full`) | Required by rmcp; child-process + timeout + sync primitives all used |
| MSRV policy | Stated in Cargo.toml; bumped only in minor releases with changelog note | |

## Core Crates

Every dependency must justify itself — the small tree is a security feature we advertise. **`rmcp` is pinned to an exact version (`=x.y.z`) and `Cargo.lock` is committed**; its API has shifted across versions and all internal doc snippets are pseudocode until verified against the pin.

| Crate | Purpose | Why this one |
|-------|---------|--------------|
| `rmcp` (exact pin) | MCP protocol — **both** roles | Official SDK; features: `server`, `client`, `transport-io`, `transport-child-process`, and `transport-streamable-http-client` + `transport-streamable-http-client-reqwest` for remote upstreams (the `-client` variant — bare `transport-streamable-http` is the server side). The proxy is a server downstream and a client upstream in one process |
| `tokio` | Async runtime | rmcp requirement; `tokio::time::timeout` wraps every upstream call; `RwLock`/`Mutex` for the registry discipline (D-007) |
| `serde`, `serde_json` | Serialization | Raw `Value` passthrough of tool arguments/results (D-004); `AggError` serialization |
| `toml` | Config parsing | Human-writable config is a product requirement |
| `clap` (`derive`) | CLI + subcommands | `serve` (default), `cred set|list|rm`; later `warm`, `auth`, `install` |
| `keyring` | OS credential store | One API over DPAPI / macOS Keychain / Secret Service (D-010) |
| `rpassword` (or equiv.) | Hidden stdin prompt | `cred set` must never take secrets on argv |
| `tracing`, `tracing-subscriber` (`json`, `fmt`) | Structured logging | Custom redaction layer scrubs secret values; JSON file output via `--log-file`. **stdout is forbidden** — it's the MCP transport |
| `process-wrap` / `command-group` | Process-tree lifetime | Job Objects (Windows) + process groups (Unix) behind one API (D-009). May require a thin custom child transport if rmcp's `TokioChildProcess` can't be wrapped — isolated in `process.rs` |
| `dirs` | Platform paths | Config (`%APPDATA%` / `~/.config`) and the v1.1 cache dir |
| `schemars` | JSON Schema helpers | Meta-tool input schemas (manual construction preferred over `#[tool]` macros — see AGG-MCP.md) |

**Transitive-only, tolerated:** the reqwest/hyper HTTP-**client** tree (`reqwest`, `hyper`, `tower`/`tower-http`, `url`, the ICU/idna stack) pulled by rmcp's `transport-streamable-http-client-reqwest` for remote upstreams — a client, not a listener, so the no-HTTP-server identity holds; TLS (`rustls`) is only linked when a real remote needs HTTPS (trimmed for the loopback test path). Direct add: `http` (for `HeaderName`/`HeaderValue`) and target-gated `libc` on Linux (for `PR_SET_PDEATHSIG`). The whole tree stays under the `< 10 MB` stripped-binary budget (CI-checked; measured 8.27 MB) and passes `cargo deny`. Windows Job Object bindings come via `process-wrap` (no separate `windows`/`win32job` needed).

## Anti-Stack (deliberately absent)

No web framework. No HTTP server. No database / SQLite / ORM. No plugin loader. No OpenTelemetry (file logs only). No Node/Docker/system services at runtime. If a PR adds one of these, it's contradicting [ROADMAP.md non-goals](ROADMAP.md#non-goals----identity-not-backlog) — flag it in review.

## Testing & CI

| | Choice |
|---|---|
| Integration fixture | **In-repo probe server** (rmcp binary: `echo_ok`, `always_error`, `slow_tool`, `dangerous_noop`, `needs_sampling`) — CI needs no Node, no real databases (D-016) |
| Test style | Spawn the compiled binary, speak JSON-RPC over stdio — real-transport tests, not mocked handlers |
| CI | GitHub Actions matrix: `windows-latest`, `macos-latest`, `ubuntu-latest` |
| Security gates | `cargo deny` (bans/licenses/sources) on every commit (advisory scanning paused pending CVSS-4.0 parser support upstream); sentinel-secret log-redaction test; hard-kill orphan-process test (Windows whole-tree; Unix graceful + direct-child) |
| Benchmarks | Token-cost benchmark (tools/list + typical session) — README numbers are generated, never hand-written |

## Build & Release Profile

```toml
[profile.release]
lto = true
codegen-units = 1
strip = "symbols"
panic = "abort"     # evaluate: smaller binary; confirm rmcp/teardown compatibility first
```

- **Targets:** `x86_64-pc-windows-msvc`, `x86_64-apple-darwin` + `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` (musl static build: evaluate for portable Linux)
- **Budgets (release gates):** binary < 10MB stripped · idle < 15MB RSS · < 50MB with 5 active upstreams · `initialize` < 500ms
- **Distribution:** GitHub Releases (signed + checksummed) and `cargo install fanin-mcp`

## Configuration & Data Locations

| Thing | Windows | macOS / Linux |
|---|---|---|
| Config | `%APPDATA%\fanin-mcp\config.toml` | `~/.config/fanin-mcp/config.toml` |
| Secrets | DPAPI (keychain entry `fanin-mcp/{server}`) | Keychain / Secret Service |
| Logs | `--log-file <path>` (no default writes) | same |
| Cache (v1.1) | `%LOCALAPPDATA%\fanin-mcp\cache\` | `~/.cache/fanin-mcp/` |

Override order: CLI args → `$FANIN_MCP_CONFIG` → platform default. Secrets resolution: preferred backend → process env → error.

## Versioning

SemVer. The public API surface is: the 3 meta-tool names + input schemas, the structured-error JSON shape, the config schema, and CLI flags/subcommands. Breaking any of these bumps major.
