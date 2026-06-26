//! probe-server — in-repo MCP probe fixture for fanin-mcp integration tests.
//!
//! GOTCHA #1: stdout is the MCP transport once `serve(stdio())` starts. No
//! `println!` / `print!` / `dbg!` exists in this crate; all diagnostics route to
//! stderr via `tracing`.
//!
//! `needs_sampling` (D-008, GOTCHA #2): when called, the probe — acting as the
//! server side of its stdio connection — SENDS a `sampling/createMessage`
//! REQUEST up to its client via `RequestContext::peer.send_request`. Nothing in
//! Phase 0 answers that request (the aggregator has no reverse-traffic handler
//! yet), so the probe must not block its own `call_tool` future on the
//! response. The outbound request is emitted on stdout as a JSON-RPC request
//! with an id; the detached send future is allowed to time out or error without
//! affecting the test outcome (the test observes the request and force-kills
//! the child).

use std::future::Future;
use std::process::Stdio as ProcessStdio;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, CreateElicitationRequest,
    CreateElicitationRequestParams, CreateMessageRequest, CreateMessageRequestParams,
    ElicitationSchema, Implementation, InitializeResult, JsonObject, ListRootsRequest,
    ListToolsResult, PaginatedRequestParams, SamplingMessage, ServerCapabilities, ServerInfo,
    ServerRequest, Tool, ToolAnnotations,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};

/// Server name advertised to clients.
const SERVER_NAME: &str = "probe-server";

/// Tool name constants — kept in sync with `tests/integration/probe.rs`.
const ECHO_OK: &str = "echo_ok";
const ALWAYS_ERROR: &str = "always_error";
const SLOW_TOOL: &str = "slow_tool";
const DANGEROUS_NOOP: &str = "dangerous_noop";
const NEEDS_SAMPLING: &str = "needs_sampling";
const ECHO_IMAGE: &str = "echo_image";
const NEEDS_ELICITATION: &str = "needs_elicitation";
const NEEDS_ROOTS: &str = "needs_roots";
/// `echo_env` — Phase 3. Echoes the requested env var's value (or "<absent>")
/// from the probe's visible environment. Used by the per-upstream env
/// isolation proof: the probe reports which env keys it can see, so a
/// sibling/ambient var that leaked through fails the test.
const ECHO_ENV: &str = "echo_env";
/// `spawn_grandchild` — Phase 3 hard-kill orphan proof. Spawns a long-lived
/// descendant process and writes a stable marker file (the descendant's PID
/// on Unix, or a stable marker path on Windows) so the test can observe
/// whether the descendant survived fanin-mcp's force-kill. The grandchild
/// sleeps for GRANDCHILD_LIFETIME_SECS so the marker remains observable
/// after the parent tree is killed.
const SPAWN_GRANDCHILD: &str = "spawn_grandchild";
/// How long the spawned grandchild sleeps before exiting on its own. Must
/// exceed the cleanup interval the hard-kill test waits after killing
/// fanin-mcp.
const GRANDCHILD_LIFETIME_SECS: u64 = 30;
const SAMPLING_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// The probe server fixture.
///
/// Stateless beyond the rmcp machinery.
#[derive(Debug, Clone, Default)]
pub struct Probe;

impl ServerHandler for Probe {
    /// Advertise server info and the `tools` capability (GOTCHA #8).
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }

    /// Return exactly the eight probe tools (D-016, master.md §P0.3).
    ///
    /// Fully static — no upstream fan-out.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(probe_tools())))
    }

    /// Dispatch a tool call to its behavior.
    ///
    /// Tool-level failures (`always_error`) return `Ok(CallToolResult::error(...))`
    /// — a structured result with `isError: true`, never a JSON-RPC error
    /// (D-005, GOTCHA #3). `needs_sampling` emits the outbound sampling request
    /// on a detached task and returns a tool result without blocking on the
    /// unanswered response (GOTCHA #2).
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let name = request.name.to_string();
        // Convert the optional JSON object so dispatch helpers can handle
        // payloads uniformly.
        let arguments = request.arguments.map(serde_json::Value::Object);
        async move { Ok(dispatch(name, arguments, context).await) }
    }
}

