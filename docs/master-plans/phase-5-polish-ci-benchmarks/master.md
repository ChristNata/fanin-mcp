---
Feature: phase-5-polish-ci-benchmarks
Scope: flat
Stack: rust
Tier: thorough
Status: draft
Created: 2026-06-27
Target: src/main.rs
Dependencies: docs/master-plans/phase-0-*, docs/master-plans/phase-1-*, docs/master-plans/phase-2-*, docs/master-plans/phase-3-*, docs/master-plans/phase-4-*
---

# Master Plan: Phase 5 Polish, CI, And Benchmarks

## What

Ship the final MVP polish layer: redacted JSON file logging, cross-platform CI
with audit/deny gates, crash-safe process containment where the platform allows
it, Streamable-HTTP header-auth coverage through an in-repo mock, namespace and
credential E2E coverage, a token benchmark that owns README figures, resource
budget checks, and release-documentation cleanup.

## Why

Phase 5 is the release gate for the MVP promised in `docs/MVP.md` lines 53-61
and `ROADMAP.md` lines 5-21: the product must work on Windows, macOS, and
Linux; secrets must stay out of logs; hard-kill containment must be proven in
CI; and token claims must be generated rather than guessed. The plan is anchored
in D-004, D-005, D-009, D-010, D-014, D-015, D-016, and D-019 in
`docs/DECISIONS.md`, plus GOTCHA #1, #11, #14, #19, #26, #30, and #31 in
`docs/GOTCHA.md`.

Corrected drift from the inputs:

- `carry-over.md` still names CARRY-2's macOS branch as an open design call at
  lines 27-30. `planner-task.md` lines 25-30 supersede it: macOS hard-kill is a
  documented MVP limitation, not a supervisor/kqueue implementation task.
- `carry-over.md` marks SECURITY.md as finalized at lines 87-89, but the actual
  `SECURITY.md` read here does not document the macOS SIGKILL orphan limitation.
  This plan includes that doc fixup instead of treating release docs as done.
- `src/process.rs` comments at lines 141-146 claim the Unix hard-kill path tears
  down all descendants, but the code only wraps with `ProcessSession` at lines
  183-186. That is graceful process-group setup, not crash-safe SIGKILL
  containment. The plan adds Linux `PDEATHSIG` and documents macOS honestly.
- `docs/MVP.md` line 57 says one real remote HTTP upstream. The task's resolved
  decision replaces that with an in-repo Streamable-HTTP mock and a manual
  release-checklist item. The plan follows the resolved decision and avoids
  live network credentials in CI.

## Dependencies

The plan depends on Phases 0-4 being present in the current tree. Verified
surfaces include:

- `src/main.rs`: CLI and `init_tracing()` are hardcoded to INFO stderr.
- `src/process.rs`: stdio spawning, redaction, stderr file appending, Windows
  post-spawn Job Object assignment, Unix `ProcessSession` wrapping.
- `src/config.rs`: config currently accepts only `stdio` transport and rejects
  any other transport.
- `src/registry.rs`: lazy stdio connection path and private `map_service_error`.
- `Cargo.toml`: no bench target; rmcp is exact-pinned to `=1.8.0`; no HTTP
  transport feature is declared.
- Repo root: no `.github/workflows/`, no `deny.toml`, no `benches/`.

Execution order is partly parallel after tests are created. Phase 1
observability and Phase 2 process hardening both touch process/logging surfaces
and must be sequenced. Phase 3 HTTP support depends on the logging redaction
path and config transport model. Phases 4 and 5 can run after Phase 3 tests are
available. Phase 6 CI consumes all earlier test/bench/gate commands. Phase 7
docs is last so it records the final behavior.

## Scope

In scope:

- Add `--log-file <path>` and `--log-level <level>` to the serve CLI and route
  structured JSON tracing to a redacting file sink without writing diagnostics
  to stdout.
- Log config load, active namespace, upstream connect/disconnect, tool calls,
  latency, and outcome in structured fields.
- Fix CARRY-1 with a Windows suspended-spawn-before-job-assignment path, or the
  thin custom child transport fallback if rmcp/process-wrap cannot expose the
  suspended child handle.
- Fix CARRY-2 on Linux with `prctl(PR_SET_PDEATHSIG, SIGKILL)` and graceful
  process-group teardown; document macOS SIGKILL orphan limits.
