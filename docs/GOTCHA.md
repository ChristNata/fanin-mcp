# fanin-mcp — Gotchas

Quick-reference trap list for anyone (human or agent) touching this codebase. Format: **symptom → cause → fix**. Items marked ✅ are already enforced by the design/tests; they're listed so nobody "simplifies" them away.

## Protocol / MCP

1. **Everything breaks with garbled JSON-RPC errors the moment you add a `println!`.** stdout *is* the MCP transport once `serve(stdio())` runs. → All output to stderr or the log file via `tracing`. ✅ (convention + review rule)

2. **A tool call to one upstream times out mysteriously, nothing in the logs.** The upstream sent a request *to us* (`sampling/createMessage`, `elicitation/create`, `roots/list`) and is blocked waiting for an answer we never gave. MCP is bidirectional. → `forward.rs` answers everything: declare no sampling/elicitation capabilities upstream, instantly reject strays, return empty `roots/list`. ✅ (Phase 1, not polish)

3. **The LLM sees "tool failed" with no usable detail.** Someone returned an upstream failure as a JSON-RPC `ErrorData` instead of `CallToolResult { isError: true }`. JSON-RPC errors are for protocol problems (bad params, unknown method); tool-level failures must stay in the conversation as structured JSON the model can read and retry on. ✅ (verified working on CC and OC)

4. **Images/resources come out of `invoke_tool` corrupted or stringified.** Result content was re-serialized through a text-only path. → Pass all content block types through byte-faithfully; never `to_string()` a content array. ✅

5. **Upstream with many tools seems to be missing some.** `list_tools()` returns one page. → Use `list_all_tools()` for upstream discovery (handles pagination).

6. **Our generous 120s server timeout "doesn't work" — calls die at ~60s anyway.** The *client* has its own MCP timeout (CC: `MCP_TOOL_TIMEOUT` env var; OC: unverified). Ours exists to fail informatively *first* and free resources; the client's is the hard ceiling. → Document the interplay; tell users to raise both for long-running tools.

7. **CC requests `tools/list` at every session start.** Any logic on the `tools/list` path that touches upstreams executes *every session* — this is exactly how the original spec accidentally destroyed lazy loading (DECISIONS.md D-003). → Meta-tool descriptions are static; nothing on that path may connect to an upstream.

8. **Capabilities not declared = feature silently dead.** No `tools` capability downstream → clients never call us. Declaring sampling/elicitation upstream → servers send requests we won't serve (MVP). → Declare exactly what we handle, both directions.

## Permissions

9. **User approves one prompt, and everything in the namespace is approved forever.** Every upstream call is the single tool `invoke_tool` — client per-tool permission prompts collapse by construction. → Namespace ACLs are the real permission layer; conservative annotations (`destructiveHint: true`) make CC prompt rather than auto-allow. ✅

10. **Annotations do nothing on OpenCode.** Verified empirically: OC exposes only name/description/schema to the model and didn't gate a `destructiveHint: true` tool. → Never rely on annotations for safety; on OC the namespace is the *only* gate.

## Windows (primary platform)

11. **Zombie `node.exe` processes accumulate until reboot.** npm servers must spawn as `cmd /c npx ...`; killing `cmd.exe` does **not** kill its children on Windows. The classic MCP-on-Windows complaint. → Job Object with kill-on-close around every spawned tree; hard-kill CI test asserts zero survivors. ✅ (D-009). **Spawn-then-assign race (closed in Phase 5):** assigning the child to the Job Object *after* it is already running leaves a window where a descendant forked at startup escapes `KILL_ON_JOB_CLOSE`. → The child is created **suspended** (`CREATE_SUSPENDED` via process-wrap), assigned to the Job Object, then resumed — no window. Regression test forks a descendant immediately at startup. ✅

