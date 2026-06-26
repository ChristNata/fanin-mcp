//! Shared JSON-RPC-over-stdio harness for Phase 0 integration tests.
//!
//! Spawns the built binary, speaks raw JSON-RPC 2.0 over its stdin/stdout,
//! and asserts on the wire JSON. Wire-level tests decouple the test contract
//! from rmcp's fast-moving Rust API (D-015) — a stub that returns the right
//! rmcp type but emits the wrong JSON still fails.
//!
//! Every spawned process is bounded by a timeout so a hang fails fast
//! (GOTCHA: needs_sampling has no responder in Phase 0). The `ChildGuard`
//! kills the child on drop so no orphaned processes survive a failed test.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

/// Hard ceiling for any single JSON-RPC round-trip. Generous enough that a
/// correct implementation never hits it, tight enough that a hang fails the
/// test instead of stalling CI.
const RPC_DEADLINE: Duration = Duration::from_secs(5);

/// Hard ceiling for the `initialize` round-trip specifically. The plan's
/// startup-laziness gate (criterion 4) requires < 500ms; we use a wider
/// ceiling here and a tighter assertion in the laziness test itself so a
/// slow CI runner does not flake on the round-trip while still proving the
/// 500ms budget is met where the plan demands it.
const INIT_DEADLINE: Duration = Duration::from_secs(10);

/// Killed on drop so a failed test never leaves an orphan behind.
pub struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    /// Send stdin EOF, wait up to 1s for a clean exit, then force-kill.
    /// Returns the exit status if the child exited on its own.
    pub async fn shutdown(mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let _ = self.child.stdin.take(); // drop stdin => EOF
        let wait = timeout(Duration::from_secs(1), self.child.wait()).await;
        match wait {
            Ok(status) => {
                // `status` is the INNER io::Result<ExitStatus>; `?` propagates a
                // wait/io error to the caller, matching the intended semantics:
                // clean exit observed within 1s -> Ok(Some(status)); wait error
                // -> propagated; outer timeout (Elapsed) handled below.
                if status.is_ok() {
                    return Ok(Some(status?));
                }
                let _ = self.child.kill().await;
                Ok(Some(status?))
            }
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                Ok(None)
            }
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Best-effort synchronous kill; the async shutdown is the primary path.
        let _ = self.child.start_kill();
    }
}

/// A live JSON-RPC connection over a spawned child's stdio.
pub struct JsonRpcChild {
    child: ChildGuard,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
    _stderr: Option<tokio::process::ChildStderr>,
    /// Buffer of responses received out of order. JSON-RPC permits responses
    /// to arrive in any order, so when a concurrent test sends ids 2 and 3 and
    /// the server returns 3 before 2, the 3 is retained here for a later
    /// `wait_for_id(3)` rather than being dropped on the floor.
    pending: HashMap<u64, Value>,
}

