# Test issue: phase-5-polish-ci-benchmarks

## Test

- `tests/integration/http_upstream.rs`
- Failing case:
  `http_upstream_invokes_with_resolved_authorization_header_and_redacts_logs`

## Problem

The loopback HTTP mock returns fixed JSON-RPC response ids during the MCP
handshake:

- `initialize` response id: `1`
- `tools/list` response id: `2`
- tool-call response id: `3`

rmcp `=1.8.0`'s Streamable-HTTP client expects the response id to match its
request id. The first `initialize` request uses id `0`, so the client rejects
the mock response before inventory can complete:

```text
upstream_connect_failed: conflict initialized response id: expected 0, got 1
```

## Why this is a test-contract issue

The Phase 3 implementer brief explicitly says to stop and report a test issue
if the loopback mock cannot complete the rmcp Streamable-HTTP handshake rather
than weakening production code to fit a broken mock. Rewriting response ids in
the proxy or relaxing rmcp's id validation would be protocol-incorrect.

## Suggested fix

Make the mock parse each incoming JSON-RPC request id and echo that exact id in
the HTTP response. Keep the existing header assertion and missing-credential
assertion unchanged.
