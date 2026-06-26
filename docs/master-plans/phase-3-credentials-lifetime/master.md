---
Feature: phase-3-credentials-lifetime
Scope: flat
Stack: rust
Tier: THOROUGH
Status: draft
Created: 2026-06-27
Target: src/credentials.rs, src/config.rs, src/registry.rs, src/process.rs
Dependencies: docs/master-plans/phase-2-multi-namespace/master.md; docs/MVP.md Phase 3
---

# Master Plan: Phase 3 Credentials + Timeouts + Process Lifetime

## What

Ship the Phase 3 operational layer for fanin-mcp: credential storage and CLI
management, spawn-time `${VAR}` resolution with least-privilege injection,
secret redaction, per-server upstream call timeouts and cancellation handling,
and crash-safe upstream process-tree lifetime on Windows and Unix.

## Why

The binding scope anchor is `docs/MVP.md` Phase 3: `CredentialStore`,
`KeyringStore`, and `EnvStore`; `cred set|list|rm`; `${VAR}` interpolation at
spawn; per-upstream variable isolation; static header credential plumbing;
log redaction with a sentinel-secret test; per-server `timeout_secs` defaulting
to 60; `upstream_timeout` structured errors; downstream cancellation handling;
and process-tree containment with Windows Job Objects / Unix process groups.

The plan is grounded in `docs/DECISIONS.md` D-005, D-009, D-010, D-011, and
D-012. Tool-level failures remain `CallToolResult { isError: true }` with the
public structured JSON shape. Every upstream child must live in a Job Object or
process group so a hard-kill of the aggregator leaves zero orphans. Secrets live
in the OS keychain first, env is fallback, `cred set` never accepts a value on
argv, and resolved secrets never enter logs. Per-server timeouts must fail
informatively before the client hard timeout when possible.

`docs/GOTCHA.md` sharpens the same requirements: #1 forbids stdout diagnostics;
#11/#14 make process-tree lifetime non-negotiable on Windows and Unix; #18/#19/#22
forbid argv secrets, log leakage, and shared child environments; #16 forbids a
registry-map lock across an upstream await. `docs/ARCHITECTURE.md` names the
module contracts for `credentials.rs`, `config.rs`, `registry.rs`, `process.rs`,
`forward.rs`, and `error.rs`.

Verified current tree state:

| Surface | Verified file | Current state | Phase 3 adds |
|---|---|---|---|
| Credential module | `src/credentials.rs` | Bare doc-comment stub. No trait, backend, resolver, prompt, or redaction helper. | `CredentialStore`, keyring/env backends, resolution chain, CLI helpers, redaction registration. |
| CLI | `src/main.rs` | `cred set`, `cred list`, and `cred rm` are stubs with no operands and return failure via tracing. No `--credential-store`. | Real subcommand args, hidden prompt for `set`, names-only list, delete, backend selection, no stdout during serve. |
| Config | `src/config.rs` | Stdio-only `transport`, `command`, `args`, literal `env`, `log_file`, and Phase 2 namespace tool filters. No `timeout_secs`, `headers`, `url`, `cwd`, or `${VAR}` resolution. | `timeout_secs` default 60, interpolation-aware env values, optional future header config decision, validation for secret-looking literals. |
| Registry | `src/registry.rs` | Lazy `Arc<RunningService>` entries, init guards, list-all-tools, and call forwarding outside the entries lock. No credential resolver, timeout wrapper, cancellation map, or HTTP branch. | Resolve per-server env before spawn, wrap calls in timeout, return `upstream_timeout`, and integrate cancellation without regressing lock discipline. |
| Process | `src/process.rs` | Spawns `TokioChildProcess::builder(cmd)`, injects `config.env` literally, captures stderr to log file. No Job Object / process group, no hard-kill containment, no `cwd`, no redaction at the log sink. | Least-privilege env injection, redacted stderr/upstream log writes, cwd if approved, Job Object / process group wrapper or custom child transport. |
| Error model | `src/error.rs` | Structured tool errors for invalid, namespace, connect, and call failures. No `upstream_timeout` or credential-resolution errors. | Public `upstream_timeout` code and credential/spawn resolution errors in the same tool-result shape. |
| rmcp pin and deps | `Cargo.toml`, `Cargo.lock` | rmcp is pinned exactly to `=1.8.0`; `keyring`, `rpassword`, `process-wrap`, and `command-group` are not currently dependencies. | Add only the Phase 3 crates actually used, pinned by Cargo.lock, without bumping rmcp incidentally. |

