---
Feature: phase-4-error-sanitization
Scope: flat
Stack: rust
Tier: THOROUGH
Status: draft
Created: 2026-06-27
Target: src/error.rs, src/registry.rs, src/server.rs, src/forward.rs
Dependencies: docs/master-plans/phase-3-credentials-lifetime/master.md; docs/MVP.md Phase 4
---

# Master Plan: Phase 4 Error Hardening + Sanitization

## What

Ship MVP Phase 4: complete upstream failure coverage in the structured tool-error
model, sanitize upstream-authored tool metadata before it reaches LLM-visible
discovery/schema text, invalidate per-session upstream tool inventories on
`notifications/tools/list_changed`, and preserve sibling isolation when one
upstream dies mid-session.

## Why

The binding scope anchor is `docs/MVP.md` Phase 4. It requires finalized
`AggError`/`ErrorCode` coverage for spawn failure, timeout, call error, and
mid-session upstream death; upstream-provided tool names/descriptions sanitized
before inclusion in LLM-readable text; cache invalidation on upstream
`notifications/tools/list_changed`; and tests proving crash isolation,
`always_error` passthrough, and clean `needs_sampling` rejection.

The plan is grounded in `docs/DECISIONS.md` D-004, D-005, D-007, D-008, D-015,
and D-016. D-004 requires raw argument passthrough and byte-faithful result
content. D-005 makes the error JSON shape public API: `server`, `tool`, `code`,
`message`, `recoverable` inside `CallToolResult { isError: true }`, never a
JSON-RPC error for tool-level upstream failures. D-007 forbids holding the
registry map lock across an upstream await. D-008 requires reverse-traffic
requests to be answered immediately, with sampling/elicitation rejected in MVP.
D-015 pins rmcp exactly and treats design-doc snippets as pseudocode until
checked against the pin. D-016 makes the in-repo probe server the integration
test fixture.

`docs/GOTCHA.md` sharpens the same requirements: #1 forbids stdout diagnostics;
#2 says unanswered upstream-originated requests hang the upstream; #3 says
tool-level failures must not become JSON-RPC errors; #4 forbids stringifying
result content arrays; #16 preserves lock discipline; #20 names upstream
tool-name/description text as a prompt-injection channel bounded by stripping
newlines/control characters and length-capping.

`docs/ARCHITECTURE.md` states the LLM-visible `list_tools` rows are
`{ server, tool, description }` with descriptions sanitized and truncated to
about 100 characters; the per-session `tools/list` cache is invalidated on
`list_changed`; and all upstream failures become structured results. `docs/AGG-MCP.md`
states that `on_tool_list_changed` sends the server name to invalidate the cache,
but its code is pseudocode. rmcp verification against the `=1.8.0` pin confirms
`ClientHandler::on_tool_list_changed(&self, context: NotificationContext<RoleClient>)
-> impl Future<Output = ()> + MaybeSendFuture + '_` exists; implementers must
still let the compiler settle exact imports and future style.

`SECURITY.md` already claims the aggregator sanitizes and length-caps upstream
descriptions. Phase 3's knowledge-sync flagged that as doc/implementation drift.
Phase 4 makes the claim true. This is corrected drift, not a reason to edit
`SECURITY.md` in this plan.

`docs/master-plans/phase-3-credentials-lifetime/master.md` explicitly deferred
this phase's scope: sanitization of upstream-provided names/descriptions, final
crash hardening for mid-session upstream death, and
`notifications/tools/list_changed` cache invalidation.

Verified current tree state:

