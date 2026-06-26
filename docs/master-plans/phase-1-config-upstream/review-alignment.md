# Review Alignment: phase-1-config-upstream

Found 0 blocker, 0 structural, 2 targeted, 0 trivial.

## Suite run

- `cargo test --test integration` passed: 56 passed, 0 failed, 2 ignored.
- `cargo test` passed: 61 passed total, 0 failed, 2 ignored.

## Findings

- File: `src/registry.rs:138`
  Severity: targeted
  Spec: `master.md` Phase 3 Key Behaviors requires cached inventory fetched
  with the pagination-safe helper when available; `rmcp-general` / Context7
  confirm `Peer<RoleClient>::list_all_tools()` exists and fetches every page.
  What: Upstream inventory uses `peer().list_tools(None)` and caches only the
  first page. A paginated upstream will have tools silently missing from
  `list_tools`, `get_tool_schema`, and `invoke_tool` lookup.
  Fix: Replace the discovery call with `peer().list_all_tools().await` and cache
  that returned `Vec<Tool>`.

- File: `src/config.rs:80`
  Severity: targeted
  Spec: `master.md` Scope Out forbids HTTP/remote transport wiring and
  Constraints state Phase 1 is single stdio upstream only; `tests.md` config
  schema says `transport` may be omitted and defaults to `"stdio"`.
  What: `transport` is deserialized but never validated. A config with
  `transport = "http"` can pass startup validation and will later be treated as
  a stdio child command, silently accepting an out-of-scope transport value.
  Fix: Validate each server's transport during config load: accept only absent
  or `"stdio"`; fail startup before serving for any other value.

## Success criteria check

| # | Verdict | Evidence |
|---|---|---|
| 1 | Pass | `config.rs:126-130`, `main.rs:107-123`; integration config tests passed. |
| 2 | Pass | `config.rs:139-204`; uppercase, underscore, and `__` rejection tests passed. |
| 3 | Pass | `config.rs:160-169`, `main.rs:107-113`; unknown namespace test passed before serving. |
| 4 | Pass | `server.rs:68-100` keeps downstream `tools/list` static; static discovery tests passed. |
| 5 | Pass | Registry is constructed but not connected at startup; lazy init tests passed under 500ms. |
| 6 | Pass with issue | `server.rs:125-184` returns live rows, but inventory caching is first-page-only; see finding 1. |
| 7 | Pass with issue | `server.rs:186-246` returns cached schema, but can miss paginated tools; see finding 1. |
| 8 | Pass | `server.rs:264-300`, `registry.rs:106-112`; first-`__` split and raw argument tests passed. |
| 9 | Pass | `registry.rs:108-118`, `error.rs:44-78`; upstream success/error result tests passed without JSON-RPC errors. |
| 10 | Pass | `server.rs:300-302` returns the upstream `CallToolResult` directly; non-text preservation test passed. |
| 11 | Pass | `registry.rs:51-84`; downstream `tools/list` no-spawn and first meta-tool spawn tests passed. |
| 12 | Pass | `registry.rs:56-67`; concurrent first-call test passed. |
| 13 | Pass | `registry.rs:51-84`, `registry.rs:98-112`; map guards are not held across `call_tool`; slow/echo test passed. |
| 14 | Pass | `forward.rs:35-40`; empty client capabilities; reverse capability proxy tests passed. |
| 15 | Pass | `forward.rs:72-77`; roots/list test passed. |
| 16 | Pass | `forward.rs:42-70`; sampling and elicitation rejection tests passed within deadline. |
| 17 | Pass | `forward.rs:79-99`, `process.rs:30-82`; log/stderr prefix tests passed. |
| 18 | Pass | `process.rs:24-28`; child stderr is piped or nulled, not inherited to stdout; stdout JSON tests passed. |
| 19 | Pass | No stdout write macros in `src/` beyond a doc comment; config failure stdout tests passed. |
| 20 | Pass | Required suite passed at 100%; ignored tests are manual live-client E2E only. |

## Scope and invariant check

- In-scope modules are present: TOML config, lazy registry, reverse handler,
  stdio child spawn/logging, namespace ACL, structured tool errors, and real
  meta-tool dispatch.
- Scope-out mostly held: no keyring, hidden prompt, timeout, cancellation,
  Job Object/process-group, HTTP client, OAuth, database, plugin loader, or
  first-class upstream tool re-export landed in `src/`.
- The one scope-adjacent drift is config accepting non-stdio transport strings;
  see finding 2.
- `tools/list` remains exactly the three static meta-tools and does not touch
  the upstream registry.
- Tool-name parsing uses `split_once("__")`, so later `__` remains part of the
  upstream tool name; server names with `_` / `__` fail config validation.
- Namespace ACL is applied to `list_tools`, `get_tool_schema`, and
  `invoke_tool`; denied paths return structured `isError` tool results.
- `invoke_tool` returns the upstream `CallToolResult` directly, preserving
  content arrays instead of stringifying them.

Verdict: PASS-WITH-ISSUES