/// Build the ten probe tool definitions (D-016, master.md §P0.3).
///
/// Fully static — no upstream fan-out.
fn probe_tools() -> Vec<Tool> {
    vec![
        echo_ok_tool(),
        always_error_tool(),
        slow_tool_tool(),
        dangerous_noop_tool(),
        needs_sampling_tool(),
        echo_image_tool(),
        needs_elicitation_tool(),
        needs_roots_tool(),
        echo_env_tool(),
        spawn_grandchild_tool(),
    ]
}

/// `echo_ok` — optional `message` string; echoes the supplied input in a
/// successful tool result.
fn echo_ok_tool() -> Tool {
    Tool::new(
        ECHO_OK,
        "Echoes the supplied input back in a successful tool result.",
        optional_string_object_schema(&["message"]),
    )
}

/// `always_error` — no arguments; always returns a structured tool result
/// with `isError: true` (D-005).
fn always_error_tool() -> Tool {
    Tool::new(
        ALWAYS_ERROR,
        "Always returns a structured tool-level error (isError: true).",
        empty_object_schema(),
    )
}

/// `slow_tool` — `delay_ms` integer; waits that long before returning.
fn slow_tool_tool() -> Tool {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    let mut props = serde_json::Map::new();
    props.insert(
        "delay_ms".to_string(),
        serde_json::json!({ "type": "integer", "description": "Milliseconds to wait before returning." }),
    );
    schema.insert("properties".to_string(), serde_json::Value::Object(props));
    Tool::new(
        SLOW_TOOL,
        "Waits for the requested delay, then returns.",
        Arc::new(schema),
    )
}

/// `dangerous_noop` — no arguments; harmless no-op that ADVERTISES destructive
/// annotations (destructiveHint: true) per D-006.
fn dangerous_noop_tool() -> Tool {
    let annotations = ToolAnnotations::from_raw(
        None,
        Some(false), // readOnlyHint
        Some(true),  // destructiveHint
        None,
        None,
    );
    Tool::new(
        DANGEROUS_NOOP,
        "A harmless no-op that advertises destructive annotations.",
        empty_object_schema(),
    )
    .with_annotations(annotations)
}

/// `needs_sampling` — no arguments; sends a `sampling/createMessage` request
/// to the client when called.
fn needs_sampling_tool() -> Tool {
    Tool::new(
        NEEDS_SAMPLING,
        "Sends a sampling/createMessage request to the client when called.",
        empty_object_schema(),
    )
}

/// `echo_image` — no arguments; returns a NON-TEXT content block (a small PNG
/// image Content). Exercises the byte-faithful non-text preservation path
/// (D-004, GOTCHA #4): a proxy that `to_string()`'d the content array would
/// collapse this to a text block.
fn echo_image_tool() -> Tool {
    Tool::new(
        ECHO_IMAGE,
        "Returns a non-text image content block (small base64 PNG).",
        empty_object_schema(),
    )
}

/// `needs_elicitation` — no arguments; sends an `elicitation/create` request
/// to the client when called (D-008, GOTCHA #2). Mirrors `needs_sampling`,
/// swapping the reverse-traffic request type.
fn needs_elicitation_tool() -> Tool {
    Tool::new(
        NEEDS_ELICITATION,
        "Sends an elicitation/create request to the client when called.",
        empty_object_schema(),
    )
}

/// `needs_roots` — no arguments; sends a `roots/list` request to the client
/// when called (D-008, GOTCHA #2). Mirrors `needs_sampling`, swapping the
/// reverse-traffic request type.
fn needs_roots_tool() -> Tool {
    Tool::new(
        NEEDS_ROOTS,
        "Sends a roots/list request to the client when called.",
        empty_object_schema(),
    )
}