- Add regression coverage for the immediate-descendant race window.
- Add Streamable-HTTP upstream support only as far as MVP requires: static
  header injection from resolved credentials, in-repo mock coverage, and no
  live external network dependency.
- Add namespace switching, credential E2E, Claude Code Tool Search composition,
  and CARRY-4 deterministic send-side error coverage where cheap.
- Add `benches/token_cost.rs`, a matching `[[bench]]` target, and generated
  README token figures.
- Add `.github/workflows/` CI, `deny.toml`, audit/deny/fmt/clippy/test gates,
  hard-kill tests on all OSes, and binary/memory budget checks or an explicit
  manual release check where CI cannot measure reliably.
- Update release/security docs for the macOS containment limitation and the
  `ToolError` naming drift.

Out of scope:

- No daemon, supervisor process, kqueue watcher, launchd agent, service manager,
  or background reaper for macOS.
- No OAuth, token refresh, browser auth flow, or v1.1 `auth` subcommand.
- No capability-mirrored forwarding of sampling, elicitation, or roots; MVP
  keeps clean rejection / empty roots only.
- No resources or prompts proxying.
- No `list_server_status`, install command, hot reload, prewarming, connection
  pooling, plugin/middleware system, database, web framework, or HTTP listener
  for fanin-mcp itself.
- No live public remote upstream in CI and no real credentials in tests.
- No parameter-level ACLs or read-only enforcement beyond existing name-level
  namespace filtering.
- No edits to `src/credentials.rs` unless the orchestrator/user first resolves
  CARRY-3's managed OC edit-deny rule; credential Phase 5 work should stay in
  tests and transport/config code if possible.
- No hand-edited README token estimates after the benchmark generator exists.

## Phases

### Phase 1 — Redacted JSON Observability

Scope: Add serve-level logging flags and structured log events while preserving
stdout as the MCP transport.

Produces:

- `src/main.rs`
- `src/process.rs`
- `src/registry.rs`
- `src/server.rs`
- `src/forward.rs`
- `tests/integration/observability.rs`
- `tests/integration/main.rs`

Key Behaviors:

- `fanin-mcp --log-file <path> --log-level <level>` configures a redacted JSON
  file sink for serve mode.
- Existing stderr logging remains redacted and never writes to stdout.
- Invalid log levels fail before `serve(stdio())` starts.
- Each tool call records server, tool, latency, and outcome. Startup records
  config load and active namespace. Upstream lifecycle records connect and
  disconnect/failure events.
- The file sink uses the same secret redaction registry as stderr and child
  stderr logging.

Depends On: existing redaction helpers in `src/process.rs` and the lazy registry
path in `src/registry.rs`.

Skills Needed: `rust-general`, `rust-test`, `rmcp-general`.

Phase Success Criteria:

1. A serve session launched with `--log-file` writes newline-delimited JSON log
   entries to that file and writes no diagnostics to stdout.
2. `--log-level debug` includes debug-level structured events; an invalid level
   exits non-zero before MCP serving begins.
3. A sentinel secret resolved for an upstream is absent from both stderr and the
   JSON log file.
4. A successful `invoke_tool` log entry contains `server`, `tool`, numeric
   latency, and success outcome fields.
5. A failing/structured-error `invoke_tool` log entry records a failure outcome
   without leaking arguments or secret values.

### Phase 2 — Process Containment Hardening

Scope: Close the Windows spawn-then-assign race, add Linux crash-safe parent
death signaling, keep graceful process-group teardown, and encode macOS as a
documented limitation.

Produces:

- `src/process.rs`
- `Cargo.toml`
- `Cargo.lock`
- `tests/probe-server/main.rs`
- `tests/integration/process_lifetime.rs`
- `tests/integration/main.rs`

Key Behaviors:

- Windows child creation uses suspended spawn before assigning the process to a
  kill-on-close Job Object, then resumes the main thread.
- If rmcp `=1.8.0` / `process-wrap` cannot expose the suspended child and thread
  handle, implementation falls back to a thin custom child transport that owns
  `CreateProcess`.
- Linux spawn config installs `PR_SET_PDEATHSIG = SIGKILL` before exec, while
  preserving process-group/session teardown for graceful shutdown.
- macOS keeps graceful process-group teardown and does not claim SIGKILL orphan
  containment.
- Regression coverage makes the probe fork a descendant immediately at child
  startup to hit the Windows race window.

Depends On: Phase 1 only where log/error events overlap; otherwise current
`src/process.rs`.

