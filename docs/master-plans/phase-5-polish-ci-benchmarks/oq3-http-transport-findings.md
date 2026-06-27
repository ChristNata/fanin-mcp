# OQ3 resolution — rmcp Streamable-HTTP client transport (verified pre-test stage)

Orchestrator inline verification (Context7, rmcp docs), 2026-06-27. Resolves
master.md Open Question #3. **Verdict: capability EXISTS — Phase 3 is viable, no
structural blocker.** The implementer MUST still re-verify exact names against the
`=1.8.0` source the lockfile resolves to (docs queried were "latest"); the pin is
law (GOTCHA #23).

## What exists in rmcp (client side)

- `rmcp::transport::streamable_http_client::StreamableHttpClientTransport`
- `StreamableHttpClientTransportConfig::with_uri("http://127.0.0.1:PORT/mcp")`
- `.auth_header(token)` — sets `Authorization: Bearer <token>`. **Token must NOT
  include the `Bearer ` prefix** (rmcp adds it). Our config shape is
  `Authorization = "Bearer ${TOKEN}"`; reconcile this (strip the `Bearer ` prefix
  before handing to `auth_header`, OR route ALL static headers through the
  custom-headers path below — preferred for generality).
- Arbitrary static headers: the `StreamableHttpClient` trait methods take
  `custom_headers: HashMap<HeaderName, HeaderValue>` — this is the path for
  arbitrary header injection (not just Bearer). Our "static headers map" model
  maps cleanly onto this.
- `StreamableHttpClientTransport::with_client(client, config)` — custom HTTP
  client path (implement the `StreamableHttpClient` trait) for when reqwest is
  not enabled.

## CORRECTION — feature-flag name (drift to fix)

The actual cargo features are:
- **`transport-streamable-http-client`** — gates the client module.
- **`transport-streamable-http-client-reqwest`** — the reqwest-backed default
  client (`StreamableHttpClientTransport::from_uri(...)` convenience).

NOT `transport-streamable-http` as `rmcp-general` SKILL.md §"Features in play"
(line ~36) and master.md/OQ3 state. **This is a doc-drift finding** → fold the
`rmcp-general` skill correction into Phase 7 / knowledge-sync. The Phase 3
implementer adds the correct client feature(s) to `Cargo.toml`; verify the exact
spelling at `=1.8.0` before committing (a wrong feature name silently no-ops or
fails to compile).

## Dependency / budget watch-item (Phase 3 → Phase 6)

- The reqwest-backed client pulls `reqwest` (+ hyper, and a TLS stack by
  default). This is an HTTP **client**, not a listener — it does NOT violate the
  "no HTTP server/listener" non-goal. But it is a non-trivial tree addition that
  the Phase 6 `cargo deny` (bans/licenses) and the **<10MB stripped binary** /
  memory budgets must absorb.
- The in-repo mock is **loopback plain HTTP** (no TLS) — so disable reqwest's
  default TLS features where possible (`default-features = false`, add only what
  the loopback client needs) to keep the tree and binary small. Decide rustls vs
  native-tls only if a real remote (release-checklist manual step) needs HTTPS;
  the automated CI path does not.

## Net effect on the plan

- OQ3 default ("enable the minimal rmcp client HTTP feature; mock loopback-only;
  surface a structural finding only if the pin lacks support") stands — and the
  pin does NOT lack support, so no structural escalation.
- Two concrete corrections for Phase 3: (1) feature name is the `-client`
  variant; (2) prefer the custom-headers path over `auth_header` for general
  static-header injection, reconciling the `Bearer ` prefix.
