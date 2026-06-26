# test-creator: phase-2-multi-namespace

Phase 2 test contract — multi-upstream proof + namespace ACL completeness.
The implementer codes against this suite; the objective gate runs it. Test
files are read-only to every later stage.

## Stack & runner

- **Runner:** `cargo test --test integration` (main suite). `cargo nextest
  run --workspace` works equivalently. Inherits the Phase 0/1 harness
  unchanged — wire-level JSON-RPC-over-stdio, no `src/` stubs.
- **Async:** `#[tokio::test]` single-threaded default; concurrency tests
  (`multi_upstream::concurrent_first_calls_to_same_cold_server_spawn_once`,
  `multi_upstream::alpha_slow_tool_does_not_block_concurrent_beta_echo`) use
  `flavor = "multi_thread", worker_threads = 2`.
- **Wire-level (D-015).** Tests spawn the built `fanin-mcp` binary and speak
  raw JSON-RPC over stdio, asserting on the wire JSON. No `src/` stubs are
  required for the suite to compile; the harness depends only on `tokio` +
  `serde_json` + `tempfile` (dev-deps).
- **Build order.** `cargo test` builds the `fanin-mcp` and `probe-server`
  `[[bin]]` targets before the test binary, so `CARGO_BIN_EXE_fanin-mcp` and
  `CARGO_BIN_EXE_probe-server` resolve. Env var names use the bin names
  EXACTLY as-declared (dashes/case preserved).

## Files created / extended

| Path | Criteria covered |
|---|---|
| `tests/common/fixtures.rs` (extended) | Phase 2 multi-upstream + namespace-ACL config builder (`MultiConfigBuilder`, `ServerEntry`, `NamespaceEntry`). Encodes the resolved Open Question #1 config schema (§Config schema). |
| `tests/integration/multi_upstream.rs` (new) | Master SC 1, 2, 3, 4, 5, 12, 13; P1.SC1–5 |
| `tests/integration/namespace_acl.rs` (new) | Master SC 6, 7, 8, 9, 10; P2.SC1–5 |
| `tests/integration/main.rs` (extended) | `mod multi_upstream;` and `mod namespace_acl;` declarations. |

Phase 0/1 files (`aggregator.rs`, `config.rs`, `discovery.rs`, `gate.rs`,
`invoke.rs`, `manual_e2e.rs`, `pinning.rs`, `probe.rs`, `registry.rs`,
`reverse_traffic.rs`, `common/expectations.rs`, `common/mod.rs`) are
unchanged. The Phase 0/1 tests still run and still pass — the Phase 2
contract is additive.

## Config schema (Phase 2 extension — binding)

