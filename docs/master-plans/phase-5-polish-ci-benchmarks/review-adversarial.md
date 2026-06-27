# Adversarial Review — Phase 5 (fanin-mcp v0.6.3)

**Lens:** adversarial  
**Scope:** review-context.md high-priority items #1–3 + cross-cutting (P2 containment, P1 redaction sink, P3 fidelity)  
**Files read:** `src/process.rs`, `src/main.rs`, `src/registry.rs`, contract tests (`process_lifetime.rs`, `http_upstream.rs`, `observability.rs`)  
**Verdict:** PASS (0 blockers, 2 targeted, 0 structural/trivial)  
**Finding count:** 2

---

## Findings

### targeted — P2: `WindowsSelfJobGuard` doc comment slightly overstates "retained self-Job-Object" role vs. per-upstream wrapper (process.rs:138-148, 258-262)

**Location:** `src/process.rs:138` (comment on `ProcessTreeGuard`) and `258-262` (spawn comment).

**What:** The comment labels the retained guard "for the fanin-mcp process itself" and the spawn comment says "retained self-Job-Object" does the real work. In reality the *per-upstream* `JobObject` wrapper (`process-wrap`) closes the CARRY-1 race; the self-guard is an *additional* outer containment.

**Why it matters:** Misleading for future maintainers auditing the race fix; could lead to incorrect assumption that upstreams rely on the outer Job.

**Fix:** Tighten the two comments to state: "outer containment for fanin itself; upstreams use process-wrap JobObject wrapper (suspended-spawn + KILL_ON_JOB_CLOSE)".

---

### targeted — P2: unused `#[allow(dead_code)]` on `ProcessTreeGuard` (process.rs:139)

**Location:** `src/process.rs:139`.

**What:** The enum variant is constructed in `contain_current_process_tree` and stored in `main`, but the field is never read after construction. The `#[allow(dead_code)]` suppresses the warning.

**Why it matters:** Minor hygiene; signals the guard is intentionally retained only for its Drop side-effect.

**Fix:** Add a one-line `// retained solely for Drop (KILL_ON_JOB_CLOSE on self)` comment above the attribute and keep the allow, or expose a trivial `fn retain(&self){}` if desired. Not a behavioral defect.

---

## Summary by scrutiny area

1. **P2 containment (CARRY-1/2)** — PASS.  
   - Windows: `process-wrap` `JobObject` wrapper (`spawn_stdio_transport:263-268`, `spawn_immediate_descendant:304-309`) performs suspended-spawn → assign → resume. Race closed. Retained self-Job-Object (`WindowsSelfJobGuard`) is orthogonal outer containment; per-upstream Jobs are independent. No nesting break for CC-spawn-fanin or sibling upstreams.  
   - Linux: `install_linux_parent_death_signal:332-349` — `pre_exec` + `PR_SET_PDEATHSIG, SIGKILL` is async-signal-safe, set on the correct child, sound.  
   - Test-gaming: none. `src/` has zero test-name/marker special-casing. Oracle is PID liveness (`process_is_alive` in contract test). Markers only communicate the grandchild PID; death is observed via `kill -0` / `tasklist`.

2. **P1 redaction on file sink** — PASS.  
   - Every path to the JSON file sink (`RedactingFileWriter::write:518`, `append_log_line:443`, `emit_stderr_line:434`) calls `redact` before the write. No `{:?}` env map or resolved secret reaches the file. Call-tool args are excluded from logs per D-004.

3. **P3 secret + protocol fidelity** — PASS.  
   - Header `${VAR}` values are resolved (and registered for redaction) in `resolve_env_value` *before* any HTTP connect or log emission.  
   - Registry init uses a short-lived per-server mutex only for the connect guard; no lock is held across the actual HTTP await (GOTCHA #16).  
   - `credential_resolution_failed` surfaces as `ToolError` → `CallToolResult{isError:true}` (D-005) without ever contacting the endpoint. Byte-faithful results preserved.

**Cross-cutting:** rmcp pin exact, no stdout writes, scope respected, 4 ignored tests legitimately deferred. No findings outside the two targeted above.

**End of adversarial review.**