| Surface | Verified file | Current state | Phase 4 adds |
|---|---|---|---|
| Error model | `src/error.rs` | The enum is `ToolError`, with public wire codes emitted by private `structured_error(...)`. Existing codes cover invalid request, namespace, unknown server/tool, connect failure, call failure, timeout, cancellation, and credential resolution. | Complete coverage for mid-session upstream death / closed pipe / dead connection with a new finalized public code. Audit every upstream communication path returns a structured result. |
| Discovery rows | `src/server.rs` | `handle_list_tools` emits upstream `tool.name` twice and `tool.description.unwrap_or_default()` directly into JSON text. | Sanitize every upstream-authored name and description before row text reaches the LLM; cap description rows around 100 characters. |
| Schema result | `src/server.rs` | `handle_get_tool_schema` returns the upstream `input_schema` object as JSON text. The schema may contain upstream-authored `title` / `description` / `$comment` / property names. | Sanitize upstream-authored schema text fields without sanitizing tool-call result content or changing argument passthrough. Preserve JSON shape. |
| Registry cache | `src/registry.rs` | `UpstreamEntry.tools` is an immutable `Vec<Tool>` captured at connect time inside an `Arc<UpstreamEntry>`. `inventory()` clones it; `call_tool()` checks it before forwarding. | Make per-entry inventory mutable/refreshable without holding the registry map lock across upstream awaits. Invalidate/refetch on `list_changed`; treat stale dead entries as structured upstream-death errors, not sibling poison. |
| Upstream handler | `src/forward.rs` | `UpstreamClientHandler` handles sampling, elicitation, roots, logging, and progress. It has no `on_tool_list_changed` handler and no channel back to the registry. | Add a per-connection invalidation callback/channel from the handler to the registry entry for that server. Do not push notifications downstream. |
| rmcp pin | `Cargo.toml` | `rmcp = "=1.8.0"` with `server`, `client`, `transport-io`, and `transport-child-process` features. | Keep the pin exact. Do not bump rmcp while adding the handler. |
| Probe fixture | `tests/probe-server/main.rs` | Exposes `echo_ok`, `always_error`, `slow_tool`, `dangerous_noop`, `needs_sampling`, non-text/image and reverse-traffic helpers, env, and grandchild helpers. | Test-creator may extend the fixture only if needed for malicious metadata or `list_changed`; planner does not edit tests. |

Corrected drift: `docs/MVP.md` and `docs/ARCHITECTURE.md` name `AggError` /
`ErrorCode`, but the current code's internal enum is `ToolError`. The public
contract is not the Rust enum name; it is the wire JSON shape and code strings
from `structured_error(...)`. The default for this phase is to complete failure
mode coverage on the existing `ToolError` and add any needed public code strings,
not to rename the enum. Renaming would add churn and risk public-code drift with
no contract benefit.

## Dependencies

- Phase 0/1/2/3 are prerequisites. The current tree shows static meta-tools,
  lazy registry entries, namespace ACLs, byte-faithful call forwarding, reverse
  traffic rejection, per-server timeouts, credential resolution, redaction, and
  process containment surfaces already present.
- This plan is sequenced after Phase 3 and before Phase 5. It must not pull in
  Phase 5's cross-platform CI gates, token benchmark, or carried process
  lifetime issues.
- The test-creator can derive Phase 4 tests from the Success Criteria. Tests are
  the read-only contract for implementer, simplifier, debugger, and reviewers.
- Shared-file ordering is explicit. `src/registry.rs` is touched by error
  hardening and cache invalidation; `src/server.rs` is touched by sanitization
  and error wrapping. Those concerns are sequenced, not assigned to concurrent
  writers.
- File-disjoint work can be parallelized only after tests exist and dependencies
  are clear: the pure sanitization helper in `src/server.rs` can proceed apart
  from `src/forward.rs` handler wiring, but final integration through
  `src/registry.rs` is sequenced.

## Scope

### In

- Finalize upstream failure coverage on the existing `ToolError` model.
- Add a structured code for mid-session upstream death / closed pipe / dead
  connection, with `server`, `tool`, `code`, `message`, and `recoverable` fields.
- Audit spawn/connect, inventory fetch, list refetch, timeout, call, and dead
  connection paths so each upstream failure is returned as
  `CallToolResult { isError: true }`, never a JSON-RPC error.
- Ensure an upstream dying mid-session does not poison sibling entries or block
  sibling tool calls.
- Sanitize upstream-authored tool names and descriptions before they enter
  `list_tools` result rows.
- Sanitize upstream-authored strings embedded in the JSON text returned by
  `get_tool_schema` when those strings are metadata visible to the LLM.
- Cap `list_tools` description text around 100 characters after control/newline
  stripping.
- Keep tool-call result content byte-faithful; sanitization does not apply to
  `invoke_tool` results.
- Add cache invalidation/refetch for upstream
  `notifications/tools/list_changed`, scoped to that upstream only.
- Wire `src/forward.rs` per-connection `on_tool_list_changed` handling back to
  `src/registry.rs` without downstream notification fan-out.
- Preserve the three public meta-tool names and their static descriptions.
- Preserve the exact rmcp `=1.8.0` pin.

### Out

- Pushing `notifications/tools/list_changed` down to the downstream client. That
  is v1.1.