/// `echo_env` — `key` string; echoes the value of the env var named `key` as
/// visible to the probe process, or the literal "<absent>" if the var is not
/// set. Used by the Phase 3 per-upstream env isolation proof: the test
/// configures different env keys on sibling servers and asserts each probe
/// sees ONLY its own keys (D-010 least-privilege).
fn echo_env_tool() -> Tool {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    let mut props = serde_json::Map::new();
    props.insert(
        "key".to_string(),
        serde_json::json!({ "type": "string", "description": "Env var name to read." }),
    );
    schema.insert("properties".to_string(), serde_json::Value::Object(props));
    Tool::new(
        ECHO_ENV,
        "Echoes the value of the env var named `key` as visible to this process.",
        Arc::new(schema),
    )
}

/// `spawn_grandchild` — `marker_path` string; spawns a long-lived descendant
/// process and writes a marker file at `marker_path` so the Phase 3 hard-kill
/// orphan test can observe whether the descendant survived fanin-mcp's
/// force-kill (D-009, GOTCHA #11/#14). The grandchild sleeps for
/// GRANDCHILD_LIFETIME_SECS and then removes the marker on a clean exit; a
/// contained process tree (Job Object / process group) kills the grandchild
/// before the lifetime expires, so the marker disappears (or never appears).
fn spawn_grandchild_tool() -> Tool {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    let mut props = serde_json::Map::new();
    props.insert(
        "marker_path".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Path where the grandchild writes its presence marker."
        }),
    );
    schema.insert("properties".to_string(), serde_json::Value::Object(props));
    Tool::new(
        SPAWN_GRANDCHILD,
        "Spawns a long-lived descendant process and writes a presence marker; \
         used by the hard-kill orphan proof.",
        Arc::new(schema),
    )
}

/// Build a JSON-schema object with optional string properties.
fn optional_string_object_schema(props: &[&str]) -> Arc<JsonObject> {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    let mut properties = serde_json::Map::new();
    for p in props {
        properties.insert((*p).to_string(), serde_json::json!({ "type": "string" }));
    }
    schema.insert(
        "properties".to_string(),
        serde_json::Value::Object(properties),
    );
    Arc::new(schema)
}

/// Build an empty JSON-schema object (no required or optional properties).
fn empty_object_schema() -> Arc<JsonObject> {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    schema.insert(
        "properties".to_string(),
        serde_json::Value::Object(Default::default()),
    );
    Arc::new(schema)
}

/// Dispatch a tool name to its behavior, returning a structured `CallToolResult`.
///
/// Unknown names return a structured error result, never a JSON-RPC error
/// (D-005).
async fn dispatch(
    name: String,
    arguments: Option<serde_json::Value>,
    context: RequestContext<RoleServer>,
) -> CallToolResult {
    match name.as_str() {
        ECHO_OK => echo_ok(arguments),
        ALWAYS_ERROR => always_error(),
        SLOW_TOOL => slow_tool(arguments).await,
        DANGEROUS_NOOP => dangerous_noop(),
        NEEDS_SAMPLING => needs_sampling(context),
        ECHO_IMAGE => echo_image(),
        NEEDS_ELICITATION => needs_elicitation(context),
        NEEDS_ROOTS => needs_roots(context),
        ECHO_ENV => echo_env(arguments),
        SPAWN_GRANDCHILD => spawn_grandchild(arguments).await,
        _ => unknown_tool_result(name),
    }
}

/// `echo_ok`: echo the supplied input.
fn echo_ok(arguments: Option<serde_json::Value>) -> CallToolResult {
    let text = match arguments.as_ref().and_then(|a| a.get("message")) {
        Some(msg) => msg
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| msg.to_string()),
        None => arguments
            .map(|a| a.to_string())
            .unwrap_or_else(|| String::from("(no input)")),
    };
    CallToolResult::success(vec![Content::text(text)])
}

/// `always_error`: structured tool result with `isError: true` carrying JSON
/// error content (D-005). Never a JSON-RPC error.
fn always_error() -> CallToolResult {
    let payload = serde_json::json!({
        "code": "always_error",
        "message": "this tool always fails with a structured tool-level error",
        "recoverable": false,
    });
    CallToolResult::error(vec![Content::text(payload.to_string())])
}

/// `slow_tool`: sleep `delay_ms` milliseconds, then return.
async fn slow_tool(arguments: Option<serde_json::Value>) -> CallToolResult {
    let delay_ms = arguments
        .as_ref()
        .and_then(|a| a.get("delay_ms"))
        .and_then(|d| d.as_u64())
        .unwrap_or(0);
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    CallToolResult::success(vec![Content::text(format!("waited {delay_ms}ms"))])
}

