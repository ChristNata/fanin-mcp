# Review (synthesis): phase-4-error-sanitization

**Verdict: PASS-with-issues.** Suite green (99/0/3), fmt + clippy clean, all 17
Success Criteria met, no scope creep, the three accepted decisions honored, rmcp
pin intact. Three independent lenses ran on gpt-5.5 (alignment / adversarial /
general). Alignment: PASS. Adversarial: PASS-with-issues (5 targeted). General:
PASS-with-issues (3 targeted, 2 overlapping adversarial). **0 blocker, 0
structural.** Findings are merged below; each was independently confirmed in the
code by the orchestrator before routing.

The pipeline is NOT hard-blocked (nothing downstream depends on these). Per
plan-format routing, all findings are `targeted` → fix in-cycle (test-creator for
corrected/added coverage, debugger for the code), then re-gate. None escalate to
the user.

## Merged findings (all `targeted`)

### F1 — Sanitizer misses Unicode separators / C1 / bidi / zero-width (security)
- Lenses: adversarial #1. Location: `src/server.rs::sanitize_upstream_text` (~:382).
- `sanitize_upstream_text` replaces only C0 (U+0000–U+001F) + DEL (U+007F). A
  poisoned description can still carry U+2028/U+2029 (line/paragraph separator),
  C1 controls (U+0080–U+009F), bidi overrides/isolates (U+202A–U+202E), BOM, and
  zero-width format chars — all LLM-visible injection/format vectors. GOTCHA #20
  is "bound description-based prompt injection"; missing the most common
  separator (U+2028) is a real bypass.
- Fix (debugger): extend the strip/replace set to also cover U+2028, U+2029,
  C1 (U+0080–U+009F), bidi controls (U+202A–U+202E, U+2066–U+2069), BOM (U+FEFF),
  and zero-width (U+200B–U+200D). Coverage (test-creator): poison-fixture chars.

### F2 — Capped/sanitized name used as the dispatch key (correctness)
- Lenses: adversarial #2. Location: `src/server.rs` list_tools rows (~:184-186).
- The row's `tool` and `name` fields (the keys the LLM uses to build
  `server__tool` for get_tool_schema/invoke_tool) are set to the
  sanitized-AND-capped display string. A legitimate >100-char upstream tool name
  is truncated → a later invoke_tool with the advertised key fails `unknown_tool`.
- Fix (debugger): the `tool`/`name` dispatch-key field must carry the REAL
  upstream tool name (control-strip defensively, but NO cap — names are
  identifiers, not prose). Apply the ~100 cap ONLY to `description`. (rmcp's
  name grammar already excludes control chars, so the real name is dispatch-safe.)
  Coverage (test-creator): a long-named probe tool stays dispatchable.

### F3 — Schema sanitizer corrupts validation data and misses other strings
- Lenses: general #1 (corrupts `enum`) + adversarial #3 (misses `default`/`const`/
  `pattern`/…). Location: `src/server.rs::sanitize_schema_metadata` /
  `sanitize_metadata_value` (~:410-458).
- The current policy sanitizes `title`/`description`/`$comment`/`examples`/`enum`.
  `enum`/`examples` are VALIDATION constants, not labels — mangling them makes
  get_tool_schema advertise values the upstream doesn't accept (general #1). At
  the same time other LLM-visible strings (`default`, `const`, `pattern`,
  `format`, `$id`, `$ref`, vendor extensions) pass verbatim (adversarial #3).
- Resolution (supersedes the plan's "sanitize enum/examples" wording): sanitize
  ONLY pure-annotation keys — `title`, `description`, `$comment` (and
  `markdownDescription` if present). Leave ALL validation/structural values
  untouched (`enum`, `const`, `default`, `examples`, `pattern`, `format`, `$id`,
  `$ref`, property keys, `type`, `required`). The residual injection channel via
  validation strings is the documented GOTCHA #20 bound ("bounds, not
  eliminates") — control-stripping them would corrupt the schema, so it is
  deliberately not done. Knowledge-sync notes the bound.
- Fix (debugger): narrow the key set to annotations; stop touching enum/examples.
  Coverage (test-creator): a string `enum`/`default` with a control char is
  PRESERVED verbatim, while `title`/`description` are sanitized.

### F4 — Send-side broken pipe misclassified as `upstream_call_failed` (robustness)
- Lenses: adversarial #4 + general #3. Location: `src/registry.rs::map_service_error`
  (~:247).
- Classification matches only `ServiceError::TransportClosed`. A child that dies
  such that the failure is first observed on the WRITE side surfaces as
  `ServiceError::TransportSend(...)` → reported as `upstream_call_failed` instead
  of the Phase 4 `upstream_disconnected`. SC 6's test passes only because the
  kill happens to surface as `TransportClosed` on this host/timing.
- Fix (debugger, verify against rmcp =1.8.0): also map `TransportSend` (and any
  transport-layer closed/broken-pipe variant) from an established upstream
  operation to `UpstreamDisconnected`; keep MCP application errors as
  `UpstreamCall`. Confirm the exact `ServiceError` variants via Context7 — do not
  guess. Coverage (test-creator): best-effort send-side death; if not
  deterministically reproducible wire-level, document as `#[ignore]` + reason.

### F5 — Failed lazy refetch clears dirty → stale cache served later
- Lenses: adversarial #5 + general #2. Location: `src/registry.rs::ensure_fresh`
  (~:220).
- `dirty.swap(false)` happens BEFORE `list_all_tools().await`. If the refetch
  fails (e.g. upstream dies between the notification and the refetch), dirty stays
  false; the caller gets one error, but a later `inventory()`/`call_tool()`
  fast-paths past the (now-clean) flag and serves the STALE pre-notification
  inventory. The list_changed signal is silently lost.
- Fix (debugger): on refetch failure, restore dirty=true (or only clear it after
  a successful install) so a subsequent read retries — while keeping the
  no-lock-across-await discipline (a per-entry refresh guard / 3-state flag is
  acceptable). Coverage (test-creator): force the first post-list_changed refetch
  to fail; prove a later read retries rather than serving stale inventory. If not
  deterministically reproducible wire-level, document as `#[ignore]` + reason.

## Routing summary

- **test-creator** (corrected/added contract): F1 separators fixtures; F2 long-
  name dispatchability; F3 enum/validation-preservation + annotation sanitization;
  F4/F5 best-effort (defer with `#[ignore]`+reason if not deterministic).
- **debugger** (gpt-5.5 — security/robustness sensitive): F1–F5 code fixes.

## Disagreements between lenses

Adversarial #3 (sanitize MORE schema strings) vs general #1 (sanitizing `enum`
is WRONG). Resolved in F3: annotation-only sanitization — the correct reconciliation
(never alter validation/structural data; the residual is the documented GOTCHA #20
bound). Alignment found no issues and verified SC/scope/decision fidelity.

## Verified invariants (carried from adversarial lens, re-checked by orchestrator)

- No registry map lock held across `call_tool().await` / `list_all_tools().await`.
- `invoke_tool` result content is never sanitized/stringified (byte-faithful).
- `on_tool_list_changed` only stores an atomic flag + returns ready; no await /
  map touch / log / panic.
- No test-name-shaped logic or hardcoded fixture strings in `src/` Phase 4 paths.
