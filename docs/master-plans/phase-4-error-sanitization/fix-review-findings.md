# Fix: phase-4-error-sanitization review findings

## Defects fixed

### F1 — Unicode sanitizer gap

- **Defect:** `src/server.rs::sanitize_upstream_text` stripped only C0 controls
  and DEL. Unicode line/paragraph separators, C1 controls, bidi controls, BOM,
  and zero-width characters survived in LLM-visible text.
- **Root cause:** The sanitizer used a too-narrow control-char check.
- **Fix applied:** Added a general character-class predicate covering C0, C1,
  DEL, U+2028/U+2029, U+202A–U+202E, U+2066–U+2069, U+FEFF, and
  U+200B–U+200D. Display text replaces those code points with spaces before the
  cap is applied.
- **Verified:** `sanitization::f1_list_tools_strips_unicode_separators_c1_bidi_zero_width`
  passed inside the full integration suite.
- **Divergence:** None.

### F2 — Advertised dispatch key was capped

- **Defect:** `list_tools` used the capped display sanitizer for row `tool` and
  `name`, so legitimate long upstream tool names could be truncated and become
  undispatchable.
- **Root cause:** One sanitizer served both prose descriptions and identifier
  fields.
- **Fix applied:** Split identifier sanitization from display sanitization. Row
  `tool` and `name` now keep the real upstream name with only defensive control
  neutralization and no length cap. The cap remains only on `description`.
- **Verified:** `sanitization::f2_long_named_tool_advertised_full_and_dispatchable`
  and `sanitization::list_tools_row_names_are_control_char_free` passed.
- **Divergence:** None.

### F3 — Schema sanitizer corrupted validation data

- **Defect:** `sanitize_schema_metadata` sanitized `enum` and `examples`, which
  can be validation constants. That changed schema semantics.
- **Root cause:** The metadata-key set mixed annotation fields with validation
  fields.
- **Fix applied:** Narrowed schema sanitization to annotation keys only:
  `title`, `description`, `$comment`, and `markdownDescription`. Validation and
  structural values remain verbatim.
- **Verified:** `sanitization::f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations`
  and existing schema sanitization coverage passed.
- **Divergence:** None. The accepted residual injection channel in validation
  strings remains bounded by GOTCHA #20.

### F4 — Send-side transport death misclassified

- **Defect:** `src/registry.rs::map_service_error` mapped only
  `ServiceError::TransportClosed` to `UpstreamDisconnected`; send-side transport
  failures stayed `UpstreamCall`.
- **Root cause:** The rmcp `ServiceError` match missed `TransportSend`.
- **Fix applied:** Verified rmcp `=1.8.0` exposes `McpError`, `TransportSend`,
  `TransportClosed`, `UnexpectedResponse`, `Cancelled`, and `Timeout`. Mapped
  `TransportClosed` and `TransportSend(_)` to `ToolError::UpstreamDisconnected`.
  Left MCP application errors and non-transport service errors as
  `UpstreamCall`.
- **Verified:** The deterministic transport-closed test
  `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable`
  passed. The F4 send-side test remains intentionally ignored because the wire
  surface cannot deterministically force `TransportSend`.
- **Divergence:** None.

### F5 — Failed refetch cleared dirty flag

- **Defect:** `ensure_fresh` cleared `dirty` before `list_all_tools().await`; a
  failed refetch left the cache clean and allowed stale inventory later.
- **Root cause:** Failure handling did not restore the invalidation bit.
- **Fix applied:** On refetch error, `ensure_fresh` restores `dirty=true` before
  returning the mapped error. Successful refetches still install the new tools
  without holding the registry map lock or tools lock across the await.
- **Verified:** `list_changed::f5_failed_refetch_retries_does_not_serve_stale_inventory`
  passed inside the full integration suite.
- **Divergence:** None.

## Verification

- `cargo fmt --all`: passed.
- `cargo clippy --all-targets`: passed with zero warnings.
- Anti-gaming grep over `src/`: no test-fixture-shaped literals found.
- `cargo test --test integration`: **103 passed, 0 failed, 4 ignored**.

## Surfaced findings

(none)
