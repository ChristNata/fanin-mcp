# review-alignment: phase-3-credentials-lifetime

Lens verdict: FAIL.

Found 2 blocker, 0 structural, 1 targeted, 0 trivial.

## Gate run

- `cargo test --test integration`: PASS — 84 passed, 3 ignored, 0 failed.
- `cargo fmt --all -- --check`: PASS.

The passing suite is not sufficient: one credential-resolution path is shaped to
satisfy the probe's `echo_env` tests rather than the plan's spawn-time failure
contract, and Unix hard-kill containment does not match D-009.

## Findings

- File: `src/registry.rs:108`
  Severity: blocker
  Pass: alignment
  What: Missing `${VAR}` credentials are recorded per env LHS and only returned
        when the later upstream tool call is exactly `echo_env` with a matching
        `key` argument (`src/registry.rs:162`). Other tools run with the missing
        variable silently omitted.
  Why: The plan requires missing credentials to produce a structured tool-level
       error that names the server and variable. This branch handles the test
       fixture shape, not the production contract. A real upstream with
       `API_TOKEN = "${TOKEN}"` can spawn without `API_TOKEN` and fail later in
       an upstream-specific way, or worse, run unauthenticated.
  Cite: D-010; master SC 2 and SC 8; master Phase 2 SC 2; fakery checklist
        "branches that handle only test inputs".
  Fix: Treat unresolved placeholders as a server connect/call failure for any
       targeted tool, returning `credential_resolution_failed` before spawning,
       or store a server-wide resolution error that every call surfaces until the
       credential exists. Do not special-case `echo_env`.

- File: `src/process.rs:183`
  Severity: blocker
  Pass: alignment
  What: The Unix path wraps the child with `ProcessSession` but retains no Unix
        containment guard and has no parent-hard-kill mechanism
        (`src/process.rs:214`). A process group/session alone does not die when
        the parent `fanin-mcp` process is force-killed.
  Why: D-009 and master SC 20/21 require Unix hard-kill to leave zero surviving
       upstream descendants. This implementation may help graceful group
       teardown, but it does not make a SIGKILL/task-kill of the parent kill the
       child tree. The hard-kill gate passed only on this Windows host.
  Cite: D-009 (`docs/DECISIONS.md:62`); GOTCHA #14; master SC 20 and SC 21.
  Fix: Add a Unix hard-kill containment design that actually survives parent
       death, then run the hard-kill orphan test on Unix. If the accepted design
       changes, re-plan D-009 rather than claiming `ProcessSession` alone meets
       the ADR.

- File: `src/main.rs:191`
  Severity: targeted
  Pass: alignment
  What: `cred set` returns success when the selected store rejects the write, and
        `EnvStore::set` is a no-op that also returns success
        (`src/credentials.rs:146`).
  Why: Master SC 3 says `cred set <server> <KEY>` stores the hidden-prompt value.
       A successful exit after no storage makes the CLI lie; later resolution can
       fail even though the user was told the credential was stored. Env fallback
       is a read resolver, not a persistent mutable store.
  Cite: D-010; master SC 3, SC 5, SC 6; GOTCHA #13.
  Fix: If keyring storage fails, return a failure with a redacted diagnostic that
       points users to env fallback. If `--credential-store env` is supported for
       reads only, make `cred set/rm/list` reject it clearly instead of no-oping.

## Per-criterion alignment notes

