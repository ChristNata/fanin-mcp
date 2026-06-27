# Knowledge-Sync: remediation-s1-d1

**Summary:** 2 docs updated (GOTCHA.md, DECISIONS.md), 0 created, 3 spec edits
applied (all targeted, shipped-state), 3 stale-doc flags surfaced (out of cycle
scope). Tier THOROUGH. Reconciled against the merged diff `6d5b66c..eddb4a7`
(pushed to origin/main).

## Change set (diff truth)

Shipped `src/`: `config.rs`, `error.rs`, `process.rs`, `registry.rs`. Matches the
plan's Phase-1/Phase-2 Produces lists exactly (test files were authored by
test-creator in the test stage). No planned-not-shipped, no shipped-not-planned.

## Per-doc updates (applied, staged)

- **docs/GOTCHA.md #30 (Morph wrong-tree)** — targeted. The fix line described an
  abstract "set current_dir at spawn / pass via args"; updated to name the
  now-shipped per-server `cwd` field (`${VAR}`, stdio-only, HTTP-ignored,
  empty/whitespace rejected at load + post-resolution) and marked ✅.
- **docs/GOTCHA.md #32 (new)** — targeted/additive. Captures the S-1 trap in the
  house symptom→cause→fix ✅ format: only `tools/call` was inside `timeout_secs`;
  connect handshake / initial `list_all_tools` / `list_changed` refetch were
  unbounded with the init guard held across connect; now all four awaits are
  bounded, with no-cache/guard-release/containment-reap on expiry.
- **docs/DECISIONS.md D-012** — targeted. "Every upstream call" was ambiguous and
  the S-1 gap exploited it; tightened to "every *blocking upstream await*" and
  enumerated the four covered awaits + the connect-timeout cleanup semantics.

## Spec drift findings

| Doc | Severity | Resolution |
|---|---|---|
| GOTCHA #30 | targeted | applied (names shipped `cwd`, ✅) |
| GOTCHA #32 | targeted | applied (new trap-now-enforced entry) |
| DECISIONS D-012 | targeted | applied (envelope scope clarified) |
| ARCHITECTURE.md "Child working directory" (65-97) | none | already accurate; code matches spec verbatim — no edit |

No structural drift: the code converged *toward* the specs (closed gaps), it did
not diverge from them.

## Stale-doc flags (surfaced, NOT edited — outside S-1/D-1 scope)

- **`--passthrough-stderr`** referenced in GOTCHA #29, STACK.md, ARCHITECTURE.md
  but unimplemented (`grep passthrough src/` → 0). Review finding D-2 / deep-dive
  #11. Decide: implement or strike from docs.
- **MVP.md checklist** boxes (e.g. line 105 per-server `timeout_secs`) still `[ ]`
  though Phase-3 timeouts + now the S-1 hardening + D-1 `cwd` shipped. The
  original full-codebase review already flagged "MVP checklist sign-off" as
  stale — a project-wide sign-off pass, not this cycle's.
- **OSS-readiness (O-1/O-2/O-3):** `Cargo.toml` `publish=false` + missing
  metadata; `SECURITY.md` `<SECURITY_CONTACT_EMAIL>` placeholder; no
  CONTRIBUTING.md. Doc/metadata work pending a follow-up.

## Pending

Staged doc edits await the follow-up `/push`. The remaining review findings
(D-2, O-1/O-2/O-3, H-* hygiene) are not part of this cycle.