Corrected drift: the task says the core crates `keyring`, `rpassword`, and a
process-tree crate are available per `CLAUDE.md`. The project preamble lists them
as intended stack, but `Cargo.toml` does not yet declare them. This is not
blocking; Phase 3 owns adding the required dependencies and committing the lock
changes. The task also repeats an older config comment saying namespace tool
filters are absent; the current tree has Phase 2 `tools.<server>` filters in
`src/config.rs` and `src/namespace.rs`. Phase 3 must preserve them, not re-plan
namespace ACLs.

rmcp verification: `Cargo.toml` pins `rmcp = "=1.8.0"`. Context7 for rmcp shows
`TokioChildProcess::builder(command: impl Into<CommandWrap>)`,
`TokioChildProcess::into_inner(self) -> Option<Box<dyn ChildWrapper>>`, and
`Peer::notify_cancelled(CancelledNotificationParam)`. The builder surface may be
enough for a wrapped command, but the child wrapper is not proof that a Job
Object / process group can be installed before spawn. The process phase below
therefore treats a thin custom transport in `process.rs` as an accepted fallback,
per D-009.

## Dependencies

- Phase 0/1/2 are prerequisite and appear landed in the current tree: three
  static meta-tools, config load, lazy registry, live discovery, namespace ACLs,
  byte-faithful forwarding, and reverse-traffic rejection exist.
- This plan is sequenced after Phase 2 and before Phase 4 error hardening and
  Phase 5 cross-platform CI / real remote HTTP validation.
- Test creation can proceed with all mandatory Phase 3 tests, but it must mark
  any HTTP-transport assertion conditional on Open Question #1 if the
  orchestrator pauses for that decision.
- Shared-file ordering is explicit. `Cargo.toml` is written first by Phase 1.
  `config.rs` is written by Phase 2 before `registry.rs` timeout work in Phase 3.
  `process.rs` is written by Phase 2 for spawn inputs/redaction and then by
  Phase 4 for process-tree containment; those phases are sequenced, not parallel.
- Phases that do not share write files may run in parallel after dependencies:
  after Phase 2, Phase 3 timeout/cancellation and Phase 4 process lifetime can
  be implemented independently except for final gate integration.

## Scope

### In

- Add `CredentialStore` abstraction and concrete `KeyringStore` / `EnvStore`
  implementations with resolution order: selected preferred backend, then env,
  then structured error.
- Add `--credential-store` selection for the preferred backend, preserving env as
  fallback regardless of preference.
- Implement `cred set <server> <KEY>`, `cred list <server>`, and
  `cred rm <server> <KEY>` against the selected store.
- Ensure `cred set` reads the secret through a hidden stdin prompt and exposes no
  flag or positional argv value for the secret.
- Ensure `cred list` emits credential names only, never values.
- Add spawn-time `${VAR}` interpolation for per-server `env` values and inject
  only the resolved vars for that server into the child process.
- Add the redaction layer used by tracing, child stderr log capture, and upstream
  logging/progress log lines; include the mandatory sentinel-secret test.
- Add per-server `timeout_secs` config with default 60 and wrap every upstream
  tool call in that timeout.
- Add `upstream_timeout` structured tool errors with `isError: true`, server,
  tool, message, and recoverable fields.
- Handle downstream cancellation notifications for in-flight upstream calls,
  preserving the registry lock discipline.
- Add process-tree containment for stdio upstreams: Windows Job Object with
  kill-on-close and Unix process group / `setsid` with group kill.
- Add the mandatory hard-kill orphan test for all supported OSes, using a stable
  probe process observable and checking no surviving upstream process remains
  after killing fanin-mcp.
- Add any Phase 3 dependencies actually used: keyring, hidden prompt, redaction,
  and process-tree wrapper crates.

