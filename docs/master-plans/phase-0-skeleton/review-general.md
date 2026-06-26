# review-general: phase-0-skeleton

Found 0 blocker, 0 structural, 1 targeted, 1 trivial.

## Verification

- `cargo build` — PASS.
- `cargo check --workspace --all-targets` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- `cargo nextest run --workspace` — PASS: 14 passed, 2 skipped.
- `cargo fmt --all -- --check` — FAIL: formatting drift in `src/`,
  `tests/probe-server/main.rs`, and test harness files.
- `cargo test --workspace --doc` — FAIL: `no library targets found in package
  fanin-mcp`; this is non-applicable to the current binary-only crate shape.

## Genuine defects

- File: `Cargo.toml:32`
  Severity: targeted
  Pass: general
  What: `rmcp` enables the `macros` feature, but Phase 0 uses manual tool
    definitions and no `#[tool]` macro.
  Why: This widens the compiled feature surface without a current use. The
    project sells a small, static Rust binary and `rust-general` requires
    explicit dependency features. Unused feature flags create maintenance and
    supply-chain weight that future reviewers will assume is intentional.
  Cite: `rust-general` §Dependency hygiene; `rmcp-general` §Meta-tools,
    static descriptions.
  Fix: Remove `macros` from the `rmcp` feature list until a later phase
    actually uses `#[tool]`, or add the concrete macro use that justifies it.

- File: `src/main.rs:76`
  Severity: trivial
  Pass: general
  What: The tree is not `rustfmt`-clean.
  Why: `cargo fmt --all -- --check` reports formatting drift, including the
    over-indented doc comment in `CredAction::Set`, import wrapping in
    `src/server.rs`, missing final newlines in stub modules, and formatting
    drift in the probe/test files. Formatting is cosmetic, but it is the Rust
    style authority and should not ship red.
  Cite: `rust-general` §Styling; `rust-review` §Stack & versions.
  Fix: Run `cargo fmt --all` in a write-capable stage and commit the formatter
    output.

## Simplify-stage suggestions

- File: `src/main.rs:1`, `src/server.rs:1`, `tests/probe-server/main.rs:1`
  Severity: trivial
  Pass: general
  What: Source comments duplicate plan and GOTCHA text at high density.
  Why: The invariants are right, but much of the phase narrative belongs in
    `master.md` and `docs/GOTCHA.md`, not every implementation module. The
    current density makes the small skeleton look more complex than it is and
    raises future edit cost when phase names or docs move.
  Cite: `capital-style` §Simple over engineered; `md-authoring` §Never.
  Fix: Keep the load-bearing boundary comments (`stdout is transport`, D-005
    tool-level errors, D-007 lock discipline) and trim repeated phase history
    during simplify.

- File: `src/server.rs:168`, `tests/probe-server/main.rs:173`
  Severity: trivial
  Pass: general
  What: JSON-schema object construction is duplicated in the aggregator and
    probe fixture.
  Why: Duplication is acceptable at Phase 0, but it will become noisy once more
    tool schemas land. Avoid a library split; just extract a tiny local helper
    if the pattern grows.
  Cite: `rust-general` §Type-level patterns; `capital-style` §Simple over
    engineered.
  Fix: Defer until a second production schema needs it, then centralize the
    object-schema builder without introducing a new crate or workspace.
