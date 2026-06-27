# Review: remediation-s1-d1 — Alignment Lens

**Scope reviewed:** only the remediation-s1-d1 change set (HEAD~1..HEAD on `src/`).

## D-012 (timeout envelope) — HONORED
- Cold connect: `get_or_connect` (registry.rs:149) wraps the entire `connect` future (spawn + serve + initial `list_all_tools`) in `timeout(effective, …)` (registry.rs:151-166). No blocking await left outside.
- Dirty-refetch: `ensure_fresh` wraps `list_all_tools` (registry.rs:320-336) identically.
- Existing `call_tool` path already wrapped (unchanged). No unbounded upstream await remains for S-1.

## D-007 / GOTCHA #16 (lock discipline) — HONORED
- `get_or_connect` resolves config + cwd with zero map lock held (registry.rs:101-148), then clones only the init guard before the timed await.
- `ensure_fresh` holds only a cloned `Arc<UpstreamEntry>` across its timed await (registry.rs:299-336); no `entries` or `tools` lock crosses the boundary.
- Timeout path returns before any map insertion (registry.rs:166), preserving the original guard.

## D-009 / GOTCHA #11/#14 (containment) — HONORED
- `ContainmentGuard` is created locally inside `connect` (registry.rs:412) and retained only on success path into `UpstreamEntry`.
- Timeout drops the future before entry construction; the guard is therefore dropped and the half-spawned tree is killed inside the test window.

## D-005 (public error shape) — HONORED
- All three new timeout sites reuse `ToolError::UpstreamTimeout { code: "upstream_timeout" }` (registry.rs:170, 334). No new public wire code minted.

## D-004 (byte-faithful) — HONORED
- No change to result passthrough paths.

## D-1 / GOTCHA #30 / ARCHITECTURE.md:97 / PRD Req 5 — HONORED
- `ServerConfig::cwd: Option<String>` added (config.rs:110) with doc matching spec.
- Resolution uses identical `resolve_env_value` path (registry.rs:134); empty-after-resolution rejected before spawn (registry.rs:136-140) → `UpstreamConnect`.
- Non-existent dir fails at `Command::spawn` → `UpstreamConnect` / `upstream_connect_failed` (no preflight).
- HTTP path explicitly skips resolution and passes `None` (registry.rs:146-147); `spawn_stdio_transport` receives `resolved_cwd` only for stdio.
- Empty/whitespace `cwd` rejected at config load via new `StartupError::EmptyCwd` (config.rs:208-212, error.rs:172).

## Scope — HONORED
- Changes confined to S-1 + D-1. No O-*/D-2/H-* work, no rmcp bump, no new deps, no meta-tool or ACL changes.

## Docs — note for knowledge-sync
- `docs/DECISIONS.md`, `docs/GOTCHA.md`, `docs/ARCHITECTURE.md`, and `docs/SECURITY.md` should record that S-1 is closed and `cwd` (D-1) is now implemented. No content change required in this cycle.

**Lens verdict: PASS**