/// `dangerous_noop`: harmless no-op. The destructive advertisement lives on the
/// tool definition (`dangerous_noop_tool`), not the call result.
fn dangerous_noop() -> CallToolResult {
    CallToolResult::success(vec![Content::text(
        "dangerous_noop: no-op completed".to_string(),
    )])
}

/// `needs_sampling`: send a `sampling/createMessage` request from the server
/// role up to the client (D-008). The request is emitted on stdout as a
/// JSON-RPC request with an id.
///
/// Nothing in Phase 0 answers it, so the probe must not block its `call_tool`
/// future on the response — that would hang the whole server (GOTCHA #2).
fn needs_sampling(context: RequestContext<RoleServer>) -> CallToolResult {
    let peer = context.peer.clone();
    let request = build_sampling_request();
    tokio::spawn(async move {
        // The load-bearing side effect is the outbound JSON-RPC request on
        // stdout. The unanswered response may time out or error.
        if tokio::time::timeout(SAMPLING_REQUEST_TIMEOUT, peer.send_request(request))
            .await
            .is_err()
        {
            tracing::warn!("probe-server sampling/createMessage request timed out");
        }
    });
    CallToolResult::success(vec![Content::text(
        "needs_sampling: sent sampling/createMessage request to client".to_string(),
    )])
}

/// Build the `sampling/createMessage` request payload.
///
/// Minimal payload that satisfies the rmcp validator.
fn build_sampling_request() -> ServerRequest {
    let messages = vec![SamplingMessage::user_text(
        "probe needs_sampling: please sample this message.",
    )];
    let params = CreateMessageRequestParams::new(messages, 64);
    ServerRequest::CreateMessageRequest(CreateMessageRequest::new(params))
}

/// `echo_image`: return a NON-TEXT image content block (D-004, GOTCHA #4).
///
/// No context, no outbound request — the load-bearing detail is the image
/// `Content` block in the success result. A proxy that `to_string()`'d the
/// content array would collapse this to a text block, breaking byte-faithful
/// non-text preservation.
fn echo_image() -> CallToolResult {
    // A 1x1 transparent PNG, base64-encoded. Tiny but a valid image payload.
    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkYPhfDwAChwGA60s6IgAAAABJRU5ErkJggg==";
    CallToolResult::success(vec![Content::image(TINY_PNG_B64, "image/png")])
}

/// `needs_elicitation`: send an `elicitation/create` request from the server
/// role up to the client (D-008). Mirrors `needs_sampling`, swapping the
/// reverse-traffic request type.
///
/// Nothing in Phase 0 answers it, so the probe must not block its `call_tool`
/// future on the response — that would hang the whole server (GOTCHA #2).
fn needs_elicitation(context: RequestContext<RoleServer>) -> CallToolResult {
    let peer = context.peer.clone();
    let request = build_elicitation_request();
    tokio::spawn(async move {
        // The load-bearing side effect is the outbound JSON-RPC request on
        // stdout. The unanswered response may time out or error.
        if tokio::time::timeout(SAMPLING_REQUEST_TIMEOUT, peer.send_request(request))
            .await
            .is_err()
        {
            tracing::warn!("probe-server elicitation/create request timed out");
        }
    });
    CallToolResult::success(vec![Content::text(
        "needs_elicitation: sent elicitation/create request to client".to_string(),
    )])
}

/// Build the `elicitation/create` request payload.
///
/// Minimal form-based payload that satisfies the rmcp validator.
fn build_elicitation_request() -> ServerRequest {
    let schema = ElicitationSchema::builder()
        .required_string("answer")
        .build()
        .expect("a minimal elicitation schema with one required string is valid");
    let params = CreateElicitationRequestParams::FormElicitationParams {
        meta: None,
        message: "probe needs_elicitation: please answer this elicitation.".to_string(),
        requested_schema: schema,
    };
    ServerRequest::CreateElicitationRequest(CreateElicitationRequest::new(params))
}

