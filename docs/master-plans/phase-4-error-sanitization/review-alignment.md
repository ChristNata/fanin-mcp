# Review (alignment lens): phase-4-error-sanitization

Found 0 blocker, 0 structural, 0 targeted, 0 trivial.

## Gate

- `cargo test --test integration` — PASS: 99 passed, 0 failed, 3 ignored.
- Reviewed `git diff 497c762 -- src/` in full. Source changes are limited to
  `src/error.rs`, `src/forward.rs`, `src/registry.rs`, and `src/server.rs`.
- Test-file changes match the test contract: new Phase 4 modules, probe fixture
  extensions, and existing probe count updates from 10 to 14.

## Findings

(none)

## Success criteria alignment

1. SC 1 met: `list_tools` sanitizes upstream row names/descriptions before
   rendering JSON text (`src/server.rs:180-190`, `src/server.rs:382-399`).
2. SC 2 met: the same helper caps sanitized display text at 100 Unicode scalar
   values after stripping controls (`src/server.rs:396-399`).
3. SC 3 met: probe fixture supplies poisoned metadata and the passing
   sanitization test exercises the real wire path (`tests/probe-server/main.rs:383-389`).
4. SC 4 met: `get_tool_schema` sanitizes metadata strings recursively while
   preserving keys/shape (`src/server.rs:258-264`, `src/server.rs:410-458`).
5. SC 5 met: `invoke_tool` result forwarding remains byte-faithful; the Phase 4
   diff does not touch the result content path (`src/server.rs:336-359`,
   `src/registry.rs:167-174`).
6. SC 6 met: dead transport maps to additive code `upstream_disconnected` with
   D-005 fields (`src/error.rs:109-114`, `src/registry.rs:247-252`).
7. SC 7 met: dead entries are per-server; registry keeps sibling entries
   independent and tests prove a sibling call succeeds after death
   (`src/registry.rs:50`, `src/registry.rs:142-179`).
8. SC 8 met: upstream `CallToolResult::error` still returns as `Ok(result)`, not
   rewrapped (`src/registry.rs:167-170`).
9. SC 9 met: sampling/elicitation rejection path is unchanged and still clean
   (`src/forward.rs:52-88`).
10. SC 10 met: `on_tool_list_changed` sets only the handler's per-server dirty
    flag (`src/forward.rs:115-125`; shared from `src/registry.rs:280-305`).
11. SC 11 met: dirty inventory lazily refetches with `list_all_tools()` on the
    next inventory/call path (`src/registry.rs:124-128`, `src/registry.rs:208-239`).
12. SC 12 met: downstream `tools/list` still returns exactly the three static
    meta-tools (`src/server.rs:68-78`, `src/server.rs:94-100`).
13. SC 13 met: static meta-tool names/descriptions are unchanged
    (`src/server.rs:31-34`, `src/server.rs:460-506`).
14. SC 14 met: D-005 fields are unchanged; Phase 4 only adds a new code
    (`src/error.rs:142-150`, `src/error.rs:109-114`).
15. SC 15 met: the registry map lock is not held across `call_tool().await` or
    `list_all_tools().await`; `get_or_connect` returns a cloned entry before
    the awaits, and `ensure_fresh` holds no map/tools lock across refetch
    (`src/registry.rs:71-118`, `src/registry.rs:226-237`).
16. SC 16 met: `rmcp` remains exactly pinned at `=1.8.0` (`Cargo.toml:29`).
17. SC 17 met: no new stdout diagnostics were added; grep finds no
    `println!`, `print!`, or `dbg!` in changed serve-path code.

## Scope and decision fidelity

- No downstream `list_changed` push was added; notification handling only marks
  the per-entry dirty flag (`src/forward.rs:115-125`).
- No sampling/elicitation forwarding, reconnect policy, resource/prompt proxying,
  HTTP expansion, rmcp bump, or meta-tool surface change landed.
- The three accepted decisions are honored: `ToolError` remains the internal
  type, the dead-upstream code is exactly `upstream_disconnected`, the cache is
  per-entry mutable with a dirty flag and lazy refetch, and there is no silent
  reconnect.
- SECURITY.md reconciliation: the previously overclaimed sanitization line is
  now true for `list_tools` descriptions. The doc still speaks at a coarser
  level than the implementation: Phase 4 also sanitizes LLM-visible tool names
  and schema metadata while intentionally not sanitizing result content. That is
  knowledge-sync follow-up, not implementation drift.

Lens verdict: PASS
