# Phase 5 — carry-over notes (deferred issues + gotchas folded in)

Orchestrator pre-flight audit, 2026-06-27. Source: phase 0–4 plan workspaces,
`docs/GOTCHA.md`, `docs/MVP.md` §Phase 5, current `src/` + repo root state.
This is the input brief handed to the `planner`. It is NOT the plan.

## A. Deferred issues carried INTO Phase 5 (must be addressed)

### CARRY-1 — Windows Job Object spawn-then-assign race  *(blocker)*
- Source: `phase-3.../issue-windows-jobobject-spawn-race.md`. Code: `src/process.rs`.
- **What:** child is assigned to the kill-on-close Job Object *after* spawn
  (`ContainmentGuard::for_transport` calls `AssignProcessToJobObject` post-spawn).
  A grandchild spawned in that window escapes `KILL_ON_JOB_CLOSE` (D-009).
  Current hard-kill test misses it — probe forks its grandchild well after init.
- **Fix:** spawn suspended (`CREATE_SUSPENDED`), assign to job, then `ResumeThread`.
  Obstacle: rmcp `=1.8.0` `TokioChildProcess` spawns running and may not expose
  the main-thread handle for `ResumeThread`. Fallback: thin custom child transport
  owning the `CreateProcess` call (D-009 sanctions Job Objects). Add a regression
  test that forks the descendant *immediately at child startup* to hit the window.
- **Verify:** Windows CI. Local Windows host can run this one.

### CARRY-2 — Unix hard-kill containment incomplete  *(blocker)*
- Source: `phase-3.../issue-unix-hardkill-containment.md`. Code: `src/process.rs` (Unix).
- **What:** `process-wrap` `ProcessSession` (setsid) isolates but does NOT kill the
  child tree on `kill -9` of fanin-mcp — `Drop`/kill-on-drop don't run on SIGKILL.
  Unix descendants survive a hard-kill → violates D-009 / master SC 20–21.
- **Fix:** Linux — `pre_exec` `prctl(PR_SET_PDEATHSIG, SIGKILL)` (cfg linux),
  crash-safe; pair with group teardown for the graceful path. **macOS — DESIGN CALL
  (open question):** no PDEATHSIG equivalent. Options: (a) supervisor/reaper process,
  (b) kqueue `NOTE_EXIT` watcher, (c) documented limitation for MVP. → Open Question.
- **Verify:** Linux + macOS CI runners only. **This Windows dev host cannot verify
  the Unix path locally** — CI is the verification surface (this is exactly why it
  was deferred to the Phase 5 3-OS matrix).

### CARRY-3 — Overbroad `**/credentials*` OC edit-deny rule  *(structural, tooling — USER DECISION)*
- Source: `phase-3.../issue-credentials-edit-deny-rule.md`. Managed OC config.
- **What:** OC `edit` deny `**/credentials*` also blocks `src/credentials.rs`, so
  child agents can't author the credential module (in Phase 3 this pushed cred logic
  into the wrong file twice). Recommended fix: add `**/src/credentials.rs: allow`
  (and `**/src/secrets.rs: allow`) precedence exception to the managed OC config.
- **Phase 5 relevance:** only blocks IF a Phase 5 implementer dispatch must edit
  `src/credentials.rs`. Phase 5's credential work (MVP item 4) is **E2E tests**
  (test-creator → `tests/`), not src edits — so likely NOT blocking. Resolve only
  if the plan puts a `src/credentials.rs` edit in scope. Security-config change ⇒
  user's call, surfaced not silently applied.

### CARRY-4 — F4 send-side wire test  *(robustness follow-up — low)*
- Source: `phase-4.../knowledge-sync.md` §Pending follow-up #2; `review.md` F4.
- **What:** `map_service_error` already maps `ServiceError::TransportSend` →
  `upstream_disconnected` (CODE FIXED). Only the dedicated wire test is `#[ignore]`d
  because OS pipe-closure timing makes TransportSend vs TransportClosed
  non-deterministic at the wire level.
