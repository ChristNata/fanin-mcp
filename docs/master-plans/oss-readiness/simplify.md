# oss-readiness — simplify stage

**Role:** simplifier (grok-4.3)
**Baseline:** v0.6.25 (review-stage tree)
**Scope:** src changes only — process.rs, registry.rs, server.rs, error.rs, main.rs

## Files simplified
(none)

## Files reverted
(none)

## Files unchanged — already minimal
- `src/process.rs` — H-1 mutex recovery + H-7 `cfg(debug_assertions)` gating +
  import gating are minimal and load-bearing for release correctness.
- `src/registry.rs` — H-3 unconditional `register_secret` is the exact required form.
- `src/server.rs` — H-2 cap (named `CAP`), H-5 `meta_tools` associated fn; the
  dead `config` field was already DROPPED in the debugger polish pass (T3).
- `src/error.rs` — H-4 dead_code removal complete.
- `src/main.rs` — H-6 comment + H-7 cfg gating minimal.

## Issues spotted
- T4 (`HeaderSeen` / `start_http_probe` duplicated across two integration test
  files) — test code, out of the simplifier's scope; noted for a future
  test-creator cleanup pass. Not fixed this cycle.

## Gate after pass
Behavior-preserving, zero edits. `cargo test --all` 135/0/4; fmt + clippy clean;
`cargo build --release` clean (0 warnings).
