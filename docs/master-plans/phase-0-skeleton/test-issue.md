# test-issue: phase-0-skeleton — harness does not compile (blocks P0.3 gate)

## What

The Phase 0 test contract (commit `91b3be5`, test stage) does not compile. The
integration test binary fails `cargo test --no-run` with **3 hard compile
errors** and 2 warnings, all in test files that are read-only to the
implementer (`tests/common/mod.rs`, `tests/integration/probe.rs`). Because the
test binary does not build, `cargo nextest run` / `cargo test` cannot run a
single test — the P0.3 phase gate is blocked at the compile step, before any
assertion is evaluated.

The implementer cannot fix these: they are in test files owned by the
test-creator, and the implementer is forbidden from editing tests. This report
routes the defects back to the test-creator, the sole authority over tests.

## Defects

### 1. `tests/integration/probe.rs:36` — inner doc comment mid-block (E0753)

```
33: /// Criterion 6 (Probe build/run gate): the probe fixture builds and runs
34: /// standalone over stdio with no Node or npx. This test spawns it via the
35: /// cargo-injected path and proves the binary answers initialize — the build
36: //! itself is enforced by the cargo build/clippy gate (criterion 1), and the
37: /// no-Node requirement is structural (the probe is a Rust bin target).
```

Line 36 starts with `//!` (inner doc comment) inside an outer `///` doc-comment
block above `probe_builds_and_runs_over_stdio_without_node`. Inner doc comments
may only appear before items; rustc rejects this with E0753 ("expected outer
doc comment"). It is a one-character typo: `//!` should be `///`.

### 2. `tests/common/mod.rs:46` — `Ok(Some(status))` type mismatch (E0308)

```
42:     let wait = timeout(Duration::from_secs(1), self.child.wait()).await;
43:     match wait {
44:         Ok(status) => {
45:             if status.is_ok() {
46:                 return Ok(Some(status));
47:             }
```

`tokio::time::timeout(dur, self.child.wait()).await` returns
`Result<io::Result<ExitStatus>, Elapsed>`. So in `Ok(status)`, `status` is
`io::Result<ExitStatus>`, **not** `ExitStatus`. The function signature is
`-> io::Result<Option<ExitStatus>>`, so `Ok(Some(status))` tries to put
`io::Result<ExitStatus>` where `ExitStatus` is expected → E0308. The
`status.is_ok()` check on line 45 confirms the author knew `status` is a
`Result`, but then unwraps it wrong. Fix: `return Ok(Some(status?));` (propagate
the io error) — or `return Ok(Some(status.unwrap_or_else(|e| ... )));` if a
non-propagating recovery is intended.

### 3. `tests/common/mod.rs:49` — same `Ok(Some(status))` mismatch (E0308)

```
48:             let _ = self.child.kill().await;
49:             Ok(Some(status))
```

Same defect as #2 on the fall-through path. Same fix (`status?` or proper
unwrap). Both #2 and #3 share a single root cause: the `match` arm binds the
*inner* `io::Result` as `status` and returns it un-extracted.

### Warnings (also in test files, non-blocking but worth noting)

- `tests/common/mod.rs:16` — unused import `AsyncReadExt`.
- `tests/integration/probe.rs:18` — unused import `crate::common::expectations as exp`.

## Impact

- `cargo test --no-run` fails (exit 101) — the integration test binary does not
  build. `tests.md` claims "the harness compiles standalone against tokio +
  serde_json only"; it does not. The test-creator appears not to have run
  `cargo test --no-run` against the harness before committing.
- The entire P0.3 phase gate (`cargo nextest run`) cannot run. No probe test,
  no aggregator test, no pinning test executes. This is not a failure of the
  probe fixture (P0.3's Produces list) — the probe builds and passes clippy on
  its own. It is a failure of the test contract to compile.

## Verification of the probe side

The implementer's Produces list (`tests/probe-server/`) is green in isolation:

- `cargo build --bin probe-server` — exit 0.
- `cargo clippy -- -D warnings` — exit 0 (whole crate, including probe).
- `cargo build` (all bins) — exit 0.

So the probe fixture itself is not the blocker; the harness is.

## Why this is a test-issue, not an implementer fix

- `tests.md` and the dispatch directive name `tests/common/mod.rs` and
  `tests/integration/probe.rs` as read-only to the implementer.
- The defects are in the test contract, not the implementation. Editing them
  would defeat the test guard — the separation the whole pipeline is built on.
- Per the implementer test-issue exception: implement what is correct (the
  probe fixture, done), surface the test defect (this report), do not edit the
  test.

## Suggested fixes (for the test-creator, not the implementer)

1. `tests/integration/probe.rs:36` — change `//!` to `///`.
2. `tests/common/mod.rs:44-49` — extract the inner `io::Result`:
   ```rust
   Ok(status) => {
       match status {
           Ok(status) => Ok(Some(status)),
           Err(_) => {
               let _ = self.child.kill().await;
               Ok(None)
           }
       }
   }
   ```
   (or the `status?` form if propagating the io error is preferred — note the
   outer fn returns `io::Result<Option<ExitStatus>>`, so `?` works).
3. Remove the two unused imports.

## Routing

Return to `test-creator` to fix the harness compile errors, then re-run the
P0.3 implement gate (`cargo nextest run`). The probe fixture is complete and
awaiting a compiling harness to verify against.