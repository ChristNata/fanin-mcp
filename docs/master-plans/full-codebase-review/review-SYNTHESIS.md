# Full-Codebase Review — Synthesis

**Target:** fanin-mcp @ v0.6.15 (HEAD 6d5b66c)
**Date:** 2026-06-27
**Orchestrator:** CC (Covenant), synthesizing 5 dispatched reviewer children.
**Lenses / models:** adversarial (gpt-5.5, deepseek-v4-pro), alignment
(grok-4.3), deep-dive/OSS (glm-5.2 — FAILED, minimax-m3).
**Test suite:** GREEN — `cargo test --all` → 115 passed / 0 failed / 4 ignored
(the 4 are the documented manual remote-HTTP / live-CC-OC checklist gates).
gpt-5.5 and minimax independently re-ran and confirmed `clippy --all-targets -D
warnings` and `fmt --check` clean.

Every NEW or CONTESTED finding below was re-verified in-source by the
orchestrator (not taken on a child's word). Citations are confirmed.

---

## Headline: one real correctness/availability gap

### S-1 — Upstream connect / initial discovery / dirty-refetch run OUTSIDE `timeout_secs`
- **Severity:** structural→blocker (gpt-5.5 said blocker; minimax said
  structural; orchestrator: treat as **blocker for the production/OSS "timeout
  is a guarantee" claim**).
- **Evidence (verified):** `registry.rs:132-133` awaits `connect(...)` with no
  timeout, while holding the per-server init guard (`registry.rs:96`). Inside
  `connect`: `handler.serve(...).await` (`registry.rs:378`/`387`, the rmcp
  initialize handshake) and `service.peer().list_all_tools().await`
  (`registry.rs:399`) are bare awaits. The `list_changed` dirty-refetch
  `ensure_fresh` is also bare (`registry.rs:281`). `timeout_secs` only wraps
  `call_tool` (`registry.rs:190`).
- **Impact:** a hung or malicious upstream (esp. a remote streamable-HTTP server
  stalling on `initialize`, or a `list_changed` event moving a healthy
  connection onto the unbounded refetch path) hangs `list_tools` /
  `get_tool_schema` / first `invoke_tool` indefinitely. The init guard is held
  across `connect`, so every later call to that server queues behind the hang.
- **Spec conflict:** violates **D-012** and the PRD/SECURITY.md promise that
  hung upstreams "fail informatively first and free resources." The guarantee
  currently covers ~1 of 4 blocking awaits.
- **Missed by:** grok's alignment pass (it verdicted D-012 HONORED by looking
  only at the `call_tool` timeout). This is the core value of the multi-lens run.
- **Fix:** wrap the spawn→serve-handshake→initial-list envelope and
  `ensure_fresh` in `tokio::time::timeout(effective, …)`, mapping expiry to the
  existing `ToolError::UpstreamTimeout` (or a distinct connect-timeout code if
  the public error shape is extended deliberately). Add a probe hang-mode test
  for `tools/list` and a list-changed-refetch timeout test.

---

## Documented-but-unimplemented (doc⇄code drift)

### D-1 — Per-server `cwd` working-directory field is specced but absent
- **Severity:** targeted (MVP-requirement gap).
- **Evidence (verified):** `PRD.md:41` (Req 5) lists `optional cwd
  working-directory override`; `ARCHITECTURE.md:70,97` specify the field,
  `${VAR}` interpolation, default, and tie it to **GOTCHA #30** (Morph edits
  the wrong tree). `ServerConfig` (`config.rs:97-127`) has no `cwd` field;
  `grep cwd|current_dir src/` → 0 hits; `spawn_stdio_transport`
  (`process.rs:247-257`) never calls `current_dir`.
- **Impact:** a directory-scoped upstream (Morph, D-019) silently runs in the
  aggregator's CWD, not the session project root — the exact trap GOTCHA #30
  warns about. A user who writes `cwd = "…"` per the docs is silently ignored.
- **Found by:** deepseek + gpt-5.5 + minimax (triple); missed by grok.

### D-2 — `--passthrough-stderr` documented, not implemented
- **Severity:** trivial (design-doc drift, not user-facing in README).
- **Evidence (verified):** `ARCHITECTURE.md:163`, `STACK.md:27` reference it;
  `grep passthrough src/` → 0 hits. Fix: implement (~few lines in
  `process.rs`) or strike from both docs.

---

## OSS-readiness blockers (the "production + OSS ready" question)

### O-1 — Crate is not publishable / not discoverable
- **Severity:** targeted.
- **Evidence (verified):** `Cargo.toml:8` `publish = false`; `[package]` has no
  `repository`, `homepage`, `keywords`, `categories`, or `readme`. Version is
  still `0.1.0`. Contradicts the `cargo install fanin-mcp` / crates.io framing
  in STACK.md / ROADMAP.md and the D-017 open item.
- **Fix:** add the five metadata keys, flip `publish`, `cargo publish
  --dry-run`.

### O-2 — SECURITY.md has a placeholder disclosure contact
- **Severity:** targeted.
- **Evidence (verified):** `SECURITY.md:80` → `report … to
  <SECURITY_CONTACT_EMAIL>`. The 72-hour-ack promise is empty until this is a
  real alias or a GitHub Security Advisories link.

### O-3 — No CONTRIBUTING.md
- **Severity:** trivial. For a project this disciplined (exact pins, binding
  GOTCHA list) its absence reads as unfinished.

---

## Hardening / hygiene (targeted → trivial)

- **H-1 (targeted):** global `std::sync::Mutex` `.expect()` on the redaction
  set / writers (`process.rs:55,63,671`) panics the whole proxy if the mutex is
  ever poisoned by a panic on a sharing thread — a latent DoS. Critical
  sections are trivial today (low likelihood). Fix:
  `.unwrap_or_else(|p| p.into_inner())`. (deepseek F2)
