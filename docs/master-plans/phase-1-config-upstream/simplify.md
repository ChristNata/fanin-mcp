# Simplify: phase-1-config-upstream

## Files simplified

- `src/server.rs` — changed `ServerHandler::call_tool` from a manual future
  wrapper to native `async fn`; this removes the src-owned clippy warning
  without changing dispatch or tool-result behavior.
- `src/config.rs` — collapsed duplicate `command` presence/blankness checks
  into one `matches!` guard and collected invalid server-name characters
  directly into a `String`; validation semantics are unchanged.
- `src/namespace.rs` — avoided cloning a whole `NamespaceConfig` just to build
  the active server set; the ACL now borrows the namespace and clones only the
  server names it stores.

## Files reverted

(none)

Recovery point: `ed9484a94da17063b4b4d4f81e9796bbf06b805b`.

## Files unchanged

- `src/registry.rs` — left unchanged. The existing structure makes the
  D-007 lock discipline visible: map lookup, per-server guard, connect, then
  upstream calls through a cloned `Arc`.
- `src/forward.rs` — left unchanged. Reverse-traffic rejection and empty roots
  handling are already direct and rmcp-signature-sensitive.
- `src/process.rs` — left unchanged. Stderr capture and log appends are simple
  enough for Phase 1; changing the spawn/log ownership model would be behavior
  risk, not free simplification.
- `src/error.rs` — left unchanged. The structured `isError` shape is public
  API and already centralized.
- `src/main.rs` — left unchanged. Startup validation remains before stdio
  serving, preserving the stdout-is-transport invariant.

## Issues spotted

- targeted — The worktree already contained modified test files
  (`tests/integration/discovery.rs`, `tests/integration/gate.rs`) before this
  pass. Tests are read-only for simplifier, so I did not inspect or edit them.
- trivial — The worktree already contained untracked local/orchestration files
  (`.claude/settings.local.json`, `.task-clippytests.md`,
  `.task-simplify.md`). Left untouched as out of scope.
