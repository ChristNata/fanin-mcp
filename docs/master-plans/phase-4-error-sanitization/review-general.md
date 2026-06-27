# Review (general lens): phase-4-error-sanitization

Found 0 blocker, 0 structural, 3 targeted, 0 trivial.

Verification run:

- `cargo test` — passed: 104 passed, 3 ignored.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- rmcp `=1.8.0` checked against Context7: `ClientHandler::on_tool_list_changed`
  signature matches the implementation, `ServiceError::TransportClosed` exists,
  and `ServiceError` is non-exhaustive.

## Findings

- Severity: targeted
  Location: `src/server.rs:432` / `sanitize_metadata_value`
  Issue: `enum` values are sanitized as if they were display metadata. In JSON
  Schema, `enum` entries are validation constants, not labels; changing them can
  make the returned schema describe arguments the upstream does not actually
  accept. A control-bearing or long string enum value would be altered in
  `get_tool_schema`, while `invoke_tool` still sends the caller's raw arguments
  to the upstream.
  Suggested fix: treat `enum` as structural data and leave its values unchanged,
  or only sanitize a separate display/annotation field if the upstream schema
  uses one. Add a schema test with a string enum value containing a control
  character and assert the enum constant is preserved while title/description
  metadata is sanitized.
  Routing: debugger for the helper change; test-creator for the enum-preservation
  test.

- Severity: targeted
  Location: `src/registry.rs:220` / `Registry::ensure_fresh`
  Issue: the dirty flag is cleared before the refetch succeeds. If
  `list_all_tools().await` returns a transient non-transport `ServiceError`, the
  caller sees one structured error, but the cache is no longer dirty. The next
  `inventory()` or `call_tool()` can serve the stale pre-notification tool list
  instead of retrying the required refetch.
  Suggested fix: leave the flag dirty on refetch failure, or restore it in the
  error branch before returning. Keep the current no-lock-across-await discipline.
  Add a test that forces the first post-`list_changed` refetch to fail and proves
  a later read retries rather than using stale inventory.
  Routing: debugger for the dirty-flag error path; test-creator for the retry
  coverage.

- Severity: targeted
  Location: `src/registry.rs:247` / `map_service_error`
  Issue: only `ServiceError::TransportClosed` maps to `upstream_disconnected`.
  rmcp `ServiceError` also has transport-send failures, and a broken pipe or
  dead child can surface as `TransportSend(...)` depending on where the transport
  observes the failure. That would be reported as `upstream_call_failed`, even
  though the Phase 4 error model distinguishes transport death from a live
  upstream call failure.
  Suggested fix: classify transport-layer failures caused by a closed/broken
  transport as `UpstreamDisconnected` as well, while keeping MCP application
  errors as `UpstreamCall`. Add a test or fixture path that makes the send side
  fail rather than the receive side so this mapping is pinned.
  Routing: debugger for the mapper; test-creator for the transport-send coverage.

Lens verdict: PASS-with-issues.
