WRITE THE TEST CONTRACT — remediation-s1-d1 (S-1 timeout envelope + D-1 cwd).

You are the test-creator, the SOLE author of test files. Write `tests.md` and the
test code in the plan workspace `docs/master-plans/remediation-s1-d1/tests.md`
plus the test/fixture files named below. Read the plan first:
`docs/master-plans/remediation-s1-d1/master.md` — derive AT LEAST one assertion
per numbered Success Criterion (17 of them). Tier thorough, stack rust.

You may edit ONLY: `tests/**` (integration tests, `tests/common/fixtures.rs`,
`tests/probe-server/main.rs`) and `tests.md`. Do NOT edit any `src/**` production
file — the implementer codes against your tests. The plan's Produces lists also
name `src/*.rs`; those are the implementer's, not yours.

## Hard requirement: the suite must COMPILE, fmt-clean, clippy-clean

The implementer will run `cargo fmt --check` and `cargo clippy --all-targets --
-D warnings` and MUST NOT need to touch test files to make them pass. So your
delivered tests must already be `cargo fmt`-clean and `clippy -D warnings`-clean
and must COMPILE. The NEW S-1/D-1 tests are expected to FAIL BY ASSERTION (red)
until the implementer lands the code — that is correct. Existing tests must stay
green. A test that fails to COMPILE is not an acceptable contract.

## S-1 tests — prove the timeout actually fires (anti-gaming is the point)

The whole value of S-1 is that a hung upstream cannot hang the proxy. Your tests
must make a REAL timeout the only way the call can return. Use a small configured
`timeout_secs` (e.g. 1–2s) and a probe that hangs FAR longer (e.g. blocks
forever / sleeps 60s+), then assert BOTH the structured error AND that it
returned well within the hang duration. A test that a coder could pass by making
the probe return quickly is a gamed test — do not write one.

Add probe-server (`tests/probe-server/main.rs`) modes, selected by CLI flag/env,
covering:
- **hang-during-initialize**: accept the stdio transport but never complete the
  rmcp initialize handshake (block before/within `serve`).
- **hang-during-list-tools**: complete initialize, then block forever on the
  initial `tools/list` (`list_all_tools`).
- **hang-during-refetch**: serve normally, emit a `notifications/tools/
  list_changed`, then block forever on the subsequent `tools/list` refetch.
- **hang-then-spawn-descendant**: a hang-during-initialize variant that FIRST
  spawns a long-lived child/descendant process (reuse the existing Phase-5
  immediate-descendant / grandchild fixture mechanism) so containment can be
  checked.

Tests to write (map each to the plan criteria in tests.md):
1. Hang-during-initialize → first `list_tools`/`invoke_tool` returns
   `CallToolResult{isError:true}` with code `upstream_timeout`, within ~the
   configured bound (assert elapsed << probe hang). (Crit 1)
2. Hang-during-list-tools → same structured `upstream_timeout` within bound.
   (Crit 2)
3. **HTTP stall**: extend the in-repo loopback Streamable-HTTP mock (the one used
   for the CI remote-HTTP tests) to accept a connection then stall during
   connect/initialize; assert `upstream_timeout` within bound. If the existing
   mock cannot express a stall, add a stall mode to it (it is test code). (Crit 3)
4. Hang-during-refetch → `upstream_timeout` within bound, the entry stays dirty
   (a subsequent inventory read RE-attempts rather than serving stale-as-fresh),
   and the prior cached inventory is not overwritten with empty. (Crit 4, 5)
5. Cold-connect timeout leaves NO cached entry and the init guard is released: a
   SECOND call to the same hung server still returns (within bound) by attempting
   a fresh connect — it does NOT queue/hang behind the first attempt. Assert the
   second call also returns a structured error within bound. (Crit 6, 7)
6. **Containment during the window (do NOT rely on a post-run count)**: using
   hang-then-spawn-descendant, capture the descendant PID, trigger the cold
   connect that times out, and assert the descendant is dead WITHIN A BOUND AFTER
   THE TIMEOUT — poll `OpenProcess`/`kill(pid,0)`/equivalent liveness for that
   specific PID during the test, while the test process is alive. A post-
   `cargo test` survivor sweep is masked by the runner's own job and is NOT
   acceptable as the sole assertion. (Crit 8)
7. Concurrent sibling: server B already connected; a hung cold connect on server
   A does not block a call to B — B returns promptly while A is timing out.
   (Crit 9 / Phase-1 crit 7)

## D-1 tests — the cwd field

The probe-server needs to REPORT its working directory. Add a probe tool (e.g.
`report_cwd`) that returns `std::env::current_dir()` as text, so a test can assert
the child's actual CWD. Then:
8. Literal `cwd = <temp dir>` → `report_cwd` returns that dir. (Crit 11)
9. `cwd = "${VAR}"` with VAR set to a temp dir → resolves via the SAME resolver as
   env/headers; `report_cwd` returns the resolved dir. (Crit 12)
10. No `cwd` → child inherits the aggregator's CWD (today's behavior), proven by
    `report_cwd` equalling the parent process CWD. (Crit 13)
11. `cwd = ""` / whitespace-only → CONFIG VALIDATION error at load, before MCP
    serving starts (not a lazy spawn failure). (Crit 10)
12. `cwd = "${VAR}"` where VAR resolves to blank/whitespace → fails BEFORE spawn
    with a structured tool-level error (not a hang, not a panic). (Crit 14)
13. Non-existent `cwd` dir → structured error with public code
    `upstream_connect_failed`. (Crit 15)
14. Streamable-HTTP server WITH `cwd` set → connects normally, `cwd` neither
    resolved nor applied (no error, no resolution attempt). (Crit 16)

Render `cwd` in the `tests/common/fixtures.rs` builders / `to_toml` helpers so
config round-trips carry it (production code gains no TOML writer — that was a
corrected drift in the plan).

## tests.md

Per plan-format: files created (path + criteria covered); a coverage map of each
master Success Criterion → test name; deferred tests (with justifying `#[ignore]`
reason — avoid ignoring the new S-1/D-1 tests, they are the contract); side-effect
assertions (containment, no-stdout, dirty-flag state).

## Watch-outs

- Keep the existing 115-test suite green; do not weaken or delete existing
  assertions to fit new fixtures.
- stdout discipline: no test may print to the aggregator's stdout transport; use
  the existing stdout-drain negative-assert helper if you touch that path.
- Make timeouts SHORT (1–2s configured) and probe hangs LONG so the suite stays
  fast but the timeout is unambiguously what fired.
- Cross-platform: containment liveness checks differ Windows vs Unix — gate
  per-`cfg` like the existing process_lifetime tests; do not assume a Unix-only
  syscall on the Windows host.

Return (as data for the orchestrator, not chat): the files you wrote, the
criteria→test coverage map, any criterion you could NOT cover and why, and any
place the plan's contract was ambiguous enough that the implementer could
satisfy the letter but miss the intent.