| # | Verdict | Code evidence |
|---|---|---|
| 1 | Met | `CredentialStore` plus `KeyringStore` and `EnvStore` exist in `src/credentials.rs:30`, `src/credentials.rs:50`, and `src/credentials.rs:132`; registry uses `build_store` at `src/registry.rs:115`. |
| 2 | Partially met | Resolution attempts preferred store then process env in `src/process.rs:106`; missing credentials produce `credential_resolution_failed` in `src/error.rs:117`. But the error is only surfaced for the test-shaped `echo_env` branch (`src/registry.rs:166`), so production calls can bypass it. Finding 1. |
| 3 | Partially met | The CLI surface has no secret argv slot (`src/main.rs:73`) and reads via `prompt_for_secret` (`src/main.rs:180`, `src/credentials.rs:172`). Storage is not guaranteed because rejected writes still exit success (`src/main.rs:197`) and env store set is a no-op (`src/credentials.rs:146`). Finding 3. |
| 4 | Met | `cred list` calls `list_names` only and prints names to stderr (`src/main.rs:213`, `src/main.rs:216`); `KeyringStore::list_names` reads only the key index (`src/credentials.rs:124`). |
| 5 | Partially met | Keyring delete removes the credential and index entry (`src/credentials.rs:114`). Env selected-store delete is a no-op success (`src/credentials.rs:150`), so the selected-store contract is weak. Covered by Finding 3. |
| 6 | Partially verified | Keyring set/get/list code exists (`src/credentials.rs:93`), but the round-trip test is ignored on this host. The code also masks keyring set failures as success (`src/main.rs:197`). |
| 7 | Met | Env fallback is implemented in `EnvStore::get` (`src/credentials.rs:139`) and as the second resolution step (`src/process.rs:109`). |
| 8 | Partially met | Embedded `${VAR}` interpolation exists (`src/process.rs:82`). Missing placeholders are not surfaced generally; Finding 1. |
| 9 | Met | Child env starts from `env_clear` and injects only resolved entries (`src/process.rs:161`). |
| 10 | Met | Non-placeholder env values pass through unchanged (`src/process.rs:79`). |
| 11 | Met | Resolved secrets are registered and redacted in tracing, child stderr, and upstream notification sinks (`src/process.rs:47`, `src/process.rs:380`, `src/process.rs:411`, `src/forward.rs:99`). |
| 12 | Met | `timeout_secs` parses on `ServerConfig` with default 60 (`src/config.rs:113`, `src/config.rs:119`). |
| 13 | Met | `Registry::call_tool` wraps upstream calls in `tokio::time::timeout` using the effective per-server timeout (`src/registry.rs:181`, `src/registry.rs:184`). |
| 14 | Met | Timeout maps to `ToolError::UpstreamTimeout` (`src/registry.rs:191`) and renders code `upstream_timeout` with the D-005 shape (`src/error.rs:105`). |
| 15 | Met | Server `call_tool` returns `Ok(CallToolResult)` and converts tool failures with `as_result`, not JSON-RPC errors (`src/server.rs:111`, `src/server.rs:342`). |
| 16 | Met | Downstream cancellation races `registry.call_tool` against `context.ct.cancelled()` (`src/server.rs:330`). |
| 17 | Met as deferred/conditional | The code explicitly does not forward cancellation because rmcp `=1.8.0` hides the upstream request id (`src/server.rs:336`). This matches resolved OQ3's honesty boundary. |
| 18 | Met | Registry locks are scoped before awaits: entries read is cloned before return (`src/registry.rs:79`), init guard map lock is dropped before `guard.lock().await` (`src/registry.rs:83`), and calls use the cloned entry (`src/registry.rs:154`). |
| 19 | Met on current Windows host | Windows assigns the spawned child to a retained Job Object with kill-on-close (`src/process.rs:222`, `src/process.rs:263`, `src/process.rs:318`). The Windows hard-kill test passed. |
| 20 | Not met | Unix uses `ProcessSession` but retains `ContainmentGuard::None` (`src/process.rs:183`, `src/process.rs:214`), which does not enforce hard-kill cleanup. Finding 2. |
| 21 | Partially met | Mandatory hard-kill test exists and passed on Windows. It does not prove Unix, and the Unix code does not meet the hard-kill contract. Finding 2. |
| 22 | Met on current host | Stdin EOF teardown test passed; child transport is retained in `UpstreamEntry` (`src/registry.rs:28`). Unix hard-kill remains separate Finding 2. |
| 23 | Met | Stderr is piped when `log_file` is set and written with `[server]` prefix after redaction (`src/process.rs:188`, `src/process.rs:386`, `src/process.rs:472`). |
| 24 | Met | `meta_tools` still returns exactly `list_tools`, `get_tool_schema`, and `invoke_tool` (`src/server.rs:68`); descriptions are static constants (`src/server.rs:31`). |
| 25 | Failed | The required tests and fmt passed, but alignment has blockers for SC 2/8 and SC 20/21. |

## Scope and open-question checks

- D-010: Partially met. Hidden stdin and no argv secret are present; list is
  names-only; `env_clear` least-privilege injection is present. Missing-credential
  surfacing and no-op successful `cred set` paths violate the storage/resolution
  contract.
- D-009: Partially met. Windows Job Object code exists and passed on this host.
  Unix hard-kill containment does not match the ADR.
- D-005: Met. New `upstream_timeout`, `credential_resolution_failed`, and
  `call_cancelled` codes preserve `server`, `tool`, `code`, `message`, and
  `recoverable` fields inside `CallToolResult { isError: true }`.
- Scope OUT: No OAuth, HTTP listener, daemon, database, plugin loader, Node
  runtime dependency, parameter-level ACL, or meta-tool name/description change
  landed in `src/` or `Cargo.toml`.
- OQ1: HTTP transport remains deferred; `transport != "stdio"` still fails
  startup (`src/config.rs:178`).
- OQ2: The implementation chose process-wrap plus explicit Windows Job Object.
  Outcome is acceptable on Windows, not on Unix.
- OQ3: Cancellation forwarding is not claimed; comments and behavior honestly
  document local abort only (`src/server.rs:336`).