12. **`Command::new("npx")` fails with "file not found" on Windows.** `npx` is a `.cmd` script, not an executable. → `cmd /c` wrapper (inside the Job Object — see #11).

## Unix

13. **`cred set` fails on a headless Linux box / container / WSL.** No D-Bus / Secret Service available for the keyring. → The resolution chain falls back to process env automatically; error messages must say *which backend* failed and point at the env fallback. ✅

14. **An upstream that forks survives session teardown.** Kill-on-drop only hits the direct child. → Fresh process group per spawn (`setsid`); **graceful** teardown (stdin-EOF) `killpg`s the whole group, grandchildren included, on **both** Linux and macOS. ✅ The **hard-kill** (`kill -9` of `fanin-mcp`) story is platform-asymmetric and stated honestly:
    - **Linux:** each direct child sets `prctl(PR_SET_PDEATHSIG, SIGKILL)` in `pre_exec`, so the kernel kills *it* when `fanin-mcp` dies — but `PDEATHSIG` is **one level only**, so a **grandchild** forked by the upstream (the `npx → node` shape) **may orphan** on a hard kill. Direct-child crash-safe; whole-tree NOT.
    - **macOS:** no `PDEATHSIG` equivalent — a hard kill may orphan the upstream and its descendants.
    - **Windows (#11):** the Job Object's `KILL_ON_JOB_CLOSE` reaps the **whole tree** on any death — the only platform with full crash-safe whole-tree containment.

    True whole-tree hard-kill on Unix would need PID namespaces / cgroups (privileged, heavier) — deliberately not done (minimal / no-daemon identity). The whole-tree hard-kill orphan test (`hard_kill_orphan_test_no_surviving_descendants`, which forks a *grandchild*) is therefore **`#[cfg(windows)]`-only**; the direct-child hard-kill test (`hard_kill_kills_immediate_startup_descendant...`) runs on Windows+Linux (PDEATHSIG covers it); the graceful `stdin_eof_teardown` test runs on all OSes. Prefer graceful shutdown on Unix. ⚠️ (D-009's Unix half — scoped honestly.)

## Naming & Parsing

15. **A server named `my__db` makes `my__db__query` ambiguous.** → Server names validated at config load (`[a-z0-9-]+`, `__` rejected); parsing splits on the **first** `__` only, since upstream *tool* names may legitimately contain `__`. ✅

## Concurrency

16. **One slow postgres query freezes every tool call in the session.** A lock was held across an upstream `call_tool` await. The original reference snippet in early drafts of AGG-MCP.md had exactly this bug. → Lock the registry map only to get/insert `Arc<RunningService>`; clone the Arc; **drop the lock**; then await. Concurrency test in CI. ✅ (D-007)

17. **A cold upstream gets spawned twice under racing first-calls.** Check-then-insert without a guard. → Per-server async init guard; re-check after acquiring. ✅

## Security

18. **Secrets visible in `ps` / Task Manager / shell history.** Someone added a `--value` flag to `cred set`. → Values come from a hidden stdin prompt only — argv is world-readable. ✅ (test the subcommand surface in review)

19. **A secret shows up in a debug log line.** Easy to do once env maps get `Debug`-printed. → The tracing redaction layer scrubs resolved values, and the sentinel-secret test (release gate) catches regressions. Never `{:?}` a resolved env map outside it. ✅

20. **A malicious upstream "speaks" to the model through its tool descriptions.** Upstream-provided names/descriptions flow into text the LLM reads — a prompt-injection channel by design. → Control-neutralization (strip C0/C1/DEL/Unicode separators/bidi/zero-width/BOM → space, trim, single-line) applies to ALL LLM-visible display annotations — `list_tools` description rows AND `get_tool_schema` `title`/`description`/`$comment`/`markdownDescription`. The ~100-char LENGTH cap is `list_tools` description ROWS ONLY, NOT `get_tool_schema` annotations (those are relayed FULL-LENGTH after neutralization, so real argument docs are not silently truncated); tool-name identifiers are capped at 200 — defense-in-depth against a non-rmcp upstream emitting an over-long raw name. `invoke_tool` arguments and result content pass through VERBATIM (D-004) — the residual, bounded, documented injection channel. SECURITY.md documents that this bounds, not eliminates, the channel. ✅

21. **`npx -y some-server` runs whatever was published five minutes ago.** Adding an upstream = executing its code as the user, and floating tags make every session a supply-chain event. → Docs and examples always pin versions (`@1.2.3`); config loader warns on unpinned npx invocations (nice-to-have).

22. **One compromised upstream reads another's credentials.** Only possible if children inherit a shared environment. → Each spawn gets *only its own* resolved vars, never the aggregator's full env. ✅ (D-010)

## rmcp / Implementation

23. **Compiler fights every snippet from AGG-MCP.md.** rmcp's API has shifted across versions (trait signatures, capability builders, transports). → The doc's banner is law: snippets are pseudocode; pin exact version; verify against the pin; commit `Cargo.lock`. ✅

24. **`RunningService` won't `Clone`.** By design. → Store `Arc<RunningService>` in the registry map (which also enables the lock discipline in #16).

## Ecosystem & Positioning

25. **"Doesn't Claude Code's Tool Search already do this?"** It defers *schemas* on CC only. It does not defer *process spawning* (8 configured servers = 8 processes at startup), does nothing for other clients, and offers no namespaces or credential centralization. → Lead with configure-once / creds / namespaces; context savings are the bonus. (README is already framed this way.)

26. **"Your 600-token claim is wrong, here's my measurement" — issue #1 after launch.** → Token figures in the README are generated by the in-repo benchmark per release, never hand-edited. ✅

27. **Name collisions are everywhere in this space.** `mcpmux` is taken *twice* (desktop gateway + npm aggregator with a near-identical meta-tool design); `plexus`/`backplane` are squatted crates. → `fanin-mcp` chosen; final `cargo publish --dry-run` check at repo creation. Treat the npm `mcpmux` as the closest competitor when writing comparisons.

## Operational

28. **Two concurrent sessions, one upstream that binds a port → second session's tool calls fail confusingly.** Per-session architecture means per-session upstream instances (D-001's accepted cost). → Documented limitation; `singleton` warning field on the v1.2 roadmap.

29. **Upstream debug spew floods the client UI.** Child stderr inherited the aggregator's stderr, which CC surfaces. → Pipe child stderr, prefix `[server]`, write to the log file. ✅

30. **A directory-scoped server (e.g. Morph fast-apply) edits files in the wrong tree.** Servers like `@morphllm/morphmcp` operate on a working directory and auto-detect the workspace root (`.git`, `package.json`, `Cargo.toml`…), falling back to the current directory. Spawned as a child, the upstream inherits *fanin-mcp's* CWD — not the coding session's project root — so auto-detection/fallback can target the wrong files. The failure is silent and confusing ("Morph edited some other repo"). → Set a per-server `cwd` in config (supports `${VAR}`); `process.rs` applies it via `Command::current_dir` at spawn — stdio only, ignored for HTTP, with empty/whitespace rejected both at config-load and after `${VAR}` resolution. ✅ (D-019; ARCHITECTURE.md "Child working directory".)

31. **A "read-only" namespace containing a full-filesystem server isn't read-only.** Morph with `ALL_TOOLS: "true"` (and similar servers) exposes create/write/list beyond its headline edit tool. Putting it in a namespace you *think* is read-only silently grants mutation. → The namespace tool-filter is name-level: expose only the read/edit tools you intend (e.g. just `edit_file`), or run the server in its own restricted mode (`ALL_TOOLS: "false"`). Argument-level safety is still the upstream's job. (D-006, D-019; SECURITY.md.)

32. **A hung or slow upstream freezes `list_tools` / `get_tool_schema` / the first `invoke_tool`, and `timeout_secs` doesn't save you.** Only the `tools/call` await was inside the per-server timeout; the connect/`initialize` handshake, the initial `list_all_tools` discovery, and the `list_changed` dirty-refetch were unbounded — and the per-server init guard is held across connect, so one hung cold-start queues every later call to that server. → The timeout envelope now wraps *all four* upstream awaits; on expiry it returns structured `upstream_timeout`, caches no entry, releases the init guard, and drops the process-containment guard to reap the half-connected child. ✅ (D-012; verified by hang-probe + during-window containment tests.)

33. **Capability advertisement starts upstreams before the user calls a tool.** `initialize.instructions` or the `list_tools` description suffix called inventory rather than rendering configured or cached data. → Both advertisement channels are config/cache-only; upstreams remain lazy until the `list_tools` meta-tool or a real invocation. ✅ (D-021)

34. **A stale capability cache exposes a denied tool.** Cache content was treated as permission state. → The cache is display-only; `invoke_tool` always checks live `ActiveNamespace::is_tool_allowed`. ✅ (D-022)

35. **A namespace writes its capability cache outside the cache directory.** A namespace name containing a separator or traversal component was used directly as a filename. → Accept only a single normal path component as the cache path stem; invalid names skip cache read/write and behave as a cache miss. ✅
