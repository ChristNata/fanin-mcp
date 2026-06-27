# Implementer task — Phase 5, plan **Phase 3: Streamable-HTTP Mock & Header Auth**

Implement ONLY plan Phase 3: add a Streamable-HTTP **client** upstream path so a
remote MCP server can be proxied with static header auth. (Plan Phase 4 needs no
implementation — its contracts already pass; do not touch its scope.)

## Read first

- `master.md` §"Phase 3 — Streamable-HTTP Mock And Header Auth".
- **`tests/integration/http_upstream.rs`** — binding READ-ONLY contract. Note the
  exact config shape it writes and the asserted error code.
- **`docs/master-plans/phase-5-polish-ci-benchmarks/oq3-http-transport-findings.md`**
  — the VERIFIED rmcp client API + the correct feature-flag names. Re-verify at
  the `=1.8.0` pin via Context7 before editing (the pin is law).
- `src/config.rs` (`ServerConfig` ~:93: `transport: Option<String>`,
  `command: Option<String>`; validation ~:177 rejects non-stdio transports —
  this is what you extend), `src/registry.rs` (the lazy connect path that
  currently builds a `TokioChildProcess` stdio transport), `src/error.rs`
  (the structured `ToolError` / startup error variants + the
  `credential_resolution_failed` code), `src/credentials.rs` is OFF-LIMITS
  (CARRY-3 — do NOT edit it; the ${VAR} resolver it exposes is already callable).
- Skills: `rmcp-general`, `rust-general`.

## Exact contract (from http_upstream.rs)

Config the tests write:
```toml
[servers.<name>]
transport = "streamable-http"
endpoint  = "http://127.0.0.1:PORT/mcp"
log_file  = '<path>'                      # per-server, already supported
[servers.<name>.headers]
Authorization = "Bearer ${TOKEN_ENV_VAR}"
```
1. **Config:** add `endpoint: Option<String>` and
   `headers: Option<HashMap<String,String>>` to `ServerConfig`. Accept
   `transport = "streamable-http"`; for it, require `endpoint` (fail startup
   if missing), allow `headers`; stdio still requires `command` and must keep
   working unchanged.
2. **Connect:** when transport is streamable-http, connect via rmcp's
   `StreamableHttpClientTransport` (reqwest-backed) to `endpoint` instead of
   spawning a child. Resolve each header value through the EXISTING credential
   chain (preferred store → env), interpolating `${VAR}`. **Register every
   resolved secret with the redaction layer** (GOTCHA #19) so it cannot leak to
   the log/JSON sink. Inject the resolved headers — prefer the arbitrary
   `custom_headers` path so any header name works; reconcile the `Bearer `
   prefix if you use `auth_header` (it wants the token WITHOUT `Bearer `).
3. **Missing credential:** if a header's `${VAR}` cannot be resolved, return the
   existing structured error with **`code: "credential_resolution_failed"`** and
   **do NOT connect/contact the endpoint** (the test asserts the mock was never
   hit). This must surface as `CallToolResult { isError: true }`, not a JSON-RPC
   error (D-005).
4. **Lock discipline (GOTCHA #16/D-007):** do not hold the registry map lock
   across the HTTP connect/await. Same Arc-clone-drop-lock-then-await pattern as
   the stdio path.
5. **stdio unchanged:** lazy spawn, namespace filtering, byte-faithful results
   all keep working (`stdio_upstream_still_lazy_and_namespace_filtered_after_http_support`).

## Cargo.toml (the one allowed manifest change here)

- Add the rmcp client HTTP feature(s): `transport-streamable-http-client` and
  `transport-streamable-http-client-reqwest` (VERIFY exact spelling at `=1.8.0`).
- reqwest comes in transitively (or add it directly only if rmcp needs it). The
  test mock is **loopback plain HTTP, no TLS** — disable reqwest default TLS
  features where possible (`default-features = false`, add only what loopback
  needs) to keep the tree small for the Phase 6 `cargo deny` + the <10MB binary
  budget. Note every dep you add in your result for Phase 6.
- rmcp stays `=1.8.0`; `Cargo.lock` committed.

## Constraints

- Scope: Phase 3 only. Do NOT edit `src/credentials.rs` (CARRY-3). Surface
  (don't fix) anything outside scope.
- Tests read-only; if the loopback mock can't complete rmcp's Streamable-HTTP
  handshake (e.g. it needs a session-id header or an SSE GET the mock doesn't
  answer), that is a **test-issue** → STOP and report it (it routes back to the
  test-creator), do NOT weaken src to fit a broken mock.
- End: `cargo fmt` clean, `cargo clippy --all-targets` zero warnings, and
  `cargo test --test integration http_upstream` green, with the full suite no
  worse than before on the stdio paths.

## Return

`impl-p3-result.md`: per-file changes, the exact rmcp feature names + API you
used (with pin evidence), every dependency added (for Phase 6), how header
secrets are registered for redaction, and any surfaced issue/test-issue —
especially if the loopback mock needed a fuller handshake than it implements.