### Out

- OAuth, browser login, refresh tokens, mid-session 401 recovery, and the future
  `auth` subcommand. D-011 defers OAuth to v1.1.
- Replacing the stdio fanin-mcp server with an HTTP listener, daemon, web
  framework, database, plugin system, or system service.
- Changing the three public meta-tool names, their static descriptions, or the
  public structured-error JSON shape beyond adding new codes.
- Reworking namespace ACLs, adding parameter-level filters, SQL parsing, path
  policy, or `readonly = true` enforcement.
- Phase 4 sanitization of upstream-provided names/descriptions, final crash
  hardening for mid-session upstream death, and `notifications/tools/list_changed`
  cache invalidation.
- Phase 5 cross-platform CI matrix wiring, `cargo audit` / `cargo deny`, token
  benchmark, binary size gates, and memory profiling, except that Phase 3 tests
  must be compatible with the later CI matrix.
- Resource and prompt proxying, capability-mirrored sampling/elicitation
  forwarding, progress forwarding, warm/cache behavior, install helpers, and hot
  config reload.
- Storing secrets in `config.toml`, accepting secrets on CLI argv, printing
  secret values, or inheriting the aggregator's full process environment into
  upstream children.
- Editing tests after `test-creator` writes them. Later stages treat tests as the
  read-only contract.

## Phases

### Phase 1 — Credential store and CLI surface

**Scope:** Build the credential abstraction, keyring/env backends, backend
selection, and real `cred` subcommands. This phase does not wire credentials into
upstream spawning yet; it creates the API Phase 2 consumes.

**Produces:** `src/credentials.rs`, `src/main.rs`, `src/error.rs`, `Cargo.toml`,
`Cargo.lock`. Test files are produced only by `test-creator`.

**Key Behaviors:**

- Define a `CredentialStore` API that can get, set, delete, and list names for a
  server-scoped service such as `fanin-mcp/<server>`.
- Implement `KeyringStore` using the `keyring` crate and `EnvStore` using the
  process environment as a read-only fallback backend.
- Keep env fallback available even when the preferred backend is keyring.
- Add `--credential-store` for the preferred backend. Supported MVP values should
  be explicit and finite, such as `keyring` and `env`.
- Implement `cred set <server> <KEY>` with hidden prompt input. Do not add any
  `--value`, positional secret, or echoing mode.
- Implement `cred list <server>` as names-only. Values must never be retrieved or
  printed for listing.
- Implement `cred rm <server> <KEY>` against the selected mutable backend.
- Route CLI diagnostics to stderr / tracing. `cred` commands run outside the MCP
  stdio server, but they still must not train later code to use stdout for serve
  diagnostics.
- Add dependency declarations only for crates used by this phase and keep rmcp at
  the exact `=1.8.0` pin.

**Depends On:** `src/credentials.rs` stub; `src/main.rs` current `CredAction`
stubs; `src/error.rs` startup/tool error pattern; `Cargo.toml` exact rmcp pin;
`docs/DECISIONS.md` D-010; `docs/GOTCHA.md` #18.

**Skills Needed:** `rust-general`, `rust-test` for test-shape awareness,
`rmcp-general` for stdout discipline, `tool-use`.

**Phase Success Criteria:**

1. `cred set <server> <KEY>` stores a value entered through a hidden stdin prompt
   and exposes no CLI argument that can contain the secret value.
2. `cred list <server>` prints or returns only credential names and never includes
   stored values.
3. `cred rm <server> <KEY>` removes a stored key, and a later lookup through the
   selected store no longer returns that value.
4. A keyring-backed round trip succeeds where the host keyring is available; a
   keyring-less/headless case can still resolve through env fallback.
5. `Cargo.toml` keeps rmcp exactly pinned and adds only the Phase 3 dependencies
   needed for credential storage and hidden prompting.

### Phase 2 — Config schema, interpolation, isolation, and redaction

**Scope:** Add Phase 3 config fields and wire secret resolution into spawn-time
stdio environment construction. Add redaction where resolved secret values can
reach tracing or log files. This phase does not add timeouts or process-tree
containment.

