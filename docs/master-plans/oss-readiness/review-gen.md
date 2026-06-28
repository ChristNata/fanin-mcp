# review-gen — oss-readiness, general lens

Lens: general (code/doc quality + OSS-reader polish).
Scope: the change set only — files in `git diff HEAD~1 -- .` (excluding `docs/master-plans/`).
Gate context: fmt clean, clippy `-D warnings` clean, `cargo test --all` 135/0/4, `cargo build --release` clean.

## Summary

The change set is mostly polished and OSS-ready. The seven phase deliverables
(CONTRIBUTING.md, Cargo.toml metadata, SECURITY.md H-8 + GitHub Advisories, the
H-1/H-2/H-3/H-4/H-5/H-6/H-7/H-8 src edits, and the D-2 doc strikes) are present
and on-spec. The general-lens sweep finds **one `targeted` defect** (a duplicate
`schemars` row in `STACK.md` introduced by the D-2 strike) and a small handful of
`trivial` polish items. No `blocker` or `structural` findings.

---

## Findings

### T1 — Duplicate `schemars` row in `STACK.md`

- **Severity:** targeted
- **Location:** `STACK.md:28-29`
- **Issue:** The change set left two identical rows in the "Core Crates" table:

  ```
  | `schemars` | JSON Schema helpers | Meta-tool input schemas ... |
  | `schemars` | JSON Schema helpers | Meta-tool input schemas ... |
  ```

  This is an OSS-reader-facing artifact of the D-2 strike: the strike targeted
  the `passthrough-stderr` reference that previously occupied the same row,
  and the new line was inserted *above* the existing one rather than *replacing*
  it (the original `schemars` row was untouched because D-2 said "grep
  `passthrough` first" — and a duplicate row, not a missing one, is the
  outcome). A first-time contributor reading `STACK.md` will see two rows and
  reasonably ask whether `schemars` is double-counted as a direct dependency or
  whether the file has a typo. The table is rendered in crates.io README,
  GitHub repo sidebar, and any third-party doc that mirrors `STACK.md`, so the
  duplicate ships externally.
- **Suggested fix:** delete one of the two rows. The remaining row at `STACK.md:28`
  is sufficient; nothing else depends on the second copy.
- **Routing:** implementer; one-line edit; gate already green so no
  re-verification needed beyond `cargo fmt --check`.

### T2 — Identical 3-line comment block duplicated in `src/main.rs`

- **Severity:** trivial
- **Location:** `src/main.rs:125-127` and `src/main.rs:186-188`
- **Issue:** Both pre-`Cli::parse()` / pre-tracing-init `eprintln!` sites carry the
  same three-line comment verbatim:

  ```
  // Diagnostics before `Cli::parse()` / before tracing-init use eprintln!;
  // everything after init uses tracing; `cred list` is intentionally raw stderr
  // for the test harness.
  ```

  The duplication is the natural reading of the H-6 brief (the task names both
  sites), and the explanation is correct at both. But for an OSS reader the
  repeated prose reads like a copy-paste habit rather than a deliberate note at
  each pre-init call site. Either keep one canonical note (e.g. a module-level
  comment near the `eprintln!` rationale) and a one-liner at the second site, or
  collapse both to a short pointer like `// pre-tracing-init: see rule above`.
- **Suggested fix:** leave one canonical explanation; trim the second to a
  one-line pointer. Pure cosmetic; no behavior change.
- **Routing:** implementer (or simplify pass).

### T3 — `Aggregator.config` field kept behind `#[allow(dead_code)]` is awkward

