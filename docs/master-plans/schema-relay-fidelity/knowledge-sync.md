# Knowledge-sync — feature `schema-relay-fidelity`

**Tier:** THOROUGH · **Cycle model:** linear-on-main · **Date:** 2026-06-30

## What shipped

Two non-routing findings from an external stress test (30 concurrent agents,
414 calls; routing/concurrency layer verified healthy and left unchanged).

- **Issue 1 (sanitization stance) — validated as already-decided design; locked
  with tests + docs, no behavior change.** The transparent-ish-proxy stance
  (D-004 byte-faithful invoke I/O + control-neutralization on display
  annotations only; no NL scrubbing, no envelope) was confirmed by the user and
  is now regression-guarded: a new `invoke_tool` BEL-round-trip lock asserts
  control chars pass verbatim through arg→response, and the `get_tool_schema`
  display-safety proof was extended to the full forbidden set (C1/U+2028-2029/
  bidi/BOM/zero-width), closing review blocker B1.
- **Issue 2 (silent ~100-char `get_tool_schema` truncation) — genuine bug,
  fixed.** `sanitize_upstream_text` bundled control-neutralization with the
  `list_tools`-row length cap, so the cap leaked into the schema-detail path and
  silently dropped real argument docs. Split into `neutralize_upstream_display`
  (display-wide, no cap) + `sanitize_list_row_description` (rows only, CAP 100).
  `get_tool_schema` annotations now relay full-length.

## Binding-doc reconciliation

In-cycle (Phase 3, implementer):
- **SECURITY.md** §Threat Model — cap is `list_tools` rows only; `get_tool_schema`
  annotations full-length; `invoke_tool` args + results verbatim (D-004) named
  as the residual bounded injection channel.
- **GOTCHA #20** — display-wide neutralization vs row-only cap made precise;
  invoke verbatim channel named; ✅ retained.

This stage (orchestrator):
- **AGG-MCP.md §Sanitization of Upstream Strings** — brought into line: display-wide
  neutralization, row-only length cap, full-length `get_tool_schema` annotations,
  verbatim validation strings + invoke I/O (D-004). Points to SECURITY.md + GOTCHA #20.

Checked and left as-is (already correct — all scoped to `list_tools` rows, which
still cap): ARCHITECTURE.md:107, PRD.md:32, MVP.md:49/94. No new ADR added —
the behavior aligns to existing D-004 + the documented list_tools-only cap, and
GOTCHA #20 (test-enforced, ✅) is the right home for the trap.

## Verification (orchestrator-verified each stage, not child self-report)

`cargo fmt --check` clean · `cargo clippy --all-targets -D warnings` clean ·
`cargo test --all` = 5 unit + 131 integration / 0 failed / 5 ignored ·
`cargo build --release` clean. The Issue-2 bite test
(`get_tool_schema_preserves_full_length_annotations_without_row_cap`) bites at
exact 235-char equality; the invoke BEL lock and the schema forbidden-set
assertion both bite a regression.

## Carried-forward (NOT this cycle's scope — for a future cycle)

- **Windows test-process teardown race.** Integration tests leak `fanin-mcp.exe`
  / `probe-server.exe` / `bun.exe` handles under load; observed blocking a
  `cargo build --release` (implementer) AND causing a transient OpenCode SQLite
  "database is locked" that killed the first reviewer dispatch (orchestrator
  cleared 3 leaked `bun.exe` and retried). Environmental / process-containment
  path, not a product defect. Worth a dedicated test-harness teardown fix.
- From oss-readiness, still open: MVP checklist final sign-off, ROADMAP v1.0
  status line, T4 test-helper duplication.
