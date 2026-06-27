# Fix: remediation-s1-d1 targeted review findings

## Defects

- **G-1 — observability event naming.** `src/registry.rs` logged three timeout
  sites as `event = "upstream_failure", code = "timeout"`.
- **G-2 — empty timeout tool field.** `ToolError::UpstreamTimeout` required a
  `String`, so connect/discovery/refetch timeouts rendered empty backticks and
  serialized `"tool": ""`.
- **G-3 — cwd doc-comment gap.** `ServerConfig::cwd` did not state that blank
  literal or resolved values are rejected before spawn.

## Fix applied

- `src/registry.rs`: renamed all three timeout trace events to
  `event = "upstream_timeout"` and removed the redundant `code = "timeout"`.
  Cold-connect and dirty-refetch timeouts now construct `tool: None`; tool-call
  timeout constructs `tool: Some(tool.to_string())`.
- `src/error.rs`: changed `ToolError::UpstreamTimeout.tool` to
  `Option<String>`. `Some(tool)` keeps the existing call wording and wire value;
  `None` renders `upstream operation on `{server}` exceeded timeout` and
  serializes `"tool": null`.
- `src/config.rs`: added the `cwd` doc line for empty / whitespace-only rejection
  before spawn, including after `${VAR}` resolution.

## Public error shape

D-005 key set is unchanged. The structured JSON still always includes `server`,
`tool`, `code`, `message`, and `recoverable`. The `tool` key is now nullable for
operation-level timeouts, not omitted. The timeout code remains
`upstream_timeout`; the call-tool timeout wire form remains unchanged.

## Files touched

- `src/registry.rs`
- `src/error.rs`
- `src/config.rs`
- `docs/master-plans/remediation-s1-d1/fix-review-targeted.md`

## Verification

- `cargo fmt --all -- --check` — pass.
- `cargo clippy --all-targets -- -D warnings` — pass.
- `cargo test --all` — pass: 134 passed, 0 failed, 4 ignored.

## Surfaced

None. No test changes were needed or made.
