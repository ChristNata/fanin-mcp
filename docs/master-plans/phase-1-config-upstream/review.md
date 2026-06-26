# Review: phase-1-config-upstream

Found 0 blocker, 0 structural, 5 targeted, 0 trivial after de-duplication.

## Verdict

PASS-WITH-ISSUES.

All three lenses returned PASS-WITH-ISSUES. No blocker or structural finding was
reported. Every consolidated finding below is targeted and routes to a debugger
fix.

## Consolidated findings

| Severity | Location | Lenses | Description | Fix direction | Routing |
|---|---|---|---|---|---|
| targeted | `src/registry.rs:138` | alignment, adversarial, general | Upstream inventory discovery caches only the first `tools/list` page, so paginated upstream tools can disappear from `list_tools`, `get_tool_schema`, and `invoke_tool`. | Replace `peer().list_tools(None)` with `peer().list_all_tools().await` for the pinned rmcp API, or otherwise loop on `next_cursor` before caching. | debugger fix |
| targeted | `src/server.rs:299` | adversarial, general | `invoke_tool` accepts missing or non-object `arguments` and still calls the upstream with no arguments. | Require `arguments` to exist and be an object before namespace lookup or dispatch; return structured `ToolError::InvalidRequest` as an `isError` tool result otherwise. | debugger fix |
| targeted | `src/process.rs:42` / `src/process.rs:60` | adversarial, general | Child stderr/log capture uses unbounded line buffering and fire-and-forget append tasks that can lose errors or build unbounded writer fan-out. | Use bounded chunk or capped-line reads, route log lines through one owned bounded writer task per sink, and surface or warn on write/flush failures. | debugger fix |
| targeted | `src/config.rs:80` | alignment | The `transport` field is deserialized but not validated, so out-of-scope values such as `http` pass config validation and are later treated as stdio commands. | Validate transport during config load: accept absent or `stdio`; reject every other value before serving. | debugger fix |
| targeted | `src/error.rs:37` / `src/registry.rs:69` | general | Source formatting drift makes `cargo fmt --all -- --check` fail for `src/error.rs` and `src/registry.rs`. | Run `cargo fmt --all` and keep the generated source formatting changes. | debugger fix |

## Solid surfaces verified

- Lock discipline held: registry map locks are used only for lookup/insert, are
  dropped before upstream inventory or `call_tool` awaits, and the per-server
  init guard is re-checked to avoid double-spawn races.
- Reverse traffic held: sampling and elicitation are rejected immediately,
  `roots/list` returns an empty list, and progress/log notifications do not wait
  on downstream work.
- Byte-faithfulness held: successful upstream `CallToolResult` values are
  returned directly; content arrays are not stringified on the invoke path.
- Stdout integrity held: no `println!`, `print!`, or `dbg!` was found in `src/`;
  tracing writes to stderr and child stderr is piped/null rather than inherited
  to stdout.

## Source lens results

| Lens | Verdict | Count |
|---|---|---|
| alignment | PASS-WITH-ISSUES | 0 blocker, 0 structural, 2 targeted, 0 trivial |
| adversarial | PASS-WITH-ISSUES | 0 blocker, 0 structural, 3 targeted, 0 trivial |
| general | PASS-WITH-ISSUES | 0 blocker, 0 structural, 4 targeted, 0 trivial |