- **Severity:** trivial
- **Location:** `src/server.rs:40-41`
- **Issue:** After H-5 promoted `meta_tools` to an associated function, the
  `config: CliConfig` field on `Aggregator` is no longer read by any method.
  The field is retained with `#[allow(dead_code)]` and a comment that says it
  is "Carried verbatim from `--namespace` / `--config` for later phases." That
  is a sensible read of the H-5 task ("make `meta_tools()` clean and the
  `config`-field resolution sensible"), and the field is genuinely used as
  input to `Aggregator::new` / `with_registry` — but the `# D-007`-style
  comment does not say *which* later phase or what shape the read takes, so a
  first-time contributor sees a field whose only purpose is "exists for
  future." That is precisely the smell `#[allow(dead_code)]` papers over.
  Two cleaner alternatives: (a) drop the field entirely and let the
  constructors take only what Phase 1 actually uses (`registry`, `namespace`),
  threading `CliConfig` through the one callsite that wants it; or (b) leave
  the field, drop the `#[allow(dead_code)]`, and accept a release-build
  warning *or* read the field once at construction time (e.g. into a `tracing`
  info event) so the warning stays away on its own merit.
- **Suggested fix:** option (a) if the field genuinely has no Phase 1 use
  (which is the case after H-5), drop it and remove the `#[allow]`. Option (b)
  if the field must stay for an explicit Phase 2/3 use that is named in the
  comment. Either way, the OSS reader should not have to read between the
  lines of an `#[allow(dead_code)]` to learn the project's intent.
- **Routing:** implementer; the right call depends on whether `config` has a
  concrete Phase 2 use, which the agent reading `master.md` "Depends On" /
  "Out" lists cannot determine — surface to the orchestrator if in doubt.

### T4 — `HeaderSeen` test fixture is duplicated near-verbatim across two files

- **Severity:** trivial
- **Location:** `tests/integration/literal_header_redaction.rs:21-121` and
  `tests/integration/http_upstream.rs:16-117`
- **Issue:** The new `literal_header_redaction` test imports a near-identical
  copy of the `HeaderSeen` struct + `start_http_probe` helper that already
  lives in `http_upstream.rs` (only differences: probe returns `format!("http://{addr}")`
  vs. `format!("http://{addr}/mcp")`, and the body emission shape is marginally
  different — `to_string()` on the JSON value then concatenated in the
  header, vs. `serde_json::to_vec(&body)` with a separate `write_all`). This
  is acceptable as long as both files live side-by-side, but it is the kind
  of drift that hides a future contract change in one and not the other.
- **Suggested fix:** extract `HeaderSeen` and `start_http_probe` to
  `tests/common/` (the existing shared test module) parameterized on the
  endpoint suffix and the response-body emission style. Not blocking; the
  duplication is bounded to two files and is currently equivalent enough that
  the test contract is preserved.
- **Routing:** simplify pass or follow-up cleanup.

### T5 — CONTRIBUTING.md gives no config-path hint for first-run users

- **Severity:** trivial
- **Location:** `CONTRIBUTING.md:3-7`
- **Issue:** "Build + Run" jumps straight from `cargo build --release` to a
  bare statement that the binary has zero runtime deps. A first-time
  external contributor — the audience of a CONTRIBUTING.md — will next ask:
  "where does the config file go?" `STACK.md` and `README.md` both name the
  per-OS path (`%APPDATA%\fanin-mcp\config.toml`, `~/.config/fanin-mcp/config.toml`),
  but CONTRIBUTING does not cross-reference them. The README Quick Start is
  the closest pointer, and an OSS reader following `CONTRIBUTING.md → build →
  run → ???` has to backtrack to README.
- **Suggested fix:** add one line after the binary-build block, e.g.
  `Config path: see README §Quick Start (per-OS defaults).` This is a
  deliberate thin pointer — keep CONTRIBUTING one page — not a copy of the
  per-OS table.
- **Routing:** implementer (cosmetic doc tweak).

### T6 — H-2 cap sits *after* a `String` round-trip that already discards multibyte info

- **Severity:** trivial
- **Location:** `src/server.rs:397-414`
- **Issue:** `sanitize_upstream_identifier` does the control-char strip and
  `trim()` on a `String`, then re-iterates the trimmed string with `chars()`
  to apply the 200-cap. The cap is on a char boundary (no multibyte panic —
  good, that is the H-2 brief) but the code reads as a two-step `.trim()` →
  re-collect with cap, with a comment ("Defense-in-depth against a non-rmcp
  upstream sending an over-long raw tool name") that does not name the
  *constant*. The constant `200` is inline; `sanitize_upstream_text` a few
  lines above uses a named `const CAP: usize = 100;` at line 393 and then
  `take(CAP)`. Mirroring that idiom here (`const CAP: usize = 200;` near the
  comment) would make the cap discoverable and consistent with the sibling
  function.
- **Suggested fix:** introduce `const CAP: usize = 200;` near the top of
  `sanitize_upstream_identifier` (or hoist both caps to a single
  `const fn`s pair in one place) and use `.chars().take(CAP).collect()`. Pure
  readability; no behavior change.
- **Routing:** implementer or simplify pass.

### T7 — D-2 strikes are otherwise clean

- **Severity:** (clean)
- **Locations:** `STACK.md:26-29` (apart from T1), `docs/ARCHITECTURE.md:36`
  and `docs/ARCHITECTURE.md:163`, `docs/PRD.md:55`, `docs/GOTCHA.md:86`.
- **Note:** The strike itself was executed correctly. The four touched sites
  read naturally post-strike: ARCHITECTURE.md line 163 now ends with "written
  to the log file." (period, no trailing clause); GOTCHA #29 ends with
  "write to the log file. ✅" (the semicolon-then-`--passthrough-stderr for
  debugging only` clause was removed cleanly, no dangling comma, no
  double-space, no orphan "for"); PRD #12 ends cleanly at "aggregator's log
  file (never mixed into the aggregator's own stderr)." with the optional
  flag clause gone. ARCHITECTURE.md line 36 also dropped
  `--passthrough-stderr` from the CLI-flag list. `grep -i passthrough docs/`
  on the bound docs (ARCHITECTURE, GOTCHA, PRD, STACK, SECURITY, README,
  CONTRIBUTING, DECISIONS) returns the intentional D-004 "byte-faithful
  passthrough" hits (which are about D-004 byte-faithful result passing,
  unrelated) and zero unintended `--passthrough-stderr` references. **D-2 is
  satisfied.**

### T8 — `Cargo.toml` metadata is tidy

- **Severity:** (clean)
- **Location:** `Cargo.toml:1-13`
- **Note:** Keys are added in a sensible order (after `description` and before
  the auto-discovery comment, matching how crates.io renders the `[package]`
  table). `keywords` are 5 of ≤20 chars each — within crates.io's 5-keyword
  cap. Both `categories` are real crates.io slugs. `repository` and
  `homepage` are identical URLs (acceptable for the OSS-readiness claim; a
  proper docs-site URL can replace `homepage` later). `publish = true` sits at
  the bottom of the metadata block, which is the conventional placement.
  No typo. License is already dual `MIT OR Apache-2.0` and matches the
  `LICENSE-MIT` / `LICENSE-APACHE` files at repo root.

### T9 — H-7 cfg-gating is consistent

- **Severity:** (clean)
- **Locations:** `src/main.rs:31-36, 116-119, 147-148, 173-181, 183-199, 223-238, 283-294`;
  `src/process.rs:9-15, 312, 343, 348`
- **Note:** Every H-7 `#[cfg(debug_assertions)]` site has a matching
  `#[cfg(debug_assertions)]` on the *declaration* (constants, fns, struct,
  impl block) and on every *use* site (imports in `process.rs` for
  `KillOnDrop` and `ChildWrapper`). The two release-unused imports
  (`KillOnDrop` in production paths, `ChildWrapper` outside the immediate-
  descendant spawn) are cfg-gated on `debug_assertions` rather than scattered
  with `#[cfg]` blocks deeper in the function body — the import-block gating
  is the tidier choice and matches the project's idiom. No duplicate cfg
  attributes; no over-gating. The H-7 release binary still compiles and links
  cleanly (verified by `cargo build --release` clean in the gate).

### T10 — H-1 poison-safe mutex recovery is applied at every callsite

- **Severity:** (clean)
- **Locations:** `src/process.rs:55-58`, `src/process.rs:63-66`, `src/process.rs:680-682`
- **Note:** All three `Mutex::lock()` calls on the global `REDACTED_SECRETS`
  and `LOG_WRITERS` OnceLocks use the same `.unwrap_or_else(|poisoned|
  poisoned.into_inner())` idiom. `grep '.lock().expect'` across `src/` returns
  zero hits. The H-1 brief is fully honored; the pattern matches the one
  already established by the project's review history.

### T11 — H-5 `meta_tools()` and `config`-field resolution

- **Severity:** (clean)
- **Location:** `src/server.rs:69-76, 97`
- **Note:** `meta_tools()` is now an associated function with no `self`. The
  `let _ = &self.config;` busy-body line is gone. The call site
  (`Self::meta_tools()` at line 97) reads correctly. The `config: CliConfig`
  field carries `#[allow(dead_code)]` per T3 — that is the only awkward
  residue from the H-5 refactor; otherwise the module is clean.

### T12 — H-6 comment quality

- **Severity:** (clean)
- **Locations:** `src/main.rs:125-127, 186-188`
- **Note:** The H-6 rationale ("diagnostics before `Cli::parse()` / before
  tracing-init use `eprintln!`; everything after init uses `tracing`; `cred
  list` is intentionally raw stderr for the test harness") is correct and
  accurate against the code (`run_serve` calls `init_tracing` after `Cli::parse()`,
  `cred list` uses `eprintln!` directly at `main.rs:356`, the immediate-
  descendant sentinel uses `eprintln!` before `init_tracing`). The only
  comment-quality issue is the duplication called out in T2.

### T13 — No leftover debug, TODO, or commented-out code

- **Severity:** (clean)
- **Note:** `grep -nE 'TODO|FIXME|XXX|dbg!|println!|commented-out'` against
  the changed source files returns only the existing intentional
  documentation references (no `println!` introduced; the existing
  `eprintln!` sites are pre-init diagnostics covered by H-6; no commented-out
  code blocks; no stray `TODO` markers in the new code). The change set is
  clean of debug residue.

### T14 — Naming + comment density matches surrounding code

- **Severity:** (clean)
- **Note:** New identifiers (`HeaderSeen`, `start_http_probe`, `meta_tools` as
  associated, `IMMEDIATE_DESCENDANT_SENTINEL` / `IMMEDIATE_DESCENDANT_LIFETIME`,
  the `200` cap on `sanitize_upstream_identifier`) all match the
  snake_case / SCREAMING_SNAKE_CASE style already established. Comments are
  doc-style `///` on public items and line `//` on internal logic, matching
  the rest of `src/server.rs`, `src/main.rs`, `src/process.rs`. No terse
  one-liners on public items, no verbose prose on internal helpers — the
  density matches the existing modules.

### T15 — SECURITY.md H-8 paragraph is well-placed and accurate

- **Severity:** (clean)
- **Location:** `SECURITY.md:25`
- **Note:** The H-8 sentence is a continuation line under Enforced Practice
  #2 ("No secrets in logs."), which is exactly where an OSS reader will look
  for the exact-substring caveat. The phrasing — "whole-secret appearances
  are caught and replaced with `[REDACTED]`; a secret that appears
  perturbed/partial (e.g. truncated by an upstream) is out of scope" —
  correctly states the honest scope: matches `process.rs::redact()`'s
  `String::replace` semantics, names the `[REDACTED]` substitution shape,
  and the "out of scope" framing is the right tone (it doesn't pretend the
  parser is fuzzy; it admits the gap and tells the reader what it is). The
  O-2 GitHub Security Advisories rewrite at `SECURITY.md:81` is also clean:
  the placeholder `<SECURITY_CONTACT_EMAIL>` is gone, the GitHub-Advisories
  path is named concretely ("repository **Security** tab → **Report a
  vulnerability**"), and the 72-hour acknowledgement line is preserved. No
  email leak.

---

## Areas inspected and clean

- `src/credentials.rs` — H-4 trait `#[allow(dead_code)]` removed at the line
  named in `state.json` (H-4-credentials-rs). Surrounding doc-comment block
  preserved. No edits elsewhere in the file; edit-deny respected.
- `src/error.rs` — H-4 variant `#[allow(dead_code)]` removed at the line
  named in the plan (the `///` comment block on `CredentialResolution` is
  intact). No structural change to `ToolError`; D-005 public-error-shape
  invariant preserved.
- `src/registry.rs` H-3 change — `.unwrap_or_else` removed; unconditional
  `register_secret(&resolved)` applied; comment block on the resolved-env /
  resolved-headers section is unchanged.
- `tests/integration/literal_header_redaction.rs` H-3 test — endpoint now
  points at the reachable loopback probe; the reachable probe returns 200
  only when the registered Authorization header value matches the expected
  one. The test asserts the sentinel secret does not appear in the log file
  (the prior `[REDACTED]` marker assertion was removed because the marker
  does not appear in every log-sink path, and the negative assertion is the
  stronger contract). Test name and module doc-comment match the existing
  style.

---

## Self-check

- Every finding carries a severity tag and a `file:line` location.
- The CONTRIB / Cargo / SECURITY / src-idiom / consistency / D-2 categories
  the lens brief asked about are all addressed (some clean, some with
  findings).
- No source edits; this file is the only write.

## Lens verdict

PASS-with-issues — one `targeted` finding (duplicate `schemars` row in
`STACK.md`) and four `trivial` polish items, all contained and fixable
without re-planning. Nothing rises to `blocker` or `structural`.
