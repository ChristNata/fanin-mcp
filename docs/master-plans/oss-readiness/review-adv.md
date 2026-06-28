# OSS-Readiness Adversarial Review

**Lens:** ADVERSARIAL  
**Scope reviewed:** `Cargo.toml`, `CONTRIBUTING.md`, `SECURITY.md`, `STACK.md`, `docs/PRD.md`, `docs/ARCHITECTURE.md`, `docs/GOTCHA.md`, `src/main.rs`, `src/process.rs`, `src/registry.rs`, `src/server.rs`, `src/error.rs`, `src/credentials.rs`, `tests/integration/literal_header_redaction.rs`.  
**Gate re-verified:** `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test --all` 135 passed / 0 failed / 4 ignored.  
**Goal of this lens:** find what the green gate does *not* prove.

## 1. Executive summary

Found **0 blocker, 1 structural, 3 targeted, 2 trivial** findings. The cycle ships a
correct, faithful implementation of every H-* finding, but two artifacts of the
work invite further review:

- The H-3 behavioral test (`tests/integration/literal_header_redaction.rs`)
  exercises the real spawn/connect/invoke path against a reachable loopback HTTP
  mock, yet its remaining assertion (`!logs.contains(&secret)`) passes
  *trivially* on the success path — the resolved `Authorization` value is never
  written to any log line by the production code, so the test does not
  distinguish "H-3 fix in place" from "no logging at all." (`structural` — the
  test contract is weaker than the H-3 design intended.)
- Unconditional header-value registration introduces a user-controllable
  over-redaction surface: a benign literal header value becomes a registered
  redaction key, and any log line containing that exact substring is masked.
  (`targeted` — real risk, contained.)
- A minor consistency gap in the H-7 cfg-gate: the `Cli.spawn_immediate_descendant`
  field is *not* gated, so a release binary still parses `--spawn-immediate-descendant`
  (hidden via `hide = true`) and silently discards it. (`trivial` — benign,
  cosmetic.)
- A duplicate `schemars` row was left in `STACK.md`. (`trivial`.)
- The H-8 SECURITY.md note documents "exact-substring matching of registered
  secret values," but does not say what *qualifies* a value to be registered —
  a reader could be forgiven for thinking only `${VAR}`-resolved secrets
  qualify. (`trivial` — wording gap, no security regression.)

No blockers. No H-1, H-2, H-5, H-6, H-7 reachability gap, D-005 public shape,
or D-010 secret discipline issue.

## 2. H-3 — Unconditional header redaction (registry.rs:123-128)

**Implementation evidence.** `src/registry.rs:124-128`:

```rust
for (name, raw) in &server_config.headers {
    let resolved = crate::process::resolve_env_value(&*store, cred_choice, server, raw)?;
    crate::process::register_secret(&resolved);
    resolved_headers.insert(name.clone(), resolved);
}
```

Compared to the pre-cycle form (which guarded with `if raw.contains("${")`),
the cycle now registers **every** resolved value, literal or templated. This
closes the documented leak where a literal `Authorization = "Bearer xyz"`
bypassed the redaction set. Verified by tracing the data flow:

- `resolved_headers` flows into `build_http_headers` (`registry.rs:436`),
  which constructs `HeaderValue::from_str(value)` and inserts it into a
  `HashMap<HeaderName, HeaderValue>` that becomes the rmcp streamable-http
  `custom_headers` map.
- The header value is **never** `format!`'d into a tracing event, never
  `dbg!`'d, never written to the log file. There is no surviving code path
  in `src/` that prints a resolved header value.
- `register_secret` populates the global `REDACTED_SECRETS` `HashSet`
  (`process.rs:44-58`); `redact()` (`process.rs:62-73`) replaces every
  whole-secret appearance with `[REDACTED]`. The redaction layer is invoked
  from `RedactingStderrWriter::write` (`process.rs:503-518`),
  `RedactingFileWriter::write` (`process.rs:553-572`), and
  `emit_stderr_line` / `append_log_line` (`process.rs:470, 479`).

