# Review Adversarial: phase-2-multi-namespace

Found 0 blocker, 0 structural, 0 targeted, 0 trivial.

## Test run

- Command: `cargo test --test integration`
- Result: PASS — 67 passed, 0 failed, 2 ignored.

## Probes

### Tool-name matching

- Attack: smuggle a second separator inside an upstream tool name, e.g.
  `alpha__tool__with__separator`.
- Code path: `server.rs:323-328` uses `split_once("__")`, so only the first
  separator selects the server; the full remaining suffix is the upstream tool
  name. `config.rs:193-221` rejects `_` in server names, so the split is
  unambiguous.
- Verdict: BLOCKED. This is covered by the implementation shape and by
  `invoke::invoke_tool_splits_on_first_double_underscore_only` in the passing
  integration suite.

- Attack: use case changes, Unicode, whitespace, or lookalike characters to
  bypass a tool allow-list.
- Code path: `namespace.rs:56-63` compares the requested tool name to each
  allow-list entry with exact Rust string equality (`t == tool`). Server names
  are constrained to ASCII lowercase/digit/hyphen in `config.rs:193-221`.
- Verdict: BLOCKED. Matching is exact and case-sensitive. Unicode/whitespace
  tool names only match if the config names the same bytes; they do not widen
  access.

- Attack: configure an empty tool allow-list, e.g. `alpha = []`, and rely on an
  empty list being treated like an absent list.
- Code path: `namespace.rs:60-63` uses `map_or(true, |list| list.iter().any(...))`.
  An absent key allows all tools; a present empty vector makes `any` false for
  every tool.
- Verdict: BLOCKED. `[]` denies every tool for that server.

### Config edge cases

- Attack: add a `tools.<server>` entry for a server not listed in that
  namespace's `servers` array.
- Code path: `ActiveNamespace::new` copies `tools` (`namespace.rs:29-38`), but
  `is_tool_allowed` first checks `is_server_allowed` and returns false before
  consulting the tool list (`namespace.rs:56-59`).
- Verdict: BLOCKED. The stray tool filter is inert; it cannot grant access to a
  server outside `servers`.

- Attack: add a `tools.<server>` entry for a server that is not configured in
  `[servers]`.
- Code path: if that server is not in the namespace `servers`, it is blocked by
  `namespace.rs:56-59`. If it is in `servers`, the request later fails the
  configured-server check in `server.rs:223-228` / `server.rs:289-294` or the
  list path's `server.rs:164-166`.
- Verdict: BLOCKED. It does not expose another server or connect upstream.
  This is not fail-fast, but `tests.md:86-90` explicitly makes tool-name
  validation optional and only says server-key validation MAY be added.

- Attack: duplicate TOML keys to override a restrictive allow-list with a wider
  one.
- Code path: `config.rs:133-136` delegates parse to `toml::from_str`; duplicate
  keys in a TOML table are parse errors, not merged maps.
- Verdict: BLOCKED by parser semantics. No source-side merging path exists.

- Attack: namespace has `servers = []` but populated `[namespaces.x.tools]`,
  hoping tools grant server visibility.
- Code path: `namespace.rs:56-59` denies any tool when the server is not in the
  server allow-list; `namespace.rs:65-70` returns no servers for discovery.
- Verdict: BLOCKED. Empty `servers` is deny-all for that namespace.

### Path coverage

- Attack: hide a denied tool from `list_tools` but invoke it directly through
  `invoke_tool`.
- Code path: `list_tools` filters each inventory row through
  `namespace.is_tool_allowed` (`server.rs:171-174`); `get_tool_schema` checks
  the same predicate before discovery (`server.rs:229-235`); `invoke_tool`
  checks it before argument extraction and forwarding (`server.rs:295-301`).
- Verdict: BLOCKED. The dangerous direct-invoke bypass is closed.

- Attack: fetch a denied tool's schema even though invocation is denied.
- Code path: `handle_get_tool_schema` parses `server__tool`, validates server
  existence, then denies through `is_tool_allowed` before `registry.inventory`
  is awaited (`server.rs:201-236`).
