//! Downstream MCP server surface.
//!
//! GOTCHA #1: stdout is the MCP transport. Nothing in this module writes to
//! stdout; all diagnostics go to stderr via `tracing`.
//!
//! The static descriptions below are FINAL design (D-003), copied verbatim
//! from `tests/common/expectations.rs` / `master.md` §Required Pattern. They
//! are not placeholder text and must not drift from the expectations file.

use std::future::Future;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, InitializeResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::config::CliConfig;
use crate::error::ToolError;
use crate::namespace::ActiveNamespace;
use crate::registry::Registry;

/// Server name advertised to clients.
const SERVER_NAME: &str = "fanin-mcp";

/// The static meta-tool descriptions — final design, verbatim from
/// `tests/common/expectations.rs` / `master.md` §Required Pattern. Changing
/// these is a SemVer-major break (ARCHITECTURE.md §Versioning).
const LIST_TOOLS_DESC: &str = "Lists the tools available through this aggregator, grouped by server, with one-line descriptions. Call this once to see what's connected; pass server to fetch a single server's tools.";
const GET_TOOL_SCHEMA_DESC: &str =
    "Get the full input schema for a tool. Format: server__tool (e.g. postgres__query).";
const INVOKE_TOOL_DESC: &str = "Call a tool by server__tool name with arguments.";

/// The downstream aggregator server.
#[derive(Debug, Clone)]
pub struct Aggregator {
    /// Carried verbatim from `--namespace` / `--config` for later phases.
    config: CliConfig,
    registry: Option<Arc<Registry>>,
    namespace: Option<ActiveNamespace>,
}

impl Aggregator {
    /// Build a new aggregator from the resolved CLI configuration.
    pub fn new(config: CliConfig) -> Self {
        Self {
            config,
            registry: None,
            namespace: None,
        }
    }

    /// Build a new aggregator with a live upstream registry.
    pub fn with_registry(
        config: CliConfig,
        registry: Arc<Registry>,
        namespace: ActiveNamespace,
    ) -> Self {
        Self {
            config,
            registry: Some(registry),
            namespace: Some(namespace),
        }
    }

    /// Build the three static meta-tools.
    fn meta_tools(&self) -> Vec<Tool> {
        // The binding keeps carried CLI config live without per-tool cost.
        let _ = &self.config;

        vec![
            list_tools_tool(),
            get_tool_schema_tool(),
            invoke_tool_tool(),
        ]
    }
}

impl ServerHandler for Aggregator {
    /// Advertise server info and the `tools` capability (GOTCHA #8).
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }

    /// Return exactly the three meta-tools with the final static descriptions.
    ///
    /// No upstream fan-out: `tools/list` is fully static (D-002, D-003,
    /// GOTCHA #7). A client sends it at every session start; any upstream touch
    /// here would destroy lazy loading and the <500ms init budget.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.meta_tools())))
    }

    /// Return a structured not-implemented `CallToolResult` for any tool name.
    ///
    /// Returns `Ok(CallToolResult::error(...))` — a tool-level result with
    /// `isError: true`, never a JSON-RPC error (D-005, GOTCHA #3).
    ///
    /// For `invoke_tool` we use the `RequestContext`'s `ct: CancellationToken`
    /// (provided by rmcp =1.8.0) to abort the local in-flight upstream call
    /// when the downstream client sends `notifications/cancelled`.
    /// We do NOT forward a `notify_cancelled` upstream (see OQ3 structural finding).
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.dispatch_tool(request, context).await)
    }
}

