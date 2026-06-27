# Review: remediation-s1-d1 (synthesis)

**Verdict: PASS-with-issues.** 0 blocker, 0 structural, 3 targeted, 4 trivial.
Three lenses, all independent of the implementer (gpt-5.5): adversarial
(grok-4.3), alignment (grok-4.3), general (minimax-m3). Full suite green
(134/0/4), fmt + clippy `-D warnings` clean. Orchestrator cross-checked the
load-bearing claims in-source.

## Lens verdicts

- **Alignment (grok-4.3): PASS, 0 findings.** D-004/005/007/009/012 and
  GOTCHA #16/#30/#11/#14 all HONORED with file:line; scope confined to S-1+D-1;
  no rmcp bump / new dep. S-1 genuinely closed (every blocking upstream await now
  inside the effective timeout); cwd matches ARCHITECTURE.md:97 spec.
- **Adversarial (grok-4.3): PASS, 0 findings.** Timeout mechanism is general (no
  probe/server-name special-casing — not gamed); ContainmentGuard drops on every
  timeout path (no orphan); no new lock-across-await; cwd interpolation can't leak
  a secret or bypass the trim rejection; no new panic vectors; public error shape
  unchanged.
- **General (minimax-m3): PASS-with-issues**, 3 targeted + 4 trivial (below).

## Findings + routing

### Targeted (route → debugger, this cycle)

- **G-1 — observability event naming inconsistent.** New timeout sites log
  `event="upstream_failure", code="timeout"`; existing failures use concrete
  `event="upstream_connect_failure"` / `upstream_disconnect`. `registry.rs`
  ~161-166, 257-263, 329-335. Fix: rename the timeout sites to
  `event="upstream_timeout"` (matches the wire code), drop the redundant
  `code="timeout"`. No test asserts log event names — debugger-safe.
- **G-2 — `UpstreamTimeout { tool: "" }` renders empty backticks / wire `"tool":""`.**
  The implementer used `tool: String::new()` for connect/discovery/refetch
  timeouts; Display becomes "upstream call to `` on `{server}` exceeded timeout"
  (LLM-visible) and the wire JSON carries `"tool":""`. **The plan (master.md S-1
  decision 3) explicitly authorized `tool: null`** here — so this is the impl
  drifting from its own plan. Fix: change `ToolError::UpstreamTimeout.tool` to
  `Option<String>`; connect/discovery/refetch pass `None` (renders a phase-aware
  message, wire `null`), call_tool passes `Some(tool)` (wire form unchanged →
  existing `timeout_cancellation` tests still pass; no test asserts the new
  paths' tool field). error.rs Display + the 3 new construction sites.
- **G-3 — `cwd` doc-comment gap.** `ServerConfig::cwd` (config.rs ~110) documents
  the stdio/HTTP split + `${VAR}` but not that empty/whitespace-after-resolution
  is rejected before spawn. Fix: one-line doc addition.

### Trivial (NOT fixed this cycle — pre-existing or deferred; listed for the record)

- T-1: connect/refetch timeout logs omit a `tool` field that `call_tool`'s
  timeout emits — minor downstream-log-schema hazard. minimax recommends
  deferring to a dedicated audit-trail hardening pass. Out of scope here.
- T-2: pre-existing `event="upstream_connect_start"` has no matching
  `upstream_connect_end` (a `_success` exists). Pre-existing, not this change set.
- T-3 / T-4: `UpstreamConnect{message:"resolved cwd is empty…"}` and the cwd
  redaction path were both checked and confirmed leak-free / defensive-correct —
  no action.

## Note for knowledge-sync (end of cycle)

DECISIONS / GOTCHA / ARCHITECTURE / SECURITY should record that S-1 is closed
(timeout envelope now covers connect + discovery + refetch) and D-1 (`cwd`)
shipped. No doc edit in the review/fix stage; folded into knowledge-sync.

## Disagreements between lenses

None. The two PASS lenses found nothing the general lens contradicts; the general
lens's findings are all quality/polish the other two did not scope.