Skills Needed: `rust-general`, `rust-test`, `rmcp-general`.

Phase Success Criteria:

1. Windows hard-kill CI kills an immediately-started descendant spawned before
   the upstream has completed MCP initialization.
2. Existing stdin-EOF teardown still terminates the full upstream tree.
3. Linux hard-kill CI kills the upstream descendant during the test window.
4. macOS CI verifies graceful teardown and records the SIGKILL orphan scenario
   as an expected documented limitation, not a false green hard-kill claim.
5. Process wrapping does not break child stderr capture or redaction.

### Phase 3 — Streamable-HTTP Mock And Header Auth

Scope: Add the MVP remote-upstream path using an in-repo mock Streamable-HTTP
server with deterministic static header auth.

Produces:

- `src/config.rs`
- `src/registry.rs`
- `src/error.rs`
- `Cargo.toml`
- `Cargo.lock`
- `tests/http-probe-server/main.rs` or equivalent in-repo fixture path
- `tests/integration/http_upstream.rs`
- `tests/integration/main.rs`
- `docs/release-checklist.md`

Key Behaviors:

- Config accepts the minimal HTTP transport shape needed for MVP: endpoint URL
  plus static headers whose values may contain `${VAR}` placeholders.
- Header placeholders resolve through the existing preferred-store -> env
  fallback chain and register resolved secrets for redaction.
- The in-repo HTTP probe asserts it received the expected header and returns a
  normal MCP tool result.
- CI uses only loopback/in-process test infrastructure; no public network and no
  real credentials.
- A manual release-checklist item records the one-time public remote check.

Depends On: Phase 1 redacted logging and current config validation.

Skills Needed: `rust-general`, `rust-test`, `rmcp-general`.

Phase Success Criteria:

1. A config with HTTP transport and `Authorization = "Bearer ${TOKEN}"` connects
   to the in-repo mock and invokes a tool successfully.
2. The mock observes the resolved header value, while the value is absent from
   stderr and JSON log output.
3. Missing header credentials return the existing structured credential error
   shape without spawning/connecting the upstream.
4. Stdio upstream behavior remains unchanged, including lazy spawn and namespace
   filtering.
5. A real-public-remote check exists only as a documented manual release step.

### Phase 4 — E2E Composition And CARRY-4

Scope: Fill the remaining integration gaps around namespaces, credentials,
Tool Search composition, and deterministic send-side error mapping.

Produces:

- `src/registry.rs`
- `tests/integration/namespace_acl.rs`
- `tests/integration/cred_store.rs`
- `tests/integration/tool_search.rs`
- `tests/integration/error_hardening.rs`
- `tests/integration/main.rs`
- `tests/common/fixtures.rs`
- `tests/common/expectations.rs`

Key Behaviors:

- A single E2E session switches namespaces via separate fanin-mcp launches and
  proves each namespace sees exactly its allowed server/tool rows.
- Credential E2E covers keyring round-trip where available and env fallback in
  CI without editing `src/credentials.rs`.
- Tool Search composition proves downstream `tools/list` exposes exactly the
  three meta-tools and does not conflict with Claude Code's deferred schema
  model.
- CARRY-4 is resolved by making `map_service_error` testable and adding a
  deterministic unit/integration check for `TransportSend -> upstream_disconnected`,
  avoiding flaky OS pipe timing.

Depends On: Phase 3 for HTTP credential/header paths; Phase 1 for log redaction
assertions.

Skills Needed: `rust-general`, `rust-test`, `rmcp-general`.

Phase Success Criteria:

1. Namespace E2E shows one namespace hides a server/tool that another namespace
   exposes, and denied calls return `namespace_denied`.
2. Credential E2E stores/lists/removes key names only on supported hosts and
   falls back to env in CI without exposing secret values.
3. Each upstream receives only its own resolved env/header values.
4. Downstream `tools/list` returns exactly `list_tools`, `get_tool_schema`, and
   `invoke_tool` throughout the composition check.
5. `ServiceError::TransportSend` maps to the public `upstream_disconnected`
   structured error in a deterministic, non-ignored test.

### Phase 5 — Token Benchmark And Generated README Figures

Scope: Add the benchmark target consumed by `/gate` and make benchmark output
the source of README token claims.

Produces:

- `benches/token_cost.rs`
- `Cargo.toml`
- `Cargo.lock`
- `README.md`
- `scripts/update-token-figures.rs` or a small Rust helper under `benches/`
- `tests/integration/token_figures.rs`