impl Aggregator {
    async fn dispatch_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        match request.name.as_ref() {
            "list_tools" => self.handle_list_tools(request.arguments).await,
            "get_tool_schema" => self.handle_get_tool_schema(request.arguments).await,
            "invoke_tool" => self.handle_invoke_tool(request.arguments, context).await,
            other => not_implemented_result(other.to_string()),
        }
    }

    async fn handle_list_tools(&self, arguments: Option<JsonObject>) -> CallToolResult {
        let Some(registry) = &self.registry else {
            return ToolError::NotImplemented {
                tool: "list_tools".to_string(),
            }
            .as_result();
        };
        let Some(namespace) = &self.namespace else {
            return ToolError::NotImplemented {
                tool: "list_tools".to_string(),
            }
            .as_result();
        };

        let servers = if let Some(server) = arguments
            .as_ref()
            .and_then(|a| a.get("server"))
            .and_then(|v| v.as_str())
        {
            if !registry.has_server(server) {
                return ToolError::UnknownServer {
                    server: server.to_string(),
                }
                .as_result();
            }
            if !namespace.is_server_allowed(server) {
                return ToolError::NamespaceDenied {
                    server: server.to_string(),
                    tool: None,
                }
                .as_result();
            }
            vec![server.to_string()]
        } else {
            namespace.allowed_servers()
        };

        let mut rows = Vec::new();
        for server in servers {
            if !registry.has_server(&server) {
                return ToolError::UnknownServer { server }.as_result();
            }
            let tools = match registry.inventory(&server).await {
                Ok(tools) => tools,
                Err(e) => return e.as_result(),
            };
            for tool in tools {
                if !namespace.is_tool_allowed(&server, &tool.name) {
                    continue; // discovery-time filter: denied tool never emitted
                }
                rows.push(serde_json::json!({
                    "server": server,
                    "tool": tool.name,
                    "name": tool.name,
                    "description": tool.description.unwrap_or_default(),
                }));
            }
        }

        CallToolResult::success(vec![Content::text(
            serde_json::Value::Array(rows).to_string(),
        )])
    }

    async fn handle_get_tool_schema(&self, arguments: Option<JsonObject>) -> CallToolResult {
        let Some(name) = arguments
            .as_ref()
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
        else {
            return ToolError::InvalidRequest {
                tool: "get_tool_schema".to_string(),
                message: "missing string `name`".to_string(),
            }
            .as_result();
        };
        let (server, tool) = match parse_server_tool(name) {
            Some(parts) => parts,
            None => {
                return ToolError::InvalidRequest {
                    tool: "get_tool_schema".to_string(),
                    message: "name must have format server__tool".to_string(),
                }
                .as_result()
            }
        };
        let Some(registry) = &self.registry else {
            return ToolError::NotImplemented {
                tool: "get_tool_schema".to_string(),
            }
            .as_result();
        };
        let Some(namespace) = &self.namespace else {
            return ToolError::NotImplemented {
                tool: "get_tool_schema".to_string(),
            }
            .as_result();
        };
        if !registry.has_server(server) {
            return ToolError::UnknownServer {
                server: server.to_string(),
            }
            .as_result();
        }
        if !namespace.is_tool_allowed(server, tool) {
            return ToolError::NamespaceDenied {
                server: server.to_string(),
                tool: Some(tool.to_string()),
            }
            .as_result();
        }
        let tools = match registry.inventory(server).await {
            Ok(tools) => tools,
            Err(e) => return e.as_result(),
        };
        let Some(found) = tools.into_iter().find(|t| t.name.as_ref() == tool) else {
            return ToolError::UnknownTool {
                server: server.to_string(),
                tool: tool.to_string(),
            }
            .as_result();
        };
        CallToolResult::success(vec![Content::text(
            serde_json::Value::Object((*found.input_schema).clone()).to_string(),
        )])
    }

    async fn handle_invoke_tool(
        &self,
        arguments: Option<JsonObject>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let Some(args) = arguments else {
            return ToolError::InvalidRequest {
                tool: "invoke_tool".to_string(),
                message: "missing arguments object".to_string(),
            }
            .as_result();
        };
        let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
            return ToolError::InvalidRequest {
                tool: "invoke_tool".to_string(),
                message: "missing string `name`".to_string(),
            }
            .as_result();
        };
        let (server, tool) = match parse_server_tool(name) {
            Some(parts) => parts,
            None => {
                return ToolError::InvalidRequest {
                    tool: "invoke_tool".to_string(),
                    message: "name must have format server__tool".to_string(),
                }
                .as_result()
            }
        };
        let Some(registry) = &self.registry else {
            return ToolError::NotImplemented {
                tool: "invoke_tool".to_string(),
            }
            .as_result();
        };
        let Some(namespace) = &self.namespace else {
            return ToolError::NotImplemented {
                tool: "invoke_tool".to_string(),
            }
            .as_result();
        };
        if !registry.has_server(server) {
            return ToolError::UnknownServer {
                server: server.to_string(),
            }
            .as_result();
        }
        if !namespace.is_tool_allowed(server, tool) {
            return ToolError::NamespaceDenied {
                server: server.to_string(),
                tool: Some(tool.to_string()),
            }
            .as_result();
        }
        let Some(raw_arguments) = args.get("arguments") else {
            return ToolError::InvalidRequest {
                tool: "invoke_tool".to_string(),
                message: "missing object `arguments`".to_string(),
            }
            .as_result();
        };
        let Some(raw_arguments) = raw_arguments.as_object().cloned() else {
            return ToolError::InvalidRequest {
                tool: "invoke_tool".to_string(),
                message: "`arguments` must be an object".to_string(),
            }
            .as_result();
        };

        // Race the upstream call against the downstream cancellation token.
        // `ct` is cancelled by rmcp when a `notifications/cancelled` arrives
        // for this request id. Dropping the call future aborts the local
        // await (including the timeout wrapper inside registry) without
        // waiting the full upstream duration.
        //
        // Per OQ3 (rmcp =1.8.0): we do NOT attempt to forward
        // `notify_cancelled` upstream because the typed `peer().call_tool`
        // future does not expose the upstream request id we would need.
        // The observable is the local abort only.
        let call_fut = registry.call_tool(server, tool, Some(raw_arguments));
        tokio::select! {
            res = call_fut => match res {
                Ok(result) => result,
                Err(e) => e.as_result(),
            },
            _ = context.ct.cancelled() => {
                ToolError::CallCancelled {
                    server: server.to_string(),
                    tool: tool.to_string(),
                }
                .as_result()
            }
        }
    }
}

