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

/// Build the eight probe tool definitions.
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

/// Structured unknown-tool result — never a JSON-RPC error (D-005).
fn unknown_tool_result(tool: String) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("unknown probe tool: {tool}"))])
}

/// Entry point. Initializes tracing to stderr (GOTCHA #1) and serves over stdio.
#[tokio::main]
async fn main() -> std::process::ExitCode {
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