/// `needs_roots`: send a `roots/list` request from the server role up to the
/// client (D-008). Mirrors `needs_sampling`, swapping the reverse-traffic
/// request type.
///
/// Nothing in Phase 0 answers it, so the probe must not block its `call_tool`
/// future on the response — that would hang the whole server (GOTCHA #2).
fn needs_roots(context: RequestContext<RoleServer>) -> CallToolResult {
    let peer = context.peer.clone();
    let request = build_roots_request();
    tokio::spawn(async move {
        // The load-bearing side effect is the outbound JSON-RPC request on
        // stdout. The unanswered response may time out or error.
        if tokio::time::timeout(SAMPLING_REQUEST_TIMEOUT, peer.send_request(request))
            .await
            .is_err()
        {
            tracing::warn!("probe-server roots/list request timed out");
        }
    });
    CallToolResult::success(vec![Content::text(
        "needs_roots: sent roots/list request to client".to_string(),
    )])
}

/// Build the `roots/list` request payload.
///
/// `ListRootsRequest` carries no params; the request itself is the payload.
fn build_roots_request() -> ServerRequest {
    ServerRequest::ListRootsRequest(ListRootsRequest::default())
}

/// `echo_env`: read the env var named `key` from the probe's visible
/// environment and echo its value, or "<absent>" if unset. The probe never
/// invents a value; a missing key is reported honestly. Used by the Phase 3
/// per-upstream env isolation proof (D-010).
fn echo_env(arguments: Option<serde_json::Value>) -> CallToolResult {
    let key = arguments
        .as_ref()
        .and_then(|a| a.get("key"))
        .and_then(|k| k.as_str())
        .unwrap_or("");
    let value = std::env::var(key).unwrap_or_else(|_| "<absent>".to_string());
    CallToolResult::success(vec![Content::text(value)])
}

/// `spawn_grandchild`: spawn a long-lived descendant process that writes a
/// presence marker at `marker_path` and sleeps for
/// GRANDCHILD_LIFETIME_SECS. The descendant re-execs the probe binary
/// itself in a "grandchild" mode (detected by a private argv sentinel),
/// which sleeps and then removes the marker on a clean exit. A contained
/// process tree (Job Object / process group) kills the descendant before
/// the lifetime expires, so the marker disappears; an uncontained tree
/// leaves the orphan alive and the marker persists — the failure the
/// hard-kill orphan test catches.
///
/// The grandchild is spawned with stdin/stdout/stderr inherited (or null)
/// so it does NOT touch the probe's MCP stdio stream (GOTCHA #1).
async fn spawn_grandchild(arguments: Option<serde_json::Value>) -> CallToolResult {
    let marker_path = arguments
        .as_ref()
        .and_then(|a| a.get("marker_path"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    if marker_path.is_empty() {
        return CallToolResult::error(vec![Content::text(
            "spawn_grandchild requires a non-empty marker_path".to_string(),
        )]);
    }

    // Re-exec the probe binary itself in grandchild mode. The sentinel argv
    // `__grandchild__` plus the marker path and lifetime make the grandchild
    // branch of `main` sleep for the lifetime and remove the marker on exit.
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return CallToolResult::error(vec![Content::text(format!(
                "spawn_grandchild: failed to resolve current_exe: {e}"
            ))]);
        }
    };

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(GRANDCHILD_SENTINEL)
        .arg(marker_path)
        .arg(GRANDCHILD_LIFETIME_SECS.to_string())
        .stdin(ProcessStdio::null())
        .stdout(ProcessStdio::null())
        .stderr(ProcessStdio::null());

    // Detach the grandchild so the probe's call_tool returns immediately.
    // The grandchild writes the marker before sleeping; on a clean exit
    // (lifetime elapsed, tree NOT killed) it removes the marker.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP so the grandchild is
        // not tied to the probe's console and can survive a plain kill of
        // the probe — the containment layer must catch it instead.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    match cmd.spawn() {
        Ok(_child) => {
            // Do NOT wait on the child — it must outlive the probe's
            // call_tool return. Drop the handle; the OS owns the lifetime.
            // The marker is written by the grandchild before it sleeps; give
            // it a brief moment to land so the test can observe it promptly.
            tokio::time::sleep(Duration::from_millis(200)).await;
            CallToolResult::success(vec![Content::text(format!(
                "spawn_grandchild: descendant started, marker at {marker_path}"
            ))])
        }
        Err(e) => CallToolResult::error(vec![Content::text(format!(
            "spawn_grandchild: failed to spawn descendant: {e}"
        ))]),
    }
}