fn parse_server_tool(name: &str) -> Option<(&str, &str)> {
    let (server, tool) = name.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

/// `list_tools` — optional `server` string filter.
fn list_tools_tool() -> Tool {
    Tool::new(
        "list_tools",
        LIST_TOOLS_DESC,
        optional_string_object_schema(&["server"]),
    )
}

/// `get_tool_schema` — required `name` string.
fn get_tool_schema_tool() -> Tool {
    Tool::new(
        "get_tool_schema",
        GET_TOOL_SCHEMA_DESC,
        required_string_object_schema(&["name"]),
    )
}

/// `invoke_tool` — required `name` string and required `arguments` object,
/// with the D-006 conservative annotations.
fn invoke_tool_tool() -> Tool {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    let mut props = serde_json::Map::new();
    props.insert("name".to_string(), serde_json::json!({ "type": "string" }));
    props.insert(
        "arguments".to_string(),
        serde_json::json!({ "type": "object" }),
    );
    schema.insert("properties".to_string(), serde_json::Value::Object(props));
    schema.insert(
        "required".to_string(),
        serde_json::json!(["name", "arguments"]),
    );

    let annotations = ToolAnnotations::from_raw(
        None,
        Some(false), // readOnlyHint
        Some(true),  // destructiveHint
        None,
        Some(true), // openWorldHint
    );

    Tool::new("invoke_tool", INVOKE_TOOL_DESC, Arc::new(schema)).with_annotations(annotations)
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

/// Build a JSON-schema object with required string properties.
fn required_string_object_schema(props: &[&str]) -> Arc<JsonObject> {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".into()),
    );
    let mut properties = serde_json::Map::new();
    let required: Vec<serde_json::Value> = props
        .iter()
        .map(|p| serde_json::Value::String((*p).into()))
        .collect();
    for p in props {
        properties.insert((*p).to_string(), serde_json::json!({ "type": "string" }));
    }
    schema.insert(
        "properties".to_string(),
        serde_json::Value::Object(properties),
    );
    schema.insert("required".to_string(), serde_json::Value::Array(required));
    Arc::new(schema)
}

/// Render the structured not-implemented tool result.
///
/// Returns `Ok(CallToolResult::error(...))` — a tool-level result the caller
/// sees and can reason about, never a JSON-RPC error (D-005).
fn not_implemented_result(tool: String) -> CallToolResult {
    let message = ToolError::NotImplemented { tool }.message();
    CallToolResult::error(vec![Content::text(message)])
}
