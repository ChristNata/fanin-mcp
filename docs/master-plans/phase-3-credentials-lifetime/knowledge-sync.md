# knowledge-sync: phase-3-credentials-lifetime

Post-merge doc reconciliation. Source of truth: the merged diff
`e310afe..34afc93` (v0.3.5 → v0.4.5) on `main`. Tier: THOROUGH.

**Summary:** 2 modules' inline docs corrected (stale "not yet wired/invoked"
comments that now lie); 0 doc files created (flat project — followed existing
convention: root `*.md` + `docs/` vault + inline rustdoc); 0 spec edits
auto-applied; 3 spec/impl drifts surfaced; 2 stale-doc flags raised. Doc edits
are STAGED, not committed — run `/push` to land them.

## Note on cycle model

This repo runs **linear-on-`main`** (no worktree, no PR, no squash-merge). The
v0.4.x stages were committed directly to `main` and pushed (`git push origin
main`, `9df1392..34afc93`). There is therefore no PR number or merge-commit to
record; `state.json` finalization below omits the canonical `merge` step rather
than fabricating one.

## Per-module doc updates (touched files)

| Module | Change |
|---|---|
| `src/main.rs` | Fixed `run_serve` comment that claimed "the loaded config is not yet wired into the aggregator" — it now IS wired (builds `ActiveNamespace` + `Registry` + `Aggregator`, lines 138-145). |
| `src/credentials.rs` | Fixed `CredentialStore` trait doc that claimed `get` is "not yet invoked" — it is invoked at spawn by the registry for `${VAR}` resolution; `set/delete/list_names` back the `cred` subcommands. |
| `src/config.rs` | No change — module schema doc was already updated in-phase (`timeout_secs` documented; env values "may contain `${VAR}` (interpolated at spawn)"). Accurate. |
| `src/error.rs`, `forward.rs`, `process.rs`, `registry.rs`, `server.rs` | No doc lies found; inline docs match behavior. `process.rs` Phase-4 containment doc is accurate. Left unchanged (no style churn). |

## Implementation drift (diff vs master.md Produces)

**Planned but not shipped (all deliberate):**
- `src/cancellation.rs` (P3 "only if needed") — not created; cancellation handled
  inline in `server.rs` via `RequestContext.ct`. Acceptable.
- `src/process/windows.rs` / `src/process/unix.rs` (P4 "only if needed") — not
  created; platform paths are `#[cfg]`-gated inside `process.rs`. Acceptable.
- `cwd` per-server field (ARCHITECTURE.md:97; P4 "only if confirmed needed") —
  NOT implemented this phase. Documented in ARCHITECTURE as a design field;
  GOTCHA #30 (Morph CWD) still references it. Carry to a later phase.
- HTTP `url` / `headers` / Streamable-HTTP transport (OQ1) — deferred to Phase 5
  by decision. ARCHITECTURE.md documents them as design; `transport != "stdio"`
  still fails startup.

**Shipped but not planned:** none. All 8 touched `src` files + `Cargo.toml` were
in the plan's per-phase Produces. New error codes (`upstream_timeout`,
`credential_resolution_failed`, `call_cancelled`) are within the D-005 envelope.

## Spec drift (THOROUGH audit)

- **STRUCTURAL — cancellation forwarding.** `docs/ARCHITECTURE.md:138` ("Client
  cancellation notifications are forwarded to in-flight upstream calls") and
  `docs/MVP.md` Phase 3 item 5 ("forward client cancellation notifications")
  claim upstream forwarding. The implementation does **local-abort only** —
  rmcp `=1.8.0` does not expose the upstream request id for a typed
  `peer().call_tool(...)` (resolved OQ3, honest by design; SC 17 was always a
  coverage boundary). **Surfaced, not auto-edited** — the user decides whether to
  amend the specs to "local-abort; upstream forwarding deferred pending an rmcp
  pin that exposes the request id," change scope, or accept as-is.
- **STRUCTURAL — D-009 hard-kill (zero orphans, all OSes).** Windows Job Object
  containment is implemented and verified (3/3). Two gaps are tracked as carried
  issues, deferred to Phase 5 by user decision:
  `issue-windows-jobobject-spawn-race.md` (Job Object assigned post-spawn; race
  window) and `issue-unix-hardkill-containment.md` (`ProcessSession` not
  SIGKILL-safe; needs Linux `PR_SET_PDEATHSIG` + macOS design). `docs/DECISIONS.md`
  D-009 and `docs/ARCHITECTURE.md:160-161` describe the intended design; the
  divergence is tracked work, not an accepted spec change — **not edited.**

No trivial or targeted spec edits were applicable — `ARCHITECTURE.md` and
`SECURITY.md` are otherwise accurate against the implementation.

## Stale-doc flags (outside the touched set — surfaced, not fixed)

- `SECURITY.md:19` — "the aggregator sanitizes and length-caps descriptions
  (strips newlines/control characters...)" is stated as present behavior, but
  description sanitization is **Phase 4** (not yet implemented). Overclaim; the
  user may leave it as intended design or qualify it until Phase 4 ships.
- `docs/ARCHITECTURE.md:97` — the `cwd` field is documented but unimplemented
  (see Implementation drift). Design doc; leave or annotate.

## Pending follow-up

1. **`/push`** to commit + version the staged doc edits (this `knowledge-sync.md`,
   the `main.rs`/`credentials.rs` comment fixes, the finalized `state.json`).
2. **`/gate`** — three new deps (`keyring`, `rpassword`, `process-wrap`) →
   `cargo audit` / `cargo deny` + token benchmark.
3. **Phase 5 carries:** the 2 process-lifetime issues; the cancellation-forwarding
   spec reconciliation; `cwd` field; HTTP transport + `headers`.
4. **User decision:** `issue-credentials-edit-deny-rule.md` (managed OC
   `**/credentials*` deny catches `src/credentials.rs`).
