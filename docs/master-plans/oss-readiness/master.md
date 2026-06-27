---
Feature: oss-readiness
Scope: flat
Stack: rust
Tier: thorough
Status: draft
Created: 2026-06-28
Target: Cargo.toml, SECURITY.md, CONTRIBUTING.md, src/{process.rs,server.rs,main.rs,error.rs,registry.rs}
Dependencies: none
---

# What
A thorough remediation plan that closes every remaining full-codebase-review finding (O-1/O-2/O-3, D-2, H-1..H-8) after the merged S-1/D-1 cycle. Produces only `master.md`; no code, no `state.json`.

# Why
The synthesis (`review-SYNTHESIS.md`) and the two adversarial/deep-dive lenses (`review-adversarial-deepseek.md`, `review-deepdive-minimax.md`) left a short, concrete list of OSS-release blockers, one documented-but-unimplemented flag, four hardening items, and four hygiene items. All are file-disjoint or trivially sequenced; none re-touch S-1/D-1. The work is the final gate before the project can claim “production + OSS ready.”

# Dependencies
- Prior remediation cycle `docs/master-plans/remediation-s1-d1/` (merged) already closed S-1 and D-1.
- Binding canon: `docs/DECISIONS.md` (D-005, D-009, D-010, D-017), `docs/SECURITY.md`, `STACK.md`, `ROADMAP.md`, `docs/ARCHITECTURE.md`, `docs/GOTCHA.md`.
- Current tree facts verified in-source: `Cargo.toml` still `publish=false` + minimal `[package]`, `SECURITY.md` still contains the placeholder, no `CONTRIBUTING.md`, `src/credentials.rs` under managed edit-deny.

# Scope

## In
- O-1: `Cargo.toml` metadata + publish flag decision.
- O-2: `SECURITY.md` security-contact placeholder (Open Question surfaced).
- O-3: new one-page `CONTRIBUTING.md`.
- D-2: decide/implement/strike `--passthrough-stderr`.
- H-1: poison-safe mutex recovery in `process.rs`.
- H-2: length cap (or documented rmcp bound) for `sanitize_upstream_identifier`.
- H-3: unconditional header redaction registration in `registry.rs`.
- H-4: remove two stale `#[allow(dead_code)]` (flag credentials.rs edit-deny).
- H-5: convert `meta_tools(&self)` to associated fn.
- H-6: minimal stderr-only subscriber before clap parse.
- H-7: decide gating strategy for test-only spawn hooks (test-contract risk).
- H-8: SECURITY.md note documenting exact-substring redaction scope.

## Out
- Any change to `src/credentials.rs` (orchestrator-only).
- Any re-work of S-1 or D-1.
- Any new feature, any version bump off 0.1.0, any rmcp pin change.
- Any test-file edit (read-only contract).
- Any `cwd` implementation (already closed as D-1).
- Any timeout-envelope work (already closed as S-1).
- Any change outside the nine findings listed above.

# Phases

## Phase A — Docs & Metadata (O-1, O-2, O-3, H-8)
**Scope:** Pure documentation + Cargo metadata. No src/ changes.  
**Produces:** updated `Cargo.toml`, `SECURITY.md`, new `CONTRIBUTING.md`.  
**Key Behaviors:** Add repository/homepage/readme/keywords/categories; decide publish flag (recommend true); surface O-2 as Open Question with GitHub-Security-Advisories default; write capital-style one-page CONTRIBUTING.md covering rmcp exact-pin, DECISIONS.md, gate commands, GOTCHA list, no-runtime-deps; add H-8 exact-substring note to SECURITY.md.  
**Depends On:** none.  
**Skills Needed:** plan-format, md-authoring, capital-style.  
**Phase Success Criteria:**
- `Cargo.toml` contains the five metadata keys and `publish = true`.
- `SECURITY.md` contains a real contact decision (or explicit Open Question) and the H-8 redaction-scope paragraph.
- `CONTRIBUTING.md` exists at repo root and matches the required content list.

