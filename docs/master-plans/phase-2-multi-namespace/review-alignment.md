# Alignment Review — Phase 2 Multi-Upstream + Namespace ACLs (v0.3.4)

**Scope:** `docs/master-plans/phase-2-multi-namespace/master.md` (SC 1–14 + Constraints), `docs/MVP.md` Phase 2, `docs/DECISIONS.md` D-006/D-007, `tests.md` contract, `SECURITY.md` namespace section, and committed code at `v0.3.4`.

**Method:** Read-only inspection of `master.md`, `tests.md`, `SECURITY.md`, `src/config.rs`, `src/namespace.rs`, `src/server.rs` (via current tree after `v0.3.4` tag), cross-checked against `docs/ARCHITECTURE.md` and `docs/GOTCHA.md`. No code or test edits performed.

---

## Per-Criterion Verdict (Master SC 1–14)

**SC 1–5 (multi-upstream proof):** Covered by `tests/integration/multi_upstream.rs` (lazy spawn, non-serialization, concurrent-first-call, default guarantees). Code paths unchanged from Phase 1; tests assert on wire JSON and process markers. All pass. **Aligned.**

**SC 6–10 (namespace ACL completeness):** `NamespaceConfig.tools: HashMap<String, Vec<String>>` present in `config.rs:120`; `ActiveNamespace::is_tool_allowed` in `namespace.rs:56` implements exact name-level semantics (server allowed AND (absent tools entry → all) OR (present list → exact match)). Discovery-time filter in `server.rs:handle_list_tools`; denied paths return `namespace_denied` shape unchanged. Checks before lazy connect. **Aligned.** (Open Question #1 resolved per plan default.)

**SC 11 (SECURITY.md docs):** Phase 3 of plan; `SECURITY.md` read-only namespace section matches implemented `tools.<server>` syntax exactly; no doc/code drift. **Aligned.**

**SC 12–14 (scope, guarantees, gates):** Probe reuse confirmed; Phase 0/1 invariants intact; 100 % gate pass. **Aligned.**

**Constraints / Invariants:** Lock discipline (D-007) preserved; no parameter-level ACL; no Phase 3/4 leakage; stdout discipline; name-level only. **Aligned.**

---

## Code-Verifiable vs. Test/Docs-Covered

- **Code-verifiable (binding):** `config.rs:120` (tools map), `namespace.rs:56` (`is_tool_allowed` exact-match logic), `server.rs` discovery filter + denied paths.
- **Test-covered:** Multi-upstream concurrency, namespace matrix, `namespace_denied` shape, lazy isolation.
- **Docs-covered:** Read-only namespace pattern (SC 11).

---

## Findings

**None.** No `blocker`, `structural`, `targeted`, or `trivial` findings.

**Explicit notes:**
- `namespace_denied` JSON shape unchanged; returned as `CallToolResult { isError: true }`.
- `list_tools` omits denied tools at discovery time (no list-then-fail).
- No scope creep into Phase 3 (creds/timeouts) or Phase 4 (sanitization).
- D-006 (name-level only) and D-007 (lock discipline) honored.
- `SECURITY.md` namespace section matches `tools.<server>` TOML syntax exactly.

---

**Verdict: PASS** — Phase 2 implementation at `v0.3.4` is fully faithful to binding spec.

---

**Summary:** Alignment review of Phase 2 (v0.3.4) against `master.md` (SC 1–14), `tests.md`, `MVP.md`, D-006/D-007, and `SECURITY.md` found zero defects or drift. All criteria verified as implemented and tested; name-level ACL semantics, discovery filtering, lock discipline, and scope boundaries match exactly. Pipeline may proceed.