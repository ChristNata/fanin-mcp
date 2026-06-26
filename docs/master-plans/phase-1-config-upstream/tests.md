# test-creator: phase-1-config-upstream

Phase 1 test contract. The implementer codes against this suite; the
objective gate runs it. Test files are read-only to every later stage.

## Stack & runner

- **Runner:** `cargo test --test integration` (main suite). `cargo nextest
  run --workspace` works equivalently. Doc-tests are N/A: this is a
  binary-only crate with no library target (Phase 0 `tests.md` rationale
  holds).
- **Async:** `#[tokio::test]` single-threaded default; concurrency tests
  (`registry::concurrent_first_calls_*`, `registry::slow_tool_call_*`) use
  `flavor = "multi_thread", worker_threads = 2`.
- **Wire-level by default (D-015).** Tests spawn the built `fanin-mcp`
  binary and speak raw JSON-RPC over stdio, asserting on the wire JSON —
  decoupling the contract from rmcp's fast-moving Rust API. No `src/`
  stubs are required for the suite to compile; the harness depends only on
  `tokio` + `serde_json` + `tempfile` (dev-deps). The implementer fills in
  `src/` to turn the suite green.
- **Build order.** `cargo test` builds the `fanin-mcp` and `probe-server`
  `[[bin]]` targets before the test binary, so `CARGO_BIN_EXE_fanin-mcp`
  and `CARGO_BIN_EXE_probe-server` resolve. The env var names use the bin
  names EXACTLY as-declared (dashes/case preserved) — uppercasing breaks
  resolution on every platform (Phase 0 harness comment).

## Files created / extended

| Path | Criteria covered |
|---|---|
| `tests/common/fixtures.rs` (new) | Phase 1 TOML config builder + temp config/log-file helpers. Encodes the binding config schema (§Config schema). |
| `tests/common/mod.rs` (extended) | `spawn_fanin_with_config`, `spawn_fanin_with_args`, `drain_stdout_raw` helpers layered on the Phase 0 harness. |
| `tests/integration/config.rs` (new) | Master SC 1, 2, 3, 19; P1.SC1–5 |
| `tests/integration/reverse_traffic.rs` (new) | Master SC 14, 15, 16, 17, 18; P2.SC1–6 |
| `tests/integration/registry.rs` (new) | Master SC 5, 11, 12, 13; P3.SC1–5 |
| `tests/integration/discovery.rs` (new) | Master SC 4, 6, 7; P4.SC1–5 |
| `tests/integration/invoke.rs` (new) | Master SC 8, 9, 10; P5.SC1–6 |
| `tests/integration/gate.rs` (new) | Master SC 4, 5, 8, 16, 17, 19, 20; P6.SC1–5 |
| `tests/integration/main.rs` (extended) | `mod` declarations for the six new modules. |

Phase 0 files (`aggregator.rs`, `probe.rs`, `pinning.rs`, `manual_e2e.rs`,
`common/expectations.rs`) are unchanged. The Phase 0 tests still run and
still pass — the Phase 1 contract is additive.

## Config schema (binding — the implementer must parse this exact shape)

ARCHITECTURE.md shows the full config (transports, env, headers, timeouts,
descriptions, cwd, tool filters). Phase 1 scope is single stdio upstream
with no credentials/HTTP/timeouts. The schema below is the minimal Phase 1
subset the tests encode; fields ARCHITECTURE shows but Phase 1 does not
exercise are omitted from the fixtures and are the implementer's to add
later (Phase 2+) without breaking these tests.

```toml
# One stdio upstream. The server name is the [servers.<name>] key.
[servers.probe]
transport = "stdio"          # optional in Phase 1; defaults to "stdio"
command = '/path/to/probe-server'   # required for stdio; literal string (backslashes safe on Windows)
args = []                    # optional; default empty
log_file = '/path/to/log'    # optional; where child stderr + log notifications land

[servers.probe.env]          # optional; literal key=value pairs (Phase 1 does NOT resolve ${VAR})
SOME_KEY = 'literal-value'

# One namespace. The namespace name is the [namespaces.<name>] key.
[namespaces.default]
servers = ["probe"]          # the servers visible in this namespace
```