impl JsonRpcChild {
    /// Send a JSON-RPC request and await the matching response (by id).
    /// Fails the test on hang (RPC_DEADLINE) or on a non-matching id.
    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params).await;
        self.wait_for_id(id).await
    }

    /// Send a JSON-RPC request and return its id WITHOUT waiting for a
    /// response. Used by tests that observe out-of-band messages on the wire
    /// (e.g. needs_sampling's outbound sampling/createMessage) rather than
    /// the matching response. The borrow on `self` ends when this returns,
    /// so the caller is free to call `read_next_message` / `read_line`.
    pub async fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = req.to_string();
        self.write_line(&line)
            .await
            .unwrap_or_else(|e| panic!("write {method} request failed: {e}"));
        id
    }

    /// Wait for a JSON-RPC response matching the given id. Out-of-order
    /// responses for other in-flight requests are buffered in `self.pending`
    /// and returned by a later `wait_for_id` call — they are NOT dropped.
    /// Notifications (no `id`) are skipped. Fails on hang (RPC_DEADLINE).
    pub async fn wait_for_id(&mut self, id: u64) -> Value {
        if let Some(msg) = self.pending.remove(&id) {
            return msg;
        }
        loop {
            let raw = self
                .read_line()
                .await
                .unwrap_or_else(|e| panic!("read response for id {id} failed: {e}"));
            if raw.trim().is_empty() {
                continue;
            }
            let msg: Value = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("response not JSON: {raw}\n{e}"));
            // A message with a numeric `id` is a response. If it matches, return
            // it; otherwise it is a response for a different in-flight request,
            // so retain it for a later `wait_for_id(that_id)`.
            if let Some(other) = msg.get("id").and_then(Value::as_u64) {
                if other == id {
                    return msg;
                }
                self.pending.insert(other, msg);
                continue;
            }
            // Notifications (no `id`) or non-numeric ids: skip as before.
        }
    }

    /// Send a notification (no response expected) and return immediately.
    pub async fn notify(&mut self, method: &str, params: Value) {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let line = req.to_string();
        self.write_line(&line)
            .await
            .unwrap_or_else(|e| panic!("write {method} notification failed: {e}"));
    }

    /// Read one raw stdout line from the child. Times out after RPC_DEADLINE.
    pub async fn read_line(&mut self) -> std::io::Result<String> {
        let mut buf = String::new();
        timeout(RPC_DEADLINE, self.stdout.read_line(&mut buf))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "child stdout did not produce a line within RPC_DEADLINE",
                )
            })??;
        Ok(buf)
    }

    /// Read the next message without asserting on id — for tests that observe
    /// unsolicited/out-of-band messages (e.g. needs_sampling's outbound
    /// sampling/createMessage). Times out after RPC_DEADLINE.
    pub async fn read_next_message(&mut self) -> Value {
        loop {
            let raw = self
                .read_line()
                .await
                .unwrap_or_else(|e| panic!("read next message failed: {e}"));
            if raw.trim().is_empty() {
                continue;
            }
            return serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("out-of-band message not JSON: {raw}\n{e}"));
        }
    }

    /// Take the stderr stream so a test can inspect diagnostics. Only the first
    /// caller gets a stream; later callers get None.
    #[allow(dead_code)] // pub harness API for future phases; no Phase 0 test uses it yet.
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self._stderr.take()
    }

    /// Drain all available stdout within `deadline` without parsing it as
    /// JSON. Returns the raw bytes. Used by startup-failure tests to assert
    /// that NO bytes were written to stdout (GOTCHA #1): a config-validation
    /// error must route to stderr/tracing, never to the JSON-RPC stream.
    ///
    /// Reads until EOF or deadline. A correct startup-failure path closes
    /// stdout on exit, so EOF within the deadline is the happy path; a hang
    /// fails the test on the deadline.
    pub async fn drain_stdout_raw(&mut self, deadline: Duration) -> Vec<u8> {
        let mut out = Vec::new();
        let _ = timeout(deadline, async {
            use tokio::io::AsyncReadExt;
            // The BufReader is already line-buffered; read_to_end would block
            // until EOF. We poll with a short per-read timeout instead so a
            // partial line (no trailing newline) still surfaces.
            let mut buf = [0u8; 1024];
            loop {
                match self.stdout.read(&mut buf).await {
                    Ok(0) => break, // EOF
                    Ok(n) => out.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
        })
        .await;
        out
    }

    async fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_u8(b'\n').await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Hand back the guard so a test can shut the child down explicitly.
    pub fn into_guard(self) -> ChildGuard {
        self.child
    }
}

/// Spawn a bin target by cargo bin name and speak JSON-RPC over its stdio.
///
/// `bin` is the `[[bin]] name = "..."` value, passed verbatim. Cargo injects
/// `CARGO_BIN_EXE_<name>` using the bin name EXACTLY as-declared — case and
/// hyphens preserved (e.g. bin `probe-server` -> `CARGO_BIN_EXE_probe-server`).
/// Do not uppercase or transform; that breaks resolution on every platform.
pub async fn spawn_bin(bin: &str) -> JsonRpcChild {
    let env_key = format!("CARGO_BIN_EXE_{bin}");
    let path = std::env::var(&env_key).unwrap_or_else(|_| {
        panic!(
            "cargo did not inject {env_key}; the test harness relies on the \
             [[bin]] target named `{bin}` being built before tests run"
        )
    });
    spawn_path(path).await
}

/// Spawn a binary at an explicit path (used for the probe fixture when it is
/// built as part of the main crate, or when the path is otherwise known).
pub async fn spawn_path(path: String) -> JsonRpcChild {
    let mut cmd = Command::new(&path);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Empty env: Phase 0 aggregator has zero upstream config and should never
    // need runtime env to answer initialize/tools/list/stub-call. If a future
    // phase needs env, pass it explicitly from the calling test.
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn `{path}`: {e}"));

    let stdin = child.stdin.take().expect("child stdin was piped");
    let stdout = child.stdout.take().expect("child stdout was piped");
    let stderr = child.stderr.take().expect("child stderr was piped");

    JsonRpcChild {
        child: ChildGuard { child },
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
        _stderr: Some(stderr),
        pending: HashMap::new(),
    }
}

// ---- Phase 1 config-aware spawn helpers -----------------------------------

/// Spawn `fanin-mcp` with the given `--config` path and optional
/// `--namespace`. Phase 1 tests use this instead of `spawn_bin("fanin-mcp")`
/// so the aggregator loads the TOML config and validates the namespace
/// before serving starts.
///
/// `config_path` is passed verbatim to `--config`; the caller is responsible
/// for keeping the temp config file alive (see `fixtures::ConfigFile`).
pub async fn spawn_fanin_with_config(config_path: &str, namespace: Option<&str>) -> JsonRpcChild {
    let mut extra: Vec<String> = Vec::new();
    extra.push("--config".to_string());
    extra.push(config_path.to_string());
    if let Some(ns) = namespace {
        extra.push("--namespace".to_string());
        extra.push(ns.to_string());
    }
    spawn_fanin_with_args(&extra).await
}

/// Spawn `fanin-mcp` with an explicit argv tail (after the bin name). Used by
/// negative startup tests that pass invalid configs/namespaces and expect the
/// process to exit BEFORE serving — those tests do not need a JSON-RPC
/// connection, just the exit status and a clean-stdout observation.
pub async fn spawn_fanin_with_args(args: &[String]) -> JsonRpcChild {
    let path = std::env::var("CARGO_BIN_EXE_fanin-mcp").unwrap_or_else(|_| {
        panic!(
            "cargo did not inject CARGO_BIN_EXE_fanin-mcp; the [[bin]] target \
             `fanin-mcp` must be built before the test binary runs"
        )
    });
    let mut cmd = Command::new(&path);
    cmd.args(args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn fanin-mcp: {e}"));

    let stdin = child.stdin.take().expect("child stdin was piped");
    let stdout = child.stdout.take().expect("child stdout was piped");
    let stderr = child.stderr.take().expect("child stderr was piped");

    JsonRpcChild {
        child: ChildGuard { child },
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
        _stderr: Some(stderr),
        pending: HashMap::new(),
    }
}

/// Perform the MCP `initialize` handshake and return the server's result
/// object. Asserts the protocolVersion is present and that the server
/// advertises the `tools` capability (clients cannot call tools/list without
/// it — GOTCHA #8).
pub async fn initialize(child: &mut JsonRpcChild) -> Value {
    let result = timeout(
        INIT_DEADLINE,
        child.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "fanin-mcp-test-harness",
                    "version": "0.0.0",
                },
            }),
        ),
    )
    .await
    .expect("initialize did not return within INIT_DEADLINE")
    .get("result")
    .cloned()
    .unwrap_or_else(|| panic!("initialize returned no result field"));

    assert!(
        result.get("protocolVersion").is_some(),
        "initialize result must carry a protocolVersion"
    );
    let caps = result
        .get("capabilities")
        .cloned()
        .unwrap_or_else(|| panic!("initialize result must carry capabilities"));
    assert!(
        caps.get("tools").is_some(),
        "server must advertise the tools capability (GOTCHA #8)"
    );

    // The initialized notification completes the handshake.
    child
        .notify(
            "notifications/initialized",
            Value::Object(Default::default()),
        )
        .await;

    result
}

