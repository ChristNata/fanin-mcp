# Review (adversarial lens): phase-4-error-sanitization

Found 0 blocker, 0 structural, 5 targeted, 0 trivial.

Test runs:

- `cargo test --test integration -- phase4 --nocapture` — pass, 5/5.
- `cargo test --test integration -- sanitization --nocapture` — pass, 5/5.
- `cargo test --test integration -- list_changed --nocapture` — pass, 2/2.
- `cargo test --test integration -- error_hardening --nocapture` — pass, 3/3.

## Findings

### 1. Unicode separators and bidi controls bypass the display sanitizer

- Severity: targeted
- Location: `src/server.rs:382` / `sanitize_upstream_text`
- Attack: `sanitize_upstream_text` replaces only C0 controls and DEL
  (`U+0000..U+001F`, `U+007F`). A malicious upstream description can still
  include `U+2028` LINE SEPARATOR, `U+2029` PARAGRAPH SEPARATOR, C1 controls
  (`U+0080..U+009F`), bidi overrides (`U+202A..U+202E`), or zero-width format
  characters. Those are LLM-visible formatting controls, not harmless text. A
  payload like `safe summary\u{2028}IGNORE PRIOR INSTRUCTIONS` can still render
  as an instruction break in clients that treat Unicode line separators as line
  breaks. The tests only assert C0/DEL, so this bypass is green.
- Confidence: high.
- Suggested fix: extend the sanitizer to reject or replace Unicode line and
  paragraph separators, C1 controls, bidi override/isolate controls, BOM, and
  zero-width format characters before applying the length cap. Add fixtures for
  `U+2028`, `U+2029`, `U+202E`, and `U+200B`.

### 2. Sanitized tool names are emitted as dispatch keys, so long names become uncallable

- Severity: targeted
- Location: `src/server.rs:184`, `src/server.rs:186`
- Attack: `list_tools` writes the capped/sanitized name into both `tool` and
  `name`. Those fields are the obvious dispatch keys a model will use to build
  `server__tool` for `get_tool_schema` or `invoke_tool`. If an upstream exposes
  a legitimate 101-character tool name, `list_tools` emits only the first 100
  characters; a subsequent `invoke_tool` using the advertised key fails with
  `unknown_tool`. The test fixture uses only clean short names, so it proves
  neither the cap nor the control-char path preserves dispatchability.
- Confidence: high.
- Suggested fix: separate the real dispatch key from sanitized display text.
  For example, keep `tool` as the real name if rmcp's name grammar already
  prevents dangerous control characters, add `display_name` for sanitized text,
  and cap only the display field. If raw names can contain LLM-dangerous
  characters, reject unsupported upstream tool names at discovery with a
  structured error instead of advertising a lossy key.

### 3. Non-targeted schema strings remain an injection channel

- Severity: targeted
- Location: `src/server.rs:410`, `src/server.rs:432`
- Attack: `sanitize_schema_metadata` only sanitizes values under `title`,
  `description`, `$comment`, `examples`, and `enum`. JSON Schema has other
  upstream-authored strings that LLMs read and that can carry prompt text:
  `default`, `const`, `pattern`, `format`, `$id`, `$ref`, `deprecated` notes in
  extension keys, and arbitrary vendor extension strings. A malicious upstream
  can return a schema with `default: "\nIGNORE PRIOR INSTRUCTIONS"` or
  `pattern: "(?x) ... # IGNORE PRIOR INSTRUCTIONS"`; `get_tool_schema` returns
  it verbatim. Recursion reaches nested metadata keys, but it intentionally
  leaves these non-targeted strings untouched, so the current tests miss the
  bypass.
- Confidence: medium-high.
- Suggested fix: define a stricter allow/deny policy for all LLM-visible schema
  strings. At minimum sanitize/cap annotation-like string values beyond the
  five current keys, including `default`, `const`, `$id`, `$ref`, `pattern`,
  `format`, and extension-key strings. Where changing the value would alter
  schema semantics, document and test the exception explicitly rather than
  leaving it as an accidental bypass.

### 4. Broken-pipe send errors can be misclassified as `upstream_call_failed`

- Severity: targeted
- Location: `src/registry.rs:247` / `map_service_error`
- Attack: dead-upstream classification depends on
  `matches!(e, ServiceError::TransportClosed)` only. The pinned rmcp error
  surface also has `ServiceError::TransportSend(DynamicTransportError)` for
  send failures. If a stdio child dies and the next request first observes the
  failure while writing to the pipe, rmcp can surface a send error rather than
  `TransportClosed`; this code then reports `upstream_call_failed` instead of
  the Phase 4 `upstream_disconnected` code. The test kills the probe and passes
  on this machine, but it does not prove every broken-pipe path or platform
  maps to `TransportClosed`.
- Confidence: medium. Context7 confirms the variant exists; the exact rmcp
  transport branch for every broken-pipe timing needs source-level confirmation.
- Suggested fix: classify transport-send errors that represent closed/broken
  pipes as `UpstreamDisconnected` too. Prefer matching the concrete dynamic
  transport error kind if rmcp exposes it; otherwise conservatively map all
  `TransportSend` from an already-established upstream operation to
  `upstream_disconnected` and reserve `upstream_call_failed` for MCP-level
  errors from a live peer.

### 5. Failed lazy refetch clears dirty before a fresh cache is installed

- Severity: targeted
- Location: `src/registry.rs:220`, `src/registry.rs:229`
- Attack: `ensure_fresh` does `dirty.swap(false)` before awaiting
  `list_all_tools()`. If the refetch fails after a `list_changed` notification
  (for example, the upstream dies between notification and refetch), the dirty
  flag stays false and the stale cache remains installed. The first caller sees
  an error, but a later `list_tools`/`inventory()` fast-paths through
  `dirty == false` and returns stale inventory for a dead or changed upstream.
  That is a silent stale-cache recovery after a failed refresh, not a fresh
  view or a repeatable disconnect signal.
- Confidence: high.
- Suggested fix: only clear dirty after a successful refetch/install, or set it
  back to true on refetch failure. A three-state `Clean / Dirty / Refreshing`
  flag or a per-entry refresh mutex would also close the concurrent stale-read
  window without holding the registry map lock across `list_all_tools().await`.

## Verified invariants

- No Phase 4 source path holds the registry `entries` lock across
  `call_tool().await` or `list_all_tools().await`; `get_or_connect` clones the
  `Arc<UpstreamEntry>` and drops the map lock before upstream awaits.
- `invoke_tool` result content is not sanitized or stringified in the changed
  path; `registry.call_tool` returns the upstream `CallToolResult` directly.
- `on_tool_list_changed` only stores an atomic flag and returns a ready future;
  it does not await, touch the registry map, log, or panic.
- No test-name-shaped implementation branch or hardcoded fixture string was
  found in `src/` for the Phase 4 paths.

Lens verdict: PASS-with-issues