- Capability-mirrored sampling/elicitation forwarding. MVP keeps clean rejection
  only.
- Resource proxying, prompt proxying, `readonly` enforcement, progress
  forwarding, `list_server_status`, OAuth, HTTP transport expansion, install
  helpers, warm/cache disk behavior, and hot-reload.
- Parameter-level ACLs, argument sanitization, SQL/path/file policy, and
  validation of upstream tool arguments.
- Sanitizing or transforming tool-call result content. Results remain
  byte-faithful per D-004.
- Reworking Phase 1/2/3 surfaces: namespace ACLs, credential stores,
  credential resolution, timeout configuration, cancellation behavior,
  redaction, process spawn API, or process-tree lifetime, except for the minimal
  touches forced by an in-scope upstream-death or cache-invalidation path.
- The two carried Phase 3 process-lifetime issues:
  `issue-windows-jobobject-spawn-race` and
  `issue-unix-hardkill-containment`. Those are Phase 5, not Phase 4.
- Renaming `ToolError` to `AggError` or introducing a public Rust error type
  solely to match old spec wording.
- Changing the public structured-error JSON field names, removing fields, or
  changing the three public meta-tool names/static descriptions.
- Editing tests after `test-creator` writes them. Later stages treat tests as
  read-only.

## Phases

### Phase 1 — Error model finalization and dead-upstream coverage

**Scope:** Complete structured error coverage for all upstream communication
paths and make mid-session upstream death observable as a tool-level structured
result. This phase owns the `ToolError` coverage decision and must not rename the
enum unless a blocking compiler/API reason appears.

**Produces:** `src/error.rs`, `src/registry.rs`, `src/server.rs`. Test files are
produced only by `test-creator`.

**Key Behaviors:**

- Add a public wire code for dead upstream / closed pipe / connection lost. A
  suitable default is `upstream_disconnected`.
- Keep `structured_error(...)` output D-005-compatible: `server`, `tool`,
  `code`, `message`, `recoverable`.
- Wrap `inventory()` failures and `call_tool()` failures caused by a dead
  `RunningService` / closed transport into `ToolError`, not `McpError` to the
  downstream client.
- Detect stale entries when an upstream dies between calls. A later call to that
  server must return a structured error or reconnect only if the existing
  lifecycle policy explicitly supports reconnect. Default: no silent reconnect
  in this phase; return a clear recoverable structured error.
- Do not remove a failed entry in a way that races sibling calls or forces global
  registry failure. Failure is per-server.
- Preserve timeout and cancellation codes added in Phase 3.
- Preserve byte-faithful `Ok(result)` passthrough for upstream tool results,
  including upstream `CallToolResult::error(...)` from `always_error`.
- Preserve the rule that malformed meta-tool requests may still be JSON-RPC-valid
  tool results; protocol-level bad params remain the only JSON-RPC-error class.

**Depends On:** Phase 3 timeout/cancellation/error surfaces.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

- Killing the `probe` upstream after initial discovery makes a subsequent
  `invoke_tool probe__echo_ok` return `CallToolResult { isError: true }` with
  D-005 fields and the new dead-upstream code.
- A dead `probe` upstream does not prevent a sibling upstream from answering
  `echo_ok` successfully in the same aggregator session.
- `probe__always_error` still returns the probe's own structured error content
  byte-faithfully, not wrapped as `upstream_call_failed` or stringified content.
- `probe__needs_sampling` still returns promptly through the existing clean
  rejection path and never hangs.

### Phase 2 — LLM-visible upstream string sanitization

**Scope:** Sanitize upstream-authored names and descriptions where they are
rendered into text the LLM reads. Keep the implementation small; prefer local
helpers in `src/server.rs` unless duplication or tests prove a tiny helper module
earns its place.

**Produces:** `src/server.rs`; optionally a small helper file only if justified
by shared sanitization logic. Test files are produced only by `test-creator`.

**Key Behaviors:**

- Strip newlines and control characters from upstream-authored tool names and
  descriptions before constructing `list_tools` rows.
- Cap sanitized descriptions around 100 characters per row after stripping.
- Preserve namespace filtering against the real upstream tool name, not the
  sanitized display name.
- Preserve `invoke_tool` dispatch against the real upstream tool name. Sanitized
  names are display text only and must not become the call key.
