# Implementer task — Phase 5, plan **Phase 1: Redacted JSON Observability**

Implement ONLY plan Phase 1 (`master.md` §"Phase 1 — Redacted JSON
Observability"). Make its contract tests green without touching any other
phase's scope.

## Read first

- `docs/master-plans/phase-5-polish-ci-benchmarks/master.md` §Phase 1 (Key
  Behaviors + Phase Success Criteria 1–5).
- **`tests/integration/observability.rs`** — your binding, READ-ONLY contract.
  Match it exactly; do not edit it (or any `tests/**` file).
- `tests/common/mod.rs` + `tests/common/fixtures.rs` for the helpers the tests
  use (`spawn_fanin_with_args`, `run_fanin_cli`, `take_stderr`, the config
  builders) so you build the CLI surface they invoke.
- `src/main.rs` (current `init_tracing()` ~:274 is hardcoded INFO→stderr; the
  `clap` Cli has `--namespace`/`--config`/credential-store globals) and
  `src/process.rs` for the EXISTING `RedactingMakeWriter` (reuse it).
- Skills: `rust-general`, `rmcp-general`.

## Exact contract to satisfy (from observability.rs)

1. **CLI:** add global `--log-file <path>` and `--log-level <level>` to serve.
   They compose with `--config`.
2. **`--log-file` set ⇒** structured logs are written as **newline-delimited
   JSON** to that file (one JSON object per line, each carrying a `level`
   field). **stdout stays pure MCP JSON-RPC** (GOTCHA #1). **stderr must NOT
   contain `{"`** — i.e. JSON diagnostics go to the FILE, not stderr (a human/
   compact stderr layer is fine, or keep stderr quiet; just no JSON on it).
3. **`--log-level debug` ⇒** the file includes at least one entry with
   `level == "debug"` (case-insensitive). Default level stays INFO.
4. **Invalid `--log-level` ⇒** exit **non-zero BEFORE `serve(stdio())`** and
   write **zero bytes to stdout**. Parse/validate the level before any serve
   logic (clap `value_parser` or an explicit pre-serve check).
5. **Per-call log:** every `invoke_tool` forward emits an NDJSON entry with
   fields **`server`** (string), **`tool`** (string), **`latency_ms`**
   (number), **`outcome`** (`"success"` on ok, `"failure"` on an upstream/tool
   error). Do **NOT** log the call arguments or any secret value.
6. **Lifecycle/startup logs:** config load, active namespace, upstream
   connect, and upstream disconnect/failure as structured events (master
   Phase SC 3 / Success Criterion 3).
7. **Redaction:** the resolved sentinel secret must be ABSENT from BOTH stderr
   AND the JSON file sink. **Reuse the existing `RedactingMakeWriter`** redaction
   path for the file layer — do NOT hand-roll a second redactor and do NOT
   `{:?}`-print resolved env maps (GOTCHA #19).

## Files you may touch (src only — tests are read-only)

`src/main.rs` (CLI flags + tracing init + pre-serve level validation),
`src/server.rs` and/or `src/registry.rs` (per-call latency/outcome + connect/
disconnect logging on the forward path), `src/forward.rs` if needed. Keep the
no-lock-across-await discipline (GOTCHA #16 / D-007) — measure latency around
the awaited call without holding the registry map lock.

## Constraints

- Scope discipline: implement Phase 1 only. Surface (do not fix) anything you
  notice outside it in your returned result.
- Tests are a read-only contract; if a test looks wrong, STOP and report a
  test-issue — do not edit it or contort src to game it.
- End state must be `cargo fmt --all` clean, `cargo clippy --all-targets` with
  zero warnings, and `cargo test --test integration observability` green.
- rmcp stays exact-pinned `=1.8.0`; no dependency additions in this phase.

## Return

A concise result: what you changed per file, how the JSON file sink + redaction
were wired (which existing writer you reused), confirmation the 4 observability
tests pass, and any out-of-scope issue or test-issue you surfaced.