Key Behaviors:

- `cargo bench --bench token_cost` runs with the exact target name expected by
  the project gate.
- The benchmark measures downstream `tools/list` meta-tool definitions and a
  representative session containing discovery, schema lookup, and invocation.
- README token numbers are bracketed/generated by the benchmark helper and are
  not hand-maintained estimates.

Depends On: current static meta-tools and Phase 3/4 composition behavior.

Skills Needed: `rust-general`, `rust-test`.

Phase Success Criteria:

1. `cargo bench --bench token_cost` exits 0.
2. Benchmark output includes separate measurements for permanent meta-tool
   definitions and representative session cost.
3. README token figures match the generated benchmark output exactly.
4. A test/gate fails if README token figure markers drift from generated output.

### Phase 6 — CI, Audit/Deny, And Resource Budgets

Scope: Add the release gate that runs on Windows, macOS, and Linux and enforces
format, lint, tests, supply-chain policy, process-lifetime tests, benchmark
availability, and resource budgets.

Produces:

- `.github/workflows/ci.yml`
- `deny.toml`
- `Cargo.toml`
- `Cargo.lock`
- `scripts/check-resource-budgets.rs` or equivalent repo-local helper
- `docs/release-checklist.md`

Key Behaviors:

- CI matrix runs on Windows, macOS, and Linux.
- Gate commands include `cargo fmt --check`, clippy, full tests, `cargo audit`,
  `cargo deny`, build, and the token benchmark target.
- Hard-kill process-lifetime tests run on all three OS runners with platform
  expectations from Phase 2.
- `deny.toml` encodes advisories, bans, licenses, and sources policy for the
  deliberately small anti-stack dependency tree.
- Binary size is checked in CI after release build and strip where platform
  tooling permits. Memory budgets are checked in CI if reliable; otherwise the
  release checklist carries a concrete command and threshold.

Depends On: Phases 1-5.

Skills Needed: `rust-general`, `rust-test`.

Phase Success Criteria:

1. CI workflow exists and defines Windows, macOS, and Linux jobs.
2. CI runs fmt, clippy, tests, audit, deny, build, and `cargo bench --bench
   token_cost` or its no-run compile/availability equivalent where full bench
   execution is not appropriate on every push.
3. `deny.toml` allows MIT OR Apache-2.0 and rejects unapproved licenses,
   unknown registries, and duplicate/banned dependencies per project policy.
4. Release binary size is measured against `< 10MB stripped` in an automated
   step where supported.
5. Idle and five-upstream memory budgets have either an automated assertion or
   a documented manual release command with `<15MB` and `<50MB` thresholds.

### Phase 7 — Release Docs And Spec Drift Cleanup

Scope: Align public and design docs with the final MVP behavior, including
macOS containment honesty and `ToolError` naming.

Produces:

- `SECURITY.md`
- `README.md`
- `ROADMAP.md`
- `docs/GOTCHA.md`
- `docs/ARCHITECTURE.md`
- `docs/MVP.md`
- `docs/release-checklist.md`
- `STACK.md` if dependency/CI policy changed materially

Key Behaviors:

- SECURITY.md documents the macOS hard-kill limitation and does not overclaim
  zero-orphan SIGKILL containment on macOS.
- GOTCHA records the Linux/macOS split clearly: Linux crash-safe PDEATHSIG,
  macOS graceful process-group teardown only for MVP.
- ARCHITECTURE/MVP drift from `AggError`/`ErrorCode` to `ToolError` is corrected
  without changing the public D-005 wire shape.
- README token and resource claims point to generated/checked sources.
- Release checklist includes manual public Streamable-HTTP check and any memory
  measurement that cannot be made reliable in CI.

Depends On: Phases 1-6.

Skills Needed: `md-authoring`, `rust-general` for command accuracy.

Phase Success Criteria:

1. SECURITY.md explicitly states that macOS MVP does not guarantee zero orphans
   after fanin-mcp is killed with SIGKILL.
2. No docs claim `AggError`/`ErrorCode` as the current internal Rust type names;
   docs use `ToolError` or describe the public wire shape without internal names.
3. README token numbers are generated from the benchmark and no longer framed as
   estimates.
4. Release checklist exists and contains the manual public HTTP upstream check,
   memory-budget command if needed, and platform support verification.
