# Implementer task — Phase 5, plan **Phase 6: CI, Audit/Deny, And Resource Budgets**

Implement ONLY plan Phase 6: the 3-OS CI release gate + supply-chain policy +
resource-budget checks. This phase has no integration unit tests — its
verification is the workflow + gate commands themselves. Keep everything correct
and runnable.

## Read first

- `master.md` §"Phase 6 — CI, Audit/Deny, And Resource Budgets" (Key Behaviors + SC).
- `.claude/commands/gate.md` (the project `/gate`: `cargo audit`, `cargo deny
  check`, `cargo bench --bench token_cost`) — CI must run the same gates.
- `STACK.md` (the deliberately small anti-stack — policy, not convention) and
  `ROADMAP.md` §"Release practice" (signed/checksummed 3-OS binaries; figures
  from the benchmark).
- **`docs/master-plans/phase-5-polish-ci-benchmarks/impl-p3-result.md`** — the
  dependency tree P3 added (reqwest/hyper/tower/tower-http/url + the ICU/idna
  stack). `deny.toml` MUST accommodate these (see below).
- Skills: `rust-general`.

## What to build

1. **`.github/workflows/ci.yml`** — matrix on `windows-latest`, `macos-latest`,
   `ubuntu-latest`. Per-OS job runs: `cargo fmt --all --check`, `cargo clippy
   --all-targets -- -D warnings`, `cargo test --test integration` (full suite,
   incl. the process-lifetime hard-kill tests — Linux PDEATHSIG + Windows job
   object are verified HERE), `cargo build --release`, and `cargo audit` +
   `cargo deny check` (install the tools in-job). The token bench: run
   `cargo bench --bench token_cost` (or a compile/availability check if a full
   bench run per push is too heavy — justify in a comment). Pin action versions;
   cache cargo. Use the standard `dtolnay/rust-toolchain` or actions-rs-free
   setup. Linux installs the linux target's needs; ensure `prctl` path compiles.
2. **`deny.toml`** — `advisories`, `bans`, `licenses`, `sources` sections.
   - Licenses: allow `MIT`, `Apache-2.0` (the project's dual license), plus the
     licenses the dependency tree actually uses — notably **`Unicode-3.0`** (the
     ICU crates pulled by reqwest/idna), and any others `cargo deny check
     licenses` flags (BSD-3-Clause, ISC, Zlib are common). Do NOT blanket-allow;
     enumerate. Run `cargo deny check` and resolve each finding deliberately.
   - Bans: encode the anti-stack intent where practical; do not ban a crate the
     HTTP client legitimately needs.
   - Sources: only crates.io (+ the registry policy).
3. **Resource budgets** (master SC 18/19): binary size `< 10MB stripped` —
   measure the release binary in CI (strip + size check step) and fail if over.
   **WATCH:** reqwest+hyper+ICU may push the stripped binary toward/over 10MB;
   if it exceeds, do NOT silently raise the limit — surface it in your result as
   a finding (the fix is trimming reqwest/ICU features, a follow-up). Memory
   budgets (`<15MB idle`, `<50MB @ 5 upstreams`): automate if a stable repo-local
   RSS probe is feasible per-OS; otherwise put a concrete command + thresholds in
   `docs/release-checklist.md` (master OQ2 default).
4. **`docs/release-checklist.md`** — create/extend with: the manual real-remote
   Streamable-HTTP header-auth check (P3.5), the memory-budget command(s) if not
   automated, and the per-OS platform support verification.

## Constraints

- Scope: Phase 6 only. Do not modify src logic. You MAY add `deny.toml`,
  `.github/workflows/`, CI helper scripts, and `docs/release-checklist.md`.
- Never `--no-verify`; the CI gate IS the ship gate. A gate that fails for a real
  reason (a license not allowed, binary over budget) is surfaced, not suppressed.
- The full `cargo test --test integration` suite must already be green locally
  before you finalize (it will be — P1/P2/P3/P5 are done). Run `cargo deny check`
  and `cargo audit` LOCALLY (install if missing) and fix `deny.toml` until
  `cargo deny check` is clean; if `cargo audit` flags an advisory in the tree,
  surface it (do not edit it away).

## Return

`impl-p6-result.md`: the CI matrix + steps, the `deny.toml` license set you
enumerated and WHY each, the LOCAL `cargo deny check` / `cargo audit` results,
the measured stripped binary size (and whether it is under 10MB — flag if not),
and the memory-budget approach (automated vs checklist).