- **H-2 (targeted):** `sanitize_upstream_identifier` (`server.rs:401-413`)
  strips control chars but does not length-cap, unlike `sanitize_upstream_text`
  (caps 100). Bounded today by rmcp 1.8.0's 128-char registration ceiling;
  no defense if that ceiling rises. Fix: cap (~200) or document the rmcp bound.
  (deepseek F4)
- **H-3 (trivial):** literal (non-`${VAR}`) HTTP header values bypass
  `register_secret` — `registry.rs:126` guards on `raw.contains("${")`.
  Defense-in-depth only (docs mandate `${VAR}`; header values don't reach child
  stderr). Fix: register unconditionally, or warn on literal-looking values
  (ARCHITECTURE.md:91 planned this for env/headers; only env got it).
  (deepseek F5)
- **H-4 (trivial):** stale `#[allow(dead_code)]` on now-wired
  `ToolError::CredentialResolution` (`error.rs:66`) and the `CredentialStore`
  trait (`credentials.rs:36`). Remove both. (deepseek F3/F6, minimax #13)
- **H-5 (trivial):** `meta_tools(&self)` (`server.rs:69-78`) only does a no-op
  `let _ = &self.config;` borrow to silence a field-never-read warning; make it
  an associated fn. (minimax #6)
- **H-6 (trivial):** startup `eprintln!` vs `tracing` inconsistency
  (`main.rs:122,177,342`). Defensible (pre-clap-parse errors; `cred list` names
  to stderr) but reads as "which is the rule?" Fix: a minimal stderr-only
  subscriber before clap parse; keep `cred list` as documented. (minimax #7,
  deepseek F7)
- **H-7 (trivial):** test-only spawn hooks shipped in the production binary —
  the `--spawn-immediate-descendant` config-arg scan and the
  `__fanin_immediate_descendant__` argv sentinel + 30s marker writer
  (`main.rs:32-34,68,113-187,211-280`). Today reachable only when the harness
  passes them, but they live in the user binary. Fix: gate behind
  `#[cfg(debug_assertions)]` or move into the probe-server. (minimax #9/#10)
- **H-8 (trivial):** `redact()` is exact-substring (`process.rs:60-71`); a
  perturbed/truncated secret isn't caught. The sentinel test only exercises the
  whole value. Fix: document the scope in SECURITY.md (honest + cheap) or add a
  prefix match. (minimax #12)

---

## What HOLDS (verified by ≥2 reviewers + spot-checks)

- **stdout-is-transport (GOTCHA #1):** no `println!`/`print!`/`dbg!` on any
  serve-reachable path; only stderr `eprintln!` pre-serve / `cred list`.
- **Lock-never-across-await (D-007/GOTCHA #16):** map lock dropped before every
  upstream await; only the per-server init guard (async mutex) is held, and
  only for cold start — except the S-1 connect-hang consequence noted above.
- **Bidirectional traffic (D-008/GOTCHA #2):** sampling/elicitation rejected
  instantly, empty roots, no silent hang on the reverse path.
- **Structured errors (D-005):** all tool failures are `CallToolResult{isError}`
  with the stable public JSON shape; JSON-RPC errors only on the upstream-client
  protocol path.
- **Byte-faithful results (D-004):** `invoke_tool` returns the upstream
  `CallToolResult` untransformed; the only `to_string()` calls are
  aggregator-owned metadata rows.
- **Process containment (D-009):** Windows suspended-spawn→Job-Object→resume +
  self-Job; Linux PDEATHSIG; macOS graceful-only gap documented honestly
  (SECURITY.md / GOTCHA #14) — no hidden wider gap.
- **Secret redaction core path (D-010):** hidden stdin prompt, `env_clear()` +
  per-server injection, redaction applied at stderr/file/child-stderr/upstream
  notifications. (Edge gaps H-3/H-8 are defense-in-depth.)
- **`unsafe` discipline:** all 10 `unsafe` in `process.rs`, each with a SAFETY
  comment naming the invariant.
- **rmcp exact pin (D-015):** `=1.8.0`, `Cargo.lock` committed,
  `autobins/autotests=false`, `deny.toml` bans the anti-stack crates.
- **Alignment otherwise:** the 4 prior-cycle drifts (DRIFT-1 AggError→ToolError,
  macOS hard-kill honesty, OQ3 `transport-streamable-http-client` feature name,
  ROADMAP/MVP) are resolved/current per grok — EXCEPT the doc drift reopened by
  D-1, D-2, O-2.

---

## Process note — glm-5.2 deep-dive FAILED

glm-5.2 dispatched `completed`/exit 0 but produced no artifact: it read 4 of 9
source files, emitted 186 chars of preamble, and its loop exited at step 4
without doing the review. Task-level no-op, not a dispatch failure. Deep-dive
lens is nonetheless fully covered by minimax-m3, so no replacement was needed.
Reliability note recorded for future routing: glm-5.2 is unreliable on a single
large full-codebase review task.

---

## Verdict

**Strong codebase, NOT yet production+OSS-ready — one structural fix and a short
OSS-prep list stand between here and ship.**

The architecture, lock discipline, error model, containment, redaction, `unsafe`
hygiene, and a wire-level 115-test suite are genuinely production-grade and
align with the binding canon. The blocker is **S-1** (the timeout guarantee is
only ~1/4 true). Before an OSS release also close **D-1** (the `cwd` the docs
promise), **O-1/O-2** (publishable metadata + a real security contact), and the
H-* hygiene list.

**Priority order:** S-1 → D-1 → O-1/O-2/O-3 → H-1/H-2 → remaining trivials.
