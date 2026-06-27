# Adversarial Review: fanin-mcp @ v0.6.15 (HEAD 6d5b66c)

**Reviewer:** deepseek-v4-pro  
**Lens:** adversarial (full-codebase security + correctness sweep)  
**Date:** 2026-06-27  
**Artifact:** `review-adversarial-deepseek.md`

## Summary

Found 0 blocker, 0 structural, 4 targeted, 3 trivial.

## Attack Surface: Verified-Holds Items

Before surfacing findings, items from the attack-surface list that **genuinely hold** with evidence:

### 1. stdout-is-transport (GOTCHA #1) — HOLDS ✅
No `println!`, `print!`, or `dbg!` exists on any `serve`-reachable path. The three `eprintln!` calls in `src/main.rs` (lines 122, 177, 342) are on the `cred` subcommand path and the pre-serve startup path — both execute before `serve(stdio())`. The test suite (`tests/integration/phase4_guard.rs` SC 17) asserts no stray stdout bytes parse as JSON. All diagnostics route to stderr via `tracing`.

### 2. Lock-across-await (D-007, GOTCHA #16) — HOLDS ✅
`registry.rs` uses `tokio::sync::Mutex` for `init_guards` (async-safe) and `tokio::sync::RwLock` for `entries`. The lock discipline is clean:

- `entries.read().await` guards are dropped within the same statement (lines 84, 97, 168-173).
- `entries.write().await` at line 134 only does synchronous `insert`/`clone`.
- `entry.tools.write().await` at line 291 only does synchronous `*guard = fresh`.
- The per-server `init_guard` (line 96) is a `tokio::sync::MutexGuard<()>` — holding it across `.await` is correct (async mutex).
- The actual upstream call (`peer().call_tool`) at line 189 holds no registry lock.

The cross-upstream concurrency test (`tests/integration/multi_upstream.rs`) asserts concurrent alpha slow + beta fast complete independently.

### 3. Bidirectional traffic (D-008, GOTCHA #2) — HOLDS ✅
`forward.rs` answers every upstream-originated request:
- `create_message` → rejected instantly with `McpError::invalid_request` (line 65).
- `create_elicitation` → rejected instantly (line 80).
- `list_roots` → returns empty list (line 94).
- `on_logging_message` → routed to redacted log file (line 97-104).
- `on_progress` → routed to redacted log file (line 106-113).
- `on_tool_list_changed` → sets per-server dirty flag, returns immediately (line 115-126).

No silent hang possible. Test `tests/integration/reverse_traffic.rs` asserts `needs_sampling` completes within deadline.

### 4. Errors as CallToolResult (D-005, GOTCHA #3) — HOLDS ✅
`Aggregator::call_tool` always returns `Ok(CallToolResult::...)` — even `not_implemented_result` wraps in `Ok`. `ToolError::as_result()` produces `CallToolResult::error(vec![Content::text(...)])`. The structured JSON shape (`server`, `tool`, `code`, `message`, `recoverable`) is consistent across all variants (`error.rs` lines 142-151). JSON-RPC errors are reserved for protocol-level problems (bad params, unknown method) on the upstream client path (`forward.rs` create_message/elicitation rejections), which is correct — those are protocol-level rejections to the upstream, not downstream visible.

### 5. Byte-faithful results (D-004, GOTCHA #4) — HOLDS ✅
`handle_invoke_tool` (server.rs line 346) calls `registry.call_tool(...)` and returns the `CallToolResult` directly without transformation. The `.to_string()` calls on lines 196 and 264 are in `handle_list_tools` (display metadata rows) and `handle_get_tool_schema` (schema display) respectively — both are `Content::text(...)` wrappers for metadata, not result passthrough. Test `sanitization.rs:invoke_tool_result_content_not_sanitized_passes_byte_faithfully` asserts non-text content blocks survive round-trip.

### 6. Process-tree lifetime (D-009, GOTCHA #11/#14) — HOLDS ✅
Windows: suspended-spawn → Job Object assignment → resume via `process-wrap` wrappers (`JobObject`, `KillOnDrop`) in `spawn_stdio_transport` (process.rs lines 268-276). The self-Job (`WindowsSelfJobGuard`) is outer containment (lines 176-224). The documented macOS hard-kill orphan gap is the ONLY gap — the code is honest: `PDEATHSIG` is `#[cfg(target_os = "linux")]` only (line 347), macOS has `ProcessSession` for graceful teardown only (line 274-276), and `ContainmentGuard::Inert` is used on non-Unix/non-Linux paths. No hidden gaps.

