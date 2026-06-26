---
name: gate
description: Run the fanin-mcp security/release gate — cargo audit, cargo deny, and the token benchmark.
argument-hint: "[--no-bench]"
disable-model-invocation: true
---

# /gate

Runs the security and release gates fanin-mcp requires before a commit or a
release. The dependency tree being small is an advertised security feature
(STACK.md) — these checks keep it honest. Pass `--no-bench` to skip the token
benchmark (the slow step) when you only need the security gates.

Run from the repo root. Report each gate's outcome explicitly; a red gate is a
failure to surface, never a step to wave through.

## 1. Supply-chain audit — `cargo audit`

```bash
cargo audit
```

Fails on any advisory in the dependency tree (RUSTSEC). If `cargo-audit` is
not installed, surface that and stop — do not silently skip. Install hint:
`cargo install cargo-audit`.

## 2. License + ban + source policy — `cargo deny`

```bash
cargo deny check
```

Runs the `advisories`, `bans`, `licenses`, and `sources` checks against
`deny.toml`. The dual MIT/Apache-2.0 license and the deliberately small tree
(no web framework, no DB, no Node — STACK.md anti-stack) are policy here, not
convention. Install hint: `cargo install cargo-deny`.

## 3. Token benchmark — `cargo bench` (skip with `--no-bench`)

```bash
cargo bench --bench token_cost
```

Measures actual `tools/list` + typical-session token costs. **README token
figures are generated from this benchmark, never hand-edited** (GOTCHA #26,
ROADMAP release practice). If the measured numbers drift from what the README
claims, the README is wrong — regenerate it, do not adjust the benchmark.

> The benchmark target lands in MVP Phase 5. Until it exists, `--no-bench` is
> the expected invocation and this step reports "benchmark not yet present."

## Report

End with a one-line verdict per gate (audit / deny / bench: pass | fail | skipped)
and the overall gate result. On any failure, show the offending output — do not
summarize a red gate as green.
