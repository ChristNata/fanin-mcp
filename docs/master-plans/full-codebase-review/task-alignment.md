FULL-CODEBASE ALIGNMENT REVIEW — fanin-mcp @ v0.6.15 (HEAD 6d5b66c)

You are the reviewer. This is a standalone, whole-codebase ALIGNMENT review.
Your single question for every piece of code: does it match the binding design
canon? The docs in this repo are not aspirational — they record decisions
already made. Code that diverges from a locked ADR or a ✅ GOTCHA is spec drift
to surface, not a style preference.

## Scope — read all of it

- Binding canon FIRST, then the code:
  - docs/DECISIONS.md — ADRs D-001..D-019. Each is a contract. For every ADR,
    find the code that implements it and confirm it actually does, or flag the
    gap. Pay special attention to the non-obvious ones (lock discipline D-007,
    bidirectional D-008, process lifetime D-009, secrets D-010, structured
    errors D-005, byte-faithful D-004).
  - docs/GOTCHA.md — every item marked ✅ claims to be enforced by design/tests.
    VERIFY each ✅ against the code and the test suite. A ✅ that the code does
    not actually enforce is a real finding.
  - docs/MVP.md — the phased plan + verification checklist. Does the shipped
    code match what the checklist says shipped? Flag checklist items that are
    signed off but not actually done, or done but not signed off.
  - docs/PRD.md, docs/ARCHITECTURE.md, docs/AGG-MCP.md (AGG-MCP is pseudocode
    until verified against the rmcp pin — check the real code matches intent,
    not the literal snippet).
  - Root SECURITY.md, STACK.md, ROADMAP.md — scope, threat model, stack
    rationale. Does the code honor the "anti-stack" (no web framework, no HTTP
    server, no DB/ORM, no plugin loader, no Node/Docker at runtime)? Does it
    keep the single-static-binary / no-runtime-deps promise?
- All source: src/*.rs. Tests: tests/.

## Known drift to confirm or refute (from the last cycle's knowledge-sync)

The Phase-5 knowledge-sync claims it reconciled these — verify the code and
docs actually agree now, or flag residual drift:
- AggError -> ToolError mapping (DRIFT-1) — error.rs vs ARCHITECTURE.md.
- macOS hard-kill limitation — documented in SECURITY.md / GOTCHA #11/#14;
  does process.rs match the documented behavior exactly (graceful group
  teardown only, SIGKILL-orphan gap), no more no less?
- OQ3 http-transport feature name — rmcp `transport-streamable-http-client`
  (the -client variant) in Cargo.toml vs what rmcp-general / STACK.md say.
- Two stale-doc flags the last cycle surfaced for the user: MVP checklist
  sign-off, ROADMAP v1.0 status. Confirm whether these are still stale.

## Output

Write your artifact to:
  docs/master-plans/full-codebase-review/review-alignment-<your-model-tag>.md

Structure: a table or list mapping each ADR (D-001..D-019) and each ✅ GOTCHA to
a verdict — HONORED (with file:line), DRIFTED (with the divergence), or
UNVERIFIABLE (and why). Then a section for doc-vs-code drift and stale docs,
each with severity tier (blocker | structural | targeted | trivial) and
file:line. End with a verdict paragraph: is the codebase aligned with its canon,
and the highest-priority drift to resolve.

Report ONLY what you can evidence. "D-0NN honored, enforced at file:line" is the
goal; do not invent drift to seem rigorous. Your returned result is data for the
orchestrator, not a human-facing chat message.