### 7. Secrets never on argv, never in logs (D-010, GOTCHA #18/#19/#22) — HOLDS ✅ (with one trivial gap noted below)
- `cred set` reads via `rpassword::prompt_password` hidden prompt (credentials.rs line 179).
- Redaction layer: `REDACTED_SECRETS` global set (process.rs line 42), `redact()` function (line 60), applied at stderr (line 496-499), file log (line 550-552), child stderr (line 461), and upstream notifications (forward.rs line 134).
- Least-privilege injection: `spawn_stdio_transport` calls `cmd.env_clear()` then injects only that server's vars (process.rs lines 253-257).
- Error messages never carry secret values — `ToolError` variants hold only server/key names, error display strings, and transport error messages.
- No secret in any `Debug`/`Display` impl reachable at runtime.

## Findings

---

### Finding 1: Missing `cwd` config field — documented but not implemented

- **File:** `src/config.rs:96-127`, `src/process.rs:247-301`
- **Severity:** targeted
- **What:** `ServerConfig` has no `cwd` field, and `spawn_stdio_transport` never calls `cmd.current_dir(...)`. ARCHITECTURE.md (line 97), PRD Req 5 (line 58), D-019, and GOTCHA #30 all specify a per-server `cwd` working-directory override. The PRD lists `cwd` as an MVP requirement ("A single config file defines all upstream servers (command, args, env, transport type, optional `timeout_secs`, optional `description`, optional `cwd` working-directory override, optional HTTP `headers`).")

- **Why exploitable:** Directory-scoped upstreams (e.g., Morph `@morphllm/morphmcp`) auto-detect a workspace root via `.git`, `Cargo.toml`, `package.json` walking, falling back to the current directory. Spawned as a child without `current_dir` set, they inherit fanin-mcp's CWD — not the coding session's project root. This causes silent misdirection: Morph edits files in the wrong repository tree with no visible error. The user cannot configure this because the field is absent.

- **Fix:** Add `cwd: Option<String>` to `ServerConfig` (support `${VAR}` interpolation via `resolve_env_value`). In `spawn_stdio_transport`, after `cmd.args(...)`, call `cmd.current_dir(resolved_cwd)` when configured. Default to inheriting fanin-mcp's CWD (current behavior).

---

### Finding 2: Poisoned global `static Mutex` — `expect()` panics on runtime-reachable path

- **File:** `src/process.rs:55`, `:63`, `:671`
- **Severity:** targeted
- **What:** Three calls to `redacted_secrets().lock().expect(...)` and `writers.lock().expect(...)` use `std::sync::Mutex` via `OnceLock`. If a thread panics while holding one of these mutexes, the mutex is poisoned. Subsequent callers hit `.expect()` and the entire process panics — a DoS for the proxy.

- **Why exploitable:** The `register_secret` and `redact` functions are called from `resolve_env_value` (called during `get_or_connect`), `spawn_stdio_transport` (spawn), `emit_stderr_line` (child stderr I/O), `append_log_line` (upstream notifications), and the tracing `Write` impls. While the code inside these critical sections is trivial string operations (unlikely to panic), Rust's poison-on-panic semantics mean any panic in ANY code that happens to share a thread with these mutexes (e.g., a `tokio::spawn` task encountering an unhandled error) could poison the mutex. The aggregator then panics on the next tool call that triggers stderr capture or secret registration.

- **Fix:** Replace `.expect("...")` with `.unwrap_or_else(|poison| poison.into_inner())` to recover from poisoning. The poisoned state doesn't corrupt the `HashSet<String>` — recovering the inner value is safe.

---

### Finding 3: Stale `#[allow(dead_code)]` on `CredentialResolution` after wiring

- **File:** `src/error.rs:66`
- **Severity:** targeted
- **What:** `ToolError::CredentialResolution` carries `#[allow(dead_code)]` with the comment "Allowed dead_code in Phase 1; wired by Phase 2 interpolation." The variant IS constructed in `process.rs:114-117` (`resolve_env_value`), so the annotation is stale.

- **Why it matters:** A stale `#[allow(dead_code)]` on an error variant that IS now reachable suppresses the compiler's dead-code detection. If the variant were accidentally made unreachable in a future refactor (e.g., removal of `resolve_env_value`), the compiler would not warn. This is a maintenance hazard, not a current behavior bug.

- **Fix:** Remove the `#[allow(dead_code)]` attribute.

---

### Finding 4: `sanitize_upstream_identifier` lacks length cap — context bloat vector

- **File:** `src/server.rs:401-413`
- **Severity:** targeted
- **What:** `sanitize_upstream_identifier` strips control characters but does NOT length-cap identifiers, unlike `sanitize_upstream_text` which caps at 100 characters. Upstream tool names appear in `list_tools` output rows under the `tool`/`name` fields. rmcp 1.8.0 registers tool names up to 128 characters. A malicious upstream (or a poorly-named legitimate tool) could produce `list_tools` output with very long tool name strings, bloating context.

