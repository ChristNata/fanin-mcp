# Review: remediation-s1-d1 — General Lens

**Scope reviewed:** only the remediation-s1-d1 change set (`git diff HEAD~1 -- src/`).
Suite green (129 passed / 0 failed / 4 ignored — `cargo test --all` was re-run
locally to confirm). Clippy `-D warnings` and `cargo fmt --check` are clean.

## Idiom & clarity

The `resolved_cwd` plumbing through `get_or_connect` → `connect` →
`spawn_stdio_transport` is **clean and correctly lifetime-anchored**.
`resolved_cwd: String` lives in `get_or_connect`, is passed by `as_deref()` →
`Option<&str>` → `connect`, then forwarded as the same `Option<&str>` into
`spawn_stdio_transport`. No needless clones, no `.to_string()` on the hot
path, no double resolution. `Option<&str>` is the right type for the
intermediate layer; it documents that the value is already-resolved and
borrowed. (src/registry.rs:131-156, 394-426; src/process.rs:242-253)

The stdio-only branch (registry.rs:131-148) reads as a clean three-way
discriminate: HTTP ⇒ `None`; stdio + `None` ⇒ `None`; stdio + `Some(raw)` ⇒
resolve + trim-check + `Some(resolved)`. No fallthrough.

`StartupError::EmptyCwd { server }` (src/error.rs:172, 214) is named
consistently with its neighbors (`StdioServerMissingCommand`,
`HttpServerMissingEndpoint`), and the Display text `server \`{server}\` has
empty or whitespace-only \`cwd\`` carries the operator context
(named server) without echoing the offending value — correct, since the value
is empty anyway. No redaction concern.

`spawn_stdio_transport`'s new `resolved_cwd: Option<&str>` parameter
(src/process.rs:246, 251-253) is the only API change to that function, sits
in the right place (right after `args`), and the doc on the function
(D-009 containment paragraph above the signature) was already up to date —
no stale prose around the new parameter.

No `unwrap`/`expect`/`panic!` introduced. No dead `#[allow]` introduced.
No stale TODOs or commented-out code.

## Duplication

The timeout-and-map-to-`upstream_timeout` shape now appears in three
call-sites: `get_or_connect` (registry.rs:158-172), `ensure_fresh`
(registry.rs:320-340), and the pre-existing `call_tool` (registry.rs:228-269).

The shared skeleton is `timeout(effective, future).await` with three
match arms (`Ok(Ok)` / `Ok(Err)` / `Err(_elapsed)`). The variation per
site is real and load-bearing:

- `get_or_connect` cannot log a `tool` (no tool yet), must not insert into
  `entries` on the error path, and on `Err` returns `UpstreamTimeout` without
  any pre-existing tool data.
- `ensure_fresh` must restore `dirty = true` on both `Err` and `Ok(Err)` paths,
  has no tool to log, and must not overwrite the cached inventory on failure.
- `call_tool` logs `latency_ms`, names the `tool`, distinguishes
  `UpstreamDisconnected` from `UpstreamCall` via `map_service_error`, and
  records the outcome via `log_tool_call`.

A helper that swallowed those differences would either grow three flags
(`on_err: fn`, `restore_dirty: bool`, `is_connect: bool`, ...) or hide them
behind trait objects — either way it would obscure the per-site invariants
that *are* the contract. Verdict: **leave the duplication.** Three short
match arms that each carry their own load-bearing invariant read more clearly
than a `map_timeout` closure that needs the reader to thread the closure
arguments back to the call-site to understand what happens on expiry. The
project's "minimal, no premature abstraction" posture (DECISIONS D-007
notes; AGG-MCP.md consistently prefers explicit over generic) applies.

No action recommended.

## Observability consistency

The new timeout sites use `event = "upstream_failure", code = "timeout"`
(registry.rs:161-166, 257-263, 329-335). The pre-existing connect-failure
sites use `event = "upstream_connect_failure"` (registry.rs:428, 451, 463).
`call_tool`'s timeout reuses the new `upstream_failure` shape (registry.rs:260).

**Finding (targeted).** The naming is internally inconsistent:
- `upstream_connect_failure` — concrete, says *what* failed.
- `upstream_failure` — generic; the disambiguator is the secondary
  `code = "timeout"` field.
- `upstream_disconnect` — concrete, says *what* happened.

Two reasonable directions, pick one:

1. **Match the existing concrete-event convention.** Rename the new sites to
   `event = "upstream_timeout"` (matches the wire code) and drop the
   redundant `code = "timeout"`. Three call-sites, mechanical change.
   Best fit because it makes the wire `code` and the log `event` the same
   string — easy to grep across both surfaces.

2. **Keep the generic event + code split** (current state) and rename the
   pre-existing `upstream_connect_failure` for symmetry. Worse option —
   bigger blast radius, no clarity gain.

Recommend option 1; the `code = "timeout"` field becomes redundant since
the event name already encodes it.

