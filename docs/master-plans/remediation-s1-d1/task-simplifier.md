SIMPLIFY the remediation-s1-d1 change set. Behavior-preserving only.

You are the simplifier. Tier thorough. Review ONLY this cycle's change set and
look for genuine, behavior-preserving simplifications. Write
`docs/master-plans/remediation-s1-d1/simplify.md` per plan-format (files
simplified / reverted / unchanged / issues spotted).

## Scope (the only files you may touch)

`src/registry.rs`, `src/process.rs`, `src/config.rs`, `src/error.rs` — the four
files changed this cycle (see `git diff HEAD~3 -- src/`, which spans the implement
+ targeted-fix commits). Context: `docs/master-plans/remediation-s1-d1/master.md`
and `review.md`.

## Rules

- **Behavior-preserving.** No observable change: same public error shape (D-005),
  same timeout semantics, same containment, same cwd behavior. The full suite
  must stay **134/0/4** green, `cargo fmt --all -- --check` and `cargo clippy
  --all-targets -- -D warnings` clean.
- **Do NOT edit any `tests/**` file.** Tests are a read-only contract.
- **Do NOT edit `src/credentials.rs`.**
- **No invented churn.** If the code is already minimal, say so and change
  nothing — "nothing worth changing" is a correct, expected result. Do not
  rename/reshuffle for taste.
- **Already-decided:** the general review judged the three timeout match-arms
  (`get_or_connect` / `ensure_fresh` / `call_tool`) as worth keeping SEPARATE —
  each carries a distinct load-bearing invariant (no-insert-on-error / restore-
  dirty / log-tool+latency). Do NOT factor them into a shared helper unless you
  can show it preserves every per-site invariant AND reads clearly better; the
  default is leave them.
- Stay in scope: surface anything out-of-scope you notice in simplify.md's
  "issues spotted" section; do not fix it.

## Good candidates to consider (only if they genuinely improve clarity)

- The stdio/HTTP/`None` cwd-resolution branch in `get_or_connect` — is there a
  clearer shape without changing behavior?
- Any genuinely redundant local, needless clone, or dead intermediate introduced
  by the timeout/cwd plumbing.

After any change, run fmt + clippy + `cargo test --all` and confirm green. Return
as data for the orchestrator: what you changed and why (or that nothing was worth
changing), and the final gate numbers.
