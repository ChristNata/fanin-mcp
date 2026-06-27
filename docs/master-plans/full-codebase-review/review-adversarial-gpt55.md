# Adversarial Review: full-codebase-review

Found 1 blocker, 0 structural, 1 targeted, 0 trivial.

Gates run: `cargo test` passed (115 integration tests passed, 4 ignored; 5
unit tests passed). `cargo clippy --all-targets -- -D warnings` passed.
`cargo fmt --all -- --check` passed. `cargo deny` could not be run under this
reviewer's bash allowlist.

## Upstream discovery and connect paths can hang outside `timeout_secs`

- Severity: blocker
- Location: `src/registry.rs:132`, `src/registry.rs:378`,
  `src/registry.rs:399`, `src/registry.rs:281`
- Evidence: `get_or_connect` awaits `connect(...)` before any timeout wrapper
  exists (`src/registry.rs:132-133`). Inside `connect`, both the upstream
  `serve(...)` handshake and the initial `peer().list_all_tools().await` run
  unbounded (`src/registry.rs:378`, `src/registry.rs:399-402`). Dirty-cache
  refetch has the same unbounded `list_all_tools().await` path
  (`src/registry.rs:281`). The only configured timeout is applied later, around
  the actual tool call (`src/registry.rs:186-190`).
- Why this breaks: a malicious or broken upstream that accepts stdio/HTTP and
  then never completes initialize or `tools/list` can hang `list_tools`,
  `get_tool_schema`, or the first `invoke_tool` indefinitely. The per-server
  init guard stays held while `connect` is stuck, so every later call to that
  server queues behind the hung future. A `notifications/tools/list_changed`
  event can also move a healthy connection onto the unbounded refetch path and
  hang later calls before the tool-call timeout is reached. This violates the
  D-012 / PRD timeout promise that hung upstreams fail informatively and free
  resources.
- Fix: apply the server's effective timeout to the whole upstream operation
  envelope that can block on remote behavior: spawn/connect handshake,
  initial `list_all_tools`, dirty-cache refetch, and `call_tool`. Return the
  existing structured `upstream_timeout` (or a distinct connect-time timeout
  code if the public error shape is extended deliberately). Add a probe mode
  that hangs during `tools/list` and a list-changed refetch test that proves the
  configured timeout fires.

## Per-server working directory is documented but not implemented

- Severity: targeted
- Location: `src/config.rs:97`, `src/process.rs:247`
- Evidence: `ServerConfig` contains `transport`, `command`, `endpoint`, `args`,
  `env`, `headers`, `log_file`, and `timeout_secs`, but no `cwd` field
  (`src/config.rs:97-127`). `spawn_stdio_transport` builds `Command::new`, adds
  args and env, and never calls `current_dir` (`src/process.rs:247-257`). Grep
  found no `current_dir` / `cwd` handling in `src/`.
- Why this breaks: D-019, ARCHITECTURE.md, and GOTCHA #30 make `cwd` load-bearing
  for directory-scoped upstreams such as Morph. Without it, those servers run
  in whatever directory launched `fanin-mcp`, not necessarily the coding
  session's project root. A filesystem-capable upstream can silently read or
  mutate the wrong repository. That is exactly the documented trap the current
  code claims to close.
- Fix: add `cwd: Option<String>` to `ServerConfig`, interpolate `${VAR}` with
  the same credential/env resolver where appropriate, validate empty paths, and
  call `cmd.current_dir(...)` before spawn. Add an integration probe that echoes
  its cwd and proves the configured value wins.

## Probes that held

- Stdout discipline held for serve paths: source grep found no runtime
  `println!` / `print!` / `dbg!` in `src/`; the remaining `eprintln!` calls are
  stderr-only and occur before serve or in `cred list` (`src/main.rs:122`,
  `src/main.rs:177`, `src/main.rs:342`).
- Registry map locks are not held across upstream tool calls. The call path
  clones an `Arc<UpstreamEntry>` through `get_or_connect`, drops the map lock,
  and only then awaits `peer().call_tool` (`src/registry.rs:83-99`,
  `src/registry.rs:163-190`). The remaining held await is the per-server init
  guard, which serializes only cold starts for that server.
- Reverse traffic is answered. The upstream client declares empty capabilities
  (`src/forward.rs:53-57`), rejects sampling and elicitation immediately
  (`src/forward.rs:60-88`), and returns empty roots (`src/forward.rs:90-95`).
- Tool-level errors stay as `CallToolResult::error` on the downstream surface:
  `ServerHandler::call_tool` wraps dispatch in `Ok(...)`
  (`src/server.rs:111-117`), and `ToolError::as_result` uses
  `CallToolResult::error` (`src/error.rs:136-139`).
- Byte-faithful result passthrough holds for `invoke_tool`: successful upstream
  `CallToolResult` is returned directly (`src/registry.rs:190-200`,
  `src/server.rs:346-350`). The only `to_string()` conversions in the server
  are for aggregator-owned JSON text results (`list_tools` and schema), not
  upstream content arrays (`src/server.rs:195-197`, `src/server.rs:262-264`).
- Sanitization holds for the documented LLM-visible metadata path:
  descriptions and schema annotation strings neutralize control/bidi/zero-width
  characters and cap annotation text to 100 chars (`src/server.rs:381-397`,
  `src/server.rs:415-428`, `src/server.rs:437-470`).
- Windows process-tree containment is wired through `process-wrap` Job Objects
  and the Windows hard-kill descendant test passed. The Unix hard-kill
  grandchild gap is documented honestly in SECURITY.md and GOTCHA #14; I did
  not find a wider code gap beyond the unimplemented `cwd` issue above.

## Verdict

No, not production- and OSS-ready from a security standpoint until the
unbounded connect/discovery/refetch paths are timed out. The most important fix
is to make `timeout_secs` cover every upstream await that can be controlled by
a malicious server, not just the final `tools/call` await.
