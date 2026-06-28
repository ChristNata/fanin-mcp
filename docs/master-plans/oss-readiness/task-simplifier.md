SIMPLIFY the oss-readiness src change set. Behavior-preserving only.

You are the simplifier. Tier thorough. Review ONLY this cycle's src changes and
look for genuine, behavior-preserving simplifications. Write
`docs/master-plans/oss-readiness/simplify.md` per plan-format.

## Scope (only files this cycle touched)
`src/process.rs` (H-1 mutex recovery, H-7 cfg-gating, import gating),
`src/registry.rs` (H-3 unconditional header registration), `src/server.rs`
(H-2 cap + named CAP const, H-5 meta_tools assoc fn, dropped config field),
`src/error.rs` (H-4), `src/main.rs` (H-6 comment, H-7 cfg-gating + cfg'd Cli
field), `src/credentials.rs` (H-4 trait attr removed — but this file is under a
managed edit-deny: do NOT edit it; only read). See `git diff` across the
oss-readiness commits. Context: `master.md`, `review.md`, `fix-polish.md`.

## Rules
- Behavior-preserving. Gate must stay green: `cargo test --all` 135/0/4,
  `cargo fmt --all -- --check` clean, `cargo clippy --all-targets -- -D warnings`
  clean, AND `cargo build --release` clean (0 warnings — the H-7 cfg-gating).
- Do NOT edit `tests/**` or `src/credentials.rs`.
- No invented churn. The change set already went through a debugger polish pass
  (named the H-2 const, dropped a dead field, de-duped a comment, cfg-gated a
  field). If nothing remains worth changing, say so and change nothing —
  "nothing worth changing" is a correct, expected result here.
- The H-7 `#[cfg(debug_assertions)]` attributes are load-bearing (release
  correctness) — do not remove or consolidate them in a way that breaks the
  release build. Re-run `cargo build --release` after any change near them.

## Known deferred item (DO NOT fix — it's test code, out of your scope)
T4: `HeaderSeen`/`start_http_probe` are duplicated across two test files. That is
test-creator's domain, not yours. Note it in simplify.md "issues spotted" only.

After any change, run fmt + clippy + `cargo test --all` + `cargo build --release`
and confirm green/clean. Return as data for the orchestrator: what you changed and
why (or that nothing was worth changing), and the final gate numbers.