**Produces:** `src/config.rs`, `src/registry.rs`, `src/process.rs`,
`src/forward.rs`, `src/main.rs` only if tracing/redaction initialization must be
installed there, and supporting helpers in `src/credentials.rs` if Phase 1 did
not expose them. Tests are produced only by `test-creator`.

**Key Behaviors:**

- Add `timeout_secs` to `ServerConfig` with default 60. Phase 3 timeout handling
  consumes it in Phase 3, but parsing and validation land here to avoid another
  `config.rs` writer.
- Add interpolation support for exact placeholder values and embedded placeholders
  in configured env values, while keeping literal non-secret values possible.
- Resolve `${VAR}` at spawn time through preferred credential backend, then
  process env fallback, then a structured resolution error.
- Inject only the env vars configured for the selected server. Do not inherit the
  aggregator process environment wholesale.
- Register resolved secret values with a redaction component before any value can
  be logged.
- Redact child stderr log lines, upstream logging notifications, progress log
  lines, and tracing fields that may contain resolved values.
- Avoid `Debug` output of resolved env maps. Tests should fail if the sentinel
  secret appears in any configured log sink.
- Add `headers` parsing only if Open Question #1 resolves to landing HTTP config
  plumbing in Phase 3; otherwise leave a narrow credential interpolation helper
  that future HTTP code can reuse without accepting `transport = "http"` yet.
- Preserve Phase 2 namespace tool filters in `config.rs` and preserve stdio-only
  startup validation unless HTTP is explicitly approved.

**Depends On:** Phase 1 credential API; `src/config.rs` current config model;
`src/registry.rs` connect path; `src/process.rs` spawn env loop and log writer;
`src/forward.rs` upstream log/progress handlers; Open Question #1.