**Finding (trivial).** The new connect-timeout log (registry.rs:161-166) and
refetch-timeout log (registry.rs:329-335) do not emit a `tool` field.
`call_tool`'s timeout (registry.rs:260) does emit `tool`. Conditional
absence of a stable field is a minor parser hazard for downstream log
consumers building a JSON schema (the field is `Option<...>` in the
implicit contract). Not worth fixing in this change set — flag for the
audit-trail hardening pass.

**Finding (trivial).** The pre-existing `event = "upstream_connect_start"`
log at registry.rs:402-406 lacks a matching `event = "upstream_connect_end"`
or `upstream_connect_success` (which exists at 469-472). Not a defect in
this change set — pre-existing — but the gap is visible from the diff.

## Error context

**Finding (targeted).** `ToolError::UpstreamTimeout { server, tool: "" }`
(empty string) is rendered (src/error.rs:115-120) as:

    upstream call to `` on `{server}` exceeded timeout

The empty backticked span is visible to the LLM reading the structured
error message. The wire JSON is `{"tool": ""}` — empty string is not the
same as `null`/`None`, and downstream consumers expecting a tool name
will see an empty-string-tool-case rather than a missing-tool case.
Three of the new timeout sites (registry.rs:169, 264, 335) pass
`tool: String::new()` for this exact reason.

**Suggested fix.** Two clean options:

1. Change `ToolError::UpstreamTimeout`'s `tool: String` field to
   `tool: Option<String>` and render with `tool.unwrap_or("discovery")` or
   similar. Connect/discovery/refetch pass `None`; the call-tool path passes
   `Some(tool.to_string())`. This is a public-error-shape change, but the
   field is already optional in the wire JSON (`"tool": ""` vs absent) and
   the plan explicitly authorizes "internal operation label" handling.
2. Cheaper: branch the message — if `tool.is_empty()`, render
   `"upstream {phase} for \`{server}\` exceeded timeout"` where `phase` is
   `connect`/`discovery`/`call`. Requires carrying the phase to the error
   variant or threading a label string.

Recommend option 1; it is the same wire shape the connect-timeout
implementation is already trying to express (no tool applies), and it
fixes both the JSON `""` payload and the empty-backticks message.

**Finding (trivial).** The new `ToolError::UpstreamConnect { server,
message: "resolved cwd is empty or whitespace-only" }` at registry.rs:137-140
correctly omits the offending value and includes the server name. Operator
context is sufficient; secret-leak risk is zero (the value is empty by
construction). No change needed.

**Finding (trivial).** No tracing call in the change set ever names `cwd`
or `resolved_cwd` as a field, so the `${VAR}`-resolved value is never
written to logs. The redaction layer is therefore not on the critical path
here, but the value *is* indirectly registered via `resolve_env_value`'s
internal `register_secret` call (process.rs:121), so any future log that
names the cwd will be auto-redacted. Defensive-correct.

## Config doc

`ServerConfig::cwd` (src/config.rs:110-116) is documented as:

> Optional stdio child working directory.
> When present for stdio upstreams this is resolved at connect time using
> the same `${VAR}` credential/env path as env and headers, then applied
> with `Command::current_dir`. When absent, the child inherits fanin-mcp's
> process working directory. Streamable-HTTP accepts but ignores this field.

This matches ARCHITECTURE.md:97 ("optional per-server `cwd` field sets the
spawned child's working directory (supports `${VAR}` interpolation).
Defaults to the aggregator's own CWD. … Ignored for HTTP upstreams.")
closely. A struct-level reader gets the stdio/HTTP split and the
${VAR} resolution; what is **not** stated:

- empty/whitespace-after-resolution is rejected at spawn (currently lives
  only in the implementation and the plan);
- the HTTP "ignored" includes "still validated for empty value at config
  load" (also in implementation, not in the doc).

Recommend a one-line addition: "Empty or whitespace-only values (literal
or post-resolution) are rejected before MCP serving / before spawn."

**Finding (targeted).** The schema-level doc comment at src/config.rs:17
(`//! cwd = '<path>' # optional; stdio child working directory; may contain
${VAR}`) correctly says `${VAR}` is interpolable. Good.

## Dead code / leftovers

None.
- No unused imports introduced.
- No stale `#[allow]` introduced (the pre-existing `#[allow(dead_code)]`
  on `ServerConfig` and `NamespaceConfig` are unchanged).
- No debug `println!` or `dbg!` introduced (GOTCHA #1).
- No leftover commented-out code.
- The diff touches only the four files named in the task; no spillover.

## Summary

| Area | Verdict |
|---|---|
| Idiom & clarity | Clean. |
| Duplication | Acceptable — leave the three match arms. |
| Observability consistency | 1 targeted (event naming) + 2 trivial (optional `tool` field, missing `connect_end`). |
| Error context | 1 targeted (`tool: ""` rendering) + 2 trivial (no leak risk, defensive-correct). |
| Config doc | 1 targeted (cwdoc gap on empty/whitespace-after-resolution). |
| Dead code | None. |

No blockers. No structurals. Three targeteds, four trivials — each is
contained, fixable without re-planning, and would be bounced by a sharp
PR reviewer but not stop the pipeline.

**Lens verdict: PASS-with-issues**