- Sanitize upstream-authored string metadata in the `get_tool_schema` JSON text.
  Required default: recursively sanitize string values on schema metadata keys
  such as `title`, `description`, `$comment`, `examples`, and `enum` display
  strings where they are upstream-authored, while preserving JSON validity.
- Do not sanitize `invoke_tool` arguments or results.
- Do not change the three static meta-tool descriptions. They are not
  upstream-authored.

**Depends On:** None for pure row sanitization; schema sanitization must respect
the current `handle_get_tool_schema` inventory flow.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

- A malicious upstream description containing embedded `\n`, `\r`, tab/control
  characters, and more than 100 visible characters appears in `list_tools` as a
  single-line, control-free, capped description.
- A malicious upstream tool name containing control/newline characters appears
  in LLM-visible discovery text only in sanitized form.
- `get_tool_schema` returns valid JSON and sanitizes upstream-authored metadata
  strings that are visible to the LLM.
- `invoke_tool` result content remains byte-faithful and is not sanitized,
  stringified, or transformed.

### Phase 3 — `list_changed` cache invalidation wiring

**Scope:** Wire upstream `notifications/tools/list_changed` into registry cache
invalidation for that upstream only. This phase owns the immutable-cache design
change in `src/registry.rs` and the per-connection callback/channel in
`src/forward.rs`.

**Produces:** `src/registry.rs`, `src/forward.rs`. Test files are produced only
by `test-creator`.

**Key Behaviors:**

- Add `UpstreamClientHandler::on_tool_list_changed` using the rmcp `=1.8.0`
  signature verified above.
- Construct the handler in `registry::connect` with a per-server invalidation
  path. Acceptable default: a lightweight `tokio::sync` channel/watch/atomic flag
  owned by the `UpstreamEntry`, not a back-reference that creates an `Arc` cycle.
- Replace immutable `UpstreamEntry.tools: Vec<Tool>` with a per-entry mutable
  cache shape. Default: store the cached inventory behind a lock plus a dirty
  flag, or store it in a separate registry cache keyed by server. Either is
  acceptable only if no registry map lock is held across `list_all_tools().await`.
- On `list_changed`, mark only that server's cache dirty. Do not contact the
  upstream inside the notification handler if that risks blocking rmcp's
  notification path; refetch lazily on the next `inventory()` / tool validation.
- On the next `inventory()` after invalidation, refetch with `list_all_tools()`
  and update the cache. If refetch fails because the upstream died, return the
  structured dead-upstream error from Phase 1.
- Preserve `call_tool()` validation semantics. It may read a fresh inventory if
  the cache is dirty before checking tool existence.
- Do not send downstream `tools/list_changed` notifications. That is v1.1.

**Depends On:** Phase 1 error code for failed refetch/dead upstream.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

- An upstream `notifications/tools/list_changed` marks only that server's cached
  inventory stale.
- A second `list_tools` / `inventory()` after the notification reflects the
  upstream's changed tool list without restarting the aggregator.
- A sibling server's inventory is not refetched or invalidated because another
  server sent `list_changed`.
- No registry map lock is held across `list_all_tools().await` or
  `call_tool().await`.

### Phase 4 — Integration gate and scope audit

**Scope:** Run the full Phase 4 gate, verify the public API did not drift, and
audit for scope creep before review. This phase is sequenced last and touches
source only for targeted fixes required by the tests.

**Produces:** Source fixes only if a Phase 4 gate failure requires them. No test
edits. No `state.json` edits.

**Key Behaviors:**

- Run the test command selected by `test-creator`; 100% pass is required.
- Confirm downstream `tools/list` still exposes exactly the three public
  meta-tools.
- Confirm the static meta-tool names and descriptions are unchanged.
- Confirm all structured tool errors retain the D-005 fields, with only additive
  new codes.
- Confirm no stdout diagnostics were added on the serve path.
- Confirm sanitization applies only to upstream-authored metadata and not to
  tool-call results.
- Confirm rmcp remains pinned exactly at `=1.8.0`.
- Confirm no Phase 5 or v1.1 out-of-scope feature landed.

**Depends On:** Phases 1, 2, and 3.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

- The complete Phase 4 test suite passes at 100%.
- `cargo test` or the project-selected test command exits 0.
- `git diff` for source changes is limited to the in-scope files/behaviors and
  contains no edits to test files by non-test-creator stages.
- Public meta-tool surface and D-005 structured-error shape remain compatible.

## Success Criteria