5. ROADMAP non-goals remain intact: no daemon, listener, plugin system,
   multi-tenancy, or parameter-level ACL is added or implied.

## Success Criteria

1. `--log-file <path>` creates redacted newline-delimited JSON logs and never
   writes diagnostics to stdout.
2. `--log-level <level>` controls tracing verbosity and rejects invalid levels
   before MCP serving begins.
3. Structured logs include config load, active namespace, upstream connect,
   upstream disconnect/failure, and per-call server/tool/latency/outcome fields.
4. The sentinel-secret test proves secrets are absent from stderr, child stderr
   logs, and the new JSON file sink.
5. Windows CI proves an immediately-started descendant cannot escape the Job
   Object assignment window.
6. Linux CI proves hard-kill containment through parent-death signaling during
   the test window.
7. macOS CI/docs do not overclaim hard-kill containment; macOS graceful teardown
   remains tested.
8. Streamable-HTTP header-auth works against an in-repo mock with no network or
   real credentials in CI.
9. Missing HTTP header credentials return the existing structured credential
   error and do not leak secret material.
10. Namespace-switching E2E proves different sessions expose different allowed
    server/tool rows and denied calls return `namespace_denied`.
11. Credential E2E proves key names only are listed and env fallback works in
    CI/keyring-less mode.
12. Tool Search composition proves clients see exactly the three meta-tools and
    no upstream schemas at startup.
13. `TransportSend` maps deterministically to `upstream_disconnected` in an
    unignored test.
14. `cargo bench --bench token_cost` runs and emits permanent-tool and
    representative-session token measurements.
15. README token figures match benchmark-generated output exactly.
16. CI matrix exists for Windows, macOS, and Linux and runs fmt, clippy, tests,
    audit, deny, and build gates.
17. `deny.toml` enforces advisories, bans, licenses, and source policy matching
    the Rust anti-stack constraints.
18. Release binary size is measured against `<10MB stripped`.
19. Idle memory `<15MB` and five-upstream memory `<50MB` are either automatically
    asserted or captured as a concrete manual release gate.
20. SECURITY.md, GOTCHA.md, ARCHITECTURE.md, MVP.md, and README.md reflect the
    final Phase 5 behavior and no longer contain the `AggError`/`ErrorCode`
    internal-name drift.
21. The full test suite passes at 100%; no ignored Phase 5 regression test is
    left as the only proof for a shipped criterion.

## Constraints / Invariants

- stdout remains the MCP transport. No `println!`, `print!`, or stdout tracing
  in serve mode.
- Errors remain `CallToolResult { isError: true }` with the D-005 structured
  JSON shape; do not convert tool failures into JSON-RPC errors.
- Result content remains byte-faithful; do not stringify content arrays.
- Registry locks are never held across upstream calls or HTTP requests.
- Secrets are never accepted on argv, never logged, and never inherited as the
  full ambient environment by children.
- Each upstream receives only its own resolved env/header values.
- rmcp remains exact-pinned. Verify any HTTP or child-transport API against the
  pinned `=1.8.0` crate, not pseudocode.
- No test file edits outside the `test-creator` stage. Implementers treat tests
  as read-only contracts.
- CARRY-3 is a user/tooling config issue, not a code workaround. Avoid planning
  `src/credentials.rs` edits unless the orchestrator resolves it.
- macOS hard-kill orphan risk is documented for MVP rather than engineered
  around with a daemon, supervisor, or watcher.
- The cycle model is linear on `main`; version stamps for this phase are
  `v0.6.x`.

## Open Questions

1. **Can rmcp `=1.8.0` / process-wrap expose enough Windows suspended-spawn
   control for CARRY-1?** Default: if it cannot expose the main-thread handle
   needed to resume after Job Object assignment, implement the sanctioned thin
   custom child transport that owns `CreateProcess`.
2. **What is the exact CI method for memory budgets?** Default: automate binary
   size in CI; automate memory on platforms where a stable repo-local helper can
   read RSS during the test window; otherwise make memory a release-checklist
   command with the same `<15MB idle` and `<50MB @ 5 upstreams` thresholds.
3. **Which rmcp feature/API is required for Streamable-HTTP client transport at
   the `=1.8.0` pin?** Default: enable the minimal rmcp/client HTTP feature that
   supports Streamable HTTP and keep the in-repo mock loopback-only. If the pin
   lacks usable support, surface a structural finding before adding a different
   HTTP stack.
