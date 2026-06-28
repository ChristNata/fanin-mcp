# Review: oss-readiness (synthesis)

**Verdict: PASS-with-issues.** 0 blocker, 1 structural, 1 targeted, 7 trivial.
Three lenses: adversarial (minimax-m3), alignment (grok-4.3), general
(minimax-m3) — adversarial + general independent of the implementer; alignment
objective doc-check. Gate green (135/0/4, fmt+clippy clean, release build clean).
Orchestrator cross-checked the load-bearing claims in source.

## Lens verdicts
- **Alignment (grok-4.3): PASS, 0 findings.** O-1 metadata well-formed,
  O-2 GitHub-Advisories, D-2 struck from all docs, D-005/009/010/015 + anti-stack
  preserved, scope confined.
- **Adversarial (minimax-m3): PASS-with-issues** — 1 structural (A2), 1 targeted
  (A1), 3 trivial (A3/A4/A5).
- **General (minimax-m3): PASS-with-issues** — 1 targeted (T1=A4 dup row), trivials
  (T2/T3/T5/T6); H-1/H-3/H-7/O-*/H-8 confirmed clean.

## Findings + routing

### STRUCTURAL — must resolve before close
- **A2 — the H-3 test does not bite its contract.**
  `tests/integration/literal_header_redaction.rs` asserts only
  `!logs.contains(&secret)` — the `[REDACTED]`-must-appear assertion was DROPPED
  to make it green. But NO production path logs a resolved header value (H-3 is
  defense-in-depth; deepseek F5 said so), so the negative assertion passes
  trivially — the test would still pass with `registry.rs:126`
  `register_secret` deleted. → **test-creator.** Either make it genuinely bite
  (probe echoes the received Authorization value back via an MCP logging
  notification so it flows through `forward.rs`'s redacted log path; then assert
  BOTH `!contains(secret)` AND `contains("[REDACTED]")`), OR — if that isn't
  cleanly achievable — replace it with an honest direct registration→redaction
  wiring test and document H-3 as defense-in-depth (like H-2). DO NOT weaken an
  assertion to pass.

### TARGETED
- **A1 — unconditional header registration → over-redaction surface.**
  `registry.rs:126` registers every header value (benign included); a value
  colliding with operational log text gets masked. Decision: KEEP unconditional
  (safest against leaks; operator controls their own header values) and DOCUMENT
  the tradeoff in SECURITY.md (folds into A5). → debugger (doc).

### TRIVIAL (bundle → debugger)
- **A4/T1 — duplicate `schemars` row** `STACK.md:28-29`. Delete one.
- **A3 — `Cli.spawn_immediate_descendant` field not cfg-gated** (`main.rs:68-70`);
  release parses a hidden no-op flag. Add `#[cfg(debug_assertions)]` to the field.
- **A5 — SECURITY.md H-8 note silent on the H-3 over-redaction.** Add one line:
  every `[headers]` value (literal or `${VAR}`) is registered; pick header values
  distinct from anything your tracing emits.
- **T2 — duplicate H-6 comment** at `main.rs:125-127` & `186-188`. Keep one
  canonical, trim the second to a pointer.
- **T3 — `Aggregator.config` unread field behind `#[allow(dead_code)]`**
  (`server.rs:40-41`) after H-5. Drop the field if cleanly unused (fix
  constructor + callsites); else improve the comment to name the concrete future
  use. Debugger's discretion; prefer dropping.
- **T5 — CONTRIBUTING.md no config-path pointer.** Add a one-line cross-ref to
  README Quick Start.
- **T6 — H-2 `200` inline.** Introduce `const CAP: usize = 200;` mirroring
  `sanitize_upstream_text`'s named cap.

### TRIVIAL (not fixed this cycle)
- **T4 — `HeaderSeen`/`start_http_probe` duplicated** across two test files.
  Test-code dedup; bounded and currently equivalent. Note for a future
  test-cleanup pass; not worth the churn now.

## Confirmed clean (≥2 lenses + orchestrator)
H-1 (poison recovery all 3 sites), H-2 (char-boundary cap), H-4 (both halves),
H-5 (assoc fn), H-6 (comment-only, behavior unchanged), H-7 (cfg-gating complete
across main.rs+process.rs, release excludes cleanly), D-005/D-010 (no new secret,
shape unchanged), O-1/O-2/D-2 (metadata, contact, strike), H-8 wording.

## Disagreements
None material. Adversarial rated the test issue structural (re-spec, not a
one-liner); the orchestrator concurs and routes it to test-creator with explicit
anti-gaming guidance.
