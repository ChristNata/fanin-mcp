# Review Adversarial: phase-1-config-upstream

Found 0 blocker, 0 structural, 3 targeted, 0 trivial.

Suite run: `cargo test --test integration` passed: 56 passed, 0 failed,
2 ignored.

Context7 check: rmcp 1.8.0 exposes `ClientHandler::{create_message,
create_elicitation,list_roots,on_custom_request}` and `Peer<RoleClient>
::list_all_tools()`, where `list_all_tools()` repeatedly calls paginated
`list_tools()` until all tools are returned.

## Findings

- File: `src/registry.rs:138`
  Severity: targeted
  Lens: adversarial
  What: Upstream discovery reads only the first `tools/list` page.
  Trigger: Configure an otherwise valid MCP upstream that paginates its
    inventory, with tool `late_tool` on page 2. The first call to the
    `list_tools` meta-tool enters `Registry::get_or_connect`, then `connect`,
    then `service.peer().list_tools(None).await` at `src/registry.rs:138-146`.
    rmcp returns only page 1 when the upstream supplies a cursor.
  Impact: `late_tool` is absent from the cached inventory for the whole
    session. `get_tool_schema server__late_tool` and `invoke_tool
    server__late_tool` return `unknown_tool` even though the upstream advertised
    the tool. This violates GOTCHA #5 and the plan's Phase 3 requirement to use
    the pagination-safe helper when available.
  Exploitability / likelihood: Medium. Small probe servers and most tests use
    one page, so the suite misses it. Any production upstream with many tools or
    a low page size triggers it deterministically.
  Fix: Replace `list_tools(None)` with rmcp's `list_all_tools().await` and cache
    the returned `Vec<Tool>`.

- File: `src/server.rs:299`
  Severity: targeted
  Lens: adversarial
  What: `invoke_tool` accepts a missing or non-object `arguments` field and
    still calls the upstream.
  Trigger: A downstream client calls the `invoke_tool` meta-tool with
    `{"name":"probe__dangerous_noop"}` or with `{"name":"probe__x",
    "arguments":null}`. `handle_invoke_tool` validates only the outer
    meta-tool object and `name`; `args.get("arguments").and_then(|v|
    v.as_object()).cloned()` silently yields `None`, then
    `registry.call_tool(server, tool, raw_arguments).await` forwards the call.
  Impact: A malformed meta-tool request can execute an upstream tool with no
    arguments despite the advertised schema requiring an `arguments` object.
    For side-effecting upstream tools, that is an unintended call rather than a
    structured `invalid_request` result. The current suite checks malformed
    names but not missing `arguments`.
  Exploitability / likelihood: Medium. Any client can send the malformed call;
    whether it causes harm depends on the upstream accepting omitted arguments.
  Fix: Require `arguments` to exist and be an object before namespace lookup or
    upstream dispatch. Return `ToolError::InvalidRequest` as
    `CallToolResult { isError: true }` when it is missing or not an object.

- File: `src/process.rs:42`
  Severity: targeted
  Lens: adversarial
  What: Child stderr/log capture has unbounded buffering and unbounded writer
    task fan-out.
  Trigger: Configure an upstream that writes a very large stderr line without a
    newline, or writes stderr/log notification lines faster than the filesystem
    can append them. `BufReader::lines()` at `src/process.rs:42-45` buffers
    until newline with no size cap. Each completed line calls
    `append_log_line`, which spawns a new task at `src/process.rs:61-82` and
    opens/flushes the log file independently.
  Impact: A hostile or noisy upstream can grow memory while a newline is
    withheld, or flood the runtime with pending append tasks and file opens.
    This can stall the proxy session even though stderr is correctly kept off
    stdout.
  Exploitability / likelihood: Medium for untrusted upstreams; low for the
    in-repo probe. The test suite asserts only that normal stderr reaches the
    log sink.
  Fix: Replace line-based unbounded reads with bounded chunk reads or cap line
    length and truncate with a marker. Use one per-server log writer task with a
    bounded channel/backpressure instead of spawning one task per line.

## Surfaces that held

- Concurrency / lock discipline: `entries` locks are used only for lookup or
  insert and are not held across `serve`, upstream inventory, or `call_tool`
  awaits. The per-server init guard is held across first connect by design and
  the map is re-checked after acquisition, so racing first calls do not
  double-spawn under the reviewed interleavings.
- Reverse traffic: sampling and elicitation are rejected immediately,
  `roots/list` returns an empty list, and progress/log notifications do not
  await downstream work. Context7 confirms rmcp also has an `on_custom_request`
  hook; the default is not used for the planned sampling/elicitation/roots
  paths.
- Byte-faithful invoke results: successful upstream `CallToolResult` values are
  returned directly from `registry.call_tool`; there is no content-array
  `to_string()` on the invoke path.
- Stdout integrity: no `println!`, `print!`, or `dbg!` was found in `src/`;
  tracing is configured to stderr and child stderr is piped/null, not inherited
  to stdout.
- Process-tree hard-kill and call timeouts are not implemented, but the master
  plan explicitly defers Job Objects / process groups, cancellation, and
  timeouts out of Phase 1.

Verdict: PASS-WITH-ISSUES
