IMPLEMENT Phases B+C+D (src hardening + hygiene) — oss-readiness. SRC ONLY.

You are the implementer for the source-code phases. Edit ONLY `src/**` EXCEPT
`src/credentials.rs` (managed edit-deny — the orchestrator handles its one line).
Do NOT touch `tests/**` or any doc/`Cargo.toml`. Read
`docs/master-plans/oss-readiness/master.md`, `tests.md`, and the `decisions`
block in `state.json`.

Current gate: `cargo test --all` → 134 passed / 1 failed / 4 ignored. The single
failure is `literal_header_redaction::literal_secret_header_value_is_registered_for_redaction`
(H-3) — your job makes it green. Done-condition: `cargo test --all` 100% green,
`cargo fmt --all -- --check` clean, `cargo clippy --all-targets -- -D warnings`
clean. Do NOT edit tests; if one seems wrong, STOP and write a
`test-issue-<slug>.md` instead.

## H-3 — unconditional header-value redaction (makes the red test pass)

In `src/registry.rs` `get_or_connect`, resolved HTTP header values are registered
for redaction only when the raw template contains `${`:
```
if raw.contains("${") { crate::process::register_secret(&resolved); }
```
Remove the `if raw.contains("${")` guard so EVERY resolved header value is
registered unconditionally (a literal secret in `headers` is then also redacted).
Keep the env-var path behavior unchanged. This is what the H-3 test asserts.

## H-1 — poison-safe global mutexes (no process-wide panic)

In `src/process.rs`, the global `std::sync::Mutex`es (the redaction set and the
writers map — around lines 55, 63, 671, anywhere `.lock().expect(...)` is called
on them) panic the whole proxy if the mutex is ever poisoned. Replace each
`.lock().expect("...")` with `.lock().unwrap_or_else(|p| p.into_inner())` — the
poisoned inner value (a `HashSet`/writers map) is safe to recover. Apply to ALL
such call-sites on those globals.

## H-2 — length-cap `sanitize_upstream_identifier` (defense-in-depth)

In `src/server.rs`, `sanitize_upstream_identifier` strips control chars but does
not length-cap (unlike `sanitize_upstream_text`, which caps at 100). A malicious
upstream can put an over-long tool NAME in its raw `tools/list` JSON. After the
control-char strip, cap the identifier at a generous length (200 chars), on a
CHAR boundary (don't split a UTF-8 codepoint) — mirror how `sanitize_upstream_text`
caps. Add a one-line comment that this is defense-in-depth against a non-rmcp
upstream (rmcp's own registration bounds well-behaved servers).

## H-4 (partial) — remove stale `#[allow(dead_code)]` in error.rs

Remove the `#[allow(dead_code)]` on `ToolError::CredentialResolution`
(`src/error.rs` ~line 66) and its stale "wired by Phase 2" comment — the variant
is constructed in `process.rs`. (The matching attribute on the `CredentialStore`
trait in `src/credentials.rs` is the orchestrator's to remove — do NOT touch that
file.) After removing, confirm `clippy -D warnings` stays clean (the variant IS
used, so no dead-code warning should fire).

## H-5 — `meta_tools` as an associated function

In `src/server.rs`, `meta_tools(&self)` only does a no-op `let _ = &self.config;`
borrow to silence a field-never-read warning, then returns the three static meta
tools. Convert it to an associated fn `fn meta_tools() -> Vec<Tool>` (drop the
`&self` and the no-op borrow); update the call site (in `list_tools`) to
`Self::meta_tools()`. Ensure the `config` field is still considered used
elsewhere — if removing the borrow now triggers a `field never read` warning on
`config`, that means the field is genuinely only used there; in that case keep the
field used via its real consumer or add a minimal justified `#[allow]` with a
clear reason. Prefer finding the real consumer; verify `clippy -D warnings` clean.

## H-6 — document the startup eprintln/tracing split (DO NOT add a pre-parse subscriber)

The finding is about CONSISTENCY, not a bug. `src/main.rs` uses `eprintln!` for
pre-`clap`-parse / pre-tracing-init diagnostics (~lines 122, 177) and `cred list`
output (~342), while the real `tracing` subscriber is initialized later in
`run_serve`. **Do NOT add a global `tracing` subscriber before clap parse** — the
later `run_serve` init is the single global default, and a second `.init()` would
panic/no-op. The correct close: add a brief code comment at the pre-parse
`eprintln!` sites explaining the deliberate rule (diagnostics before
`Cli::parse()` / before tracing-init must use `eprintln!` to stderr; everything
after init goes through `tracing`; `cred list` is intentionally raw stderr for the
test harness). This removes the "which is the rule?" ambiguity without breaking
init. Do not change the runtime behavior.

## H-7 — gate test-only spawn hooks behind `#[cfg(debug_assertions)]`

`src/main.rs` ships test-only machinery in the production binary: the
`__fanin_immediate_descendant__` argv sentinel branch, the
`--spawn-immediate-descendant` config-arg scan, and the 30s marker-writer
(~lines 32-34, 68, 113-187, 211-280 — find the actual spans). Gate ALL of it
behind `#[cfg(debug_assertions)]` so a release build excludes it but `cargo test`
(which builds debug) still sees it. CRITICAL: the Phase-5 containment tests
(`tests/integration/process_lifetime.rs`) drive these hooks — after gating, run
`cargo test --all` and CONFIRM every containment test still passes (they run in
debug, so the hooks are present). If gating them breaks a test, the gating
boundary is wrong — fix the boundary, do NOT edit the test. Keep the CLI flag
hidden (`hide = true`) as before. Ensure no `dead_code`/`unused` warning fires in
release builds for the now-cfg'd items (add `#[cfg(debug_assertions)]` to any
helper fn/const they use too).

## Finish

Run `cargo fmt --all`, then confirm `cargo clippy --all-targets -- -D warnings`
AND `cargo test --all` are clean/green. Also sanity-check a release build compiles
with the cfg-gated code excluded: `cargo build --release` should succeed with no
unused-code warnings. Return as data for the orchestrator: each src file changed
and the core of the change; confirmation H-3 turns the red test green; the H-7
gating boundary and that containment tests still pass; the H-5 resolution for the
`config` field; the final gate numbers; any out-of-scope issue you spotted but did
not touch.
