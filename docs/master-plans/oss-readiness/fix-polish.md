# Fix Polish — oss-readiness review findings

**Scope:** `src/main.rs`, `src/server.rs`, `SECURITY.md`, `STACK.md`, `CONTRIBUTING.md` only. No tests, no registry/process/credentials.

## A4 — duplicate `schemars` row (STACK.md)
- Defect: Lines 28-29 contained two identical `schemars` rows.
- Fix: Deleted the duplicate row; single canonical entry remains.
- Verified: No other table rows duplicated.

## A3 — cfg-gate `spawn_immediate_descendant` (src/main.rs)
- Defect: Field declared unconditionally; release binary parsed a hidden no-op flag.
- Fix: Added `#[cfg(debug_assertions)]` to the field declaration (line ~68).
- Verified: `cargo build --release` succeeds (field absent); debug builds and tests continue to parse/use it.

## A5 — H-3 over-redaction tradeoff (SECURITY.md)
- Defect: H-8 redaction note silent on literal header values.
- Fix: Added one sentence after the H-8 scope note: every `[headers]` value (literal or `${VAR}`) is registered; choose header values distinct from operational log text.
- Files: SECURITY.md:25.

## T2 — de-duplicate H-6 comment (src/main.rs)
- Defect: 3-line comment duplicated verbatim at the two pre-tracing `eprintln!` sites.
- Fix: Kept the canonical explanation at the first site; replaced the second with a one-line pointer (`// pre-tracing-init diagnostic — see rationale above.`).
- Files: src/main.rs (two sites).

## T3 — `Aggregator.config` unread field (src/server.rs)
- Decision: **Dropped** (preferred path).
- Changes: Removed `config: CliConfig` field, the `#[allow(dead_code)]`, the import, both constructor signatures, and all call sites. `Aggregator::new()` and `Aggregator::with_registry(...)` now take only the parameters they actually use.
- Justification: No current consumer reads the carried config; future use can re-introduce a justified field.
- Verified: `clippy -D warnings` clean.

## T5 — CONTRIBUTING config-path pointer
- Fix: Added one thin pointer line after the Build+Run block: `Config path: see README Quick Start (per-OS defaults).`
- Files: CONTRIBUTING.md.

## T6 — name the H-2 cap constant (src/server.rs)
- Fix: Introduced `const CAP: usize = 200;` inside `sanitize_upstream_identifier`, mirroring the named-cap idiom in `sanitize_upstream_text`. Pure readability; behavior unchanged.
- Files: src/server.rs:393.

## Final gate results
- `cargo fmt --all -- --check`: clean
- `cargo clippy --all-targets -- -D warnings`: clean
- `cargo test --all`: 135 passed, 0 failed, 4 ignored
- `cargo build --release`: clean (0 warnings)

All edits confined to the five named files. No test files touched. No structural escalation required.