## Phase B — Hardening (H-1, H-2, H-3)
**Scope:** Behavioral changes in `process.rs`, `server.rs`, `registry.rs`. Requires tests.  
**Produces:** poison-safe mutex recovery, length-capped (or documented) identifier sanitiser, unconditional header redaction.  
**Key Behaviors:** Replace `.expect()` with `.unwrap_or_else(|p| p.into_inner())`; add 200-char cap (or rmcp-bound doc note) to `sanitize_upstream_identifier`; register every resolved header value for redaction.  
**Depends On:** Phase A (no file overlap).  
**Skills Needed:** rust-general, rmcp-general.  
**Phase Success Criteria:**
- No `.expect()` on the two global `Mutex`es remains.
- `sanitize_upstream_identifier` either caps at 200 or carries an explicit rmcp-bound comment.
- `registry.rs` registers header values unconditionally.

## Phase C — Hygiene (H-4, H-5, H-6, D-2-if-implemented)
**Scope:** Small src/ clean-ups; D-2 decision executed here.  
**Produces:** removal of two stale attributes, `meta_tools()` as associated fn, minimal stderr subscriber, and (if chosen) the passthrough flag.  
**Key Behaviors:** Delete `#[allow(dead_code)]` on `CredentialResolution` and the trait (flag credentials.rs); change `meta_tools(&self)` signature; add pre-clap stderr subscriber while preserving `cred list` raw output; implement or strike passthrough-stderr per D-2 decision.  
**Depends On:** Phase B (server.rs / process.rs overlap).  
**Skills Needed:** rust-general.  
**Phase Success Criteria:**
- Both stale `#[allow(dead_code)]` attributes are gone (or credentials.rs edit is explicitly routed to orchestrator).
- `meta_tools` is an associated function.
- Startup path uses a minimal stderr subscriber; `cred list` still emits raw names.
- D-2 decision recorded and (if implement) the flag exists without violating GOTCHA #1.

## Phase D — Test-Only Spawn Hooks (H-7)
**Scope:** Isolated decision on production-binary pollution.  
**Produces:** either `#[cfg(debug_assertions)]` gating or explicit routing to test-creator.  
**Key Behaviors:** Verify that `cargo test` (debug) still sees the hooks; confirm no test asserts the current argv shape; state whether gating preserves the read-only test contract.  
**Depends On:** none (parallelisable with A).  
**Skills Needed:** rust-general, rust-test.  
**Phase Success Criteria:**
- Decision recorded: gating keeps every containment test green, or test-creator work is required.
- No silent test-contract violation.

# Success Criteria
1. Every finding O-1/O-2/O-3/D-2/H-1..H-8 is either closed or has an explicit Open Question for the user.
2. All phases gate independently; no phase touches a file another phase is editing in the same compile unit.
3. 100 % of existing tests continue to pass (read-only contract preserved).
4. No violation of D-005, D-009, D-010, GOTCHA #1, no-runtime-deps promise, or rmcp `=1.8.0` pin.
5. `src/credentials.rs` is never edited by an implementer child.

# Constraints / Invariants
- Tests are a read-only contract; only test-creator may edit them.
- Preserve D-005 public error shape, D-009 containment, D-010 secrets discipline, GOTCHA #1 stdout transport, the single-static-binary promise, and the exact rmcp pin.
- Scope is exactly the nine findings; no new features, no S-1/D-1 rework.
- `src/credentials.rs` is under managed edit-deny; any required change must be flagged for orchestrator handling.

# Open Questions (with recommended defaults)
- **O-2 security contact:** GitHub Security Advisories private link (no email to leak) — user must confirm or supply an alias.
- **D-2 passthrough-stderr:** Strike the flag from all docs (lower risk, fully honest) — implement only if user explicitly chooses the debug-only mirror.
- **O-1 publish flag + version:** Add metadata and flip `publish=true` now; leave version at 0.1.0 for the release process.
- **H-7 gating:** Gate behind `#[cfg(debug_assertions)]`; verify tests still pass before routing to test-creator.

# Blocking Drift
None. All source claims in the synthesis were re-verified against the actual files listed in the per-lens deep-dives. The prior remediation cycle already closed S-1/D-1; this plan does not re-touch those items.
