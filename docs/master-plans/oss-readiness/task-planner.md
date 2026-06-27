PLAN the OSS-readiness remediation cycle — close all remaining full-codebase
review findings. Produce ONLY `master.md` in
`docs/master-plans/oss-readiness/`, per the `plan-format` spec. Tier
**thorough**, scope **flat**, stack **rust**. Do NOT write `state.json`
(orchestrator owns it). Do NOT write code.

## Context to read first

- `docs/master-plans/full-codebase-review/review-SYNTHESIS.md` — the source of
  every finding below. Also the per-lens detail in the same dir
  (`review-deepdive-minimax.md`, `review-adversarial-deepseek.md`) for exact
  file:line evidence.
- The just-completed `docs/master-plans/remediation-s1-d1/` cycle (already merged
  to main) closed S-1 and D-1 — do NOT re-touch those.
- Binding canon: `docs/DECISIONS.md` (esp. D-005 public error shape, D-010
  secrets, D-017 name+dual-license, D-009 containment), `docs/SECURITY.md`,
  `STACK.md`, `ROADMAP.md`, `docs/ARCHITECTURE.md`, `docs/GOTCHA.md`.
- Current tree facts: `Cargo.toml` has `publish=false`, version `0.1.0`,
  name/edition/rust-version/license/description only (no repository/homepage/
  keywords/categories/readme). `LICENSE-MIT`+`LICENSE-APACHE` exist.
  `.github/workflows/ci.yml` exists. No `CONTRIBUTING.md`. Git remote:
  `https://github.com/ChristNata/fanin-mcp`.

## Findings to remediate (the entire scope of this plan)

### OSS-release blockers
- **O-1 — crate not publishable / discoverable.** `Cargo.toml:8` `publish=false`;
  `[package]` lacks `repository`, `homepage`, `readme`, `keywords`, `categories`.
  Version still `0.1.0`. Decide the right `keywords`/`categories` (e.g. mcp,
  proxy, llm; command-line-utilities, development-tools). Whether to flip
  `publish` to true now or keep gated until a tagged release is a decision to
  state (recommend: add all metadata; flip publish=true since LICENSE + dual
  license are present, and add a `cargo publish --dry-run` checklist item).
  Whether to bump version off 0.1.0 is a decision — recommend leaving the crate
  version to the release process, not this cycle, UNLESS you justify otherwise.
- **O-2 — dead security contact.** `SECURITY.md` `<SECURITY_CONTACT_EMAIL>`
  placeholder. The real disclosure channel is a DECISION the plan must surface as
  an Open Question for the user (a real email vs a GitHub Security Advisories
  private link). Recommend GitHub Security Advisories (no address to leak); flag
  it for the user to confirm — do not invent an email.
- **O-3 — no CONTRIBUTING.md.** Add a one-page guide in the repo's voice
  (capital-style): rmcp exact-pin discipline, read docs/DECISIONS.md, the gate
  (`cargo fmt`, `clippy --all-targets -D warnings`, `cargo test --all`), the
  binding GOTCHA list, no-runtime-deps identity, PR runs full CI matrix.

