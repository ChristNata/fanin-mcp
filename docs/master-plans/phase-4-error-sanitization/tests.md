# test-creator: phase-4-error-sanitization

Phase 4 test contract — error hardening, LLM-visible sanitization, and
`notifications/tools/list_changed` cache invalidation. The implementer
codes against this suite; the objective gate runs it. Test files are
read-only to every later stage.

## Stack & runner

- **Runner:** `cargo test --test integration` (main suite). `cargo nextest
  run --workspace` works equivalently. Inherits the Phase 0/1/2/3 harness
  unchanged — wire-level JSON-RPC-over-stdio, no `src/` stubs.
- **Async:** `#[tokio::test]` single-threaded default; the Phase 4
  concurrency guard (`phase4_guard::slow_upstream_does_not_serialize_concurrent_sibling`)
  uses `flavor = "multi_thread", worker_threads = 2`.
- **Wire-level (D-015).** Tests spawn the built `fanin-mcp` binary and speak
  raw JSON-RPC over stdio, driving the real `probe-server` as upstream and
  asserting on the returned JSON. Tests reference NO `src/` symbols; they
  depend only on `tokio`, `serde_json`, `tempfile` (dev-deps), and
  `CARGO_BIN_EXE_fanin-mcp` / `CARGO_BIN_EXE_probe-server`. The suite
  compiles clean against the CURRENT (pre-Phase-4) tree and fails RED on
  absent behavior, not on missing symbols or compile errors.
- **Build order.** `cargo test` builds the `fanin-mcp` and `probe-server`
  `[[bin]]` targets before the test binary, so `CARGO_BIN_EXE_fanin-mcp` and
  `CARGO_BIN_EXE_probe-server` resolve. Env var names use the bin names
  EXACTLY as-declared (dashes/case preserved).

## Red-until-Phase-4-lands (the contract, by design)

These tests are **RED** until the implementer lands Phase 4. That is the
contract: a red test gives the implementer a target; a green test before
implementation would be a hollow assertion. The suite compiles clean and
every test that fails does so on a **behavior assertion**, not on a compile
error, a missing symbol, or a malformed harness. The implementer turns each
RED green by building the Phase 4 logic in `src/error.rs`, `src/server.rs`,
`src/registry.rs`, and `src/forward.rs`.

Against the current (pre-Phase-4) tree, the run-and-fail state is:
**94 passed, 5 failed RED, 3 ignored.** The 5 RED failures are exactly the
Phase 4 behaviors that do not exist yet (enumerated in §Run-and-fail
confirmation below).

## Files created / extended

| Path | Criteria covered |
|---|---|
| `tests/probe-server/main.rs` (extended) | Adds `poison_meta` (poisoned description), `poison_schema` (poisoned schema metadata), `mutate_tools` (runtime tool-list toggle + `notifications/tools/list_changed` emission), `self_pid` (returns the probe's own PID for the mid-session-death proof), and the runtime-added `added_tool` (toggled by `mutate_tools`). Probe now exposes 14 static tools (+1 dynamic when toggled). |
| `tests/common/fixtures.rs` (unchanged) | The existing `MultiConfigBuilder` already supports N named servers; Phase 4 reuses it for the `probe` + `probe2` two-upstream configs. No new fixture builder needed. |
| `tests/integration/error_hardening.rs` (new) | Master SC 6, 7, 8, 9; contributes to SC 14 / SC 15. |
| `tests/integration/sanitization.rs` (new) | Master SC 1, 2, 3, 4, 5. |
| `tests/integration/list_changed.rs` (new) | Master SC 10, 11. |
| `tests/integration/phase4_guard.rs` (new) | Master SC 12, 13, 14, 15, 16, 17. |
| `tests/integration/main.rs` (extended) | `mod error_hardening;`, `mod list_changed;`, `mod phase4_guard;`, `mod sanitization;` declarations. |
| `tests/integration/probe.rs`, `discovery.rs`, `multi_upstream.rs`, `namespace_acl.rs` (extended) | `PROBE_TOOL_NAMES` constant updated 10 → 14 to reflect the extended probe; prose comments + count assertions updated. Behavioral assertions unchanged. |

Phase 0/1/2/3 behavioral guarantees are preserved. The only change to
existing tests is the probe tool-count constant (10 → 14), which is a
factual correction matching the extended probe fixture — not a weakening of
any behavioral assertion. The static-3-meta-tools, byte-faithful,
reverse-traffic, lazy-startup, namespace-ACL, timeout, cancellation,
process-lifetime, and credential invariants are unchanged.

## Probe-fixture additions

The probe fixture (`tests/probe-server/main.rs`) is extended with four new
static tools and one dynamic tool, all owned by `test-creator`:

- **`poison_meta`** — a tool whose DESCRIPTION carries embedded `\n`, `\r`,
  tab, vertical tab (`\u{000B}`), form feed (`\u{000C}`), and well over 100
  visible characters (including a "IGNORE previous instructions"
  prompt-injection payload). The REAL tool name is clean (`poison_meta`)
  because rmcp validates tool names on registration; the poisoned content
  lives in the description so the aggregator's description sanitization is
  what the test exercises. Used by the SC 1/2/3 proof.
- **`poison_schema`** — a tool whose `input_schema` JSON object carries
  upstream-authored `title`, `description`, and `$comment` strings with
  embedded control chars and long content. Used by the SC 4 proof: the
  aggregator must sanitize those metadata strings while preserving the
  schema's structural shape (`type`, `properties`, `required`, property
  keys).