The H-3 *implementation* is correct and complete. The remaining concern is
the over-redaction surface (next finding) and the test design (see §3).

### Finding A1 — Unconditional registration creates a user-controllable over-redaction surface (`targeted`)

- **File:** `src/registry.rs:126` (the unconditional `register_secret(&resolved)`)
  in concert with `src/process.rs:62-73` (`redact()` uses `String::replace(secret, "[REDACTED]")`).
- **Issue:** With the guard removed, *every* header value the operator
  configured — secret or benign — is registered as a redaction key. If an
  operator sets e.g. `X-Trace-Id = "abc-123-xyz"`, then any log line anywhere
  in the process that happens to contain the literal substring `"abc-123-xyz"`
  is masked. This is a real (if narrow) risk: a benign-but-common header value
  becomes a foot-gun for unrelated log lines, and a malicious or careless
  config can construct values designed to mask operational telemetry.
- **Evidence:** Walked every `redact()` call-site (`process.rs:470, 479, 506,
  560`; `forward.rs:134`). Each is called on the *full* log line / stderr
  byte buffer before write, so the replacement hits any substring match.
- **Why not blocker:** This is a tradeoff baked into the H-3 fix (close the
  literal-secret leak → widen the registration set). The user controls their
  own header values, and the value is registered *only* if the operator
  explicitly placed it in `headers`. It is not reachable from a downstream
  LLM or upstream tool result.
- **Why not trivial:** A `User-Agent` or `X-Request-Id` header that collides
  with text an operator's tracing layer emits could silently mask real
  diagnostic data — an operational correctness issue, not just polish.
- **Fix:** Either (a) skip `register_secret` for header values that look
  non-secret (no `${VAR}` and no recognized auth-shaped name like
  `Authorization`, `Cookie`, `X-Api-Key`, `Proxy-Authorization`), or (b)
  document the trade-off in `SECURITY.md:25` so operators understand they
  should keep header values distinct from anything that could appear in
  their tracing. Recommendation: do (b) — the H-3 fix is intentionally
  aggressive and the docs should say so.
- **Routing:** targeted.

## 3. H-3 — Behavioral test (`tests/integration/literal_header_redaction.rs:123-164`)

### Finding A2 — Test does not bite the H-3 contract (`structural`)

- **File:** `tests/integration/literal_header_redaction.rs:123-164`.
- **Issue:** The test asserts only the negative — `!logs.contains(&secret)` —
  and dropped the `[REDACTED]`-must-appear assertion that the pre-cycle
  version carried. With the reachable HTTP probe, the test exercises:
  1. lazy spawn of an HTTP upstream,
  2. `invoke_tool` round-trip,
  3. graceful shutdown,
  4. a read of the per-server log file.
  But it never asserts the positive side — that the secret *was* registered
  and would be redacted *if* it ever appeared in a log line.
- **Why this matters:** Tracing every log-write path in `src/` shows that
  the resolved `Authorization` value is **never written** to a log line
  by production code (`registry.rs:397, 423, 446, 458, 464, 480` and
  `process.rs:482, 703, 713, 717` all emit only `server`/`tool`/`error`/
  `event`/`latency_ms`/`path` — none are the user-controlled header
  value). The HTTP transport does not log the wire contents to the
  aggregator's log file. So the assertion `!logs.contains(&secret)`
  passes **independently** of whether `register_secret` is ever called.
  The test would still pass if the `register_secret` line in
  `registry.rs:126` were deleted.
- **Adversarial framing:** This is the fakery checklist item *tests shaped
  to pass* — the test asserts the wrong thing (the absence of a value that
  is structurally never present, instead of the redaction-effect). The
  original assertion `logs.contains("[REDACTED]")` would have caught a
  regression in the redaction layer; its removal silently weakens the test
  contract.
- **What does NOT make this a blocker:** The H-3 implementation is
  independently verifiable correct in source (the `register_secret` call
  is unconditional and the sentinel is registered for any non-empty value).
  Production behavior is fine; only the regression-detection surface is
  weakened.