**Skills Needed:** `rust-general`, `rust-test` for side-effect assertions,
`rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. A server env value containing `${TOKEN}` resolves from keyring when available
   and from process env when the keyring backend is unavailable or lacks the key.
2. Missing credentials produce a structured tool-level error that names the
   server and variable but does not include the secret or a secret-looking value.
3. A spawned upstream receives exactly the configured env keys for that server;
   it does not receive credentials configured for sibling servers.
4. Literal non-secret env values continue to reach the selected upstream.
5. The sentinel-redaction test proves the sentinel secret is absent from tracing
   output, child stderr log output, and upstream notification log output.
6. `timeout_secs` parses with default 60 without changing existing configs.
7. If HTTP config is deferred, `transport = "http"` still fails startup as a
   later-phase transport and no unusable header path is exposed as working.

### Phase 3 — Upstream call timeouts and cancellation

**Scope:** Wrap every upstream tool call in a per-server timeout and integrate
client cancellation with in-flight upstream calls. Preserve the no-lock-across-
await registry invariant.

**Produces:** `src/registry.rs`, `src/server.rs`, `src/error.rs`, and a small
cancellation helper module only if needed, such as `src/cancellation.rs`. Tests
are produced only by `test-creator`.

**Key Behaviors:**

- Carry each server's effective `timeout_secs` from config into the call path.
- Wrap the upstream `peer().call_tool(...)` future in `tokio::time::timeout`.
- On timeout, return a structured tool result with code `upstream_timeout`, the
  server, the tool, a timeout message, and `recoverable: true`.
- Do not convert timeout into a JSON-RPC error.
- Do not hold `entries`, `init_guards`, or any registry map lock while awaiting
  connect, list, call, timeout, or cancellation operations.
- Use the downstream `RequestContext<RoleServer>` instead of ignoring it if rmcp
  exposes cancellation metadata there.
- Track in-flight calls with the smallest stable key rmcp exposes. On downstream
  cancellation, abort the local future/result path and forward a cancellation
  notification upstream when rmcp exposes the required request identity.
- If rmcp only supports `Peer::notify_cancelled` without exposing the peer call's
  request id, surface that API gap as a structural finding rather than faking a
  forwarded cancellation.

**Depends On:** Phase 2 `timeout_secs` config; `src/registry.rs` current
`call_tool`; `src/server.rs` current `RequestContext<RoleServer>` argument;
`src/error.rs` structured error helper; Context7-verified rmcp cancellation API;
Open Question #3.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. A server with `timeout_secs = 1` calling a probe tool that sleeps longer than
   one second returns `CallToolResult { isError: true }` with JSON containing
   `code: "upstream_timeout"`, the server, the tool, a message, and
   `recoverable: true`.
2. A server with no `timeout_secs` uses the default 60-second effective timeout,
   observable through config/unit coverage without waiting 60 seconds in a slow
   integration test.
3. A successful fast upstream call still passes through byte-faithfully and is
   not wrapped as an error.
4. A cancelled downstream request aborts or detaches the local in-flight call so
   fanin-mcp does not wait for the full upstream duration before freeing its own
   resources.
5. Where rmcp exposes the needed request identity, fanin-mcp sends an upstream
   cancellation notification for the in-flight call.
6. Concurrency coverage still proves a slow timed or cancelled call on one
   upstream does not block a sibling upstream.

### Phase 4 — Process-tree lifetime

**Scope:** Replace direct uncontained child spawning with a platform-contained
stdio child transport. Windows uses a Job Object with kill-on-close; Unix uses a
fresh process group and group kill. This phase owns `process.rs` after Phase 2
has landed env/redaction changes.

**Produces:** `src/process.rs`, `Cargo.toml`, `Cargo.lock`, and platform-specific
support modules only if needed, such as `src/process/windows.rs` and
`src/process/unix.rs`. Tests are produced only by `test-creator`.

**Key Behaviors:**

- Evaluate `process-wrap` first against rmcp `=1.8.0` and the existing
  `TokioChildProcess::builder(CommandWrap)` surface.
- Use a crate wrapper only if it can install Job Object / process-group behavior
  before spawn while preserving stdin/stdout pipes and stderr capture.
- If the rmcp child transport cannot be wrapped safely, implement a thin custom
  child transport isolated in `process.rs` rather than weakening D-009.
- On Windows, ensure the wrapper covers the full tree, including `cmd /c npx`
  and descendant `node.exe` processes, with kill-on-close behavior that survives
  aggregator hard-kill.
- On Unix, start each child in a fresh process group/session and kill the group
  on teardown.
- Preserve stderr capture, redaction, and `[server]` log prefixing from Phase 2.
- Preserve least-privilege env injection and do not reintroduce inherited full
  env.
- Add `cwd` support only if the implementer confirms it is a contained
  spawn-config addition needed for D-019 and it does not expand Phase 3 beyond
  process spawn behavior.

**Depends On:** Phase 2 spawn input/redaction changes; `src/process.rs` current
`TokioChildProcess` use; `Cargo.toml` dependency policy; `docs/DECISIONS.md`
D-009; `docs/GOTCHA.md` #11/#14/#30; Open Question #2.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. The hard-kill orphan test starts fanin-mcp with an upstream that spawns a
   descendant, kills fanin-mcp forcefully, and observes zero surviving upstream
   descendants after the allowed cleanup interval.
2. On Windows, the hard-kill test specifically catches the `cmd /c npx` shape or
   an equivalent descendant process and leaves no orphaned `node.exe` / child
   process behind.
3. On Unix, an upstream that forks or spawns a child in the group is killed when
   the fanin-mcp process is force-killed.
4. Normal stdin EOF session teardown also terminates the full upstream tree.
5. Stderr capture to the configured log file still works after process wrapping,
   with `[server]` prefixing and redaction intact.
6. The process wrapper does not change the downstream MCP stdout transport and
   does not leak child stderr into fanin-mcp stderr unless an explicit debug-only
   passthrough option is later added outside this plan.

### Phase 5 — Phase 3 gate and scope cleanup

**Scope:** Run the objective gate for Phase 3, repair only in-scope defects, and
reject leakage into Phase 4/5/v1.1 work. This is the integration and audit phase
for the plan.

**Produces:** Only files already owned by Phases 1-4 if a gate failure requires a
scoped fix: `src/credentials.rs`, `src/main.rs`, `src/config.rs`,
`src/registry.rs`, `src/process.rs`, `src/forward.rs`, `src/error.rs`,
`Cargo.toml`, `Cargo.lock`. No test edits by implementer, simplifier,
debugger, or reviewer.

**Key Behaviors:**

- All tests written by `test-creator` and pre-existing tests pass at 100%.
- Existing Phase 0/1/2 guarantees remain true: exactly three static meta-tools,
  lazy startup, namespace filtering, byte-faithful forwarding, reverse-traffic
  rejection, and no stdout diagnostics.
- Security audit confirms no argv secret path, no secret in logs, no full-env
  child inheritance, and no public shape drift for structured errors.
- Process audit confirms hard-kill containment works on the current OS and the
  implementation has platform-specific paths for all supported OSes.
- Scope audit confirms OAuth, HTTP remote transport if deferred, sanitization,
  CI matrix wiring, benchmark work, and parameter ACLs did not leak in.

**Depends On:** Phases 1-4; all Phase 3 tests; current rmcp pin; project binding
rules in `CLAUDE.md` #5/#6.

**Skills Needed:** `rust-general`, `rust-test`, `rmcp-general`, `tool-use`.

**Phase Success Criteria:**

1. The full required Rust test command and formatting gate pass at 100%.
2. No later stage modifies test files after `test-creator` has written them.
3. Existing public meta-tool names and static descriptions remain unchanged.
4. Existing namespace and byte-faithful forwarding tests still pass.
5. No plan-out scope item is implemented accidentally.

## Success Criteria

1. `src/credentials.rs` defines and uses a server-scoped credential abstraction
   with keyring and env backends.
2. Credential resolution order is preferred backend, then process env fallback,
   then a structured missing-credential error.
3. `cred set <server> <KEY>` stores a hidden-prompt value and exposes no CLI arg
   or flag capable of carrying the secret value.
4. `cred list <server>` returns credential names only and never returns stored
   values.
5. `cred rm <server> <KEY>` removes the key so a later lookup cannot resolve it
   from the selected mutable backend.
6. A keyring-backed credential round trip through `cred set` succeeds on hosts
   with an available keyring.
7. Env fallback works in a keyring-less/headless case without requiring a keyring
   service.
8. `${VAR}` interpolation in a server's configured `env` resolves at spawn time
   and supports both keyring-sourced and env-sourced values.
9. Each spawned upstream receives only its own configured env vars; sibling
   credentials and the aggregator's full environment are not inherited.
10. Literal non-secret env values still reach the selected upstream unchanged.
11. The mandatory sentinel-redaction test proves the sentinel secret never
    appears in tracing output, child stderr logs, or upstream notification logs.
12. `timeout_secs` parses per server and defaults to 60 when omitted.
13. Every upstream tool call is wrapped in the effective per-server timeout.
14. A timed-out upstream call returns `CallToolResult { isError: true }` with
    JSON containing `code: "upstream_timeout"`, server, tool, message, and
    recoverable fields.
15. Timeout failures are not returned as JSON-RPC errors.
16. Downstream cancellation of an in-flight call frees fanin-mcp's local call
    resources without waiting for the upstream's full duration.
17. When the rmcp `=1.8.0` API exposes the needed request identity, downstream
    cancellation sends a cancellation notification to the upstream peer.
18. Registry locks are never held across upstream spawn, initialize, list, call,
    timeout, or cancellation awaits.
19. Windows process spawning places every upstream tree in a Job Object that
    kills descendants on fanin-mcp hard-kill.
20. Unix process spawning places every upstream tree in a fresh process group or
    session and kills descendants on fanin-mcp hard-kill.
21. The mandatory hard-kill orphan test kills fanin-mcp forcefully and observes
    zero surviving upstream descendants; on Windows it catches the no-orphaned
    `node.exe` case or an equivalent descendant-process probe.
22. Normal stdin EOF teardown terminates the full upstream tree.
23. Child stderr capture still writes `[server]`-prefixed redacted lines to the
    configured log file after process wrapping.
24. The public downstream MCP surface remains exactly three meta-tools:
    `list_tools`, `get_tool_schema`, and `invoke_tool`.
25. All required gates pass at 100%; failures are fixed in scope or surfaced for
    routing, never thresholded.

## Constraints / Invariants

- Secrets never appear on argv, in shell history, in config as resolved values,
  or in logs. `cred set` reads a hidden prompt only.
- `cred list` prints or returns names only. It never retrieves or displays values.
- Env fallback is a fallback resolver, not permission to inherit the aggregator's
  full environment into every upstream.
- Each upstream receives only its own resolved vars and literal env entries.
- stdout is the MCP transport. No `println!`, `print!`, `dbg!`, inherited child
  stdout/stderr diagnostics, or stdout logging on the serve path.
- Tool-level failures return `CallToolResult { isError: true }` with structured
  JSON content. Timeout and credential-resolution failures are not JSON-RPC
  errors.
- The public structured-error JSON shape remains compatible with D-005; adding
  `upstream_timeout` is allowed, removing or renaming fields is not.
- Never hold the registry map lock across upstream spawn, initialize,
  `list_all_tools`, `call_tool`, timeout, cancellation forwarding, or teardown.
- Process-tree lifetime is non-negotiable: Windows Job Object / Unix process
  group containment must survive hard-kill of fanin-mcp.
- rmcp remains exactly pinned unless a separate explicit task authorizes a pin
  bump. Verify signatures against `=1.8.0`, not memory or pseudocode.
- Tests are a read-only contract after `test-creator` writes them. No later stage
  edits test files.
- This is the v0.4.x checkpoint series. Do not change the public meta-tool names,
  static meta-tool descriptions, namespace semantics, or byte-faithful result
  path.
- No OAuth, HTTP listener, daemon, database, plugin loader, Node runtime
  dependency, Docker dependency, or system service enters this phase.

## Open Questions

1. **Should Phase 3 introduce HTTP transport and `headers` injection now, or
   defer HTTP wiring?** MVP Phase 3 names static header injection for HTTP
   upstreams, but the current tree and `src/config.rs` validation are stdio-only,
   and `docs/MVP.md` Phase 5 is where one real remote HTTP upstream is verified.
   D-011 says static headers are MVP, but it does not require the full HTTP
   transport to land in this exact checkpoint. Proposed default: defer live HTTP
   transport to Phase 5, keep `transport = "http"` rejected in Phase 3, and land
   reusable credential interpolation/redaction plumbing so a later HTTP branch can
   resolve `headers = { Authorization = "Bearer ${TOKEN}" }` without redesign.
   If the orchestrator chooses to land HTTP now, add `url`, `headers`, and the
   rmcp `transport-streamable-http` feature in a separate sequenced subphase so
   `config.rs` and `registry.rs` ownership is explicit.

2. **Can `process-wrap` or `command-group` wrap rmcp's `TokioChildProcess`, or is
   a custom child transport required?** Context7 confirms rmcp `TokioChildProcess`
   has a `builder(CommandWrap)` surface and `into_inner`, but that alone does not
   prove a Job Object / process group can be installed before spawn while keeping
   rmcp's stdin/stdout transport intact. Proposed default: evaluate
   `process-wrap` first because D-009 prefers it; accept it only if the wrapper is
   applied before spawn and the hard-kill test passes. If it cannot wrap rmcp's
   child safely, implement the thin custom child transport in `process.rs` rather
   than weakening process-tree containment.

3. **How exactly does downstream `notifications/cancelled` map to an upstream
   `call_tool` in rmcp `=1.8.0`?** The current `src/server.rs` ignores
   `RequestContext<RoleServer>`, and Context7 confirms `Peer::notify_cancelled`
   exists, but the plan still needs the pinned API's stable request identity or
   cancellation hook to correlate a downstream cancellation with the hidden
   upstream `peer().call_tool(...)` request. Proposed default: use the request id
   or cancellation token exposed by `RequestContext` to track an in-flight call,
   abort the local future on downstream cancellation, and forward
   `notify_cancelled` upstream when rmcp exposes the upstream request identity. If
   rmcp hides that identity for typed `peer().call_tool`, surface a structural
   finding instead of pretending cancellation was forwarded.
