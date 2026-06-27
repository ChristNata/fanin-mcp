# Implementer result: phase-5-polish-ci-benchmarks

## Verdict

Blocked by a test-contract issue. The code type-checks and the two non-handshake
HTTP contract tests pass, but the phase is not green because the mock returns
fixed JSON-RPC ids that rmcp `=1.8.0` correctly rejects.

Test-issue report written:

- `docs/master-plans/phase-5-polish-ci-benchmarks/test-issue-http-upstream-handshake-id.md`

## Per-file changes

- `Cargo.toml`
  - Kept `rmcp` pinned to `=1.8.0`.
  - Added rmcp features:
    - `transport-streamable-http-client`
    - `transport-streamable-http-client-reqwest`
  - Added direct `http = "1"` for `HeaderName` / `HeaderValue`, matching the
    rmcp custom-header API.
- `Cargo.lock`
  - Updated by Cargo after enabling the HTTP client feature path.
- `src/config.rs`
  - Added `endpoint: Option<String>` and `headers: HashMap<String, String>` to
    `ServerConfig`.
  - Accepted `transport = "streamable-http"`.
  - Kept stdio default and stdio `command` validation unchanged.
  - Added startup validation that Streamable-HTTP servers require a non-empty
    `endpoint`.
- `src/error.rs`
  - Added `StartupError::HttpServerMissingEndpoint` for invalid HTTP configs.
  - Updated unsupported-transport wording to name both supported transports.
- `src/registry.rs`
  - Resolved HTTP header values through the existing credential chain via
    `process::resolve_env_value`.
  - Registered resolved placeholder-bearing header values with the existing
    redaction registry via `process::register_secret`.
  - Built arbitrary custom headers with `http::HeaderName` and
    `http::HeaderValue`.
  - Connected Streamable-HTTP upstreams with rmcp's reqwest-backed
    `StreamableHttpClientTransport` path.
  - Preserved the existing lazy `Arc` clone / drop-lock / await discipline.
  - Left stdio spawn behavior on the existing `spawn_stdio_transport` path.

No test files were edited. `src/credentials.rs` was not edited.

## rmcp API and feature evidence

Context7 re-verification for rmcp Streamable-HTTP client confirmed:

- Feature `transport-streamable-http-client` gates the client module.
- Feature `transport-streamable-http-client-reqwest` enables the reqwest-backed
  default client.
- `StreamableHttpClientTransportConfig::with_uri(...)` creates transport config.
- `.custom_headers(HashMap<HeaderName, HeaderValue>)` sets arbitrary static
  request headers.
- `StreamableHttpClientTransport::from_uri(...)` is the reqwest convenience path.

The implementation uses the same pinned module at compile time:

```rust
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport,
    StreamableHttpClientTransportConfig,
};
```

The compiled code uses
`StreamableHttpClientTransportConfig::with_uri(...).custom_headers(...)` and
`StreamableHttpClientTransport::from_config(...)` against the exact `=1.8.0`
crate resolved by Cargo.

## Dependencies added for Phase 6 deny/budget review

Direct dependency added:

- `http v1.4.2`

Transitive dependencies newly locked after enabling rmcp's reqwest-backed
Streamable-HTTP client feature:

- `atomic-waker v1.1.2`
- `base64 v0.22.1`
- `displaydoc v0.2.6`
- `form_urlencoded v1.2.2`
- `http-body v1.0.1`
- `http-body-util v0.1.3`
- `httparse v1.10.1`
- `hyper v1.10.1`
- `hyper-util v0.1.20`
- `icu_collections v2.2.0`
- `icu_locale_core v2.2.0`
- `icu_normalizer v2.2.0`
- `icu_normalizer_data v2.2.0`
- `icu_properties v2.2.0`
- `icu_properties_data v2.2.0`
- `icu_provider v2.2.0`
- `idna v1.1.0`
- `idna_adapter v1.2.2`
- `ipnet v2.12.0`
- `litemap v0.8.2`
- `percent-encoding v2.3.2`
- `potential_utf v0.1.5`
- `reqwest v0.13.4`
- `sse-stream v0.2.3`
- `stable_deref_trait v1.2.1`
- `sync_wrapper v1.0.2`
- `synstructure v0.13.2`
- `tinystr v0.8.3`
- `tower v0.5.3`
- `tower-http v0.6.11`
- `tower-layer v0.3.3`
- `tower-service v0.3.3`
- `try-lock v0.2.5`
- `url v2.5.8`
- `utf8_iter v1.0.4`
- `want v0.3.1`
- `wasm-bindgen-futures v0.4.76`
- `wasm-streams v0.5.0`
- `web-sys v0.3.103`
- `writeable v0.6.3`
- `yoke v0.8.3`
- `yoke-derive v0.8.2`
- `zerofrom v0.1.8`
- `zerofrom-derive v0.1.7`
- `zerotrie v0.2.4`
- `zerovec v0.11.6`
- `zerovec-derive v0.11.3`

Budget note: `reqwest v0.13.4`, `hyper v1.10.1`, `tower`, `tower-http`, `url`,
and the ICU/idna stack are the meaningful size-review items for Phase 6.

## Verification

Commands run:

```bash
cargo check --all-targets
cargo test --test integration http_upstream
cargo fmt
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```

Results:

- `cargo check --all-targets`: passed.
- `cargo test --test integration http_upstream`: failed one test:
  `http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs`.
- Passing tests in that command:
  - `missing_http_header_credential_returns_structured_error_without_connecting`
  - `stdio_upstream_still_lazy_and_namespace_filtered_after_http_support`
- `cargo fmt`: completed.
- `cargo fmt -- --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.

Failure text:

```text
HTTP mock result must return:
{"code":"upstream_connect_failed","message":"conflict initialized response id: expected 0, got 1","recoverable":true,"server":"http-1","tool":null}
```

The phase remains red solely because of the test-contract issue above.