- **`mutate_tools`** — toggles a runtime-added `added_tool` in the probe's
  tool list, then emits `notifications/tools/list_changed` toward the
  aggregator via `peer.notify_tool_list_changed()` (detached task so the
  probe's `call_tool` returns immediately). Used by the SC 10/11 proof.
  The toggle is repeatable (a second call removes the tool), proving the
  wiring is not a one-shot.
- **`self_pid`** — returns the probe's own process id as a decimal string.
  Used by the SC 6/7 mid-session-death proof: the test discovers the
  upstream, asks the probe for its PID, kills that PID mid-session, then
  asserts a subsequent call returns `upstream_disconnected` while a sibling
  stays callable. Without this tool the test could not address the probe's
  PID specifically (it is a grandchild of the test, spawned by fanin-mcp).
- **`added_tool`** (dynamic) — the runtime-added tool toggled by
  `mutate_tools`. Clean name + description so dispatch and discovery both
  work once it is visible. Appears in `list_tools` only when `MUTATE_ADDED`
  is set.

The probe now exposes 14 static tools. The Phase 0/1/2/3 `PROBE_TOOL_NAMES`
constant is updated 10 → 14 in `probe.rs`, `discovery.rs`,
`multi_upstream.rs`, and `namespace_acl.rs` — a factual correction, not a
weakening.

## Coverage map — master Success Criteria

| # | Master Success Criterion | Test(s) |
|---|---|---|
| 1 | `list_tools` returns sanitized rows for upstream-authored names/descriptions: embedded `\n`, `\r`, tab/control chars absent from LLM-visible row text | `sanitization::list_tools_sanitizes_poisoned_description_strips_control_and_caps_length`; `sanitization::list_tools_row_names_are_control_char_free` |
| 2 | `list_tools` caps each upstream-authored description row at about 100 visible characters after sanitization | `sanitization::list_tools_sanitizes_poisoned_description_strips_control_and_caps_length` |
| 3 | A probe/fixture tool with an upstream description containing newlines, control chars, and more than 100 chars is observably emitted as a single-line capped description | `sanitization::list_tools_sanitizes_poisoned_description_strips_control_and_caps_length` |
| 4 | `get_tool_schema` for an upstream tool returns valid JSON and sanitizes upstream-authored metadata strings visible to the LLM, without changing the schema's object shape needed by callers | `sanitization::get_tool_schema_sanitizes_poisoned_metadata_preserves_shape` |
| 5 | Sanitization does not apply to `invoke_tool` result content: non-text and structured content pass byte-faithfully | `sanitization::invoke_tool_result_content_not_sanitized_passes_byte_faithfully`; `sanitization::invoke_tool_dispatches_on_real_tool_name_not_sanitized_display` (dispatch on real name, not sanitized display) |
| 6 | Killing an upstream mid-session makes a later call to that upstream return `CallToolResult { isError: true }` with `server`, `tool`, `code`, `message`, `recoverable`; the `code` is `upstream_disconnected` | `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` |
| 7 | After one upstream dies mid-session, a sibling upstream remains callable and returns a successful `echo_ok` result in the same aggregator session | `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` |
| 8 | `probe__always_error` round-trips the probe's upstream-provided error result intact; not converted into a JSON-RPC error, double-wrapped, or stringified | `error_hardening::always_error_round_trips_upstream_error_content_byte_faithfully` |
| 9 | `probe__needs_sampling` receives a clean rejection path and completes without hanging | `error_hardening::needs_sampling_completes_without_hanging` |
| 10 | An upstream `notifications/tools/list_changed` invalidates only that upstream's cached inventory | `list_changed::list_changed_invalidates_only_that_server_not_sibling` |
| 11 | A second `list_tools` / `inventory()` after `list_changed` reflects the changed upstream tool inventory without restarting fanin-mcp | `list_changed::list_changed_notification_invalidates_cache_reflects_new_inventory` |
| 12 | The public downstream `tools/list` surface still exposes exactly three meta-tools | `phase4_guard::phase4_context_preserves_three_meta_tools_and_static_descriptions`; Phase 0/1/2/3 tests unchanged |
| 13 | The static names and descriptions of the three public meta-tools do not change | `phase4_guard::phase4_context_preserves_three_meta_tools_and_static_descriptions` (asserts each description verbatim via `exp::assert_desc`) |
| 14 | The structured-error JSON shape remains D-005-compatible: no field rename, no field removal, and only additive new `code` values | `phase4_guard::structured_error_json_keeps_d005_fields_additive_codes`; `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` (asserts all five D-005 fields + the new `upstream_disconnected` code) |
| 15 | The registry never holds the entries/map lock across `call_tool().await` or `list_all_tools().await`; slow/dead upstream behavior cannot serialize sibling calls | `phase4_guard::slow_upstream_does_not_serialize_concurrent_sibling`; `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` (sibling stays callable after probe death) |
| 16 | The rmcp dependency remains pinned exactly to `=1.8.0` | `phase4_guard::rmcp_remains_pinned_exactly_at_1_8_0`; Phase 0 `pinning::cargo_toml_pins_rmcp_exactly_and_lockfile_exists` (unchanged) |
| 17 | No serve-path `println!`, `print!`, or `dbg!` reaches stdout | `phase4_guard::no_stdout_diagnostics_on_phase4_serve_path` (explicit); every wire test implicitly asserts clean JSON on stdout (harness panics on non-JSON) |

## Coverage map — Phase sub-criteria

| Phase | Criterion | Test |
|---|---|---|
| P1.1 | Killing the `probe` upstream after discovery makes a subsequent `invoke_tool probe__echo_ok` return `CallToolResult { isError: true }` with D-005 fields + the new dead-upstream code | `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` |
| P1.2 | A dead `probe` upstream does not prevent a sibling upstream from answering `echo_ok` in the same session | `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` |
| P1.3 | `probe__always_error` round-trips the probe's structured error content byte-faithfully, not wrapped as `upstream_call_failed` or stringified | `error_hardening::always_error_round_trips_upstream_error_content_byte_faithfully` |
| P1.4 | `probe__needs_sampling` returns promptly through the existing clean rejection path and never hangs | `error_hardening::needs_sampling_completes_without_hanging` |
| P2.1 | Malicious upstream description with `\n`, `\r`, tab/control chars, >100 chars appears in `list_tools` as single-line, control-free, capped | `sanitization::list_tools_sanitizes_poisoned_description_strips_control_and_caps_length` |
| P2.2 | Malicious upstream tool name with control/newline chars appears in LLM-visible discovery text only in sanitized form | `sanitization::list_tools_row_names_are_control_char_free` (asserts every row name is control-free; the probe registers clean names because rmcp validates, so the invariant is asserted on all rows) |
| P2.3 | `get_tool_schema` returns valid JSON and sanitizes upstream-authored metadata strings visible to the LLM | `sanitization::get_tool_schema_sanitizes_poisoned_metadata_preserves_shape` |
| P2.4 | `invoke_tool` result content remains byte-faithful and is not sanitized, stringified, or transformed | `sanitization::invoke_tool_result_content_not_sanitized_passes_byte_faithfully` |
| P2.5 | `invoke_tool` dispatches on the REAL (unsanitized) upstream tool name — sanitization is display-only, not the call key | `sanitization::invoke_tool_dispatches_on_real_tool_name_not_sanitized_display` |
| P3.1 | Upstream `notifications/tools/list_changed` marks only that server's cached inventory stale | `list_changed::list_changed_invalidates_only_that_server_not_sibling` |
| P3.2 | A second `list_tools` / `inventory()` after the notification reflects the changed tool list without restarting the aggregator | `list_changed::list_changed_notification_invalidates_cache_reflects_new_inventory` |
| P3.3 | A sibling server's inventory is not refetched or invalidated because another server sent `list_changed` | `list_changed::list_changed_invalidates_only_that_server_not_sibling` |
| P3.4 | No registry map lock is held across `list_all_tools().await` or `call_tool().await` | `phase4_guard::slow_upstream_does_not_serialize_concurrent_sibling`; `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable` |
| P4.1 | The complete Phase 4 test suite passes at 100% | The full suite (Phase 0 + 1 + 2 + 3 + 4) is the gate. Phase 4 of the plan runs it. |
| P4.2 | `cargo test` or the project-selected test command exits 0 | Same — the full suite. |
| P4.3 | `git diff` for source changes is limited to in-scope files/behaviors and contains no edits to test files by non-test-creator stages | Structural — review verifies. No test asserts on git diff content. |
| P4.4 | Public meta-tool surface and D-005 structured-error shape remain compatible | `phase4_guard::phase4_context_preserves_three_meta_tools_and_static_descriptions`; `phase4_guard::structured_error_json_keeps_d005_fields_additive_codes` |

## Side-effect assertions

Every Phase 4 test asserts the observable effect, not just a return
value, so a stub that returns the right shape without doing the work fails.

- **Dead-upstream is a side-effect assertion on the dead PROCESS and the
  structured error.**
  `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable`
  discovers the upstream, asks the probe for its PID via `self_pid`, KILLS
  that PID mid-session (the test kills the upstream directly, simulating an
  external crash — the aggregator's containment layer is NOT involved),
  polls for the process death within a bounded window, THEN asserts a
  subsequent `invoke_tool probe__echo_ok` returns `upstream_disconnected`
  (not `upstream_call_failed`), and a sibling `probe2__echo_ok` still
  succeeds in the same session. A stub that returns a generic error
  without detecting the death fails the `code: "upstream_disconnected"`
  assertion; a stub that serializes the session fails the sibling-isolation
  assertion. The dead PID is the oracle that the kill landed; the
  structured error is the oracle that the aggregator observed it.
- **Sibling isolation is a side-effect assertion on the sibling's
  SUCCESS.** The sibling `probe2` was spawned earlier and must still answer
  `echo_ok` after `probe` dies. A registry that poisoned siblings on one
  death, or that held a lock across the dead-upstream detection, would make
  the sibling call fail or hang.
- **`always_error` round-trip is a side-effect assertion on the upstream's
  OWN error content.**
  `error_hardening::always_error_round_trips_upstream_error_content_byte_faithfully`
  parses the result text as JSON and asserts the probe's `code: "always_error"`
  and `recoverable: false` are present. A re-wrapping stub that replaces
  the content with its own `upstream_call_failed` JSON fails the `code`
  assertion; a stub that always sets `recoverable: true` fails the
  `recoverable: false` assertion.
- **`needs_sampling` clean rejection is a wall-clock side-effect assertion.**
  `error_hardening::needs_sampling_completes_without_hanging` bounds the
  call with `REJECT_DEADLINE` and asserts elapsed < 5s. A stub that hangs
  on the unanswered sampling request (the GOTCHA #2 trap) fails the
  deadline.
- **Description sanitization is a side-effect assertion on the row text.**
  `sanitization::list_tools_sanitizes_poisoned_description_strips_control_and_caps_length`
  asserts the row description is a single line, free of C0 control chars,
  and ≤ 120 visible characters. The raw description is well over 200 chars
  with embedded `\n`/`\r`/tab/VT/FF — a non-sanitizing implementation passes
  it through verbatim and fails all three assertions.
- **Schema metadata sanitization is a side-effect assertion on the JSON
  text.**
  `sanitization::get_tool_schema_sanitizes_poisoned_metadata_preserves_shape`
  parses the returned schema as JSON, asserts the structural shape
  (`type=object`, `properties.key`, `required=["key"]`) is preserved,
  AND asserts the `title` / `description` / `$comment` strings are
  single-line, control-free, and capped. A sanitization that mangled the
  shape fails the structural assertions; a non-sanitizing impl fails the
  metadata assertions.
- **Byte-faithful result content is a side-effect assertion on the
  non-text block.**
  `sanitization::invoke_tool_result_content_not_sanitized_passes_byte_faithfully`
  asserts at least one content block has a non-text `type` (image). A proxy
  that stringified the content array — or that sanitized result content
  (the Phase 4 trap) — would produce only text blocks and fail.
- **`list_changed` cache invalidation is a side-effect assertion on the
  reflected inventory.**
  `list_changed::list_changed_notification_invalidates_cache_reflects_new_inventory`
  triggers `mutate_tools` (which emits the notification), then asserts the
  SECOND `list_tools` includes the `added_tool` row — proving the cache
  was invalidated and refetched. A stub that ignores the notification
  returns the stale cached inventory and the `added_tool` row is missing.
  The test also toggles the tool OFF and asserts the cache invalidates
  again, proving the wiring is repeatable.
- **Per-server scope is a side-effect assertion on the sibling's stable
  inventory.**
  `list_changed::list_changed_invalidates_only_that_server_not_sibling`
  triggers `mutate_tools` on `probe` and asserts `probe2`'s inventory is
  unchanged (same count, same tool set) across the notification. A
  registry-wide invalidation that refetched ALL servers would still pass
  the set assertion (probe2's set is stable), but the concurrency guard
  (`phase4_guard::slow_upstream_does_not_serialize_concurrent_sibling`)
  covers the lock-discipline half — a registry that held a map lock across
  one server's refetch would serialize siblings.
- **Concurrency is a wall-clock side-effect assertion.**
  `phase4_guard::slow_upstream_does_not_serialize_concurrent_sibling`
  issues a slow `alpha__slow_tool` (800ms) and a concurrent `beta__echo_ok`,
  and asserts the beta echo completes within 400ms — strictly shorter
  than the slow delay. A registry lock held across the slow await would
  serialize the session and make the beta echo timeout.
- **rmcp pin is a file-content side-effect assertion.**
  `phase4_guard::rmcp_remains_pinned_exactly_at_1_8_0` reads `Cargo.toml`
  and asserts the rmcp dependency is pinned with exact `=x.y.z` syntax. A
  bumped or unpinned rmcp fails.
- **No stdout diagnostics (GOTCHA #1 / SC 17) is a stream-cleanliness
  side-effect assertion.**
  `phase4_guard::no_stdout_diagnostics_on_phase4_serve_path` drives a
  reverse-traffic exchange AND a `list_changed` exchange (exercising the
  new `on_tool_list_changed` path), then reads any remaining stdout and
  asserts every line parses as JSON. A stray `println!` in the new handler
  or the sanitization helper would corrupt the stream and panic the parse.
  Every wire test also implicitly asserts clean JSON on stdout (the
  harness panics on a non-JSON line).

## Deferred tests

(none)

No Phase 4 test is `#[ignore]`'d. Every criterion is testable wire-level
against the probe fixture. The mid-session-death proof uses the probe's
`self_pid` tool to address the probe's PID directly (no implementation hook
needed); the `list_changed` proof uses the probe's `mutate_tools` tool to
emit the notification; the sanitization proof uses the probe's `poison_meta`
/ `poison_schema` tools to carry the poisoned metadata.

## Coverage gaps & boundaries

These are criteria the suite does NOT fully prove at the wire level, with
the reason and the proxy/boundary that does cover them:

- **P2.2 (malicious upstream tool NAME with control chars).** The probe
  cannot register a tool whose NAME contains control chars — rmcp's
  `validate_and_warn_tool_name` validates names on registration and the
  `Tool::new` constructor enforces the `[A-Za-z0-9_.-]` charset. The
  aggregator's name-sanitization path is therefore asserted indirectly via
  `sanitization::list_tools_row_names_are_control_char_free`, which asserts
  every row's name field is control-char-free. A proxy that forwarded a
  control-bearing upstream name verbatim would fail (the assertion is on
  all rows); the boundary is that the probe cannot construct the poisoned
  name fixture directly. The load-bearing observable — "the LLM-visible
  row name text is control-free" — is still asserted.
- **P3.4 (no map lock across refetch) — concurrency proxy.** The
  `list_changed` tests assert the inventory updates (the refetch happens)
  but do NOT directly assert the map lock is NOT held across the refetch;
  that is covered by the concurrency guard
  (`phase4_guard::slow_upstream_does_not_serialize_concurrent_sibling`),
  which proves a slow upstream does not serialize a sibling. The
  list_changed refetch happens on the next `inventory()` call, which is
  the same lock-discipline path the concurrency guard exercises. A
  registry that held the map lock across `list_all_tools().await` would
  serialize sibling calls — caught by the concurrency guard.
- **P4.3 (git diff limited to in-scope files).** Structural — review
  verifies. No test asserts on git diff content. The test files are
  read-only to non-test-creator stages by convention; the gate runs the
  full suite.
- **Dead-upstream cleanup (side-effect on the killed process).** The
  `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable`
  test kills the probe PID directly (simulating an external crash). The
  aggregator's containment layer is NOT involved in this kill — the test
  owns the killed process. The aggregator must OBSERVE the death on the
  next call (broken pipe / closed transport), not clean up the killed
  process. Process-tree containment on aggregator teardown is covered by
  the Phase 3 `process_lifetime` tests (unchanged). The killed probe PID
  is reaped by the OS; the test polls for its death within a 2s window
  before the dead-upstream call so the assertion runs against a confirmed
  dead upstream.

## Run-and-fail confirmation

The suite compiles clean (`cargo build --tests` — zero warnings),
`cargo fmt --all -- --check` is CLEAN, and `cargo clippy --all-targets` is
CLEAN (zero warnings).

Against the current (pre-Phase-4) tree, the run-and-fail state is:

- **8 Phase 4 tests total** (3 in `error_hardening.rs`, 5 in `sanitization.rs`,
  2 in `list_changed.rs`, 5 in `phase4_guard.rs`). 0 ignored.
- **`cargo test --test integration` (current tree): 94 passed, 5 failed
  RED, 3 ignored.** The 3 ignored are the Phase 0/3 carried ignores
  (manual E2E + keyring round-trip on headless hosts).
- The 5 RED failures are all Phase 4 behavior assertions:
  - `error_hardening::dead_upstream_returns_structured_error_and_sibling_stays_callable`
    — the current tree returns `upstream_call_failed` for a broken pipe,
    not the finalized `upstream_disconnected` code (state.json decision).
    The sibling-isolation half still passes (the current lock discipline
    already supports concurrency), but the `code` assertion fails.
  - `sanitization::list_tools_sanitizes_poisoned_description_strips_control_and_caps_length`
    — the current tree emits `tool.description.unwrap_or_default()` verbatim
    into the row text, so the poisoned description passes through with
    `\n`/`\r`/tab/VT/FF intact and uncapped.
  - `sanitization::get_tool_schema_sanitizes_poisoned_metadata_preserves_shape`
    — the current tree returns the upstream `input_schema` object as JSON
    text verbatim, so the poisoned `title` / `description` / `$comment`
    strings pass through with control chars intact and uncapped.
  - `list_changed::list_changed_notification_invalidates_cache_reflects_new_inventory`
    — the current `UpstreamClientHandler` has no `on_tool_list_changed`
    handler and `UpstreamEntry.tools` is immutable, so the second
    `list_tools` returns the stale cached inventory and the `added_tool`
    row is missing.
  - `list_changed::list_changed_invalidates_only_that_server_not_sibling`
    — same root cause; the `probe` inventory does not update, so the
    `added_tool` assertion fails.
- The 3 Phase 4 tests that PASS are:
  - `error_hardening::always_error_round_trips_upstream_error_content_byte_faithfully`
    (byte-faithful forward already works — the current tree forwards
    upstream `CallToolResult::error` unchanged).
  - `error_hardening::needs_sampling_completes_without_hanging`
    (the Phase 1 reverse-traffic rejection already works).
  - `sanitization::invoke_tool_result_content_not_sanitized_passes_byte_faithfully`
    (byte-faithful non-text forward already works).
  - `sanitization::list_tools_row_names_are_control_char_free`
    (the probe registers clean names, so the assertion holds today; it
    catches a proxy that forwarded a control-bearing name verbatim).
  - `sanitization::invoke_tool_dispatches_on_real_tool_name_not_sanitized_display`
    (dispatch on the real name already works).
- The 5 `phase4_guard` tests all PASS — they are regression guards
  asserting invariants that already hold on the current tree (3 meta-tools,
  static descriptions, D-005 shape, rmcp pin, concurrency, no stdout
  diagnostics). They re-assert the invariants in the Phase 4 context so a
  later regression surfaces here.
- **The red is meaningful:** every RED test fails on a Phase 4 behavior
  assertion (dead-upstream code, description/schema sanitization,
  `on_tool_list_changed` cache invalidation), not on a compile error, a
  missing symbol, or a malformed harness. The implementer turns each RED
  green by building the Phase 4 logic in `src/error.rs` (add
  `upstream_disconnected`), `src/server.rs` (sanitize upstream-authored
  name/description/schema metadata), `src/registry.rs` (mutable per-entry
  cache + lazy refetch), and `src/forward.rs` (`on_tool_list_changed`
  handler wiring).

## Per-module inventory

| Module | Tests |
|---|---|
| `error_hardening` (Phase 4) | 3 |
| `sanitization` (Phase 4) | 5 |
| `list_changed` (Phase 4) | 2 |
| `phase4_guard` (Phase 4) | 5 |
| **Phase 4 total** | **15** (0 ignored) |
| Phase 0/1/2/3 modules (behavior unchanged; probe count corrected 10 → 14) | 94 (3 ignored) |
| **Grand total** | **109** (3 ignored) |

## Notes for the implementer

- **`upstream_disconnected` is the finalized dead-upstream code** (state.json
  `decisions.error-type-name` + `decisions.reconnect-policy`). Add a
  `ToolError` variant (or a code path in the existing `UpstreamCall` /
  broken-pipe detection) that returns `code: "upstream_disconnected"` with
  the D-005 fields (`server`, `tool`, `code`, `message`, `recoverable`).
  The current tree returns `upstream_call_failed` with `message: "Transport
  closed"` for a broken pipe — that fails the SC 6 `code` assertion. The
  `recoverable` field is present in the current tree (true); Phase 4 keeps
  the field and may set it true or false per the death semantics, but MUST
  NOT remove it.
- **No silent reconnect** (state.json `decisions.reconnect-policy`). A dead
  upstream must surface `upstream_disconnected` on the next call, not
  silently reconnect. The test kills the probe PID directly; the aggregator
  observes the broken pipe on the next `call_tool` and returns the
  structured error. Reconnect policy is out of scope for Phase 4.
- **Sanitization is display-only.** `src/server.rs::handle_list_tools`
  currently emits `tool.description.unwrap_or_default()` verbatim and
  `tool.name` twice into the row JSON. Phase 4 must strip C0 control chars
  (0x00–0x1F + 0x7F) from the upstream-authored `description` (and any
  LLM-visible name text) and cap the description at ~100 visible characters
  AFTER stripping. `handle_get_tool_schema` must sanitize the upstream-
  authored metadata strings (`title`, `description`, `$comment`,
  `examples`, `enum` display strings) in the returned JSON text while
  preserving the schema's structural shape (`type`, `properties`,
  `required`, property keys). Sanitization does NOT apply to `invoke_tool`
  arguments or results (D-004 byte-faithfulness). The description cap is
  "about 100" — the test accepts ≤ 120 to give the implementer room to cap
  at 100, 120, or a similar round number; the load-bearing assertion is
  that a description well over 200 chars is NOT emitted verbatim.
- **`invoke_tool` dispatch is on the REAL upstream tool name.** The
  sanitized display name is NOT the call key. `handle_invoke_tool` splits
  on `__`, checks the namespace ACL against the real server/tool, and
  forwards. Sanitization must not change the dispatch path. The probe's
  `poison_meta` / `poison_schema` tools have clean real names; the poisoned
  content lives in the description / schema metadata, so dispatch still
  works and the test asserts both the sanitized display AND the real-name
  dispatch.
- **`on_tool_list_changed` wiring.** `src/forward.rs::UpstreamClientHandler`
  currently has no `on_tool_list_changed` handler. Add one using the rmcp
  `=1.8.0` signature
  (`ClientHandler::on_tool_list_changed(&self, context: NotificationContext<RoleClient>)
  -> impl Future<Output = ()> + MaybeSendFuture + '_`). The handler must
  mark ONLY that server's cached inventory stale (per-server scope, SC 10)
  and NOT refetch inside the notification handler if that risks blocking
  rmcp's notification path (lazy refetch on the next `inventory()` /
  `list_tools`, per state.json `decisions.cache-shape`). Wire a per-
  connection invalidation path (channel/watch/atomic flag owned by the
  `UpstreamEntry`, not a back-reference that creates an `Arc` cycle) from
  the handler to the registry entry for that server.
- **Mutable per-entry cache.** `UpstreamEntry.tools` is currently an
  immutable `Vec<Tool>` captured at connect time inside an
  `Arc<UpstreamEntry>`. Phase 4 must make it mutable/refreshable (dirty flag
  + lazy refetch, or a separate registry cache keyed by server) WITHOUT
  holding the registry map lock across `list_all_tools().await` (D-007 /
  GOTCHA #16, SC 15). On `list_changed`, mark only that server's cache
  dirty. On the next `inventory()` after invalidation, refetch with
  `list_all_tools()` and update the cache. If the refetch fails because
  the upstream died, return the structured `upstream_disconnected` error
  from Phase 1. The `list_changed` tests assert the cache updates on the
  next `list_tools` after the notification — a stub that ignores the
  notification returns the stale cached inventory and fails.
- **Phase 0/1/2/3 tests are read-only and unchanged in behavior.** The
  probe tool-count correction (10 → 14) is the only edit to existing tests
  — it is a factual update matching the extended probe fixture. Do not
  weaken the static-3-meta-tools, byte-faithful, reverse-traffic,
  lazy-startup, namespace-ACL, timeout, cancellation, process-lifetime, or
  credential assertions.
- **The rmcp pin stays at `=1.8.0`.** Phase 4 adds an
  `on_tool_list_changed` handler and a mutable cache but does NOT bump
  rmcp. Any rmcp signature used for `on_tool_list_changed`, notification
  contexts, or peer APIs must be verified against the pin; the
  `phase4_guard::rmcp_remains_pinned_exactly_at_1_8_0` test and the Phase 0
  `pinning.rs` test both enforce this.

## Review-fix coverage (F1–F5)

The THOROUGH review (`review.md`) surfaced five targeted findings against
the landed Phase 4 implementation. This section records the test-creator's
coverage for each: the new test(s) that lock the CORRECTED behavior, the
RED/ignored status against the current tree, and the honest-unknowns (F2
registerability, F4/F5 determinism). The new tests are the contract for
the debugger; they are RED now and turn GREEN when the debugger lands the
fixes. Existing green tests are unchanged in behavior (the only edit to
existing tests is the probe tool-count constant 14 → 16, a factual
correction matching the extended probe fixture).

### Probe-fixture additions (review-fix pass)

- **`poison_meta` description extended (F1).** The description now embeds
  `U+2028` (line sep), `U+2029` (paragraph sep), `U+0085` (NEL, C1),
  `U+0080` (pad, C1), `U+202E` (RLO bidi override), `U+200B` (ZWSP), and
  `U+FEFF` (BOM) — placed EARLY (within the first 100 chars) so the
  aggregator's ~100-char description cap does NOT truncate them. A C0-only
  strip leaves them in the LLM-visible row text; the F1 test catches that.
- **`toggle_long_tool` + dynamic `long_named_tool` (F2).** A new static
  tool `toggle_long_tool` toggles a dynamic `long_named_tool` whose REAL
  name is 120 chars (`long_named_tool_` + 104 `a`s) — longer than the
  aggregator's ~100-char description cap, under rmcp 1.8.0's 128-char
  registration ceiling, valid `[A-Za-z0-9_.-]`. The long-named tool is
  DYNAMIC (off by default) so the existing static-set discovery tests stay
  green against the current (pre-F2-fix) tree; the F2 test toggles it ON.
  `toggle_long_tool` emits `notifications/tools/list_changed` so the
  aggregator's next `list_tools` deterministically refetches (the F2
  contract is name dispatchability, but the advertisement depends on the
  inventory refreshing — emitting list_changed makes it deterministic).
  Dispatch routes the long-named tool to `echo_ok` so the round-trip is
  observable.
- **`poison_validation` (F3).** A new static tool whose `input_schema`
  carries BOTH annotation fields (`title`, `description`) with control
  chars AND validation fields (`enum: ["clean", "wei\u{0007}rd"]`,
  `default: "def\u{000A}ault"`, `const: "const\u{000B}val"`) carrying
  control-bearing string values. The F3 test asserts `get_tool_schema`
  returns the validation values VERBATIM while the annotations are
  sanitized.

The probe now exposes **16 static tools** (14 Phase 4 + `toggle_long_tool`
+ `poison_validation`). The `PROBE_TOOL_NAMES` constant is updated 14 → 16
in `probe.rs`, `discovery.rs`, `multi_upstream.rs`, `namespace_acl.rs` —
a factual correction, not a weakening. The runtime-added `added_tool`
(`mutate_tools`) and `long_named_tool` (`toggle_long_tool`) are NOT in
the static set; they appear only after their respective toggles.

### Coverage map — review findings

| Finding | Test | Status (current tree) | Locks |
|---|---|---|---|
| F1 — Unicode separators / C1 / bidi / zero-width stripped | `sanitization::f1_list_tools_strips_unicode_separators_c1_bidi_zero_width` | RED | The `list_tools` row description for `poison_meta` contains NONE of U+2028/U+2029/U+0085/U+0080/U+202E/U+200B/U+FEFF (and is still single-line + C0-free + capped). RED because the current `sanitize_upstream_text` strips only C0+DEL. |
| F2 — Long upstream tool name stays dispatchable | `sanitization::f2_long_named_tool_advertised_full_and_dispatchable` | RED | `list_tools` advertises the FULL 120-char real name (not truncated to 100) in the row `tool`/`name` field, AND `invoke_tool` using that advertised key SUCCEEDS (round-trip, not `unknown_tool`). RED because the current code caps the name field at 100 → truncated → dispatch fails. |
| F3 — Schema validation data preserved; only annotations sanitized | `sanitization::f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations` | RED | `get_tool_schema` returns `enum` members, `default`, `const` VERBATIM (control chars intact) WHILE `title`/`description` are sanitized (control-free). RED because the current `is_schema_metadata_key` includes `enum` → sanitizes its members → corrupts validation data. |
| F4 — Send-side death → `upstream_disconnected` | `error_hardening::f4_send_side_death_returns_upstream_disconnected_not_call_failed` | IGNORED | Stub. NOT deterministic wire-level (OS pipe-closure race; Windows/Unix differ). The existing `dead_upstream_returns_structured_error_and_sibling_stays_callable` covers the `TransportClosed` path; F4 covers the `TransportSend` path. |
| F5 — Failed refetch retries (no stale cache) | `list_changed::f5_failed_refetch_retries_does_not_serve_stale_inventory` | RED | After a failed post-`list_changed` refetch (probe killed), the SECOND `list_tools` returns `upstream_disconnected` (retried), NOT a successful stale inventory. RED because `ensure_fresh` does `dirty.swap(false)` before the await → on failure dirty stays false → the second call fast-paths and serves stale. |

### F2 registerability outcome

rmcp 1.8.0 registers tool names up to **128 chars** (`validate_tool_name`
rejects names > 128 or with invalid chars; the `Tool::new` constructor does
NOT enforce a length cap — it stores the name as `Cow<'static, str>` and
emits a tracing warning via `validate_and_warn_tool_name` for non-conforming
names, but registration proceeds). The F2 fixture's 120-char name
(`long_named_tool_` + 104 `a`s) is within the 128-char ceiling and a valid
`[A-Za-z0-9_.-]` identifier, so the probe registers it cleanly (verified:
the probe's `debug_assert!`s in `long_named_tool_tool` pass, and
`tools/list` from the probe returns the 120-char name). **The finding is
NOT moot** — the cap CAN truncate a registerable name, and the F2 test is
RED against the current tree (the aggregator caps the `tool`/`name` field
at 100 → the row advertises a 100-char truncation → `invoke_tool` with the
truncated key fails `unknown_tool`). The fix: the `tool`/`name` dispatch-key
field must carry the REAL upstream tool name (control-strip defensively,
but NO cap — names are identifiers, not prose); apply the ~100 cap ONLY to
`description`.

### F4 determinism decision

NOT deterministic wire-level. Whether a killed upstream's next call surfaces
as `ServiceError::TransportClosed` (the transport worker already detected
the EOF/closure) vs `ServiceError::TransportSend(...)` (the send fails
before the worker notices) is a race between the OS pipe-closure propagation
and the aggregator's next `call_tool` send. Windows pipe behavior differs
from Unix. Forcing the send-side observation deterministically requires
injecting into the transport (a hook to make the send fail before the
worker detects closure), which is below the wire-level surface. Per the
doctrine (no flaky test), F4 is an `#[ignore = "..."]` stub with a concrete
reason and unblock trigger (a wire-level transport wrapper that forces
send-side failure, OR a unit test against `map_service_error` once that
function is extracted/testable). The F4 code fix (also map `TransportSend`
from an established upstream operation to `UpstreamDisconnected`) is
verified by the existing `TransportClosed` path test plus the code review;
the send-side path is the documented gap.

### F5 determinism decision

DETERMINISTIC wire-level. The F5 sequence forces the post-`list_changed`
refetch to fail by killing the probe upstream AFTER it emits `list_changed`
(so the aggregator marks dirty) but BEFORE the next `list_tools` (so the
refetch fails). The first `list_tools` after the kill returns
`upstream_disconnected` (the refetch fails). The SECOND `list_tools` is the
load-bearing assertion: under the F5 bug it fast-paths (dirty=false) and
serves the stale pre-mutate inventory as a SUCCESS; under the fix it
retries (dirty restored) and returns `upstream_disconnected` again. The
assertion (second call returns an error, not a stale success) is
deterministic — no race, no timing window. The kill is owned by the test
(simulating an external crash between the notification and the refetch);
the aggregator's containment layer is NOT involved.

### Side-effect assertions (review-fix tests)

- **F1 is a side-effect assertion on the row text.** The test asserts the
  `list_tools` row description for `poison_meta` contains NONE of the F1
  code points — observable in the LLM-visible row text, not just a return
  value. A sanitizer that left U+2029/U+0085/U+202E/U+200B in the description
  fails the per-codepoint assertion. The code points are placed within the
  first 100 chars so the description cap does NOT hide them.
- **F2 is a side-effect assertion on the advertised name + the dispatch.**
  The test asserts `list_tools` advertises the FULL 120-char name in the
  row `tool`/`name` field (observable row text), AND that `invoke_tool`
  using that advertised key SUCCEEDS (the round-trip is the observable
  effect). A proxy that truncated the name to 100 fails the advertised-name
  assertion; a proxy that used the truncated name as the call key fails
  the round-trip assertion (`unknown_tool`).
- **F3 is a side-effect assertion on the schema JSON.** The test parses
  `get_tool_schema`'s text content as JSON and asserts the `enum`/`default`/
  `const` values are VERBATIM (control chars intact) while `title`/
  `description` are control-free. A sanitizer that touched `enum` (the
  current `is_schema_metadata_key` includes it) fails the verbatim
  assertion; a sanitizer that touched `default`/`const` would fail too
  (pinned for the future).
- **F5 is a side-effect assertion on the stale-cache behavior.** The test
  kills the probe (the dead PROCESS is the oracle), triggers `list_changed`,
  kills the probe, and asserts the SECOND `list_tools` after the failed
  refetch returns `upstream_disconnected` (retried) — NOT a successful
  stale inventory. A registry that cleared dirty before the await and did
  not restore it on failure serves the stale inventory as a success and
  fails the `isError` assertion.

### Deferred tests (review-fix pass)

- **F4** — `error_hardening::f4_send_side_death_returns_upstream_disconnected_not_call_failed`
  is `#[ignore]`d. Reason: send-side broken pipe surfaces as `TransportSend`
  vs `TransportClosed` non-deterministically (OS pipe-closure race;
  Windows/Unix differ). The existing
  `dead_upstream_returns_structured_error_and_sibling_stays_callable`
  covers the `TransportClosed` path. Unblock trigger: a wire-level
  transport wrapper that forces send-side failure, OR a unit test against
  `map_service_error` once that function is extracted/testable.

No other review-fix test is `#[ignore]`d. F1, F2, F3, F5 are RED wire-level
tests against the current tree (the contract for the debugger).

### Run-and-fail confirmation (review-fix pass)

The suite compiles clean (`cargo build --tests` — zero warnings),
`cargo fmt --all -- --check` is CLEAN, `cargo clippy --all-targets` is
CLEAN (zero warnings).

Against the current (post-Phase-4, pre-review-fix) tree, the run-and-fail
state is:

- **4 new review-fix tests total** (1 in `error_hardening.rs` — F4 ignored;
  3 in `sanitization.rs` — F1, F2, F3 RED; 1 in `list_changed.rs` — F5 RED).
  F4 is the only `#[ignore]`.
- **`cargo test --test integration` (current tree): 99 passed, 4 failed
  RED, 4 ignored.** The 4 RED failures are exactly the review-fix behaviors:
  - `sanitization::f1_list_tools_strips_unicode_separators_c1_bidi_zero_width`
    — the current `sanitize_upstream_text` strips only C0+DEL, leaving
    U+2029/U+0085/U+202E/U+200B in the row description.
  - `sanitization::f2_long_named_tool_advertised_full_and_dispatchable`
    — the current code caps the `tool`/`name` field at 100, so the row
    advertises a 100-char truncation of the 120-char real name, and
    `invoke_tool` with the truncated key fails `unknown_tool`.
  - `sanitization::f3_get_tool_schema_preserves_validation_data_sanitizes_only_annotations`
    — the current `is_schema_metadata_key` includes `enum`, so the
    `enum` member `"wei\u{0007}rd"` is sanitized to `"wei rd"` (control
    →space), corrupting the validation data.
  - `list_changed::f5_failed_refetch_retries_does_not_serve_stale_inventory`
    — the current `ensure_fresh` does `dirty.swap(false)` before the await,
    so on refetch failure dirty stays false; the second `list_tools`
    fast-paths and serves the stale pre-mutate inventory as a success.
- The 4 ignored are the 3 Phase 0/3 carried ignores (manual E2E + keyring
  round-trip on headless hosts) + F4 (send-side non-determinism).
- All 99 previously-green tests STILL PASS (the only edit to existing tests
  is the probe tool-count constant 14 → 16, a factual correction; the
  `list_tools_returns_probe_tool_rows` count assertion 14 → 16; the
  `multi_upstream` 3-server count `PROBE_TOOL_NAMES.len() * 3` with the
  updated comment "48 rows (3 servers x 16 tools)"). No behavioral
  assertion is weakened.
- **The red is meaningful:** every RED test fails on a review-fix behavior
  assertion (Unicode separator survival, name truncation, enum
  corruption, stale-cache serve), not on a compile error, a missing
  symbol, or a malformed harness. The debugger turns each RED green by
  landing the F1–F5 fixes in `src/server.rs` (extend the strip set; uncap
  the name field; narrow `is_schema_metadata_key` to annotations only)
  and `src/registry.rs` (restore dirty on refetch failure; also map
  `TransportSend` to `UpstreamDisconnected` for F4).

### Per-module inventory (review-fix pass)

| Module | Tests (review-fix additions) |
|---|---|
| `sanitization` (Phase 4) | 5 (existing) + 3 (F1, F2, F3) = 8 |
| `error_hardening` (Phase 4) | 3 (existing) + 1 (F4 ignored) = 4 (1 ignored) |
| `list_changed` (Phase 4) | 2 (existing) + 1 (F5) = 3 |
| `phase4_guard` (Phase 4) | 5 (unchanged) |
| **Phase 4 total** | **15 (existing) + 5 (review-fix) = 20** (1 ignored) |
| Phase 0/1/2/3 modules (behavior unchanged; probe count corrected 14 → 16) | 99 (3 ignored) |
| **Grand total** | **119** (4 ignored) |