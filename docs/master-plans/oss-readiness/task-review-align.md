## LENS: ALIGNMENT — write review-align.md

Verify the change honors the canon, the plan, and the locked decisions:

- **O-1 / D-017.** `Cargo.toml`: are `repository`, `homepage`, `readme`,
  `keywords` (≤5, each ≤20 chars), `categories` (valid crates.io categories)
  correct and well-formed? Is `publish = true` consistent with D-017 (name + dual
  license) and the LICENSE-MIT/LICENSE-APACHE files present? Version still 0.1.0
  per the decision? Would `cargo publish --dry-run` plausibly succeed (readme
  path exists, license files referenced)?
- **O-2.** SECURITY.md contact is GitHub Security Advisories (the locked
  decision), no leaked email, 72h intent retained.
- **D-2.** Is `--passthrough-stderr` struck from ALL docs (ARCHITECTURE, STACK,
  GOTCHA #29, PRD) with surrounding prose left correct? Any dangling reference?
- **D-005 / D-010 / D-009 / D-015.** Public error shape unchanged (H-4 only
  removed dead_code attrs); secrets discipline intact (H-3 strengthens it);
  containment unchanged (H-7 only cfg-gates TEST hooks, not real containment);
  rmcp pin still `=1.8.0`, no dep added.
- **Anti-stack / no-runtime-deps / single-binary.** Nothing in the change adds a
  runtime dep or violates the identity. CONTRIBUTING.md correctly states these.
- **GOTCHA consistency.** Does GOTCHA #29 still read correctly after the D-2
  strike? Do the H-* changes contradict any ✅ GOTCHA item?
- **Scope.** Confined to O-1/O-2/O-3, D-2, H-1..H-8? Any change outside the nine
  findings? (The plan's Scope-out is binding.)
- **Docs to update for knowledge-sync:** note (do not edit) any binding doc that
  should record what shipped (e.g. SECURITY.md already got the H-8 note; is
  anything else now stale?).