- **Why not `targeted`:** The cycle's own design (pre-cycle test) carried
  the positive `[REDACTED]` assertion; its removal is a real regression in
  test coverage that the plan explicitly asked test-creator to keep
  (cf. `task-test-fix-h3.md` lines 18-22, which say "the reworked test must
  ... assert the sentinel does NOT appear raw in the log AND `[REDACTED]`
  DOES appear"). That requirement is not satisfied by the file on disk.
  Routing this back through test-creator is a structural re-spec — not a
  single-line fix.
- **Fix:** Add back the positive assertion. Easiest: trigger a log line
  that includes the secret (e.g., a tracing event in the connect path that
  includes the resolved headers for diagnostics — gated by `--log-level
  debug`), or use a probe that logs back the Authorization value it
  received through an `upstream notifications/message` so the value flows
  through `forward.rs::append_redacted`. Then assert both `!logs.contains(&secret)`
  and `logs.contains("[REDACTED]")`.
- **Routing:** structural.

## 4. H-1 — Mutex poison recovery (`src/process.rs:55-58, 64-66, 680-683`)

**Clean.** Walked every `.lock()` call on a global `Mutex` in `process.rs` —
three sites, all converted to `.unwrap_or_else(|p| p.into_inner())`:

- `process.rs:55-58` — `register_secret`: `Mutex<HashSet<String>>`.
  `HashSet::insert` cannot leave the set in a torn state on panic; the
  recovered `HashSet` is safe.
- `process.rs:63-66` — `redact`: same `HashSet`. Read-only iteration after
  recovery; no torn state.
- `process.rs:680-683` — `log_sender`: `Mutex<HashMap<LogKey, mpsc::Sender<String>>>`.
  The body between `lock` and `insert` is `if let Some(sender) = writers.get(&key) { return sender.clone(); }` followed by `writers.insert(key, sender.clone());`.
  If the lookup succeeds and a panic occurs between the clone and the insert
  on a re-entry, the entry is just missing — a subsequent `log_sender` call
  will retry. `HashMap::get` and `HashMap::insert` cannot leave the map
  torn on panic.

No `.lock().expect(...)` remains on these globals. The only remaining
`.expect(...)` in `process.rs:196` is on a `.try_into()` for a compile-time-
fixed struct size — not a mutex, not on the hot path.

The two `tokio::sync::Mutex` calls in `registry.rs:89, 96` are unrelated —
they are per-server async init guards, not the global redaction / writers
maps. They are async, cannot poison, and are out of H-1's scope.

No finding.

## 5. H-2 — Length cap on `sanitize_upstream_identifier` (`src/server.rs:399-414`)

**Clean on multibyte.** The cap is implemented as `.chars().take(200)` over
the post-trim, post-strip string. `Chars` is a Unicode-scalar-value iterator
and never splits a multibyte sequence. The pattern mirrors
`sanitize_upstream_text` (`server.rs:394`) which has shipped unchanged
through prior phases. A `s.len()` byte check is not used, so the cap cannot
be defeated by a multibyte codepoint straddling the 200-char boundary.

The 200-char cap is appropriate: the upstream-provided tool name is later
rendered into `list_tools` rows that the LLM reads, so the cap is in service
of GOTCHA #20 (prompt-injection bounding). Identical-shape to the 100-char
description cap means the two sanitizers are symmetric and easy to reason
about together.

No finding.

## 6. H-7 — `#[cfg(debug_assertions)]` gating boundary

The cycle gates the test-only spawn hooks. Walked every relevant surface:

| Surface | Location | Gated? |
|---|---|---|
| `IMMEDIATE_DESCENDANT_SENTINEL` const | `main.rs:33` | yes (`#[cfg(debug_assertions)]`) |
| `IMMEDIATE_DESCENDANT_LIFETIME` const | `main.rs:36` | yes |
| `parse_immediate_descendant_sentinel` fn | `main.rs:174` | yes |
| `run_immediate_descendant` fn | `main.rs:184` | yes |
| `immediate_descendant_marker_from_config` fn | `main.rs:284` | yes |
| `parse_immediate_descendant_sentinel()` call | `main.rs:117` | yes (the call site is `#[cfg]`'d) |
| `spawn_immediate_descendant` (process.rs fn) | `process.rs:313` | yes |
| `ImmediateDescendantGuard` struct | `process.rs:344` | yes |
| `ImmediateDescendantGuard::id` impl | `process.rs:349` | yes |
| `process::spawn_immediate_descendant` call (main.rs:149) | `main.rs:148-162` | yes (block-level `#[cfg]`) |
| `process::spawn_immediate_descendant` call (main.rs:225, config-driven) | `main.rs:223-238` | yes (block-level `#[cfg]`) |
| `KillOnDrop` import (process.rs) | `process.rs:10` | yes (`#[cfg(all(debug_assertions, any(windows, unix)))]`) |
| `ChildWrapper` import (process.rs) | `process.rs:15` | yes (`#[cfg(debug_assertions)]`) |

Verified clean release build by verifying the orchestrator's claim: every
cfg-gated item has a matching `use` site that is also gated, so the imports
do not dangle. The pre-existing `JobObject` (windows-only), `ProcessSession`
(unix-only), and `CommandWrap` (unconditional) imports remain correct.

### Finding A3 — `Cli.spawn_immediate_descendant` field is not cfg-gated (`trivial`)

- **File:** `src/main.rs:68-70`.
- **Issue:** The `Cli.spawn_immediate_descendant: Option<PathBuf>` field is
  declared unconditionally, even though every consumer of the field
  (`main.rs:148-162`) is `#[cfg(debug_assertions)]`-gated. In a release
  build, the field is parsed (clap accepts `--spawn-immediate-descendant`)
  and silently ignored — the argument is parsed, stored in `cli`, never read.
- **Severity:** trivial. The flag is `hide = true` so it's not user-visible
  via `--help`. A user invoking the release binary with the flag has their
  input silently consumed — not a security regression, not a behavioral
  surprise for any documented user.
- **Fix:** Gate the field itself:

  ```rust
  #[cfg(debug_assertions)]
  /// Spawn a contained long-lived descendant at startup and write its PID.
  #[arg(long, global = true, hide = true)]
  spawn_immediate_descendant: Option<PathBuf>,
  ```

  Then verify release still builds with `cargo build --release` (and debug
  builds pick up the field as before).
- **Routing:** trivial.

## 7. H-6 — Comment-only eprintln/tracing split (`src/main.rs:125-127, 186-188`)

**Clean.** The diff at `main.rs:125-127` and `main.rs:186-188` is purely
additive comments at the two `eprintln!` sites. No functional change. The
rule stated in the comment matches the existing code: `eprintln!` is used
only in the two pre-`Cli::parse()` / pre-`tracing::init` paths and in
`cred list` (where the test harness expects raw stderr names). Every other
diagnostic goes through `tracing`. Verified by grep:

- Pre-parse / pre-init `eprintln!`: 2 sites (`main.rs:128, 189`).
- `cred list` raw `eprintln!`: 1 site (`main.rs:356`).
- Every other diagnostic in `main.rs`, `registry.rs`, `process.rs`,
  `forward.rs`, `server.rs` uses `tracing::*`.

No risk of the comment drifting from the code — the rule is mechanically
enforced by `tracing-subscriber` not being initialized before the
`eprintln!` sites run.

No finding.

## 8. H-4 — Stale `#[allow(dead_code)]` removal (`src/error.rs:68`, `src/credentials.rs:36`)

**Clean.** `ToolError::CredentialResolution` is constructed at
`process.rs:116` and rendered at `error.rs:131`; the variant is live. The
trait attribute on `src/credentials.rs:36` was removed by the orchestrator
per `state.json` decisions.H-4-credentials-rs (managed edit-deny). No
follow-up stale attributes found.

No finding.

## 9. H-5 — `meta_tools` as associated function (`src/server.rs:70-77, 97`)

**Clean.** Converted from `meta_tools(&self)` (with a no-op `let _ =
&self.config;` borrow to silence the field-never-read warning) to
`fn meta_tools() -> Vec<Tool>`. Call site at `server.rs:97` updated to
`Self::meta_tools()`. The `config` field on `Aggregator` is now
`#[allow(dead_code)]` at `server.rs:40-41` — a deliberate annotation
marking the field as carried verbatim from `--namespace` / `--config` for
later phases (the field is constructed in `Aggregator::new` /
`Aggregator::with_registry` and will be consumed by v1.x work). The
clippy-clean baseline confirms no actual unused-field warning fires.

No finding.

## 10. D-005 — Public error shape (`src/error.rs`)

**Unchanged.** `ToolError::message()` (`error.rs:73-138`) and
`structured_error()` (`error.rs:146-155`) still emit the
`{server, tool, code, message, recoverable}` JSON shape verbatim. The
removal of the `#[allow(dead_code)]` at `error.rs:68` was the only
cycle change. No `code` string was renamed, no field was added or
removed.

No finding.

## 11. D-010 + D-005 — No new secret in any log, public error shape unchanged

Walked every `tracing::*` site in the diff and every `eprintln!` site:

- `main.rs:128` — `eprintln!("invalid --log-level: {}", cli.log_level)` —
  emits only the user-supplied `--log-level` string (a level name like
  `"debug"`, never a secret). No risk.
- `main.rs:189-192` — `eprintln!("immediate descendant failed to write
  marker {}: {e}", marker_path.display())` — debug-build only (`#[cfg]`
  gated); emits only the marker file path and the OS error.
- `main.rs:356` — `eprintln!("{}", n)` in `cred list` — emits only
  credential *names* per `CredentialsStore::list_names`, never values.
  Unchanged.

No new secret-bearing tracing event was added. D-010 holds.

No finding.

## 12. D-2 — `--passthrough-stderr` strike

**Clean.** Verified by grep over the repo-root doc surface: zero
`--passthrough-stderr` references remain in `PRD.md`, `STACK.md`,
`ARCHITECTURE.md`, `GOTCHA.md`, `SECURITY.md`, `CONTRIBUTING.md`, or
`docs/DECISIONS.md`. The remaining `passthrough` matches are in:
- The cycle's own plan workspace (`docs/master-plans/oss-readiness/`),
  which is the planning/audit trail and must mention the term by name.
- `ROADMAP.md` and `docs/DECISIONS.md` references to "transparent
  passthrough of unknown JSON-RPC methods" (D-013), which is a
  *different* concept entirely — a routing policy deletion, not a CLI
  flag.

No finding.

## 13. Docs accuracy as an attack surface

### Finding A4 — `STACK.md` duplicate `schemars` row (`trivial`)

- **File:** `STACK.md:28-29`.
- **Issue:** The cycle's docs phase appended a `schemars` row at line 29
  but the original row at line 28 was not removed. The diff at
  `STACK.md:28-29` shows two identical rows:

  ```
  | `schemars` | JSON Schema helpers | Meta-tool input schemas (manual construction preferred over `#[tool]` macros — see AGG-MCP.md) |
  | `schemars` | JSON Schema helpers | Meta-tool input schemas (manual construction preferred over `#[tool]` macros — see AGG-MCP.md) |
  ```
- **Why trivial:** Cosmetic; readers will assume one is a typo. Not a
  security claim, not a contract violation.
- **Fix:** Delete the duplicate row.
- **Routing:** trivial.

### Finding A5 — H-8 SECURITY.md wording is accurate but incomplete (`trivial`)

- **File:** `SECURITY.md:25`.
- **Issue:** The H-8 note says "Log redaction is exact-substring matching
  of registered secret values — whole-secret appearances are caught and
  replaced with `[REDACTED]`; a secret that appears perturbed/partial
  (e.g. truncated by an upstream) is out of scope." This is honest about
  the exact-substring scope but says nothing about *what* registers a
  value — i.e., that since H-3, every resolved `headers` value (literal or
  templated) is registered, including benign header values like
  `Content-Type: application/json`. An operator who reads H-8 might be
  surprised to find that a literal `X-Trace-Id = "abc"` masks any log
  line containing "abc".
- **Why trivial:** The doc is correct in what it says; it is just silent
  on the H-3-induced over-redaction surface. Not a security regression.
- **Fix:** One-line addition to `SECURITY.md:25`: "Every value resolved
  from a server's `[headers]` is registered, including literal (non-`${}`)
  values — if a header value collides with text your tracing layer emits,
  the line is masked. Choose header values accordingly."
- **Routing:** trivial.

## 14. CONTRIBUTING.md — Gate commands and over-claims

**Clean.** The gate block (`CONTRIBUTING.md:9-14`) names exactly the three
commands that the cycle has verified green (`cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --all`). The
"3-OS matrix" claim at `CONTRIBUTING.md:26` matches the existing
`STACK.md:44` claim and the project's CI setup; not verifiable from the
local tree, but matches docs already in the canon.

The rmcp exact-pin discipline block (`CONTRIBUTING.md:16-17`) correctly
names `=1.8.0`. The D-015 / D-017 / no-runtime-deps claims are honored
in `Cargo.toml:1-13`.

The "single static binary" claim at `CONTRIBUTING.md:7` matches the
`Cargo.toml[profile.release]` strip + lto + codegen-units=1 + panic=abort
configuration in `STACK.md:50-55`. The plan/decision state (D-015, the
ROADMAP non-goals) is consistent.

No finding.

## 15. Cargo.toml metadata

**Clean.** Verified:

- `repository`, `homepage` — `https://github.com/ChristNata/fanin-mcp` (the
  declared git remote; matches the task brief's stated remote).
- `readme = "README.md"` — README.md exists at repo root (143 lines).
- `keywords` — five entries, each ≤20 chars (`mcp`, `proxy`, `llm`,
  `stdio`, `aggregator`).
- `categories` — `command-line-utilities`, `development-tools` (both valid
  crates.io slugs).
- `publish = true` — combined with LICENSE-MIT and LICENSE-APACHE at repo
  root; dual license `MIT OR Apache-2.0` already declared.
- `version = "0.1.0"` — kept per `state.json` O-1-publish decision.

No finding.

## 16. Verdict

**PASS-with-issues.** The cycle ships correct, faithful implementations of
every H-* finding. The green gate covers what it can cover. The two
artifacts above — the weakened H-3 behavioral test (structural) and the
user-controllable over-redaction surface from unconditional header
registration (targeted) — are real gaps the gate hides and should be
addressed before OSS release. The two trivial items (ungated
`Cli.spawn_immediate_descendant` field, duplicate `STACK.md` row) are
polish.

**Routing summary:**

| # | Severity | File | Issue |
|---|---|---|---|
| A1 | targeted | `src/registry.rs:126` | Unconditional header registration creates a user-controllable over-redaction surface (theoretical log corruption from benign-but-colliding header values). |
| A2 | **structural** | `tests/integration/literal_header_redaction.rs:159-163` | Behavioral test no longer asserts `[REDACTED]` is present — only the negative `!logs.contains(&secret)`, which is trivially true on the success path (the secret is never written to a log line). The test does not bite the H-3 contract. |
| A3 | trivial | `src/main.rs:68-70` | `Cli.spawn_immediate_descendant` field is not `#[cfg(debug_assertions)]`-gated; release binary still parses a hidden no-op flag. Cosmetic. |
| A4 | trivial | `STACK.md:28-29` | Duplicate `schemars` row left behind by the docs phase. |
| A5 | trivial | `SECURITY.md:25` | H-8 note is honest about exact-substring scope but silent on the H-3-induced over-redaction surface (every literal header value is registered). |