/// The private argv sentinel that selects the grandchild branch of `main`.
/// Picked to be implausible as a real subcommand so it never collides with
/// the normal CLI surface.
const GRANDCHILD_SENTINEL: &str = "__grandchild__";

/// Structured unknown-tool result — never a JSON-RPC error (D-005).
fn unknown_tool_result(tool: String) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("unknown probe tool: {tool}"))])
}

/// Entry point. Initializes tracing to stderr (GOTCHA #1) and serves over stdio.
///
/// The grandchild sentinel branch (`__grandchild__ <marker_path> <secs>`) is
/// the detached descendant spawned by `spawn_grandchild` for the Phase 3
/// hard-kill orphan proof. It writes the marker, sleeps for the requested
/// lifetime, and removes the marker on a clean exit — so a contained process
/// tree (Job Object / process group) that kills the grandchild before the
/// lifetime elapses makes the marker disappear, while an uncontained tree
/// leaves the orphan alive and the marker persists.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    // Grandchild sentinel branch: re-exec'd by `spawn_grandchild`. Must run
    // BEFORE tracing/serve setup so it never touches the MCP stdio stream.
    if let Some(args) = parse_grandchild_args() {
        return run_grandchild(args.marker_path, args.lifetime).await;
    }

    init_tracing();

    let probe = Probe;
    let running = match probe.serve(rmcp::transport::stdio()).await {
        Ok(running) => running,
        Err(e) => {
            tracing::error!(error = %e, "probe-server failed to start stdio MCP server");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(e) = running.waiting().await {
        tracing::error!(error = %e, "probe-server stdio MCP task failed");
        return std::process::ExitCode::FAILURE;
    }

    std::process::ExitCode::SUCCESS
}

/// Parsed grandchild-mode arguments.
struct GrandchildArgs {
    marker_path: String,
    lifetime: Duration,
}

/// If argv[1] is the grandchild sentinel, parse the rest into a
/// [`GrandchildArgs`]; otherwise return `None` (normal probe-server mode).
fn parse_grandchild_args() -> Option<GrandchildArgs> {
    let mut args = std::env::args().skip(1);
    let first = args.next()?;
    if first != GRANDCHILD_SENTINEL {
        return None;
    }
    let marker_path = args.next()?;
    let secs: u64 = args.next()?.parse().ok()?;
    Some(GrandchildArgs {
        marker_path,
        lifetime: Duration::from_secs(secs),
    })
}

/// Run the grandchild branch: write the presence marker, sleep for `lifetime`,
/// then remove the marker and exit cleanly. A force-kill of the process tree
/// (Job Object / process group) terminates this process before the sleep
/// elapses, so the marker is NOT removed — the test observes the marker's
/// presence or absence to decide containment.
async fn run_grandchild(marker_path: String, lifetime: Duration) -> std::process::ExitCode {
    // Write the marker first; if this fails, exit non-zero so the test sees
    // the marker is absent (the grandchild never started cleanly).
    if let Err(e) = std::fs::write(&marker_path, std::process::id().to_string()) {
        eprintln!("grandchild: failed to write marker {marker_path}: {e}");
        return std::process::ExitCode::FAILURE;
    }
    tokio::time::sleep(lifetime).await;
    // Clean exit: remove the marker so a re-run does not see a stale file.
    let _ = std::fs::remove_file(&marker_path);
    std::process::ExitCode::SUCCESS
}

/// Initialize `tracing` with a stderr writer so diagnostics never corrupt the
/// JSON-RPC stream on stdout (GOTCHA #1).
fn init_tracing() {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    fmt()
        .with_max_level(LevelFilter::INFO)
        .with_writer(std::io::stderr)
        .init();
}
