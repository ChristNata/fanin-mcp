# Reviewer task — feature `schema-relay-fidelity` (adversarial + alignment)

Do a combined **adversarial + alignment** review of this completed bugfix
cycle. The change set is small but security-adjacent, and this exact
sanitization area was test-gamed in a prior cycle — be skeptical. Write your
findings to `docs/master-plans/schema-relay-fidelity/review.md` and return a
verdict (PASS / PASS-with-issues / FAIL) plus a severity-tagged finding list
(blocker / structural / targeted / trivial).

## What changed (review the committed diff: v0.6.31..HEAD)
- `src/server.rs`: split `sanitize_upstream_text` into
  `neutralize_upstream_display` (strip+trim, no cap) and
  `sanitize_list_row_description` (neutralize + CAP 100). `get_tool_schema`
  annotations now relay full-length; `list_tools` rows keep the cap.
- `tests/integration/sanitization.rs` + `tests/probe-server/main.rs`: flipped
  the stale `get_tool_schema` cap assertions, added the full-length bite test
  and the invoke control-char round-trip lock, added a clean long fixture.
- `SECURITY.md` + `docs/GOTCHA.md` #20: doc lock.

## Read
- `docs/master-plans/schema-relay-fidelity/master.md` (Success Criteria,
  Constraints) and `state.json` (locked decisions).
- `docs/DECISIONS.md` D-004; `SECURITY.md`; `docs/GOTCHA.md` #20.
- The `rmcp-general`, `rust-review` skills.

## Adversarial lens — attack the change
1. **Test honesty / anti-gaming.** Do the new tests genuinely bite? Confirm
   `get_tool_schema_preserves_full_length_annotations_without_row_cap` asserts
   EXACT full-string content (not a weak `len() > cap`), so re-introducing the
   cap fails it. Confirm `invoke_tool_arguments_with_control_chars_round_trip_
   verbatim` asserts the BEL is actually present in the response (not just
   "no error"). If you can re-introduce the Issue-2 bug and have the suite stay
   green, that is a BLOCKER.
2. **Did removing the cap reintroduce a risk the cap guarded?** The cap was a
   `list_tools` token-budget control, NOT a security control (the stress test
   confirmed payloads fit the window). Verify `get_tool_schema` annotations are
   still control-neutralized + single-line (no `\n`/bidi/zero-width leak) at
   FULL length — i.e. fidelity was restored without dropping the display-safety
   guarantee. Note (do not block on) that an upstream can now make a single
   on-demand `get_tool_schema` annotation arbitrarily long; confirm this is the
   intended, documented design (list_tools bulk path stays capped).
3. **Edge cases in the split.** Empty string, all-control string (collapses to
   empty after trim), multibyte-boundary safety (char iterator, not bytes),
   nested `properties.*.description`. Any panic or corruption path?
4. **D-004 integrity.** Confirm NOTHING now sanitizes `invoke_tool` args/results
   or schema validation strings (`enum`/`const`/`default`/`pattern`/`examples`).

## Alignment lens — does it match the spec
5. Each master Success Criterion (1-8) actually met by code + tests?
6. Locked decisions honored: `I1-sanitization-stance` (no NL scrub, no
   envelope), `I2-truncation-fix`, `I2-test-contract-conflict`.
7. Docs accurate to code: SECURITY.md + GOTCHA #20 statements match the actual
   behavior (cap = list_tools rows only; get_tool_schema full-length; invoke
   verbatim). No overclaim.
8. Scope discipline: only `src/server.rs` + the two docs + the test files
   changed; nothing adjacent silently rewritten.

## Constraints on you
- You REVIEW; you do not edit code, tests, or docs. Findings go in `review.md`.
- Do not run a fix. Route nothing yourself — the orchestrator routes.
- If you assert a finding, make it concrete (file:line, the exact failing
  scenario). Vague findings are not actionable.

## Return
Verdict + severity-tagged findings + a one-line confidence statement on whether
this is launch-ready. Note any out-of-scope observations separately. You do NOT
write or advance `state.json`.