- **Unblock:** a transport wrapper that forces send-side failure, OR a unit test
  once `map_service_error` is made testable (expose it). Small Phase 5 task at most.

## B. Doc / spec drift flagged but unresolved

### DRIFT-1 — `AggError`/`ErrorCode` naming in docs vs `ToolError` in code
- Source: `phase-4.../knowledge-sync.md` §Stale-doc flags.
- `docs/ARCHITECTURE.md:141` + `docs/MVP.md` Phase 4 item 1 say `AggError`/`ErrorCode`;
  code uses `ToolError` (accepted OQ1 decision). Public wire contract unaffected
  (code strings + D-005 shape intact) — internal Rust name only. Fold into Phase 5
  knowledge-sync or a doc fixup. Not blocking.

## C. Gotchas that bind Phase 5 work specifically

- **#1 stdout is the transport.** The new `--log-file` / JSON `tracing` output must
  go to the file or stderr — NEVER stdout. (MVP P5 item 1.)
- **#19 redaction sentinel test (release gate).** The new file/JSON log writer MUST
  pass through the redaction layer too — the sentinel-secret test must stay green
  against the file sink, not just stderr. Don't `{:?}` a resolved env map.
- **#11 / #14 process tree.** CARRY-1 / CARRY-2 are these gotchas' unfinished halves.
  Hard-kill CI test asserts zero survivors on ALL OSes.
- **#26 token figures from benchmark, never hand-edited.** The new `cargo bench
  --bench token_cost` is the source of README token numbers. (MVP P5 item 5.)
- **#30 / #31 (D-019).** CWD-at-spawn + name-level namespace filtering — relevant to
  the Phase 5 integration tests (real remote HTTP upstream, namespace switching).

## D. Current state of Phase 5 deliverables (real gaps vs. already-done)

| MVP P5 item | State |
|---|---|
| 1. `tracing` JSON file out (`--log-file`, `--log-level`), per-call logging | **GAP.** `init_tracing()` is hardcoded INFO→stderr only; no flags, no JSON, no call log. |
| 2. CI matrix (Win/mac/Linux) + `cargo audit` + `cargo deny` | **GAP.** No `.github/` at all; no `deny.toml`. |
| 3. Integration vs CC/OC/probe + one real remote HTTP upstream w/ header auth | **PARTIAL.** Probe suite exists; real remote HTTP upstream unverified. |
| 4. Namespace switching, credential E2E, CC Tool Search composition | **PARTIAL.** Unit/integration exist; E2E + Tool-Search check outstanding. |
| 5. Token benchmark (`cargo bench --bench token_cost`) | **GAP.** No `benches/`, no bench target; `/gate` already expects it. |
| 6. Hard-kill orphan CI on all OSes; binary <10MB stripped; mem <15MB idle/<50MB@5 | **GAP** (ties to CARRY-1/2). |
| 7. SECURITY.md finalized; dual-license files; `license = "MIT OR Apache-2.0"` | **DONE.** SECURITY.md tightened in P4; LICENSE-MIT/APACHE present; license field set. |

## E. Fixed environment facts (no re-derivation needed)

- **Cycle model: linear-on-`main`** — no worktree, no PR, no merge. Commit each stage
  to `main`; ONE end-of-cycle push to `origin/main`. CI triggers on that push.
  (`/sync-knowledge` omits the merge step.) [[cycle-model-linear-on-main]]
- **Version stamp: `v0.6.x`** — MVP Phase 5 ⇒ B = 5+1 = 6; C resets to 1.
  (Phase 4 was v0.5.x.) [[commit-version-format]]
- **Tier: THOROUGH** — cross-platform process hardening + security gates + two
  carried blockers. Matches Phases 3/4.
- **fmt/clippy gate at the test checkpoint** so the implementer's fmt sweep never
  edits read-only test files. [[fmt-check-at-test-checkpoint]]
- **Process-containment verification must check PID liveness DURING the test
  window** — post-`cargo test` survivor counts are masked by the runner's own job.
  [[process-containment-verification-masking]]