- Verdict: BLOCKED. Schema read cannot reveal a denied tool's input shape.

### Denial before connect

- Attack: trigger a denied server/tool request that still spawns the upstream,
  leaking intent and wasting a process.
- Code path: denied-server checks happen before `registry.inventory` or
  `registry.call_tool` on all direct paths: `list_tools` server filter denies
  at `server.rs:150-156`; `get_tool_schema` denies at `server.rs:229-235`;
  `invoke_tool` denies at `server.rs:295-301`. Only after these checks does
  code await `inventory` or `call_tool`.
- Verdict: BLOCKED. The passing
  `namespace_acl::denied_server_is_not_spawned_to_reject_call` test adds a
  log-sink proof that the denied upstream is not spawned.

- Attack: denied tool on an allowed server still causes a connection.
- Code path: `get_tool_schema` and `invoke_tool` deny at `server.rs:229-235`
  and `server.rs:295-301`, before `registry.inventory` / `registry.call_tool`.
  `list_tools` must connect to an allowed server to learn its inventory, then
  filters rows at `server.rs:171-174`.
- Verdict: BLOCKED for direct schema/invoke. Discovery of an allowed server may
  connect by design; the denied tool is never emitted.

### Concurrency and lock discipline

- Attack: hold the registry map lock across an upstream await, serializing a
  slow call on `alpha` against a fast call on `beta`.
- Code path: `Registry::get_or_connect` clones an `Arc<UpstreamEntry>` out of
  the read map lock before returning (`registry.rs:51-54`, `registry.rs:64-67`),
  inserts under a write lock without awaiting inside the insert block
  (`registry.rs:78-83`), and `call_tool` awaits `peer().call_tool(...)` only
  after it owns the cloned entry (`registry.rs:98-113`).
- Verdict: BLOCKED. The map lock is not held across the upstream call. The
  same-server init guard is held across connect (`registry.rs:64-78`), but that
  serializes only cold initialization for one server and is the intended
  double-spawn guard.

- Attack: discovery-time filtering calls the registry while holding a namespace
  or registry lock.
- Code path: `ActiveNamespace` is an immutable value with no lock. `list_tools`
  builds the server vector (`server.rs:139-160`), awaits `registry.inventory`
  per server (`server.rs:163-170`), then filters the returned `Vec<Tool>` with
  `is_tool_allowed` (`server.rs:171-174`).
- Verdict: BLOCKED. Filtering does not hold a registry map lock across await.

### Default and unknown namespace

- Attack: omit `--namespace` to expose more than intended.
- Code path: omitted namespace becomes an empty CLI string (`config.rs:50-57`),
  which validation resolves to `default` (`config.rs:175-187`) and
  `ActiveNamespace::new` resolves the same way (`namespace.rs:21-39`).
- Verdict: BLOCKED. Omission selects only `[namespaces.default]`, not all
  namespaces.

- Attack: name an unknown namespace and fall back to a broad namespace or an
  empty, silently-permissive ACL.
- Code path: `TomlConfig::validate` rejects an active namespace missing from
  `[namespaces]` with `StartupError::UnknownNamespace` (`config.rs:175-187`).
- Verdict: BLOCKED. Startup fails before serving.

## Theoretical concerns, not findings

- Denied-server responses distinguish configured-but-denied servers from
  unknown servers because `server.rs:223-235` and `server.rs:289-301` check
  `registry.has_server` before namespace denial. That creates a server-name
  existence oracle. I am not tagging it as a finding here because the Phase 2
  plan and tests explicitly require structured `namespace_denied` for denied
  server paths, including the denied server name (`tests.md:106-109`). If the
  intended security boundary is "hidden means indistinguishable from unknown,"
  the plan must change the public error contract.

## Verdict

The Phase 2 namespace ACL holds against the probed bypasses: denied tools are
blocked on list, schema, and invoke; denied direct calls reject before connect;
and registry map locks are not held across upstream awaits.