### Choices recorded (ARCHITECTURE left these open)

- **`transport`** is optional in Phase 1, defaulting to `"stdio"`. The
  fixtures always write it explicitly for clarity, but the parser must
  accept its absence and treat the server as stdio. HTTP transport is out
  of scope (Phase 1 plan §Out).
- **`command`** is required for stdio servers. A stdio server table
  missing `command` fails startup (`config::stdio_server_without_command_fails_startup`).
- **`args`** is optional, defaulting to an empty array.
- **`env`** is an optional sub-table of literal `key = 'value'` pairs.
  Phase 1 does NOT resolve `${VAR}` placeholders (credentials are Phase 3,
  out of scope). Literal values are passed to the spawned child as-is. The
  fixtures provide a `.env(key, value)` builder method for future use; no
  Phase 1 test exercises it (kept for the implementer's forward path).
- **`log_file`** is an optional per-server field naming the log sink for
  that server's child stderr + upstream log notifications. The
  `reverse_traffic::child_stderr_lands_in_log_sink_with_server_prefix` and
  `gate::full_phase1_path_*` tests assert against it. ARCHITECTURE shows a
  global `--log-file` flag (Phase 5); Phase 1 uses the per-server field as
  the simplest shape that the wire tests can drive. The implementer may
  also support `--log-file`; the tests do not require it.
- **`namespaces.<name>.servers`** is the only namespace field Phase 1
  reads. The `tools.<server>` filter (ARCHITECTURE) is Phase 2; the
  fixtures do not write it and the parser may ignore it.
- **Server-name validation:** `[a-z0-9-]+`, rejecting `__` (GOTCHA #15) and
  any other character (uppercase, underscore, space, etc.). Validated at
  config load, before serving.
- **Unknown `--namespace`** fails startup before serving.
- **Default namespace:** when `--namespace` is omitted, the `default`
  namespace is selected. A config that declares `[namespaces.default]`
  works with the flag omitted.

## Coverage map — master Success Criteria

| # | Master Success Criterion | Test(s) |
|---|---|---|
| 1 | Valid Phase 1 TOML config starts the aggregator | `config::valid_phase1_config_starts_aggregator`; `config::default_namespace_preserved_when_flag_omitted`; `config::config_with_nonexistent_command_still_starts_due_to_lazy_spawn` (lazy-spawn edge) |
| 2 | Invalid server names fail startup (uppercase/underscore/`__`/other) | `config::invalid_server_name_uppercase_fails_startup_before_serving`; `config::server_name_with_double_underscore_fails_startup`; `config::server_name_with_single_underscore_fails_startup`; `config::stdio_server_without_command_fails_startup`; `config::no_config_failure_writes_to_stdout` |
| 3 | Unknown `--namespace` fails startup before serving | `config::unknown_namespace_fails_startup_before_serving` |
| 4 | Downstream rmcp `tools/list` static: exactly 3 meta-tools, Phase 0 descriptions/annotations unchanged | `discovery::downstream_tools_list_still_returns_three_static_meta_tools`; `gate::full_phase1_path_config_to_reverse_traffic_passes` (re-asserts after full exercise); Phase 0 `aggregator::static_discovery_*` + `aggregator::invoke_tool_carries_conservative_annotations` still run |
| 5 | Startup/initialize opens zero upstreams, < 500ms | `registry::initialize_with_config_opens_zero_upstreams_under_500ms`; `registry::downstream_tools_list_does_not_spawn_upstream` (zero-upstream-at-init via log-sink observation); `gate::full_phase1_path_*` (init < 500ms) |
| 6 | `list_tools` meta-tool returns probe tool rows for active namespace | `discovery::list_tools_returns_probe_tool_rows`; `discovery::list_tools_filtered_by_server_returns_only_that_server_rows`; `discovery::list_tools_filtered_by_unknown_server_returns_structured_error`; `gate::full_phase1_path_*` |
| 7 | `get_tool_schema` for `probe__echo_ok` returns the probe's input schema | `discovery::get_tool_schema_returns_probe_echo_ok_input_schema`; `discovery::get_tool_schema_unknown_server_returns_structured_error_not_rpc_error`; `discovery::get_tool_schema_known_server_unknown_tool_returns_structured_error`; `gate::full_phase1_path_*` |
| 8 | `invoke_tool` parses on first `__`, forwards raw args, returns `probe__echo_ok` output | `invoke::invoke_tool_probe_echo_ok_returns_probe_success`; `invoke::invoke_tool_forwards_raw_arguments_unchanged`; `invoke::invoke_tool_splits_on_first_double_underscore_only`; `invoke::invoke_tool_name_without_delimiter_returns_structured_error`; `invoke::invoke_tool_empty_name_returns_structured_error`; `invoke::invoke_tool_probe_slow_tool_honors_delay`; `gate::full_phase1_path_*` |
| 9 | Upstream success/error pass through as `CallToolResult`; tool failures `isError:true`, not JSON-RPC errors | `invoke::invoke_tool_probe_always_error_returns_upstream_is_error_content`; `invoke::invoke_tool_unknown_server_returns_structured_error`; `invoke::invoke_tool_known_server_unknown_tool_returns_structured_error`; `invoke::invoke_tool_probe_dangerous_noop_returns_success` (success pass-through); all invoke tests assert `assert_no_rpc_error` |
| 10 | Content arrays never stringified; non-text blocks stay structured | `invoke::invoke_tool_preserves_content_array_structure` (single-block structure); **non-text block test DEFERRED — probe fixture has no non-text-returning tool (§Gaps)** |
| 11 | Lazy connection: probe not spawned until first targeting meta-tool call | `registry::downstream_tools_list_does_not_spawn_upstream` (log-sink observation: no probe line after `tools/list`, probe line after `list_tools`); `config::config_with_nonexistent_command_still_starts_due_to_lazy_spawn` |
| 12 | Concurrent first calls spawn exactly one upstream | `registry::concurrent_first_calls_spawn_exactly_one_upstream` (consistent-success proxy; strict process-count is §Boundaries) |
| 13 | No registry map lock held across an upstream `call_tool` await | `registry::slow_tool_call_does_not_block_independent_call_issue` (echo issued during slow_tool completes without hanging; true slow-A-does-not-block-B is Phase 2 — §Boundaries) |
| 14 | Upstream client declares no sampling/elicitation capabilities | `reverse_traffic::upstream_client_rejects_sampling_within_deadline_proxy_for_no_capability` (wire-level proxy: rejection completes; direct capability assertion is §Deferred unit test) |
| 15 | Upstream `roots/list` receives empty list | **DEFERRED — probe has no tool that sends `roots/list` (§Deferred)** |
| 16 | Upstream sampling/elicitation requests receive immediate structured rejection, not a hang | `reverse_traffic::needs_sampling_call_completes_within_deadline_not_hung`; `reverse_traffic::reverse_traffic_does_not_destabilize_downstream_server`; `gate::full_phase1_path_*`; **elicitation DEFERRED — probe has no elicitation tool (§Deferred)** |
| 17 | Upstream log notifications + child stderr to log sink with `[server]` prefix | `reverse_traffic::child_stderr_lands_in_log_sink_with_server_prefix` (stderr half); `gate::full_phase1_path_*`; **log-notification half is §Gaps (probe emits no `notifications/message`)** |
| 18 | Child stderr does not corrupt stdout / is not inherited | `reverse_traffic::child_stderr_does_not_reach_aggregator_stdout` (every stdout line parses as JSON); implicit in every wire test (harness panics on non-JSON stdout) |
| 19 | All diagnostics after `serve(stdio())` avoid stdout | `config::no_config_failure_writes_to_stdout`; `config::*_fails_startup_*` (all assert empty stdout via `assert_does_not_serve`); implicit in every wire test |
| 20 | All required tests pass at 100% | `gate::full_phase1_path_config_to_reverse_traffic_passes` (composition gate); `gate::phase1_does_not_accept_phase3_credential_store_flag` (no scope creep) |

## Coverage map — Phase sub-criteria

| Phase | Criterion | Test |
|---|---|---|
| P1.1 | Valid config starts | `config::valid_phase1_config_starts_aggregator` |
| P1.2 | Server name outside `[a-z0-9-]+` fails before serving | `config::invalid_server_name_uppercase_fails_startup_before_serving`; `config::server_name_with_single_underscore_fails_startup` |
| P1.3 | Server name with `__` fails before serving | `config::server_name_with_double_underscore_fails_startup` |
| P1.4 | Unknown `--namespace` fails before serving | `config::unknown_namespace_fails_startup_before_serving` |
| P1.5 | Default namespace preserved when flag omitted | `config::default_namespace_preserved_when_flag_omitted` |
| P1.6 | No config failure emits to stdout | `config::no_config_failure_writes_to_stdout` + all `config::*_fails_*` via `assert_does_not_serve` |
| P2.1 | Upstream client advertises no sampling/elicitation capabilities | `reverse_traffic::upstream_client_rejects_sampling_within_deadline_proxy_for_no_capability` (proxy) |
| P2.2 | `roots/list` receives empty list | **DEFERRED (§Deferred)** |
| P2.3 | Sampling request receives bounded rejection, not a hang | `reverse_traffic::needs_sampling_call_completes_within_deadline_not_hung` |
| P2.4 | Elicitation request receives bounded rejection | **DEFERRED (§Deferred)** |
| P2.5 | Upstream log notifications + child stderr in log sink with server name | `reverse_traffic::child_stderr_lands_in_log_sink_with_server_prefix` (stderr half) |
| P2.6 | No child stderr inherited into fanin-mcp stdout | `reverse_traffic::child_stderr_does_not_reach_aggregator_stdout` |
| P3.1 | `tools/list` opens zero upstream processes | `registry::downstream_tools_list_does_not_spawn_upstream`; `registry::initialize_with_config_opens_zero_upstreams_under_500ms` |
| P3.2 | First targeting meta-tool call spawns exactly one upstream | `registry::downstream_tools_list_does_not_spawn_upstream` (probe line appears after `list_tools`) |
| P3.3 | Concurrent first calls spawn exactly one upstream | `registry::concurrent_first_calls_spawn_exactly_one_upstream` |
| P3.4 | Registry map lock not held across an upstream await | `registry::slow_tool_call_does_not_block_independent_call_issue` (boundary — §Boundaries) |
| P3.5 | Inventory cached and reused for the session | `registry::inventory_cached_and_reused_for_session` |
| P4.1 | `list_tools` returns probe tool rows for active namespace | `discovery::list_tools_returns_probe_tool_rows` |
| P4.2 | `list_tools` filtered by server returns only that server's rows | `discovery::list_tools_filtered_by_server_returns_only_that_server_rows` |
| P4.3 | `get_tool_schema probe__echo_ok` returns the probe's schema | `discovery::get_tool_schema_returns_probe_echo_ok_input_schema` |
| P4.4 | `get_tool_schema` unknown server/tool returns `isError:true`, not JSON-RPC error | `discovery::get_tool_schema_unknown_server_returns_structured_error_not_rpc_error`; `discovery::get_tool_schema_known_server_unknown_tool_returns_structured_error` |
| P4.5 | Downstream rmcp `tools/list` still exactly 3 static meta-tools | `discovery::downstream_tools_list_still_returns_three_static_meta_tools` |
| P5.1 | `invoke_tool probe__echo_ok` returns probe success | `invoke::invoke_tool_probe_echo_ok_returns_probe_success` |
| P5.2 | Raw arguments forwarded unchanged | `invoke::invoke_tool_forwards_raw_arguments_unchanged` |
| P5.3 | Upstream error tool returns `isError:true` content, not JSON-RPC error | `invoke::invoke_tool_probe_always_error_returns_upstream_is_error_content` |
| P5.4 | Tool names with extra `__` after delimiter stay part of upstream tool name | `invoke::invoke_tool_splits_on_first_double_underscore_only` |
| P5.5 | Denied/unknown namespace/server/tool returns structured `isError:true` | `invoke::invoke_tool_unknown_server_returns_structured_error`; `invoke::invoke_tool_known_server_unknown_tool_returns_structured_error`; `invoke::invoke_tool_name_without_delimiter_returns_structured_error`; `invoke::invoke_tool_empty_name_returns_structured_error`; `discovery::list_tools_filtered_by_unknown_server_returns_structured_error` |
| P5.6 | Non-text content blocks preserved, not stringified | **DEFERRED (§Deferred)** |
| P6.1 | Full Phase 1 integration suite passes at 100% | `gate::full_phase1_path_config_to_reverse_traffic_passes` |
| P6.2 | Phase 0 tests still pass without weakening | Phase 0 `aggregator::*`, `probe::*`, `pinning::*` unchanged + still run |
| P6.3 | `needs_sampling` receives clean rejection, completes within deadline | `reverse_traffic::needs_sampling_call_completes_within_deadline_not_hung`; `gate::full_phase1_path_*` |
| P6.4 | Probe stderr captured in log sink with `[probe]`/configured-name prefix | `reverse_traffic::child_stderr_lands_in_log_sink_with_server_prefix`; `gate::full_phase1_path_*` |
| P6.5 | No Phase 2/3/4/5 functionality accidentally introduced | `gate::phase1_does_not_accept_phase3_credential_store_flag` (light gate; heavy enforcement is review's job) |

## Side-effect assertions

Every Phase 1 test asserts the observable effect, not just a return value,
so a stub that returns the right shape without doing the work fails.

- **Startup-failure exits before serving.** `assert_does_not_serve` sends
  `initialize` and asserts NO stdout bytes arrive within 2s. The Phase 0
  stub ignores `--config` and serves `initialize` (147 bytes) — so the
  negative config tests fail RED against the stub and pass GREEN once the
  implementer rejects the config before `serve(stdio())`. This is the
  load-bearing distinction: "non-zero exit + empty stdout" alone was too
  weak (the stub exits non-zero on EOF too); requiring "no initialize
  response" makes the test meaningful.
- **No stdout diagnostics (GOTCHA #1).** Every wire test implicitly asserts
  clean JSON on stdout — the harness panics on a non-JSON line. The
  startup-failure tests assert empty stdout explicitly via
  `drain_stdout_raw`. `reverse_traffic::child_stderr_does_not_reach_aggregator_stdout`
  reads all stdout after a child spawn and asserts every line parses as
  JSON — a leaked child stderr line fails.
- **Lazy spawn is observable via the log sink.**
  `registry::downstream_tools_list_does_not_spawn_upstream` reads the log
  file after downstream `tools/list` (asserts NO `probe` line) and after
  `list_tools` meta-tool (asserts a `probe` line appeared). A
  non-lazy/eager impl that spawns on `tools/list` fails the first
  assertion; a stub that never spawns fails the second.
- **Child stderr lands in the log file.**
  `reverse_traffic::child_stderr_lands_in_log_sink_with_server_prefix`
  spawns the aggregator with a `log_file`, exercises `echo_ok` (which
  spawns the probe), then reads the log file and asserts it contains
  `probe`. A stub that inherits child stderr onto the aggregator's own
  stderr writes nothing to the log file and fails.
- **Reverse traffic does not hang (GOTCHA #2).**
  `needs_sampling_call_completes_within_deadline_not_hung` wraps the call
  in `timeout(10s)`. A stub with no `ClientHandler` would let the probe
  wait forever on the unanswered sampling request; the deadline catches
  the hang. The test also asserts the forward SUCCEEDS (the probe's
  "sent sampling/createMessage" text), so a not-implemented stub fails
  RED rather than passing trivially.
- **Raw arguments round-trip byte-faithfully (D-004).**
  `invoke_tool_forwards_raw_arguments_unchanged` sends a payload with
  nested quotes, braces, unicode, and escape sequences; asserts the
  echoed text contains the payload verbatim. A proxy that
  re-serialized/stringified the args would mangle it.
- **Slow_tool delay is a wall-clock effect.**
  `invoke_tool_probe_slow_tool_honors_delay` asserts `elapsed >= 150ms` —
  a stub that returns instantly fails.
- **`always_error` content passes through byte-faithfully (D-005).**
  Asserts the probe's `code: "always_error"` JSON body appears in the
  forwarded text — a proxy that swallowed the upstream error and
  substituted its own message fails.
- **Lock discipline observable.**
  `slow_tool_call_does_not_block_independent_call_issue` issues a
  `slow_tool` (300ms) without awaiting, immediately issues an `echo_ok`,
  and asserts the echo completes within the deadline. A lock held across
  the slow await serializes the session; the echo would block. Requires
  real forwarding (echo success) so a not-implemented stub fails RED.

## Deferred tests

| Test | File | `#[ignore]` reason | Unblock trigger |
|---|---|---|---|
| `upstream_roots_list_receives_empty_list` | `reverse_traffic.rs` | probe fixture has no tool that sends `roots/list` | Re-enable when a probe-fixture tool that triggers an upstream `roots/list` request lands (probe-fixture update routed by orchestrator). |
| `upstream_elicitation_request_receives_bounded_rejection` | `reverse_traffic.rs` | probe fixture has no tool that sends `elicitation/create` | Re-enable when a probe `needs_elicitation` tool lands (probe-fixture update routed by orchestrator). |
| `invoke_tool_preserves_non_text_content_block_not_stringified` | `invoke.rs` | probe fixture has no tool that returns a non-text content block (every probe tool returns `Content::text`) | Re-enable when a probe `echo_image`/`echo_resource` tool that returns `Content::image`/`Content::embedded_resource` lands (probe-fixture update routed by orchestrator). The byte-faithful non-text assertion is otherwise unprovable. |
| `live_cc_discovers_exactly_three_meta_tools` | `manual_e2e.rs` (Phase 0) | live CC E2E — no MCP client in CI | Re-enable when a CI job with a real CC MCP client is wired. |
| `live_oc_discovers_exactly_three_meta_tools` | `manual_e2e.rs` (Phase 0) | live OC E2E — no MCP client in CI | Re-enable when a CI job with a real OC MCP client is wired. |

The three Phase 1 deferred tests all share one root cause: the probe
fixture does not exercise the reverse-traffic/non-text paths the plan
calls out. They are `#[ignore]`'d with a concrete unblock trigger (a
probe-fixture update), not deleted. The orchestrator should route a
single probe-fixture update that adds: (a) a `needs_roots` tool that
sends `roots/list`, (b) a `needs_elicitation` tool that sends
`elicitation/create`, (c) an `echo_image` tool that returns a non-text
content block. Once those land, the three tests are re-enabled and the
corresponding criteria (15, 16-elicitation, 10) gain direct wire-level
coverage.

## Coverage gaps

These are criteria the suite does NOT fully prove at the wire level, with
the reason and the proxy/boundary that does cover them:

- **Master SC 10 / P5.SC6 (non-text content block preservation).** The
  probe returns only `Content::text`. The deferred
  `invoke_tool_preserves_non_text_content_block_not_stringified` test is
  the direct assertion; until the probe gains a non-text-returning tool,
  SC 10 is covered only by `invoke_tool_preserves_content_array_structure`
  (single text-block structure preservation), which is a partial proxy.
  **Flagged for the orchestrator to route a probe-fixture update.**
- **Master SC 14 (no sampling/elicitation capabilities declared).** The
  direct assertion is a unit test against the `ClientInfo` the aggregator
  passes to `serve_client` — not observable over the downstream stdio
  without instrumenting the probe's initialize response. The wire-level
  proxy is `upstream_client_rejects_sampling_within_deadline_proxy_for_no_capability`
  (the rejection completes, which is only possible if the `ClientHandler`
  is wired). A true capability-declaration unit test is deferred until
  `forward.rs` exposes a constructible handler the unit test can
  instantiate.
- **Master SC 15 (empty roots/list).** Deferred — probe has no
  `roots/list`-sending tool (§Deferred).
- **Master SC 16 elicitation half.** Deferred — probe has no
  elicitation-sending tool (§Deferred).
- **Master SC 17 logging-notification half.** The probe initializes
  `tracing` to stderr (child stderr) but does NOT emit an MCP
  `notifications/message` (logging) request on its own, and no probe tool
  triggers one. The stderr half is covered by
  `child_stderr_lands_in_log_sink_with_server_prefix`; the
  logging-notification half needs a probe tool that sends
  `notifications/message`. **Flagged for the orchestrator to route a
  probe-fixture update** (same update as the §Deferred batch).
- **Master SC 12 (exactly one spawn).** The wire suite asserts the
  observable consequence (both concurrent first-calls SUCCEED with
  consistent inventory) rather than a process count. A strict
  process-count assertion is platform-specific (counting probe children)
  and brittle in CI; the consistent-success proxy is what the plan
  sanctions for the wire suite.
- **Master SC 13 (no lock across upstream await).** True
  slow-A-does-not-block-sibling proof needs two upstreams (Phase 2).
  Phase 1 asserts the guard/issue behavior on a single upstream
  (`slow_tool_call_does_not_block_independent_call_issue`): an echo
  issued during a slow_tool completes without hanging. The Phase 2
  multi-upstream concurrency test is the full proof.
- **P3.SC5 (cache reuse).** `inventory_cached_and_reused_for_session`
  asserts two `list_tools` calls return consistent content (a re-fetch
  would also return the same rows). A strict cache-hit proof (asserting
  no second upstream `tools/list` round-trip) needs instrumentation the
  wire suite does not have.
- **P6.SC5 (no scope creep).** `phase1_does_not_accept_phase3_credential_store_flag`
  is a light gate (clap rejects unknown flags). Full "no Phase 2/3/4/5
  functionality" enforcement is review's job, not a unit test's.

## `list_tools` row shape (schema choice)

The `list_tools` meta-tool returns its rows as a JSON array serialized
inside a text content block. The tests accept either of two row shapes —
each row may carry the tool name under `tool` OR `name`, and may carry a
`server` field. The load-bearing assertions are:

- The text content parses as a JSON array.
- Each row carries a tool name (under `tool` or `name`).
- The set of tool names matches the probe's five.
- When filtered by server, every row carrying a `server` field belongs to
  the requested server.

The implementer chooses the exact row fields (`server`, `tool`, `name`,
`description`); the tests do not pin the full row shape, only the
tool-name presence and the server-filter behavior. This keeps the row
schema open for Phase 2 (per-server description enrichment) without
breaking Phase 1 tests.

## `get_tool_schema` return shape (schema choice)

The `get_tool_schema` meta-tool returns the upstream tool's input schema
as JSON inside a text content block. The test asserts the schema is a
JSON object with `type: "object"` and a `properties.message` of type
`string` (the probe's `echo_ok` schema). The implementer may wrap the
schema in an envelope or return it bare; the test parses the text content
as JSON and asserts the `type` and `properties.message` fields, so a
bare schema or an `{"schema": {...}}` envelope both satisfy it as long
as the schema fields are reachable. (If the implementer chooses a
deeply-nested envelope, the test would need adjustment — flagged as a
schema choice the implementer should keep simple.)

## dev-dependencies added

The implementer owns `Cargo.toml`; this is the Phase 1 addition to the
Phase 0 dev-deps.

```toml
[dev-dependencies]
# ... Phase 0 dev-deps unchanged ...
tempfile = "3"   # config-fixture + log-file path helpers (tests/common/fixtures.rs)
```

Rationale: `tempfile` provides `NamedTempFile` for the config files
(held alive for the spawned child's lifetime so the path stays valid on
Windows) and a unique log-file path. No `mockall`/`proptest`/`insta`/
`rstest` — Phase 1 has no trait mocks, properties, snapshots, or
parameterized matrices. Adding them is scope creep.

## Run-and-fail confirmation

The suite compiles clean (`cargo build --tests` — zero warnings after the
`#[allow(dead_code)]` on the fixture builder) and runs RED against the
Phase 0 stubs:

- **58 tests total** (5 `#[ignore]`'d: 3 Phase 0 manual E2E + 2 Phase 1
  deferred — wait, 5 ignored = 2 manual_e2e + 2 reverse_traffic + 1
  invoke).
- **Against the Phase 0 stubs:** 28 pass, 25 fail RED, 5 ignored.
  - The 25 RED failures are every test that requires real Phase 1
    behavior (config validation, lazy spawn, forwarding, reverse-traffic
    handling, stderr capture, discovery, the composition gate).
  - The 28 greens are: Phase 0 tests (unchanged, still pass), the
    legitimate-guarantee tests the stub trivially satisfies (valid config
    starts, init < 500ms with no upstream, static `tools/list`, structured
    error paths for unknown tools, the no-scope-creep flag gate), and the
    deferred tests' placeholder bodies (ignored, not run).
- **The red is meaningful:** every RED test fails on the assertion that
  requires Phase 1 behavior, not on a compile error, a missing symbol, or
  a malformed harness. The implementer turns each RED green by filling in
  `src/` (config model, registry, forward, process, server dispatch).

## Per-module inventory

| Module | Tests |
|---|---|
| `aggregator` (Phase 0) | 5 |
| `config` (Phase 1) | 9 |
| `discovery` (Phase 1) | 7 |
| `gate` (Phase 1) | 2 |
| `invoke` (Phase 1) | 12 |
| `manual_e2e` (Phase 0) | 2 (ignored) |
| `pinning` (Phase 0) | 1 |
| `probe` (Phase 0) | 8 |
| `registry` (Phase 1) | 5 |
| `reverse_traffic` (Phase 1) | 7 (2 ignored) |
| **Total** | **58** (5 ignored) |

## Notes for the implementer

- The config schema in `tests/common/fixtures.rs` is the binding contract.
  Parse exactly that shape; the tests write those TOML files and spawn
  `fanin-mcp --config <path>`.
- `CARGO_BIN_EXE_probe-server` and `CARGO_BIN_EXE_fanin-mcp` use the bin
  names EXACTLY as-declared (dashes/case preserved). Do not uppercase.
- The negative config tests use `assert_does_not_serve`: they send
  `initialize` and assert NO stdout response within 2s. A correct impl
  exits before serving; the stub serves and fails the test. Do not make
  the negative tests pass by writing to stdout — they must fail by the
  config validation path exiting before `serve(stdio())`.
- The reverse-traffic tests require the forward to SUCCEED (the probe's
  text), not a not-implemented error. Wire the full path: lazy spawn ->
  forward -> `ClientHandler` rejection -> byte-faithful result.
- The log-sink tests read the `log_file` after a short flush window
  (200–300ms). If the implementer buffers stderr aggressively, increase
  the flush — but a correct line-buffered pipe flushes promptly.
- The `list_tools` row shape is flexible (tool name under `tool` or
  `name`); keep it simple. The `get_tool_schema` return should be the
  schema JSON in a text block, not a deeply-nested envelope.
- Phase 0 tests are read-only and unchanged. Do not weaken their
  assertions to make Phase 1 pass.