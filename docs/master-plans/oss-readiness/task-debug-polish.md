FIX review polish findings — oss-readiness. SRC + DOCS (no tests).

You are the debugger. Apply exactly these fixes from
`docs/master-plans/oss-readiness/review.md`. Edit ONLY: `src/main.rs`,
`src/server.rs`, `SECURITY.md`, `STACK.md`, `CONTRIBUTING.md`. Do NOT touch
`tests/**`, `src/registry.rs`, `src/process.rs`, or `src/credentials.rs`. Write a
`docs/master-plans/oss-readiness/fix-polish.md` summary.

Done-condition: `cargo fmt --all -- --check` clean, `cargo clippy --all-targets --
-D warnings` clean, `cargo test --all` 100% green (135/0/4), `cargo build
--release` clean (0 warnings).

## A4 — duplicate `schemars` row — `STACK.md`
Lines ~28-29 have two IDENTICAL `schemars` rows in the Core Crates table. Delete
ONE (keep a single row). Verify no other table row was duplicated by the earlier
doc edit.

## A3 — cfg-gate the `Cli.spawn_immediate_descendant` field — `src/main.rs`
The field (~lines 68-70) is declared unconditionally though every consumer is
`#[cfg(debug_assertions)]`. A release binary parses the hidden flag and silently
ignores it. Add `#[cfg(debug_assertions)]` to the field declaration. Confirm
`cargo build --release` still compiles (field absent) AND `cargo test --all`
(debug) still parses/uses it.

## A5 — document the H-3 over-redaction tradeoff — `SECURITY.md`
At the H-8 redaction-scope note (~line 25), add one sentence: since H-3, EVERY
value resolved from a server's `[headers]` is registered for redaction — literal
(non-`${VAR}`) values included — so if a header value collides with text your
tracing layer emits, that line is masked; choose header values distinct from
operational log text.

## T2 — de-duplicate the H-6 comment — `src/main.rs`
The same 3-line comment appears verbatim at both pre-parse `eprintln!` sites
(~125-127 and ~186-188). Keep ONE canonical explanation; trim the second to a
one-line pointer (e.g. `// pre-tracing-init diagnostic — see rationale above.`).

## T3 — `Aggregator.config` unread field — `src/server.rs`
After H-5, the `config: CliConfig` field (~lines 40-41) is stored but never read,
kept behind `#[allow(dead_code)]`. PREFER dropping the field: remove it from the
struct, from `Aggregator::new` / `Aggregator::with_registry` signatures, and from
every call site (thread nothing that isn't used). Remove the `#[allow(dead_code)]`.
If — and only if — a call site genuinely needs to pass config for a real current
use, instead keep the field and replace the `#[allow]` comment with a concrete
justification naming that use. Verify `clippy -D warnings` clean either way.

## T5 — CONTRIBUTING config-path pointer — `CONTRIBUTING.md`
After the Build+Run block, add ONE line pointing first-run users at the config
path, e.g. `Config path: see README Quick Start (per-OS defaults).` Keep it a thin
pointer; do not copy the per-OS table.

## T6 — name the H-2 cap constant — `src/server.rs`
`sanitize_upstream_identifier` uses an inline `200`. Introduce `const CAP: usize =
200;` (mirroring `sanitize_upstream_text`'s named `CAP`/100 idiom) and use it.
Pure readability; no behavior change.

## Finish
Run fmt, clippy, `cargo test --all`, AND `cargo build --release`; confirm all
clean/green. In `fix-polish.md`: each finding, the change, files touched, the T3
decision (dropped vs kept-with-justification), and the final gate numbers.
