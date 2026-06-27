FULL-CODEBASE ADVERSARIAL / SECURITY REVIEW — fanin-mcp @ v0.6.15 (HEAD 6d5b66c)

You are the reviewer. This is a standalone, whole-codebase ADVERSARIAL review.
Your job is to BREAK this code, not bless it. Default to skeptical: assume a
finding is real until you prove it isn't. The user's explicit ask: "no hidden
issues / security problem that was glossed over." Hunt for exactly that.

## Scope — read all of it

- All source: src/main.rs, server.rs, registry.rs, forward.rs, process.rs,
  namespace.rs, credentials.rs, error.rs, config.rs.
- Tests: tests/integration/, tests/common/, tests/probe-server/.
- Binding design canon (read before judging a subsystem — these are decisions
  already made, not suggestions): docs/DECISIONS.md (ADRs D-001..D-019),
  docs/GOTCHA.md (trap list; items marked ✅ claim to be enforced — VERIFY that
  claim against the code), docs/ARCHITECTURE.md, docs/PRD.md, docs/AGG-MCP.md,
  and root SECURITY.md (the threat model), STACK.md, ROADMAP.md.

## Attack surface to probe hard (project's binding rules — each is a trap that bites)

1. **stdout is the MCP transport.** Any `println!`/`print!`/`dbg!` to stdout
   after `serve(stdio())` corrupts the JSON-RPC stream. Grep for stray stdout
   writes anywhere reachable at runtime. (GOTCHA #1, D-… )
2. **Never hold a lock across an upstream await.** registry/forward must lock
   only to get/clone the `Arc<RunningService>`, drop the lock, THEN await the
   upstream call. Find any `.await` while a registry guard is held — it
   serializes the whole session and can deadlock. (D-007, GOTCHA #16)
3. **Bidirectional traffic answered from Phase 1.** No sampling/elicitation
   caps declared upstream; strays rejected instantly; `roots/list` returns
   empty. An unanswered upstream request hangs that server forever. Verify the
   reverse path can't hang. (D-008, GOTCHA #2)
4. **Errors are `CallToolResult { isError: true }`, never JSON-RPC errors.**
   The structured-error JSON shape is public API. Find any path that leaks a
   protocol-level error where a tool-result error is required. (D-005)
5. **Secrets never on argv, never in logs.** `cred set` reads a hidden stdin
   prompt; a redaction layer + sentinel test guard logs; each upstream gets
   only its own env vars. Try to find a leak: a secret in a Debug/Display impl,
   an error message, a trace span, a panic payload, or argv. (D-010, GOTCHA
   #18/#19/#22)
6. **Process-tree lifetime.** Every upstream lives in a Job Object (Windows) /
   process group (Unix); hard-kill must leave zero orphans. Inspect process.rs
   for the suspended-spawn→assign→resume race on Windows, the Linux PDEATHSIG
   path, and the documented macOS SIGKILL-orphan gap. Is the documented gap the
   ONLY gap, or are there others? (D-009, GOTCHA #11/#14)
7. **Results pass byte-faithfully.** No `to_string()` on a content array —
   it corrupts images/resources/binary. Find any lossy content round-trip.
   (D-004, GOTCHA #4)
8. **Sanitization** of poisoned upstream tool descriptions (control-char strip,
   length cap) — can it be bypassed or panic on adversarial input?

## Also hunt the generic Rust hazards

- `unwrap()`/`expect()`/`panic!`/array-index/slice on any runtime-reachable
  (non-test, non-startup-invariant) path — a panic is a DoS for a proxy.
- Integer/`as` truncation, unchecked arithmetic on sizes/lengths.
- TOCTOU / races beyond the registry lock (config reload, process state).
- Unbounded growth: a child that floods stderr, a map that never evicts, a
  timeout that doesn't actually cancel the upstream.
- `unsafe` blocks — justify each or flag it.
- Resource leaks on the error path (a spawned child not reaped on early return).

## Output

Write your artifact to:
  docs/master-plans/full-codebase-review/review-adversarial-<your-model-tag>.md

Structure: one section per finding. Each finding MUST carry:
- a severity tier: blocker | structural | targeted | trivial (rust-review/plan-format tiers),
- the exact `file:line` evidence,
- why it's exploitable / what breaks,
- a concrete fix or the question that resolves it.
End with a one-paragraph verdict: is this production- and OSS-ready from a
security standpoint, yes/no, and the single most important thing to fix.

Report ONLY what you can evidence in the code. Do not confabulate findings to
look thorough — an honest "I probed X and it holds, here's why" is worth more
than an invented bug. If a GOTCHA ✅ item genuinely holds, say so and cite the
code that enforces it. Your returned result is data for the orchestrator, not a
human-facing chat message.