The Phase 2 config is a strict superset of the Phase 1 shape
(`tests/common/fixtures.rs::ConfigBuilder`). The implementer's Phase 1
parser must already accept it; Phase 2 adds parsing of the per-server tool
allow-list (the resolved Open Question #1) and threads it through
`ActiveNamespace::is_tool_allowed`.

```toml
# N named stdio upstreams. Each is the same probe binary under a distinct
# configured name; the configured name is what the aggregator routes on.
[servers.alpha]
transport = "stdio"
command = '/path/to/probe-server'
args = []
log_file = '/path/to/log'   # optional

[servers.beta]
transport = "stdio"
command = '/path/to/probe-server'
args = []

# N namespaces. servers = [...] is the server allow-list. An optional
# [namespaces.<name>.tools] sub-table with <server> = ["tool", ...] entries
# is the per-server name-level tool allow-list.
[namespaces.default]
servers = ["alpha", "beta"]

[namespaces.filtered]
servers = ["alpha", "beta"]
[namespaces.filtered.tools]
alpha = ["echo_ok"]          # present list => EXACT name-level allow-list
# beta has NO tools entry but is present in `servers` => ALL its tools visible
```

### Choices recorded (Open Question #1 resolved)

- **`[namespaces.<name>.tools]`** is an optional sub-table. Each key is a
  server name (must also be in `servers` for the filter to apply); each
  value is an array of tool name strings. A present list is an EXACT
  name-level allow-list — only those tool names are visible/callable for
  that server in that namespace. An absent entry for an allowed server
  means ALL its tools are visible.
- **Name-level only.** No parameter-level ACL, argument inspection, SQL
  parsing, or path filtering (D-006, GOTCHA #31, ROADMAP). The tool filter
  is a list of tool NAME strings; the proxy never inspects arguments.
- **Tool names are NOT validated at startup.** Tools are only known after
  lazy discovery, so a tool filter may name a tool the upstream does not
  expose — that name is simply never matched. The implementer MAY validate
  that tool-filter server keys reference a server in `servers` (fail-fast on
  a typo), but the tests do not require it.
- **No `readonly = true` enforcement.** Per-server `readonly = true` based
  on upstream annotations is v1.1 (D-006 follow-up). Phase 2 does not add
  it. The read-only namespace PATTERN is documented in `SECURITY.md`
  (Phase 3 of this plan), not enforced in code.

## Coverage map — master Success Criteria

| # | Master Success Criterion | Test(s) |
|---|---|---|
| 1 | A config with 2–3 probe-backed upstreams starts; `tools/list` exposes exactly the 3 static meta-tools | `multi_upstream::multi_upstream_config_starts_aggregator`; `multi_upstream::multi_upstream_preserves_phase0_phase1_guarantees` (re-asserts after 3-upstream exercise) |
| 2 | Starting fanin-mcp and calling `tools/list` opens ZERO upstream connections when multiple upstreams are configured | `multi_upstream::downstream_tools_list_with_multi_upstream_opens_zero_connections` (log-sink observation: no `alpha`/`beta` line after `tools/list`) |
| 3 | Targeting one upstream proves lazy isolation: an untargeted second upstream is untouched until a request names it | `multi_upstream::targeting_alpha_leaves_beta_unspawned_until_beta_targeted` (log-sink: `alpha` line after targeting alpha, NO `beta` line; `beta` line only after targeting beta) |
| 4 | Concurrent first calls to the same cold upstream initialize/spawn that upstream exactly once | `multi_upstream::concurrent_first_calls_to_same_cold_server_spawn_once` (consistent-success proxy; strict process-count is §Boundaries) |
| 5 | A delayed `slow_tool` call on `alpha` does not block a concurrent successful call to `beta`; `beta` completes inside a deadline shorter than the configured slow delay | `multi_upstream::alpha_slow_tool_does_not_block_concurrent_beta_echo` (the D-007 / GOTCHA #16 cross-upstream proof; PROOF_DEADLINE = 400ms < SLOW_DELAY_MS = 800ms) |
| 6 | Omitting `--namespace` selects `default` and exposes exactly the servers in `[namespaces.default]` | `namespace_acl::omitted_namespace_selects_default_exposing_only_declared_servers` (default lists only `alpha`; `beta` hidden + denied) |
| 7 | A server visible in one namespace appears in `list_tools` and is invokable; the same server denied in another is hidden and returns `namespace_denied` from `get_tool_schema` and `invoke_tool` | `namespace_acl::server_visibility_matrix_across_namespaces` (beta visible+invokable in `open`; hidden+denied in `restricted`, denied from both `get_tool_schema` and `invoke_tool`) |
| 8 | `tools.<server> = [...]` enforces name-level tool filtering: allowed tools listed/callable, denied tools hidden and `namespace_denied` when addressed directly | `namespace_acl::tool_level_acl_filters_list_schema_and_invoke` (alpha lists only `echo_ok`: list shows `echo_ok`, hides `dangerous_noop`; schema for `echo_ok`, denied for `dangerous_noop`; invoke succeeds for `echo_ok`, denied for `dangerous_noop`; beta has no tools entry => all 8 visible) |
| 9 | `namespace_denied` is a tool-level `CallToolResult { isError: true }` with JSON text carrying `code`, server, denied tool when applicable, message, recoverable — never a JSON-RPC error | `namespace_acl::namespace_denied_error_shape_for_denied_server_and_tool` (denied-server and denied-tool paths; asserts `code: "namespace_denied"`, `server`, `tool` on the tool-denial path, `message`, `recoverable`; `assert_no_rpc_error` on every path) |
| 10 | Denied server checks happen before upstream connection; a denied server is not spawned just to reject the request | `namespace_acl::denied_server_is_not_spawned_to_reject_call` (log-sink: NO `beta` line after denied `beta__echo_ok`; `alpha` line appears after allowed `alpha__echo_ok` — proves the log sink works and the beta absence is meaningful) |
| 11 | `SECURITY.md` documents the read-only namespace pattern and the name-level filtering boundary | **Docs-only criterion — not a test.** Covered by Phase 3 of this plan (docs phase). The test contract does not assert on `SECURITY.md` content; review verifies it. |
| 12 | The existing probe binary is reused under distinct configured server names; no second fixture binary is added | Every Phase 2 test uses `fx::MultiConfigBuilder` which registers `probe_bin_path()` under distinct names (`alpha`, `beta`, `gamma`). Structural: no second `[[bin]]` target was added (Cargo.toml unchanged). |
| 13 | Existing Phase 0/1 guarantees remain intact: static meta-tools, lazy startup, raw argument forwarding, byte-faithful results, reverse-traffic handling, stdout discipline | `multi_upstream::multi_upstream_preserves_phase0_phase1_guarantees` (3-upstream: static 3 meta-tools, 24-row list_tools, byte-faithful `beta__echo_ok`, reverse-traffic `alpha__needs_sampling` completes within deadline, final static `tools/list`); Phase 0/1 tests unchanged + still run |
| 14 | All required gates pass at 100%; failures surfaced and fixed in scope or routed, never thresholded | The full suite (Phase 0 + 1 + 2) is the gate. Phase 4 of this plan runs it. The 2 expected-red tool-ACL tests are the work Phase 2's implementer must complete to turn the gate green. |

## Coverage map — Phase sub-criteria

| Phase | Criterion | Test |
|---|---|---|
| P1.1 | 2–3 probe-backed upstreams start; `tools/list` = 3 static meta-tools | `multi_upstream::multi_upstream_config_starts_aggregator`; `multi_upstream::multi_upstream_preserves_phase0_phase1_guarantees` |
| P1.2 | `tools/list` opens zero upstream connections with multiple upstreams configured | `multi_upstream::downstream_tools_list_with_multi_upstream_opens_zero_connections` |
| P1.3 | Targeting `alpha` leaves `beta` untouched until `beta` is targeted | `multi_upstream::targeting_alpha_leaves_beta_unspawned_until_beta_targeted` |
| P1.4 | Racing first calls to the same cold server spawn exactly once (strongest stable observable) | `multi_upstream::concurrent_first_calls_to_same_cold_server_spawn_once` (consistent-success proxy) |
| P1.5 | `alpha__slow_tool` delay does not block concurrent `beta__echo_ok`; beta completes inside a deadline shorter than the slow delay | `multi_upstream::alpha_slow_tool_does_not_block_concurrent_beta_echo` |
| P2.1 | Omitting `--namespace` selects `default`; `default` exposes exactly its declared servers | `namespace_acl::omitted_namespace_selects_default_exposing_only_declared_servers` |
| P2.2 | Server visible in namespace `open` appears in `list_tools` + invokable; denied in `restricted` hidden + `namespace_denied` from `get_tool_schema` and `invoke_tool` | `namespace_acl::server_visibility_matrix_across_namespaces` |
| P2.3 | Tool filters: allowed server with only `echo_ok` exposes `echo_ok` in `list_tools`, hides `dangerous_noop`; schema for `echo_ok`, `namespace_denied` for `dangerous_noop` invocation | `namespace_acl::tool_level_acl_filters_list_schema_and_invoke` |
| P2.4 | Namespace checks before lazy connection for denied servers; denied server not spawned to reject | `namespace_acl::denied_server_is_not_spawned_to_reject_call` |
| P2.5 | No parameter-level ACL, argument parsing, or destructive/read-only annotation enforcement added | Structural — enforced by review, not by a unit test. No Phase 2 test asserts on argument contents (name-level only, per the hard constraint). |
| P3.1–P3.4 | `SECURITY.md` docs | **Docs-only — Phase 3.** Not in the test contract. |
| P4.1–P4.4 | Gate + scope cleanup | Phase 4 runs the full suite; no new tests. |

## Side-effect assertions

Every Phase 2 test asserts the observable effect, not just a return value,
so a stub that returns the right shape without doing the work fails.

- **Multi-upstream lazy startup is observable via the log sink.**
  `downstream_tools_list_with_multi_upstream_opens_zero_connections` reads
  the log file after downstream `tools/list` (asserts NO `alpha`/`beta`
  line). An eager impl that spawns on `tools/list` fails the assertion.
- **Lazy isolation is observable per-upstream via the log sink.**
  `targeting_alpha_leaves_beta_unspawned_until_beta_targeted` reads the log
  after targeting `alpha` (asserts `alpha` line present, `beta` line ABSENT),
  then after targeting `beta` (asserts `beta` line appeared). A non-lazy
  impl that spawns all upstreams on first contact fails the first beta
  assertion; a stub that never spawns fails the alpha assertion.
- **Non-serialization is a wall-clock effect (D-007 / GOTCHA #16).**
  `alpha_slow_tool_does_not_block_concurrent_beta_echo` issues `alpha__slow_tool`
  (800ms) without awaiting, immediately issues `beta__echo_ok`, and asserts
  the beta echo completes within 400ms — strictly shorter than the slow
  delay. A registry lock held across the alpha slow await serializes the
  session; the beta echo blocks until alpha finishes (>= 800ms) and the
  400ms deadline times out. Requires REAL forwarding on both upstreams (alpha
  slow success + beta echo success), so a not-implemented stub fails RED
  rather than passing trivially.
- **Single-spawn under race is the consistent-success proxy.**
  `concurrent_first_calls_to_same_cold_server_spawn_once` sends two
  concurrent `list_tools { server: "alpha" }` calls and asserts both
  SUCCEED with consistent inventory (same sorted tool-name set). A
  double-spawn race would either error one call or return inconsistent
  rows. Requiring success makes the test fail RED against a stub. The
  strict process-count assertion is platform-specific and brittle on
  Windows — see §Boundaries.
- **Tool-level ACL hides denied tools in `list_tools` (not just at invoke).**
  `tool_level_acl_filters_list_schema_and_invoke` asserts the `alpha` rows
  contain EXACTLY `["echo_ok"]` — a stub that lists then fails at invocation
  time fails this assertion. The filter must be applied at discovery, not
  deferred to invoke.
- **Denied-server-not-spawned is observable via the log sink.**
  `denied_server_is_not_spawned_to_reject_call` asserts NO `beta` line
  appears in the log after a denied `beta__echo_ok` call, then asserts an
  `alpha` line DOES appear after an allowed `alpha__echo_ok` call — proving
  the log sink works and the beta absence is meaningful. A stub that
  connects-then-denies would spawn beta and leave a log line, failing the
  assertion.
- **`namespace_denied` shape is asserted on the wire JSON.**
  `namespace_denied_error_shape_for_denied_server_and_tool` parses the
  CallToolResult text content as JSON and asserts `code: "namespace_denied"`,
  `server`, `tool` (on the tool-denial path), `message`, `recoverable`. A
  stub that returns a generic error string without the structured shape
  fails the JSON parse or the field assertions. `assert_no_rpc_error` on
  every path enforces D-005 (tool-level failure stays in the conversation,
  never a JSON-RPC error).
- **Byte-faithful round-trip under multi-upstream (D-004).**
  `multi_upstream_preserves_phase0_phase1_guarantees` asserts
  `beta__echo_ok` echoes the payload verbatim — a proxy that re-serialized
  the args would mangle it.
- **Reverse-traffic rejection under multi-upstream (GOTCHA #2).**
  `multi_upstream_preserves_phase0_phase1_guarantees` asserts
  `alpha__needs_sampling` completes within 10s (the aggregator rejects the
  sampling request, not a hang) and forwards the probe's success result. A
  stub with no `ClientHandler` would let the probe wait forever; the
  deadline catches the hang.
- **No stdout diagnostics (GOTCHA #1).** Every wire test implicitly asserts
  clean JSON on stdout — the harness panics on a non-JSON line. Phase 2
  adds no stdout-writing path.

## Deferred tests

None. Every Phase 2 test runs. The 2 expected-red tool-ACL tests are NOT
deferred — they are the test-first contract for the implementer's Phase 2
work and turn green once the per-server tool allow-list filter is built.

## Coverage gaps & boundaries

These are criteria the suite does NOT fully prove at the wire level, with
the reason and the proxy/boundary that does cover them:

- **Master SC 4 / P1.SC4 (exactly one spawn under race) — strict process
  count.** The wire suite asserts the observable consequence (both
  concurrent first-calls SUCCEED with consistent inventory) rather than a
  process count. A strict process-count assertion is platform-specific
  (counting probe children) and brittle in CI on Windows — the probe
  children are `probe-server.exe` instances, and counting them reliably
  across a parallel test run is flaky. The consistent-success proxy is what
  the plan sanctions for the wire suite. **The implementer MAY add a
  stricter spawn-sentinel assertion (a log/spawn marker counted exactly
  once) if the harness grows one; the test is written to accept that
  stronger observable without restructuring.**
- **Master SC 11 / P3.SC1–4 (`SECURITY.md` docs).** Docs-only criterion,
  owned by Phase 3 of this plan. The test contract does not assert on
  `SECURITY.md` content; review verifies the read-only namespace pattern,
  the name-level-filtering boundary statement, the full-filesystem warning,
  and the no-annotation-enforcement claim.
- **P2.SC5 (no parameter-level ACL).** Structural — enforced by review. No
  Phase 2 test asserts on argument contents (name-level only, per the hard
  constraint). A unit test cannot prove the absence of a feature; review
  confirms no argument inspection / SQL parsing / path filtering was added.
- **Tool-filter server-key validation.** The plan's Phase 2 allows
  (but does not require) the implementer to validate that
  `[namespaces.<name>.tools]` keys reference a server in `servers`
  (fail-fast on a typo). The tests do not exercise this — a tool filter on
  a non-allowed server is moot (the server is already denied by `servers`).
  If the implementer adds the validation, it must not reject the configs the
  tests write (all test tool-filter keys reference servers in `servers`).
- **Phase 3 / Phase 4 boundary.** This contract does NOT test credentials,
  keyring/env fallback, `${VAR}` interpolation, auth headers, `timeout_secs`,
  cancellation forwarding, process-tree Job Object / process group lifetime
  (Phase 3); nor name/description sanitization, length-capping, final
  public error enum hardening, upstream crash isolation, or
  `notifications/tools/list_changed` cache invalidation (Phase 4). Those
  are out of scope. A Phase 2 implementation that adds any of them is scope
  creep — surface it in review, do not certify it with these tests.

## `list_tools` row shape (unchanged from Phase 1)

The Phase 1 row shape is preserved: each row is a JSON object with a `tool`
(or `name`) field, a `server` field, and a `description`. Phase 2 tests
assert on the `server` field (for namespace visibility) and the `tool`/`name`
field (for tool-level ACL). The implementer may add fields; the tests do not
pin the full row shape.

## Run-and-fail confirmation

The suite compiles clean (`cargo build --tests` — zero warnings) and runs
with the expected test-first state against the current tree (Phase 1 code
landed, Phase 2 tool-ACL filter NOT yet built):

- **11 Phase 2 tests total** (6 in `multi_upstream.rs`, 5 in
  `namespace_acl.rs`). 0 ignored. 0 deferred.
- **Against the current tree (Phase 1 code):** 9 pass, 2 fail RED.
  - The 2 RED failures are the tool-level ACL tests
    (`tool_level_acl_filters_list_schema_and_invoke` and
    `namespace_denied_error_shape_for_denied_server_and_tool`). Both fail
    because `is_tool_allowed` currently delegates to `is_server_allowed`
    (the tool filter is not enforced): `list_tools` returns all 8 alpha
    tools instead of just `echo_ok`, and `alpha__dangerous_noop` is not
    denied. These are the intended test-first failures — the implementer
    turns them green by building the per-server tool allow-list filter
    (Phase 2 of this plan).
  - The 9 greens are the multi-upstream proof (config start, zero-spawn at
    `tools/list`, lazy isolation, single-spawn proxy, the cross-upstream
    non-serialization proof, the Phase 0/1 regression guard) and the
    server-level ACL tests (default namespace selection, server visibility
    matrix, denied-server-not-spawned). These pass against the existing
    Phase 1 code because Phase 1 already built server-level ACL checks and
    the registry lock discipline.
- **Full suite (Phase 0 + 1 + 2):** 65 passed, 2 failed (the expected-red
  tool-ACL tests), 2 ignored (Phase 0 manual E2E). No regressions to
  Phase 0/1 tests.
- **The red is meaningful:** every RED test fails on the assertion that
  requires Phase 2 tool-ACL behavior, not on a compile error, a missing
  symbol, or a malformed harness. The implementer turns each RED green by
  implementing the `tools.<server> = [...]` filter in `src/config.rs`,
  `src/namespace.rs`, and `src/server.rs`.

## Per-module inventory

| Module | Tests |
|---|---|
| `multi_upstream` (Phase 2) | 6 |
| `namespace_acl` (Phase 2) | 5 |
| **Phase 2 total** | **11** |
| Phase 0/1 modules (unchanged) | 58 (2 ignored) |
| **Grand total** | **69** (2 ignored) |

## Notes for the implementer

- The config schema in `tests/common/fixtures.rs::MultiConfigBuilder` is the
  binding Phase 2 contract. Parse exactly the `[namespaces.<name>.tools]`
  sub-table with `<server> = ["tool", ...]` entries; an absent entry for an
  allowed server means all tools visible, a present list is an exact
  name-level allow-list.
- The tool filter is applied at DISCOVERY (`list_tools` must omit denied
  tool rows), at SCHEMA lookup (`get_tool_schema` returns
  `namespace_denied` for a denied tool), and at INVOKE (`invoke_tool`
  returns `namespace_denied` for a denied tool). Hiding at invoke-time only
  is NOT sufficient — `list_tools` must hide denied tools.
- `is_tool_allowed(server, tool)` is the single chokepoint. Currently it
  delegates to `is_server_allowed`; Phase 2 makes it consult the per-server
  tool allow-list when one is present, and fall back to
  `is_server_allowed` when no tool list is present for that server.
- The `namespace_denied` error shape is already implemented in
  `src/error.rs::ToolError::NamespaceDenied` (renders JSON with `code`,
  `server`, `tool`, `message`, `recoverable` inside `CallToolResult::error`).
  Phase 2 does not change the shape — it wires the tool-level denial path
  to use it. The server-denial path (no `tool` in the name) already works.
- The denied-server-not-spawned contract (SC 10) is already satisfied by
  the current `server.rs` dispatch order: `is_tool_allowed` is checked
  before `registry.call_tool` (which is the lazy-connect path). The test
  confirms this continues to hold once the tool filter is wired.
- The cross-upstream non-serialization proof (SC 5) already passes against
  the current `registry.rs` lock discipline (clone-Arc-then-drop-lock). The
  test confirms it continues to hold; no code change is expected unless the
  implementer introduces a regression.
- Phase 0/1 tests are read-only and unchanged. Do not weaken their
  assertions to make Phase 2 pass.