1. `list_tools` returns sanitized rows for upstream-authored names/descriptions:
   embedded `\n`, `\r`, tab/control characters are absent from LLM-visible row
   text.
2. `list_tools` caps each upstream-authored description row at about 100 visible
   characters after sanitization.
3. A probe or fixture tool with an upstream description containing newlines,
   control characters, and more than 100 characters is observably emitted as a
   single-line capped description.
4. `get_tool_schema` for an upstream tool returns valid JSON and sanitizes
   upstream-authored metadata strings visible to the LLM, without changing the
   schema's object shape needed by callers.
5. Sanitization does not apply to `invoke_tool` result content. Non-text content
   and structured content continue to pass byte-faithfully.
6. Killing an upstream mid-session makes a later call to that upstream return
   `CallToolResult { isError: true }` with `server`, `tool`, `code`, `message`,
   and `recoverable`; the `code` is a finalized dead-upstream code such as
   `upstream_disconnected`.
7. After one upstream dies mid-session, a sibling upstream remains callable and
   returns a successful `echo_ok` result in the same aggregator session.
8. `probe__always_error` round-trips the probe's upstream-provided error result
   intact; it is not converted into a JSON-RPC error, double-wrapped, or
   stringified.
9. `probe__needs_sampling` receives a clean rejection path and completes without
   hanging.
10. An upstream `notifications/tools/list_changed` invalidates only that
    upstream's cached inventory.
11. A second `list_tools` / `inventory()` after `list_changed` reflects the
    changed upstream tool inventory without restarting fanin-mcp.
12. The public downstream `tools/list` surface still exposes exactly three
    meta-tools: `list_tools`, `get_tool_schema`, and `invoke_tool`.
13. The static names and descriptions of the three public meta-tools do not
    change.
14. The structured-error JSON shape remains D-005-compatible: no field rename,
    no field removal, and only additive new `code` values.
15. The registry never holds the entries/map lock across `call_tool().await` or
    `list_all_tools().await`; slow/dead upstream behavior cannot serialize
    sibling calls.
16. The rmcp dependency remains pinned exactly to `=1.8.0`.
17. No serve-path `println!`, `print!`, or `dbg!` reaches stdout.

## Constraints / Invariants

- stdout is the MCP transport. No stdout diagnostics on or after `serve(stdio())`.
- Tool-level failures are `CallToolResult { isError: true }`, never JSON-RPC
  errors. The D-005 JSON shape is public API and additive-only.
- Results pass byte-faithfully. Never `to_string()` a content array or convert
  non-text content into text. Sanitization applies only to upstream-authored
  name/description/schema metadata surfaced to the LLM, never to tool-call
  results.
- Raw `invoke_tool` arguments pass through unchanged.
- Never hold the registry map lock across an upstream await. Clone/access the
  per-server handle/cache state, drop the map lock, then await.
- rmcp stays pinned at `=1.8.0`. Any rmcp signature used for
  `on_tool_list_changed`, notification contexts, or peer APIs must be verified
  against the pin; `docs/AGG-MCP.md` snippets are pseudocode.
- The three public meta-tool names and static descriptions do not change.
- Cache invalidation from upstream `list_changed` is per-session and per-server.
  No downstream push in MVP.
- Capability-mirrored sampling/elicitation forwarding remains out of scope. MVP
  rejection must stay clean and immediate.
- Versioning for this checkpoint series is `v0.5.x` if a later orchestrator asks
  for commits; this planner does not commit.

## Open Questions

1. **Internal error type name.** The docs say `AggError` / `ErrorCode`; the tree
   uses `ToolError`. Proposed default: keep `ToolError` and finalize coverage +
   public code strings. Rename only if the orchestrator explicitly chooses churn
   for naming consistency.
2. **Cache mutability shape.** `UpstreamEntry.tools` is currently immutable inside
   `Arc<UpstreamEntry>`. Proposed default: make the inventory a per-entry mutable
   cache with a dirty flag and lazy refetch on next inventory/call validation,
   with no registry map lock held across the refetch. A separate registry-level
   `tool_cache` is also valid if it preserves the same lock discipline and
   per-server isolation.
3. **Reconnect after death.** The docs require mid-session death to surface as a
   structured error; they do not require automatic reconnect. Proposed default:
   return `upstream_disconnected` for the dead entry and leave reconnect policy
   out of Phase 4 unless existing rmcp/service behavior already reconnects
   safely without extra design.
