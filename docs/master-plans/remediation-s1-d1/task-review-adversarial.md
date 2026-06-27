## LENS: ADVERSARIAL — write review-adversarial.md

Try to BREAK the new code. Default skeptical. Hunt specifically:

- **Test-gaming / generality.** Does the timeout mechanism generalize to ANY hung
  upstream, or is it secretly keyed to the probe/test? Confirm there is no
  probe/server-name special-casing, no flag only the test sets, no sleep-race
  shortcut. Would a brand-new hung upstream (never seen by a test) also time out
  and get reaped?
- **Containment on timeout (D-009).** Trace the `ContainmentGuard` lifetime across
  the timed connect future. On a timeout mid-`serve` or mid-`list_all_tools`, is
  the guard GUARANTEED to drop and kill the child tree? Any path where the guard
  is moved out, forgotten, or kept alive past cancellation → orphan. Does a
  timeout during `serve` leave a half-initialized rmcp client or a dangling task?
- **Race / lock.** The init guard is held across the new timed connect. Can two
  concurrent `get_or_connect` calls for the same server interleave badly? Is any
  registry map lock (or `tools` lock) now held across an await (D-007/GOTCHA #16)?
  Can a timeout + a concurrent success double-insert or poison the cache?
- **cwd abuse.** Can the `${VAR}` cwd interpolation leak a secret into a log/error,
  or expand to an attacker-influenced path? Is the empty/whitespace rejection
  bypassable (e.g. a tab/newline, a `${VAR}` resolving to spaces, a path that is
  technically non-empty but invalid)? TOCTOU between cwd validation and spawn?
- **Panics / DoS.** Any new `unwrap`/`expect`/index/`as` truncation on a
  runtime-reachable path? Can a crafted config or upstream make the new code
  panic?
- **Error shape (D-005).** Confirm the public `code` set is unchanged on the wire
  (StartupError::EmptyCwd is a CLI/config surface, not a JSON-RPC tool error —
  verify that's actually where it surfaces).
