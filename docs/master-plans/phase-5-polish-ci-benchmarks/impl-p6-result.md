# Implementer result: Phase 6 CI, Audit/Deny, And Resource Budgets

## Verdict

Phase 6 is implemented and green locally. No `src/` or test files were edited
by this phase.

## Files produced

- `.github/workflows/ci.yml`
- `deny.toml`
- `docs/release-checklist.md`
- `docs/master-plans/phase-5-polish-ci-benchmarks/impl-p6-result.md`

## CI matrix and gates

The workflow defines one matrix job over:

- `windows-latest`
- `macos-latest`
- `ubuntu-latest`

Each OS runs:

- `cargo fmt --all --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --test integration`
- `cargo build --release`
- stripped release-binary size check, failing at `>= 10,485,760` bytes
- `cargo audit`
- `cargo deny check`
- `cargo bench --bench token_cost`

Actions are version-pinned and cargo is cached with `Swatinem/rust-cache`.
`cargo-audit` and `cargo-deny` are installed in CI with `taiki-e/install-action`.

## `deny.toml` policy

Sections present:

- `advisories`: denies yanked crates and uses the advisory DB.
- `bans`: denies wildcard deps, denies new duplicate versions unless explicitly
  skipped, and bans anti-stack crates: web frameworks/HTTP server crates,
  database/ORM crates, plugin loading, and OpenTelemetry.
- `licenses`: enumerates only licenses accepted by the current tree.
- `sources`: denies unknown registries and git sources; allows crates.io only.

Allowed licenses and why:

| License | Why allowed |
|---|---|
| `MIT` | Project dual license and the dominant Rust ecosystem license in the tree. |
| `Apache-2.0` | Project dual license; used by `rmcp`, `rpassword`, `process-wrap` alternatives, and many core transitive crates. |
| `Unicode-3.0` | Required by the ICU/idna crates introduced by the reqwest-backed Streamable-HTTP path. |
| `Unlicense` | Used by `byteorder`/`memchr` license expressions in the current dependency tree. |

`BSD-3-Clause`, `ISC`, `Zlib`, and `Apache-2.0 WITH LLVM-exception` were not
kept in the allow list because `cargo deny` did not encounter them as selected
licenses in this resolved graph, and `unused-allowed-license = "deny"` keeps the
allow list tight. Several crates publish OR expressions containing those terms;
the accepted branch is already covered by `MIT` or `Apache-2.0`.

Duplicate-version policy is `deny` with explicit skips for current unavoidable
transitive duplicates:

- `schemars 0.8.22` / `schemars_derive 0.8.22` alongside rmcp's `1.2.1` line.
- `windows-sys 0.59.0` and `0.60.2` alongside the current Windows line.
- `windows-targets 0.52.6`, `windows_x86_64_gnu 0.52.6`, and
  `windows_x86_64_msvc 0.52.6` alongside the newer Windows target crates.

New duplicate crates still fail the gate.

## Local supply-chain results

- `cargo deny check`: passed.
- `cargo audit`: not installed locally (`cargo` reported no `audit` subcommand).
  I did not spend the phase budget compiling it from source. CI installs and runs
  `cargo audit` on every OS.

## Resource budgets

Binary size is automated in CI by copying the release binary, stripping the copy,
and checking it against `<10MB`. Linux/macOS use platform `strip`; Windows uses
`llvm-strip.exe` when present or `rust-objcopy.exe --strip-all` from
`llvm-tools-preview`.

Local Windows release measurement:

- `target/release/fanin-mcp.local-stripped.exe`: `8,669,184` bytes.
- Result: under the `<10MB` limit.

Memory budgets are documented as manual release checks in
`docs/release-checklist.md` because a stable repo-local cross-OS RSS harness was
not feasible within Phase 6 scope. The checklist gives per-OS commands and the
release-blocking thresholds:

- idle fanin-mcp RSS `<15MB`
- fanin-mcp RSS with five active upstreams `<50MB`

## Release checklist

`docs/release-checklist.md` now covers:

- manual real public Streamable-HTTP upstream check with placeholder-based header
  auth and log-redaction verification;
- Linux/macOS/Windows memory-budget commands and thresholds;
- per-OS platform verification, including Windows Job Object hard-kill, Linux
  PDEATHSIG hard-kill, and macOS graceful teardown without SIGKILL overclaiming;
- recording CI stripped binary sizes for all release OSes.

## Verification

Commands run locally:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --test integration
cargo build --release
cargo bench --bench token_cost
cargo deny check
```

Results:

- `cargo fmt --all --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test --test integration`: passed, `115 passed; 0 failed; 4 ignored`.
- `cargo build --release`: passed.
- `cargo bench --bench token_cost`: passed.
- `cargo deny check`: passed.

The full integration suite is still green.