- **Why it matters:** While rmcp 1.8.0 caps registration at 128 characters (limiting the practical maximum), `sanitize_upstream_identifier` is applied to the raw tool name without any cap. The F2 test fixture (`sanitization.rs` `f2_long_named_tool...`) uses a 120-character name that fits under rmcp's cap. A proxy that connected to an upstream with a name at the 128-char ceiling would emit a 128-char `tool` field in every `list_tools` row — not catastrophic, but a bounded prompt-injection/context-bloat channel. More importantly, if rmcp's registration ceiling were raised in a future version, fanin-mcp would have no defense.

- **Fix:** Apply a generous length cap (e.g., 200 chars) on identifiers after sanitization, matching rmcp's current ceiling plus headroom. This preserves dispatchability for realistic tool names while bounding adversarial bloat. Alternatively, document that this is bounded by rmcp's registration ceiling (128 chars as of 1.8.0) and the cap is intentionally absent to avoid breaking dispatch on long-but-valid upstream names.

---

### Finding 5: Literal HTTP header values bypass secret registration

- **File:** `src/registry.rs:124-130`
- **Severity:** trivial
- **What:** In `get_or_connect`, resolved header values are registered for redaction ONLY when the raw template contains `${VAR}`:
  ```rust
  if raw.contains("${") {
      crate::process::register_secret(&resolved);
  }
  ```
  A literal header value like `Authorization = "Bearer sk-abc123"` (anti-pattern, but syntactically valid in the config) bypasses `register_secret`. The value is never added to the redaction set.

- **Why negligible:** The documented config pattern is to use `${VAR}` placeholders for all secrets. Literal header values are an antipattern the docs and examples explicitly discourage. Furthermore, header values don't reach child stderr (they're only sent over HTTP as request headers), so the exposure surface is limited to: (a) tracing spans that might include header values (none exist in the current code), and (b) HTTP transport internals which are outside fanin-mcp's logging path. Defense-in-depth gap only.

- **Fix:** Register resolved header values unconditionally (remove the `if raw.contains("${")` guard) OR emit a startup warning when a literal-looking value is found in the `headers` map (heuristic: long random strings). The config validation comment at ARCHITECTURE.md line 91 mentions "Literal-looking secrets in `env`/`headers` values (heuristic) → warning" — this was planned but not implemented for headers.

---

### Finding 6: `#[allow(dead_code)]` on `CredentialStore` trait is unnecessary

- **File:** `src/credentials.rs:36`
- **Severity:** trivial
- **What:** The `CredentialStore` trait carries `#[allow(dead_code)]`. The trait is actively used via `dyn CredentialStore` in `build_store` (line 161), `resolve_env_value` (process.rs line 75), and the `KeyringStore`/`EnvStore` impls. The annotation suppresses dead-code warnings for the trait definition, but no dead code exists.

- **Fix:** Remove the attribute. It may have been needed when the trait was introduced before consumers existed; the consumers now exist.

---

### Finding 7: `CredentialSet` subcommand writes credential names to stdout via `eprintln!`

- **File:** `src/main.rs:342`
- **Severity:** trivial
- **What:** The `CredAction::List` handler uses `eprintln!("{}", n)` to print credential names. This is stderr, not stdout — correct per the comment on line 340-341. However, the comment asserts "Print names to stderr (via tracing)" but the actual output uses `eprintln!`, not `tracing`. This means `cred list` output bypasses the tracing redaction layer.

- **Why negligible:** `cred list` prints credential NAMES (keys), never values. These are user-chosen identifiers like `POSTGRES_URL`, not secrets. The output goes to stderr which is a CLI surface (not the JSON-RPC transport). No secret exposure.

- **Fix:** Switch `eprintln!` to `tracing::info!` for consistency, or leave as-is with a corrected comment.

---

## Verdict

**This code is production-ready from a security standpoint.** The critical attack surfaces — lock discipline, byte-faithful passthrough, bidirectional traffic handling, error routing (CallToolResult vs JSON-RPC), process-tree containment, and secret redaction — are all correctly implemented with confirming tests. The four targeted findings are all contained and fixable without re-planning: missing `cwd` config field (feature gap, not a vulnerability), mutex poisoning recovery (defensive hardening), stale `#[allow(dead_code)]` (maintenance hygiene), and identifier length cap (defense-in-depth).

**Single most important thing to fix:** Add the `cwd` field to `ServerConfig` and wire it into `spawn_stdio_transport` (Finding 1). This is a documented MVP requirement that silently breaks directory-scoped upstreams like Morph — the most impactful real-world correctness gap.
