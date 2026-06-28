FINISH the src implementation — oss-readiness. A prior implementer completed
`src/process.rs` then stalled. Complete the REMAINING findings. SRC ONLY.

Edit ONLY `src/**` EXCEPT `src/credentials.rs` (managed deny — orchestrator
handles its one line). Do NOT touch `tests/**` or docs/Cargo.toml. Read
`docs/master-plans/oss-readiness/master.md`, `tests.md`, and the `decisions`
block in `state.json`.

## ALREADY DONE (do not redo)
- `src/process.rs`: H-1 (the three global-mutex `.expect()` → `.unwrap_or_else(|p|
  p.into_inner())`) is COMPLETE. The process.rs HALF of H-7 is done:
  `spawn_immediate_descendant`, `ImmediateDescendantGuard` (struct + impl) are now
  `#[cfg(debug_assertions)]`. Leave process.rs alone unless a remaining item
  requires a tweak there.

## REMAINING WORK (this is your task)

Current gate: `cargo test --all` → 1 failure
(`literal_header_redaction::...`, H-3). `cargo build` (debug) passes, but
`cargo build --release` is currently BROKEN because `main.rs` still references the
now-cfg-gated `spawn_immediate_descendant`. You must fix that via H-7 below.

### H-3 — unconditional header redaction (makes the red test green) — `src/registry.rs`
In `get_or_connect`, resolved HTTP header values are registered for redaction only
when the raw contains `${`:
```
if raw.contains("${") { crate::process::register_secret(&resolved); }
```
Remove the `if raw.contains("${")` guard so EVERY resolved header value is
registered unconditionally. (Env-var path unchanged.)

### H-7 (main.rs half) — gate test hooks behind `#[cfg(debug_assertions)]` — `src/main.rs`
The production binary ships test-only machinery: the `__fanin_immediate_descendant__`
argv sentinel branch, the `--spawn-immediate-descendant` config-arg scan, and the
30s marker-writer (find the spans). Gate ALL of it behind `#[cfg(debug_assertions)]`
so release builds exclude it AND so main.rs no longer references the cfg-gated
`process::spawn_immediate_descendant` in a release build. Keep the CLI flag hidden
(`hide = true`). CRITICAL acceptance: `cargo build --release` must succeed with NO
unused/dead-code warnings, and `cargo test --all` (debug) must keep every
containment test in `tests/integration/process_lifetime.rs` GREEN (debug builds see
the hooks). If gating breaks a test, the boundary is wrong — fix the boundary, do
NOT edit the test.

### H-2 — length-cap `sanitize_upstream_identifier` — `src/server.rs`
After the control-char strip, cap the identifier at 200 chars on a CHAR boundary
(don't split a UTF-8 codepoint) — mirror `sanitize_upstream_text`'s cap. One-line
comment: defense-in-depth against a non-rmcp upstream sending an over-long raw tool
name.

### H-5 — `meta_tools` as associated fn — `src/server.rs`
Convert `meta_tools(&self)` (it only does a no-op `let _ = &self.config;`) to
`fn meta_tools() -> Vec<Tool>`; update the call site to `Self::meta_tools()`. If
dropping the `&self.config` borrow now triggers a `field never read` warning on
`config`, find the field's real consumer; if there genuinely is none, keep a
minimal justified `#[allow(dead_code)]` with a clear reason. Verify `clippy -D
warnings` clean.

### H-4 (error.rs half) — remove stale `#[allow(dead_code)]` — `src/error.rs`
Remove the `#[allow(dead_code)]` (and its stale "wired by Phase 2" comment) on
`ToolError::CredentialResolution` (~line 66). The variant is constructed in
process.rs, so no dead-code warning should fire. (The `CredentialStore` trait
attribute in credentials.rs is the orchestrator's — do NOT touch that file.)

### H-6 — document the startup eprintln/tracing split — `src/main.rs`
Do NOT add a global tracing subscriber before clap parse (the `run_serve` init is
the single global default; a second `.init()` would panic/no-op). Just add a brief
comment at the pre-parse `eprintln!` sites (~lines 122, 177) explaining the rule:
diagnostics before `Cli::parse()` / before tracing-init use `eprintln!`; everything
after init uses `tracing`; `cred list` is intentionally raw stderr for the test
harness. No behavior change.

## Finish
Run `cargo fmt --all`; confirm `cargo clippy --all-targets -- -D warnings`,
`cargo test --all` (100% green), AND `cargo build --release` (clean, no warnings).
Return as data for the orchestrator: each src file changed + the core change; that
H-3 turns the red test green; the H-7 main.rs gating boundary and that release
builds clean + containment tests stay green; the H-5 `config`-field resolution; the
final gate numbers; any out-of-scope issue spotted but not touched.
