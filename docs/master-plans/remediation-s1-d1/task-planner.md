PLAN A REMEDIATION CYCLE — fanin-mcp, two findings from the full-codebase review.

You are the planner. Produce `master.md` (and only `master.md`) in the plan
workspace `docs/master-plans/remediation-s1-d1/`, following the `plan-format`
spec exactly. Tier is **thorough**, scope is **flat**, stack **rust**. Do NOT
write `state.json` (the orchestrator owns it) and do NOT write any code.

## Context to read first (binding)

- `docs/master-plans/full-codebase-review/review-SYNTHESIS.md` — the review this
  remediates. Findings S-1 and D-1 are the entire scope of this plan.
- `docs/DECISIONS.md` — esp. D-005 (errors are `CallToolResult{isError:true}`;
  the JSON error shape is PUBLIC API), D-007 (lock discipline), D-009 (process
  containment), D-012 (per-server timeout + cancellation), D-019 (Morph as a
  directory-scoped stdio upstream).
- `docs/GOTCHA.md` — #16 (lock never across await), #30 (cwd / Morph wrong-tree).
- `docs/PRD.md:41` (Req 5 lists `cwd`), `docs/ARCHITECTURE.md:70,97` (cwd field
  spec: `${VAR}` interpolation, default = aggregator CWD, ignored for HTTP).
- Source: `src/registry.rs` (get_or_connect, connect, ensure_fresh, call_tool,
  effective_timeout), `src/config.rs` (ServerConfig + any ConfigBuilder/to_toml
  writer), `src/process.rs` (spawn_stdio_transport, resolve_env_value,
  ContainmentGuard), `src/error.rs` (ToolError variants + codes).
- The `rmcp-general` skill for the exact rmcp =1.8.0 client API.

## Finding S-1 — bound the upstream connect/discovery/refetch awaits (THE blocker)

Today `timeout_secs` wraps ONLY `call_tool` (`registry.rs:190`). These awaits are
unbounded: the connect `handler.serve(...)` initialize handshake
(`registry.rs:378`/`387`), the initial `list_all_tools` (`registry.rs:399`), and
the `list_changed` dirty-refetch in `ensure_fresh` (`registry.rs:281`). The
per-server init guard (`registry.rs:96`) is held across `connect`
(`registry.rs:132`), so a hung upstream queues every later call to that server.
This violates D-012 and the PRD/SECURITY "fail informatively first, free
resources" promise.

The plan must specify wrapping each of these awaits in the server's effective
timeout. The LOAD-BEARING design decisions you must resolve in the plan (state
the decision; if any changes public API, surface it as an Open Question, do not
silently choose):

1. **Resource cleanup on connect timeout.** When the serve-handshake or initial
   `list_all_tools` times out, the spawned child process must NOT leak. Spell out
   that the `ContainmentGuard` (the OS job/process-group kill handle) is dropped
   on the timeout/cancel path so the child is reaped — i.e. timing out must kill
   the half-connected upstream, not orphan it (ties to D-009). Name exactly where
   the guard lives across the connect await and how cancellation drops it.
2. **Init-guard release + no poisoned cache.** On timeout the entry must NOT be
   inserted into the map, the init guard must release (so a later call retries
   cleanly), and the error returned is the structured shape — never a hang and
   never a half-inserted entry.
3. **Error code / public shape.** Decide: reuse `ToolError::UpstreamTimeout`, or
   reuse `ToolError::UpstreamConnect` with a timeout message, or add a distinct
   connect-timeout code. D-005 makes the `code` string PUBLIC API — adding/renaming
   one is a deliberate public-API change to flag in Open Questions. Recommend the
   least-surprising option and justify it.
4. **Lock discipline preserved (D-007/GOTCHA #16).** The timeout wrapper must not
   introduce a registry map-lock held across an await. Confirm the envelope sits
   where only the init guard (already held for cold start) and the cloned Arc are
   in scope.
5. **Dirty-refetch on timeout.** `ensure_fresh` already re-sets the dirty flag on
   error; confirm a refetch timeout keeps that behavior (re-mark dirty, return
   structured error, serve no stale-yet-also-no-hang).

## Finding D-1 — implement the documented per-server `cwd` field

`ServerConfig` has no `cwd`; nothing calls `current_dir`. ARCHITECTURE.md:97 and
PRD Req 5 specify it. The plan must specify:

- Add `cwd: Option<String>` to `ServerConfig` (`config.rs`), serde-optional,
  documented.
- Interpolate `${VAR}` in the value through the SAME resolver as env/headers
  (`process::resolve_env_value`) at connect/spawn time — not a second code path.
- Apply via `cmd.current_dir(resolved)` in `spawn_stdio_transport`
  (`process.rs`); default (None) = inherit the aggregator's CWD (current
  behavior, no regression).
- **Ignored for streamable-HTTP** upstreams (ARCHITECTURE.md:97) — specify where
  that is enforced/validated.
- Validation: reject an empty/whitespace cwd at config load or resolve; decide
  whether a non-existent dir fails at spawn (let the OS error surface as
  `UpstreamConnect`) — state the choice.
- If `config.rs` has a `ConfigBuilder`/`to_toml` writer (the review mentioned
  one), render `cwd` there too so round-trips don't drop it.

## Phasing guidance (you decide, but mind this)

S-1 touches `registry.rs` (connect/ensure_fresh) and possibly `error.rs`. D-1
touches `config.rs` + `process.rs` + the `connect` site in `registry.rs`. The two
phases therefore OVERLAP on `registry.rs::connect` — do NOT mark them
file-disjoint/parallelizable; sequence them (S-1 first, since it restructures the
connect await envelope that D-1's cwd-interpolation plugs into). Keep each phase's
Produces list explicit.

## Constraints / invariants (put these in master.md)

- Tests are a read-only contract written only by test-creator; the implementer
  codes against them. 100% pass, no thresholds.
- Scope OUT (binding): do NOT touch any other review finding (O-1/O-2/O-3, the
  H-* hygiene items, glm). This cycle is S-1 + D-1 ONLY. List them in Scope-out.
- Preserve D-004 byte-faithful, D-005 public error shape (unless deliberately
  extended per S-1 decision 3), D-007 lock discipline, D-009 containment.
- No new runtime dependency (the no-runtime-deps product promise).

## Success criteria

Write a numbered, observable Success Criteria list where each maps to a phase and
each is something test-creator can turn into one assertion — including, at
minimum: a hung-during-initialize upstream makes the first `list_tools`/
`invoke_tool` return a structured timeout within ~the configured bound (not hang);
a hang during a `list_changed` refetch likewise times out; a connect timeout
leaves zero surviving child processes (containment, verified DURING the window,
not by a post-run count); a configured `cwd` is the child's actual working
directory; an unset `cwd` preserves today's behavior; `cwd` is ignored for HTTP.

Surface every unresolved design decision in an Open Questions section with a
recommended default. Your returned result is data for the orchestrator: name the
phases, the key decisions you made, and any blocking drift. Not a human-facing
chat message.
