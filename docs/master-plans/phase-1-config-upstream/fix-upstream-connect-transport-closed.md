# Fix: upstream connect transport closed

## Defect

Every proxy call that needed to connect to the `probe` upstream returned a
structured `upstream_connect_failed` error with message `Transport closed`.
The defect was in `src/process.rs`, in upstream stdio child construction.

## Root cause

`spawn_stdio_transport` configured the raw `tokio::process::Command` with
piped stdin/stdout/stderr and then configured rmcp's `TokioChildProcessBuilder`
with the same streams again. That duplicated ownership setup around the streams
rmcp must own for the JSON-RPC transport.

The second failure was load-bearing: stderr was always piped, but when no
`log_file` was configured the returned `ChildStderr` handle was dropped
immediately. The probe writes tracing startup lines to stderr; with the pipe's
read end closed, the child died during initialize and rmcp surfaced EOF as
`Transport closed`.

## Fix applied

`src/process.rs` now builds the raw command with only command, args, and env.
rmcp's `TokioChildProcessBuilder` owns stdin/stdout for the transport.

stderr is conditional:

- with `log_file`, stderr is piped and drained into the configured log sink;
- without `log_file`, stderr is set to `Stdio::null()` so child diagnostics are
  discarded without closing a pipe under the child.

No test files were edited.

## Verification

- `cargo build 2>&1 | tail -5` passed.
- Targeted repro passed:
  `cargo test --test integration invoke::invoke_tool_probe_echo_ok_returns_probe_success 2>&1 | tail -20`.
- Plan-scope integration suite is not green:
  `test result: FAILED. 52 passed; 4 failed; 2 ignored; 0 measured; 0 filtered out; finished in 5.46s`.

## Suggested-fix divergence

The suggested rmcp construction pattern was correct but incomplete. Removing the
raw stdio piping alone did not fix the repro. The actual root cause also required
not dropping an unused piped stderr handle when no log file is configured.

## Surfaced for routing

- targeted: `tests/integration/discovery.rs` still expects exactly five probe
  tools in `list_tools`, while `tests/probe-server/main.rs` advertises eight and
  `probe::probe_exposes_exactly_eight_named_tools` asserts that eight-tool
  surface. This is a test/spec conflict, not part of the connect-path fix.
- targeted: integration tests that use `fixtures::empty_log_file_path()` share a
  single process-id-based log path. Under full-suite execution, probe log lines
  from one test can pollute another test's pre-spawn assertion. This is test
  fixture isolation drift; it was not repaired because tests are read-only for
  debugger runs.