/// Assert a JSON-RPC response carries no top-level `error`.
pub fn assert_no_rpc_error(msg: &Value, ctx: &str) {
    if let Some(err) = msg.get("error") {
        panic!("{ctx}: expected a result, got JSON-RPC error: {err}");
    }
}

/// Assert a CallToolResult-shaped result has `isError: true` and at least one
/// content block. This is the structured not-implemented contract (D-005).
pub fn assert_is_error_result(result: &Value, ctx: &str) {
    let is_error = result
        .get("isError")
        .unwrap_or_else(|| panic!("{ctx}: CallToolResult missing isError"));
    assert!(
        is_error.as_bool() == Some(true),
        "{ctx}: expected isError: true, got {is_error}"
    );
    let content = result
        .get("content")
        .unwrap_or_else(|| panic!("{ctx}: CallToolResult missing content"));
    assert!(content.is_array(), "{ctx}: content must be an array");
    assert!(
        !content.as_array().unwrap().is_empty(),
        "{ctx}: not-implemented error must carry at least one content block"
    );
}

/// Convenience: call a tool by name with the given arguments object and return
/// the raw JSON-RPC response.
pub async fn call_tool(child: &mut JsonRpcChild, name: &str, arguments: Value) -> Value {
    child
        .request(
            "tools/call",
            serde_json::json!({
                "name": name,
                "arguments": arguments,
            }),
        )
        .await
}

/// Convenience: list tools and return the raw JSON-RPC response.
pub async fn list_tools(child: &mut JsonRpcChild) -> Value {
    child
        .request("tools/list", Value::Object(Default::default()))
        .await
}

pub mod expectations;
pub mod fixtures;
