# knowledge-sync: phase-5-polish-ci-benchmarks

Post-cycle doc reconciliation. Tier: THOROUGH. Source of truth: the cycle diff
`32eb0bc..HEAD` (v0.5.6 → v0.6.5) on `main`. This stage also absorbs **plan
Phase 7** (the docs phase), which was routed here rather than a separate
implementer dispatch (doc reconciliation is knowledge-sync's job; doing it twice
would churn the docs).

## Cycle model

Linear-on-`main` (no worktree, no PR, no squash-merge — see
[[cycle-model-linear-on-main]]). Stages committed directly to `main`
(`v0.6.1`..`v0.6.5`); this artifact + the doc edits are the `v0.6.6`
knowledge-sync checkpoint, pushed with the cycle in the single end-of-cycle
`git push`. `state.json` finalization omits the canonical `merge` step (no PR /
merge commit exists to populate it).

**Summary:** 5 doc files edited (accuracy/honesty fixes for shipped behavior +
2 carried drifts); 0 doc files created; 1 doc edit done in-cycle by the
implementer (README token block, P5); 2 spec/skill drifts resolved; 2 stale-doc
flags surfaced (MVP checklist, ROADMAP MVP status) for the user.

## Per-doc updates (this stage)

| Doc | Change |
|---|---|
| `SECURITY.md` | ADDED Enforced-Practice #6 "Process-tree containment" — Windows suspended-spawn + Job Object and Linux `PR_SET_PDEATHSIG` are crash-safe and CI-tested; **macOS hard-kill (`SIGKILL`) may leave orphans — documented MVP limitation, no daemon** (the user-locked CARRY-2 decision; previously undocumented — the Phase-4 sync flagged this gap). Updated the Supply-Chain bullet: the reqwest/hyper HTTP **client** for remote upstreams (client not listener), TLS only for HTTPS remotes, `cargo deny` + `<10MB` budget. |
| `.claude/skills/rmcp-general/SKILL.md` | FIXED the "Features in play" line: `transport-streamable-http` → `transport-streamable-http-client` + `transport-streamable-http-client-reqwest` (the `-client` variant; bare name is the server side). Resolves the OQ3 drift — the skill would otherwise misguide future HTTP-upstream work. |
| `docs/ARCHITECTURE.md` | Resolved DRIFT-1: the `error.rs` snippet (`AggError`/`ErrorCode`) annotated as illustrative; the shipped Rust type is **`ToolError`**, and the **public contract is the wire shape** (D-005 fields + `code` strings), not the type name. Noted the Phase-5 additions `upstream_disconnected` + `credential_resolution_failed`. Updated the line-141 `AggError` reference to `ToolError`. |
| `docs/GOTCHA.md` | #11 — added the spawn-then-assign race + its Phase-5 suspended-spawn fix (✅). #14 — split the Unix story honestly: `setsid` covers graceful teardown only; **Linux** adds crash-safe `PR_SET_PDEATHSIG` (✅), **macOS** has no equivalent → hard-kill may orphan (⚠️ documented limitation, excluded from the zero-orphan test claim). |
| `STACK.md` | Fixed the rmcp feature-name (line 18, same `-client` correction) and rewrote the "Transitive-only, tolerated" line to describe the actual reqwest/hyper/ICU client tree + the direct `http`/`libc` adds, with the `<10MB` (measured 8.27MB) + `cargo deny` guardrails. |
| `README.md` | No edit needed — the token-figures block (`<!-- fanin-token-figures:start/end -->`, lines 77-84) was generated + inserted in-cycle by P5; `cargo bench --bench token_cost` owns it (GOTCHA #26). |

## Implementation drift (diff vs master.md Produces)

- **Planned but not shipped (deliberate):** plan-Phase-4 produced NO code — its
  contracts (`tool_search`, namespace/cred reuse, the CARRY-4 `TransportSend`
  source-guard) already passed from prior phases + the Phase-4 mapping. Verified
  no-op, not a gap. Plan-Phase-7's doc edits landed here (knowledge-sync), not as
  a separate dispatch.
- **Shipped but not planned:** direct deps `http` (HeaderName/HeaderValue) and
  target-gated `libc` (Linux PDEATHSIG) — both within P3/P2 intent, noted for the
  deny/budget review (P6) which passed. No rmcp pin change (`=1.8.0` intact).

## Spec / skill drift (THOROUGH audit)

- **TARGETED — resolved:** rmcp HTTP feature-name (rmcp-general SKILL.md + STACK.md)
  and the `AggError`→`ToolError` naming (ARCHITECTURE.md). Both are doc-accuracy
  corrections to match shipped reality, applied directly.
- **No structural drift.** D-004/005/007/009/010, GOTCHA #1/#16/#19/#20 all
  upheld (re-verified by the 3-lens review). The macOS containment gap is a
  *documented, user-accepted* limitation, not an undisclosed divergence.

## Stale-doc flags (surfaced, NOT auto-edited — user decides)

1. **`docs/MVP.md` Verification Checklist** — the Phase-5 automated items
   (sanitization, structured errors, hard-kill containment on Win/Linux, token
   benchmark, `cargo deny`, binary <10MB, namespace/cred, 3 meta-tools) are now
   satisfied by the green suite + CI + this cycle. The **manual** items (CC & OC
   each spawn and see 3 tools; one real remote HTTP upstream; idle/5-upstream
   memory profile; macOS graceful-teardown on a real mac) remain on
   `docs/release-checklist.md`. The checklist boxes were left unchecked — ticking
   them is a release-sign-off decision the user owns, not a doc-sync auto-edit.
   (`docs/MVP.md` Phase-4 item-1 still says `AggError/ErrorCode` in the historical
   plan text; left as historical — ARCHITECTURE.md carries the canonical note.)
2. **`ROADMAP.md`** — v1.0 MVP is marked `🔨 in progress`. With Phase 5 done the
   MVP is **code-complete**; flipping it to launched/`v1.0` is a release decision
   gated on the manual release-checklist items above. Left for the user.

## Pending follow-up

1. **End-of-cycle push** lands `v0.6.1..v0.6.6` on `origin/main` (this artifact +
   the 5 doc edits + finalized `state.json` are the `v0.6.6` commit). CI (the new
   3-OS matrix) triggers on that push — **the Linux PDEATHSIG hard-kill path and
   the macOS graceful path are verified there, not on this Windows host.** Watch
   the first CI run.
2. **MVP release sign-off** (user): run the manual release-checklist items, then
   tick the MVP checklist + flip ROADMAP to v1.0 if satisfied.
3. **Deferred tests** unchanged: F4 send-side wire test + keyring round-trip stay
   `#[ignore]`d with documented reasons (neither is the sole proof of a shipped
   criterion).
