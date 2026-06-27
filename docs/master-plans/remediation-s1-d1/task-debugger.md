FIX three targeted review findings — remediation-s1-d1. Production code only.

You are the debugger. Apply exactly the three targeted fixes below (G-1, G-2,
G-3) from `docs/master-plans/remediation-s1-d1/review.md`. Read review.md and
`docs/master-plans/remediation-s1-d1/master.md` (S-1 decision 3 authorizes
`tool: null` for connect/discovery timeouts). Then write a
`docs/master-plans/remediation-s1-d1/fix-review-targeted.md` summary.

## ABSOLUTE RULES
- Edit `src/**` ONLY. Do NOT edit any `tests/**` file. If a fix seems to require a
  test change, STOP and report it instead — do not touch the test.
- Done-condition: `cargo test --all` 100% green (still 134/0/4), `cargo fmt --all
  -- --check` clean, `cargo clippy --all-targets -- -D warnings` clean.
- Scope is EXACTLY these three findings. No other changes, no drive-by refactors.
- Do NOT edit `src/credentials.rs`.

## G-1 — observability event naming (registry.rs)
The new timeout sites currently log `event = "upstream_failure", code = "timeout"`
(the cold-connect timeout, the `call_tool` timeout, and the `ensure_fresh`
refetch timeout). Rename all three to `event = "upstream_timeout"` and REMOVE the
now-redundant `code = "timeout"` field. This matches the wire error code
`upstream_timeout` and the existing concrete-event convention
(`upstream_connect_failure`, `upstream_disconnect`). No test asserts log event
names; verify your change does not alter any `code`-on-the-wire value (the
structured `ToolError` code stays `upstream_timeout`).

## G-2 — `UpstreamTimeout.tool` empty string → `Option<String>` (error.rs + registry.rs)
Currently `ToolError::UpstreamTimeout` has `tool: String`, and the three new
connect/discovery/refetch timeout sites pass `tool: String::new()`, which renders
an empty-backtick message and serializes `"tool": ""`.

- Change the field to `tool: Option<String>`.
- Update the JSON/`message`/`Display` builder in `error.rs` so:
  - `Some(t)` → the existing wording, e.g. ``upstream call to `{t}` on `{server}`
    exceeded timeout``, wire `"tool": "{t}"`.
  - `None` → a phase-neutral message with NO empty backticks, e.g. ``upstream
    operation on `{server}` exceeded timeout``, wire `"tool": null`.
- IMPORTANT: keep the `tool` key ALWAYS PRESENT in the wire JSON (serialize `None`
  as JSON `null`, do NOT `skip_serializing_if` / omit it) — a stable key set
  matters for downstream consumers (D-005 public shape; the key set stays the
  same, the value just becomes nullable).
- Construction sites: the cold-connect timeout, the `ensure_fresh` refetch
  timeout, and any other connect/discovery timeout pass `None`; the `call_tool`
  timeout passes `Some(tool.to_string())` so its wire form is UNCHANGED (the
  existing `timeout_cancellation` tests assert `code` and must stay green).
- Confirm no other reader of `.tool` on `UpstreamTimeout` breaks (search usages).

## G-3 — `ServerConfig::cwd` doc gap (config.rs)
Add one line to the `cwd` field doc-comment stating that empty or whitespace-only
values — literal at config load, or after `${VAR}` resolution at connect — are
rejected before spawn. Keep it consistent with ARCHITECTURE.md:97 wording.

## Finish
Run fmt, clippy, and `cargo test --all`; confirm green. In
`fix-review-targeted.md` record: each finding, the exact change, the files
touched, confirmation the public error key set is unchanged (tool now nullable
but always present; code unchanged), the final gate numbers, and anything you
could not do without a test change (should be none). Return that summary as data
for the orchestrator.
