# review: phase-0-skeleton

## Verdict

PASS.

The implementation satisfies Phase 0's product criteria. The only red command is
`cargo test --workspace --doc`, which is a non-applicable runner line for this
binary-only Phase 0 crate, not a failing product test. The 14 non-deferred wire
and pinning tests pass, `cargo clippy -- -D warnings` passes, and the cited
Phase 0 code matches the scope-out list.

Final count: 0 blocker, 0 structural, 3 targeted, 3 trivial. One additional
D-005 item is carried to Phase 4, not counted as a Phase 0 defect.

## Merged findings

- Severity: targeted
  Type: test-contract correction
  Location: `docs/master-plans/phase-0-skeleton/tests.md:8`
  Lenses: alignment, adversarial
  Issue: The documented runner includes `cargo test --workspace --doc`, but
    Phase 0 is a binary-only crate with explicit `[[bin]]` targets and no
    library target. Cargo exits with `no library targets found in package
    fanin-mcp`. This is a bad test-contract line, not a product blocker: the
    actual Phase 0 wire suite and pinning tests pass.
  Suggested fix: Route `tests.md` back through `test-creator` to remove,
    defer, or conditionally document doc-tests until a library target exists.
    Do not add a library target just to satisfy this line unless the product
    plan changes.
  Routing: test-creator.

- Severity: targeted
  Type: fixture resource leak
  Location: `tests/probe-server/main.rs:287-292`
  Lenses: adversarial
  Issue: `needs_sampling` spawns a detached task that awaits
    `peer.send_request(request)` with no bound. Phase 0 tests observe the
    outbound request and kill the fixture, but repeated calls in a longer-lived
    probe session can accumulate pending tasks and request state if no client
    answers.
  Suggested fix: Bound the detached request with a short timeout and log the
    timeout to stderr/tracing, or use an emission path that does not retain a
    pending response future after writing the request frame.
  Routing: debugger.

- Severity: targeted
  Type: dependency hygiene
  Location: `Cargo.toml:29-35`
  Lenses: general
  Issue: `rmcp` enables the `macros` feature, but Phase 0 builds manual tool
    definitions and uses no `#[tool]` macro. The extra feature widens the
    compiled surface without a current use.
  Suggested fix: Remove `macros` from the `rmcp` feature list until a later
    phase actually uses it, or add the concrete macro use that justifies it.
  Routing: debugger.

- Severity: trivial
  Type: formatting
  Location: `src/main.rs:76-78`, `src/server.rs`,
    `tests/probe-server/main.rs`, test harness files
  Lenses: general
  Issue: `cargo fmt --all -- --check` reports rustfmt drift. This is cosmetic,
    but it should not remain red.
  Suggested fix: Run `cargo fmt --all` in a write-capable stage. Keep routing
    split by ownership: source/probe fixture/Cargo formatting through debugger;
    test-file formatting through test-creator because tests are read-only to
    other roles.
  Routing: debugger for source/probe fixture; test-creator for test files.

- Severity: trivial
  Type: code-health simplification
  Location: `src/main.rs:1`, `src/server.rs:1`,
    `tests/probe-server/main.rs:1`
  Lenses: general
  Issue: The source comments duplicate plan and GOTCHA text at high density.
    The load-bearing invariants are useful, but repeated phase narrative raises
    future edit cost.
  Suggested fix: Keep boundary comments for stdout transport, tool-level
    errors, and lock discipline; trim repeated phase-history prose during a
    future simplify pass.
  Routing: debugger if taken this cycle; otherwise carry.

- Severity: trivial
  Type: code-health simplification
  Location: `src/server.rs:168`, `tests/probe-server/main.rs:173`
  Lenses: general
  Issue: JSON-schema object construction is duplicated between the aggregator
    and probe fixture. Duplication is acceptable in Phase 0, but it may become
    noisy as more schemas land.
  Suggested fix: Defer until another production schema needs the same builder,
    then extract a tiny local helper. Do not introduce a workspace, sub-crate,
    or library split for this.
  Routing: carry; debugger only if the pattern grows.

## Deferred and carried items

- D-005 structured upstream error shape is deferred to Phase 4.
  Location: `src/server.rs:213-216`, `src/error.rs:20-21`,
  `docs/MVP.md:46-52`, `docs/DECISIONS.md:36-41`.
  The adversarial lens called the Phase 0 not-implemented text a blocker
  because it is not JSON with `server`, `tool`, `code`, `message`, and
  `recoverable`. I do not carry that as a Phase 0 defect. D-005's public JSON
  shape is for upstream proxy failures, and those fields are upstream-specific.
  Phase 0 has no upstream, no registry, and no proxy path; MVP Phase 4 is where
  `AggError` / `ErrorCode` are finalized. Phase 0 still satisfies the binding
  part relevant here: tool-level failure returns `Ok(CallToolResult::error(...))`
  with `isError: true`, not a JSON-RPC error. Carry the final JSON error model
  to Phase 4.

## Lens disagreements

- `tests.md:8` doc-test runner: alignment and adversarial tagged this as a
  blocker and alignment marked the lens FAIL. Final severity is targeted. The
  line is wrong in the test contract, but it is non-applicable to the declared
  binary-only crate and does not invalidate the passing 14 wire/pinning tests.
- D-005 not-implemented error content: adversarial tagged this as a blocker.
  Final disposition is deferred/carried to Phase 4. The Phase 0 stub has no
  upstream context for the D-005 upstream-error fields.
- `cargo fmt --all -- --check`: general tagged rustfmt drift trivial. Final
  severity remains trivial; fix ownership is split because some formatted files
  are test files.

## This cycle vs carried

Fix this cycle:

- Correct the `tests.md` doc-test runner line through `test-creator`.
- Bound the probe fixture's detached `needs_sampling` request through
  `debugger`.
- Remove or justify the unused `rmcp/macros` feature through `debugger`.
- Run rustfmt through the appropriate write-capable owner for each file class.

Carry/defer:

- Final D-005 upstream structured JSON error shape to Phase 4 Error Hardening.
- Comment-density trimming unless a simplify/debugger pass is already open.
- JSON-schema helper extraction until duplication grows in production code.