### Doc-vs-code drift
- **D-2 — `--passthrough-stderr` documented but unimplemented.** Referenced in
  `docs/ARCHITECTURE.md:163`, `STACK.md:27`, `docs/GOTCHA.md` #29; `grep
  passthrough src/` → 0. DECISION the plan must make and state: implement the
  flag (mirror child stderr to the aggregator's stderr — a few lines in
  `process.rs`'s stderr task + a clap flag, debug-only) OR strike it from all
  three docs. Recommend the lower-risk, fully-honest option and justify it; if
  implement, note it must NOT pollute the stdout transport (GOTCHA #1) and is
  off by default.

### Hardening (behavioral — will need tests)
- **H-1 — mutex-poison `.expect()` DoS.** `process.rs` global `std::sync::Mutex`
  (~lines 55/63/671) `.expect()` panics the whole proxy if poisoned. Fix:
  `.unwrap_or_else(|p| p.into_inner())` (poisoned `HashSet`/writers recover
  safely). Low-likelihood but a latent process-wide panic.
- **H-2 — `sanitize_upstream_identifier` no length cap.** `server.rs:401-413`
  strips control chars but does not length-cap (unlike `sanitize_upstream_text`
  which caps 100). Bounded today by rmcp 1.8.0's 128-char ceiling. Fix: add a
  generous cap (e.g. 200) OR document the rmcp bound as the intentional limit —
  state the choice.
- **H-3 — literal HTTP header values bypass redaction.** `registry.rs` registers
  a resolved header for redaction only when the raw contains `${`. A literal
  secret in `headers` is never added to the redaction set. Fix: register resolved
  header values unconditionally, OR warn on literal-looking secrets. Defense-in-
  depth (docs mandate `${VAR}`), but cheap to close.

### Hygiene (mostly trivial)
- **H-4 — stale `#[allow(dead_code)]`** on now-wired `ToolError::CredentialResolution`
  (`error.rs:66`) and the `CredentialStore` trait (`credentials.rs:36`). Remove
  both. NOTE: `src/credentials.rs` is under a managed-OC edit-deny — if removing
  the trait attribute requires editing credentials.rs, FLAG it; the orchestrator
  will handle that file specially (it cannot be edited by an OC child).
- **H-5 — `meta_tools(&self)` dead-field borrow** (`server.rs:69-78`): make it an
  associated fn `meta_tools()`.
- **H-6 — `eprintln!` vs `tracing` startup inconsistency** (`main.rs:122,177,342`):
  add a minimal stderr-only subscriber before clap parse; keep `cred list` raw
  (documented). Behavioral-adjacent — confirm no test asserts the current
  startup output shape.
- **H-7 — test-only spawn hooks shipped in the production binary.** The
  `__fanin_immediate_descendant__` argv sentinel + the `--spawn-immediate-
  descendant` config-arg scan + 30s marker writer live in `src/main.rs`
  (~32-34, 68, 113-187, 211-280). DECISION + RISK the plan MUST resolve: these
  are used by the Phase-5 containment tests (`tests/integration/process_lifetime.rs`
  and the probe fixture). Determine whether gating them behind
  `#[cfg(debug_assertions)]` (cargo test builds debug, so tests still see them)
  keeps every existing containment test green, OR whether moving them changes the
  read-only test contract (→ then it routes to test-creator, NOT a silent move).
  State the finding and the safe path; if it can't be done without a test change,
  say so and scope it as test-creator work.
- **H-8 — `redact()` exact-substring weakness** (`process.rs:60-71`): minimax's
  recommendation is to DOCUMENT the scope in SECURITY.md ("redaction is
  exact-substring; whole-secret appearances are caught, perturbed-byte
  appearances are out of scope"), not to add fuzzy matching. Recommend the doc
  note; state if you disagree.

## Phasing guidance

Group by nature and file-disjointness so phases can gate independently and some
may parallelize:
- A docs/metadata phase (O-1 Cargo.toml, O-2 SECURITY.md, O-3 CONTRIBUTING.md,
  H-8 SECURITY.md note) — no `src/` code, low risk, likely no behavioral test.
- A hardening phase (H-1, H-2, H-3) — `src/` behavioral, needs tests.
- A hygiene phase (H-4, H-5, H-6, D-2-if-implemented) — `src/`, small.
- H-7 — isolate it; it has test-contract risk.
Note file overlaps (several touch `server.rs`/`process.rs`/`main.rs`) and mark
which phases may NOT run in parallel. Give each phase explicit Produces, Key
Behaviors, Depends On, Phase Success Criteria.

## Constraints / invariants (put in master.md)

- Tests are a read-only contract (only test-creator writes them); 100% pass.
- Preserve D-005 public error shape, D-009 containment, D-010 secrets discipline,
  GOTCHA #1 stdout transport, the no-runtime-deps / single-static-binary promise,
  the rmcp `=1.8.0` exact pin (no bump).
- Scope is EXACTLY O-1/O-2/O-3, D-2, H-1..H-8. No new features, no S-1/D-1 rework.
- `src/credentials.rs` cannot be edited by an OC child (managed deny) — any fix
  needing it must be flagged for orchestrator handling.

## Open Questions

Surface, with a recommended default, at minimum: the O-2 security contact
(needs the user's real channel), the D-2 implement-vs-strike decision, the O-1
publish-flag + version-bump decision, and the H-7 gating-vs-test-change
determination. Your returned result: name the phases, the decisions you made,
the Open Questions needing the user, and any blocking drift. Data for the
orchestrator, not chat.
