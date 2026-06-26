# Fix: concurrent first-call test issue

## Defect

`tests/integration/registry.rs` `concurrent_first_calls_spawn_exactly_one_upstream`
reports a 5s timeout when run alone.

## Root cause

TEST-ISSUE, not an implementation deadlock.

The registry lazy-connect path already uses the required D-007 shape: it checks
the entries map, clones the per-server init guard, releases the guard-map mutex,
acquires the per-server guard, re-checks the entries map, then awaits
`connect()`. The entries map lock is not held across upstream spawn, handshake,
inventory discovery, or tool calls.

The failing test sends ids 2 and 3 back-to-back, then calls
`wait_for_id(id_a)` followed by `wait_for_id(id_b)`. `wait_for_id` discards any
non-matching response instead of buffering it. JSON-RPC allows responses to
arrive out of order. Manual wire reproduction showed the aggregator returning
both responses, usually in `[3, 2]` order. When `wait_for_id(2)` reads and
discards id 3, the later `wait_for_id(3)` waits for a response that was already
consumed and times out.

## Fix applied

No source fix applied. Changing `src/` to force response ordering would contort
the implementation around a broken harness assumption and weaken the JSON-RPC
concurrency contract.

## Verification

- `cargo build 2>&1 | tail -5`
  - `Finished dev profile [unoptimized + debuginfo] target(s) in 0.26s`
- Three exact-test repeats:
  - `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 57 filtered out; finished in 5.12s`
  - `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 57 filtered out; finished in 6.12s`
  - `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 57 filtered out; finished in 5.16s`
- Registry module gate:
  - `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 53 filtered out; finished in 0.60s`
- Full integration suite:
  - `test result: ok. 56 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 1.03s`

## Suggested-fix divergence

The suspected lock-held-across-await deadlock was not found. The per-server
init guard is held across `connect().await` by design to prevent double-spawn;
the global entries lock and init-guard map lock are not held across upstream
awaits.

## Surfaced

- targeted: `tests/common/mod.rs` `wait_for_id` drops non-matching JSON-RPC
  responses. It should buffer unmatched responses by id, or the concurrent test
  should read two responses into a map before asserting ids. The test file is
  read-only for this role, so this is surfaced for test-creator routing.
