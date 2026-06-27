## LENS: GENERAL — write review-general.md

Code quality and production polish on the change set only:

- **Idiom & clarity.** Naming, function length, the `resolved_cwd` plumbing
  through `connect`/`spawn_stdio_transport`, error messages (e.g. the
  `StartupError::EmptyCwd` Display text), comment quality. Anything a sharp PR
  reviewer bounces.
- **Duplication.** The timeout-and-map-to-`upstream_timeout` logic now appears in
  multiple places (cold connect + ensure_fresh + the existing call_tool). Is there
  duplicated logic that should be factored into one helper? Weigh factoring vs.
  the project's "minimal, no premature abstraction" posture — recommend only if it
  genuinely reads better.
- **Observability consistency.** Check the tracing taxonomy: the new timeout paths
  emit `event = "upstream_failure", code = "timeout"`, while existing connect
  failures use `event = "upstream_connect_failure"`. Is the event/key naming
  consistent with the established convention in this file and with STACK.md /
  SECURITY.md's described audit trail? Flag any inconsistency.
- **Error context.** Do the new error variants/messages carry enough operator
  context (which server, which path) without leaking secrets (a resolved cwd could
  contain a `${VAR}`-injected value — is it safe to log)?
- **Config doc.** Is `ServerConfig::cwd` documented in-code to match
  ARCHITECTURE.md:97? Will a user reading the struct understand the semantics?
- **Dead code / leftovers.** Any unused import, stale comment, `#[allow]`, or
  debug leftover introduced by the change.
