# Planner task — feature `schema-relay-fidelity`

Write the master plan for a THOROUGH-tier bugfix cycle closing two findings
from an external stress test of the fanin-mcp aggregator. The routing /
concurrency layer was found fully healthy; only these two non-routing issues
remain. **Most of the root-cause analysis is already done for you below —
your job is to turn it into a precise `master.md` (+ `tests.md` skeleton),
not to re-derive it.** Do not widen scope beyond these two issues.

## Read first (binding context)
- `docs/master-plans/schema-relay-fidelity/state.json` — the locked decisions
  (`I1-sanitization-stance`, `I2-truncation-fix`, `I2-test-contract-conflict`).
  These are USER/ORCHESTRATOR-confirmed; treat them as fixed inputs.
- `src/server.rs` lines ~150-253 (the `list_tools` and `get_tool_schema`
  handlers) and ~359-463 (the sanitize_* helpers).
- `tests/integration/sanitization.rs` — the EXISTING contract (read in full).
- `SECURITY.md` §Threat Model (the prompt-injection paragraph) and
  `docs/GOTCHA.md` #20. `docs/DECISIONS.md` D-004 (byte-faithful results).

## The two issues

### Issue 1 (HIGH as filed; validated as ALREADY-DECIDED design — test+doc LOCK only)
fanin relays adversarial text and control chars. Validated stance (locked,
decision `I1-sanitization-stance`): this is the intended transparent-ish-proxy
behavior, NOT a bug to "fix":
- Control chars in LLM-visible DISPLAY annotations (`title`/`description`/
  `$comment`/`markdownDescription`, and `list_tools` rows) ARE already
  neutralized.
- Natural-language injection text ("IGNORE previous instructions"), schema
  VALIDATION strings (`enum`/`const`/`default`/`pattern`), and `invoke_tool`
  arg+result BYTES pass through verbatim BY DESIGN (D-004 byte-faithfulness;
  SECURITY.md "bounds, cannot eliminate" the channel).
- Existing tests already lock most of this: `f3_get_tool_schema_preserves_
  validation_data_sanitizes_only_annotations` and `invoke_tool_result_content_
  not_sanitized_passes_byte_faithfully`.
- **The only gaps to close:** (a) NO test currently asserts the `invoke_tool`
  control-char ROUND-TRIP — sending an arg value containing a C0 control (e.g.
  BEL U+0007) and asserting it echoes back verbatim in the RESPONSE
  (byte-faithful, the desired behavior). (b) SECURITY.md does not explicitly
  name the invoke arg/result channel as an intentional verbatim pass-through.

### Issue 2 (MEDIUM; validated as a GENUINE bug)
`get_tool_schema` silently truncates schema string fields (`description`,
`title`, `$comment`, property descriptions) at ~100 chars, mid-word, dropping
real argument documentation on real tools.
- Root cause: `sanitize_upstream_text` (server.rs:369) bundles TWO concerns —
  control-neutralization AND a `CAP=100` length cap — and `get_tool_schema`
  reuses it (server.rs:251 → `sanitize_metadata_value` → `sanitize_upstream_text`).
  The cap is documented/intended for `list_tools` ROWS ONLY (SECURITY.md
  "length-caps each list_tools description row"; GOTCHA #20). It leaks into the
  schema-detail fidelity path.
- Fix (decision `I2-truncation-fix`): decouple control-neutralization (both
  paths) from length-capping (`list_tools` rows only). `get_tool_schema`
  annotations stay control-neutralized + single-line but FULL-LENGTH.
- **Test-contract conflict (decision `I2-test-contract-conflict`):** existing
  `sanitization.rs` assertions (`get_tool_schema_sanitizes_..._preserves_shape`
  ~lines 411-446, and the F3 annotation block ~946-968) ASSERT
  `get_tool_schema` annotations are `<= DESC_CAP`. Those lock the bug. The plan
  must call out that `test-creator` FLIPS them to assert FULL pass-through
  (keeping control-free + single-line), and ADDS a regression: a tool with a
  >100-char (post-strip) annotation returns the FULL string from
  `get_tool_schema`. `list_tools`-row cap assertions stay unchanged.

## Deliverables
- `master.md` per `plan-format`: problem statement, the locked decisions
  restated, phased breakdown, per-phase Success Criteria that are
  OBJECTIVELY verifiable at the wire level, explicit Produces/Depends-On, and
  an explicit note on the test-contract flip (so test-creator has authority
  framing). Keep phases minimal — this is a small change set (one src
  decouple, doc edits in SECURITY.md/GOTCHA #20, and the test changes).
- A `tests.md` skeleton enumerating the test contract: (1) get_tool_schema
  full-length annotation pass-through (NEW, the Issue-2 bite — must assert
  exact/near-exact full content so a re-introduced cap FAILS loudly);
  (2) invoke_tool control-char round-trip verbatim (NEW, Issue-1);
  (3) the existing get_tool_schema cap assertions FLIPPED; (4) confirmation
  that list_tools-row cap + validation-string-verbatim + byte-faithful-result
  tests remain GREEN/unchanged.
- Note any probe-fixture additions test-creator may need (e.g. a tool with a
  long CLEAN description, or reuse of the existing long-but-control-laden
  poison fixtures whose post-strip length still exceeds the cap).

## Constraints
- Do NOT propose sanitizing invoke I/O or validation strings — that violates
  D-004 and the locked stance.
- Do NOT propose natural-language scrubbing or an untrusted-content envelope —
  explicitly rejected in `I1-sanitization-stance`.
- Scope discipline: touch only what these two issues name. Surface anything
  adjacent in your returned result; do not fold it into the plan.
- You do NOT write or advance `state.json` — that is the orchestrator's.

Write `master.md` and `tests.md` into
`docs/master-plans/schema-relay-fidelity/`. Return a short summary: the phase
list, any open questions, and any out-of-scope items you noticed.
