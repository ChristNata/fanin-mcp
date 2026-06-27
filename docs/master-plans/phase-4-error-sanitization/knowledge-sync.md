# knowledge-sync: phase-4-error-sanitization

Post-cycle doc reconciliation. Source of truth: the merged diff
`497c762..2dacf21` (v0.4.6 → v0.5.5) on `main`. Tier: THOROUGH.

**Summary:** 1 GitHub-facing doc edited (`SECURITY.md` — the Phase-3-flagged
sanitization overclaim is now TRUE and was tightened to the shipped scope);
0 doc files created (flat project — root `*.md` + `docs/` vault + inline
rustdoc, existing convention followed); inline rustdoc in the 4 touched modules
was kept accurate in-cycle (no post-hoc fixes needed); 1 spec/plan drift recorded
(superseded plan wording, resolved in review); 1 stale-doc flag raised
(`ARCHITECTURE.md` AggError naming); 1 carried follow-up (F4 send-side wire test).

## Note on cycle model

This repo runs **linear-on-`main`** (no worktree, no PR, no squash-merge), as in
Phase 3. The v0.5.x stages were committed directly to `main`; this artifact and
the doc edit are the knowledge-sync checkpoint (`v0.5.6`), pushed with the cycle
in the single end-of-cycle `git push`. There is no PR number or merge commit, so
`state.json` finalization omits the canonical `merge` step rather than fabricating
one.

## Per-module doc updates (touched files)

| Module | Change |
|---|---|
| `SECURITY.md` (§"What it does NOT protect against", prompt-injection bullet) | EDITED. The Phase-3 knowledge-sync flagged this line as overclaiming sanitization that did not yet exist. Phase 4 shipped it, so the claim is now true. Tightened to the actual scope: neutralizes C0/C1/DEL, Unicode line/para separators, bidi-override, zero-width — across `list_tools` names+descriptions and `get_tool_schema` annotation strings; honestly notes that schema *validation* strings and tool-call *results* are intentionally NOT modified (byte-faithful), so they remain a bounded residual channel. |
| `src/error.rs` | No edit. The new `ToolError::UpstreamDisconnected` variant carries an accurate doc comment ("transport/connection closed mid-session"). |
| `src/server.rs` | No edit. The implementer/debugger kept the `sanitize_upstream_text` / `sanitize_upstream_identifier` / `sanitize_schema_metadata` rustdoc accurate to the final annotation-only + identifier-vs-display behavior. |
| `src/registry.rs` | No edit. `ensure_fresh` and `map_service_error` doc comments match the shipped behavior (dirty-restore on failure; TransportClosed+TransportSend → disconnected). |
| `src/forward.rs` | No edit. `on_tool_list_changed` doc comment is accurate (per-server dirty flag, no map touch, no await-block). |

## Implementation drift (diff vs master.md Produces)

**Planned but not shipped (all deliberate):**
- `src/cancellation.rs` — never needed (Phase 3 already handled cancellation inline).
- A separate sanitize helper module — the plan allowed "a small helper file only
  if justified"; the helpers live in `src/server.rs` (justified: no cross-module
  reuse), per the plan's stated preference for the smallest footprint.

**Shipped but not planned:** none. All 4 touched `src` files were in the plan's
per-phase Produces. New error code `upstream_disconnected` is within the D-005
additive envelope. No `Cargo.toml`/`Cargo.lock` change; rmcp stays `=1.8.0`.

## Spec / plan drift (THOROUGH audit)

- **TARGETED — schema-sanitization key set (resolved in-cycle).** `master.md`
  Phase 2 listed the sanitized schema keys as "title, description, $comment,
  examples, and enum display strings." The THOROUGH review (general lens F-#1)
  correctly identified that `enum`/`examples` are JSON-Schema **validation
  constants**, not display labels — sanitizing them corrupts the advertised
  schema. Resolution (review.md F3): narrowed to **annotation-only** keys
  (`title`/`description`/`$comment`/`markdownDescription`); validation/structural
  values pass verbatim. This supersedes the plan's wording; it is a corrected
  in-cycle refinement, not an open divergence. No spec file claims the broader
  set, so no spec edit is required — `ARCHITECTURE.md`/`AGG-MCP.md` describe
  "descriptions" generically and remain accurate.
- **No structural drift.** `D-004`/`D-005` (byte-faithful results, structured-
  error shape), `GOTCHA #20` (sanitize+cap, "bounds not eliminates"), and
  `GOTCHA #16`/`D-007` (lock discipline) are all upheld by the implementation.
  `GOTCHA #20`'s ✅ (enforced-by-design/tests) is now genuinely backed by tests.

## Stale-doc flags (outside the touched set — surfaced, not fixed)

- `docs/ARCHITECTURE.md:141` (and `docs/MVP.md` Phase 4 item 1) name the error
  type **`AggError`/`ErrorCode`**; the code uses **`ToolError`** (the accepted
  OQ1 decision — see `state.json` `decisions.error-type-name`). The *public* wire
  contract (the `code` strings + D-005 JSON shape) is unaffected; only the
  internal Rust type name differs. Pre-existing; deliberate. The user may either
  annotate the docs ("internally `ToolError`") or leave them as design-level
  naming. Not edited (outside the touched-doc set; a deliberate naming choice).

## Pending follow-up

1. **End-of-cycle push** lands `v0.5.1..v0.5.6` on `origin/main` (this artifact +
   the `SECURITY.md` edit + the finalized `state.json` are the `v0.5.6` commit).
2. **F4 carry (Phase 5 / robustness):** the send-side mid-session-death path
   (`ServiceError::TransportSend`) is **fixed in code** (`map_service_error` now
   maps it to `upstream_disconnected`), but its dedicated wire test is `#[ignore]`d
   because the OS pipe-closure race makes `TransportSend` vs `TransportClosed`
   non-deterministic at the wire level. Unblock trigger: a transport wrapper that
   forces send-side failure, or a unit test once `map_service_error` is exposed.
3. **`/gate` (optional):** Phase 4 added **no** dependencies, so `cargo audit` /
   `cargo deny` posture is unchanged from Phase 3; the token benchmark is Phase 5.
   No gate run is required by this phase.
4. **MVP checklist:** the Phase 4 verification items (descriptions sanitized +
   truncated; upstream crash mid-session → structured error, siblings unaffected;
   `always_error` round-trips; `needs_sampling` clean reject) are now satisfied
   and covered by the integration suite.
