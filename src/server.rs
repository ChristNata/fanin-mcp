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
                let disp_name = sanitize_upstream_text(&tool.name);
                let disp_desc = sanitize_upstream_text(&tool.description.unwrap_or_default());
                rows.push(serde_json::json!({
                    "server": server,
                    "tool": disp_name,
                    "name": disp_name,
                    "description": disp_desc,
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
        // Sanitize only upstream-authored metadata strings inside the schema
        // (title/description/$comment/examples/enum display strings).
        // Structural keys (type, properties, required, property keys, etc.)
        // are left untouched so the returned JSON remains a valid schema shape.
        let sanitized_schema =
            sanitize_schema_metadata(&serde_json::Value::Object((*found.input_schema).clone()));
        CallToolResult::success(vec![Content::text(sanitized_schema.to_string())])
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

/// Sanitize upstream-authored text (tool names, descriptions, schema metadata strings)
/// for LLM-visible display only.
///
/// - Strips every C0 control character (U+0000–U+001F) and DEL (U+007F),
///   including `\n`, `\r`, `\t`, VT, FF. Each is replaced by a single ASCII space.
/// - The result is always a single logical line (no newlines or control chars).
/// - After stripping, length is capped to ~100 Unicode characters (scalar values),
///   never splitting a multibyte UTF-8 sequence.
/// - This is DISPLAY-ONLY. It is never applied to `invoke_tool` arguments or
///   result content (see D-004 / GOTCHA #4). Dispatch and namespace checks
///   continue to use the real upstream tool name from the registry inventory.
fn sanitize_upstream_text(s: &str) -> String {
    // Replace every control with a single space (produces single-line, control-free).
    let stripped: String = s
        .chars()
        .map(|c| {
            let u = c as u32;
            if u <= 0x1F || u == 0x7F {
                ' '
            } else {
                c
            }
        })
        .collect();

    // Cap after stripping. Use char count (Unicode scalars), not bytes.
    const CAP: usize = 100;
    let mut capped: String = stripped.chars().take(CAP).collect();

    // If we introduced leading/trailing spaces from boundary controls, trim the outer ones
    // only for cleanliness; inner runs are left (tests do not require collapse).
    // Trimming does not affect control-freedom or the cap (trim happens after take).
    let trimmed = capped.trim();
    if trimmed.len() != capped.len() {
        // Re-apply cap after trim if needed (trim can only shorten).
        capped = trimmed.chars().take(CAP).collect();
    } else {
        capped = trimmed.to_string();
    }

    capped
}

/// Recursively sanitize string values that appear under upstream-authored
/// schema metadata keys, while preserving the JSON structure required by
/// JSON Schema consumers (type, properties, required, property keys, etc.).
///
/// Targeted metadata keys (per plan): "title", "description", "$comment",
/// "examples", "enum". For the array-valued keys we sanitize the string
/// *elements* they contain. All other values and keys are left structurally
/// identical; only the *contents* of those specific metadata strings change.
fn sanitize_schema_metadata(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                let sanitized = if is_schema_metadata_key(key) {
                    sanitize_metadata_value(val)
                } else {
                    sanitize_schema_metadata(val)
                };
                out.insert(key.clone(), sanitized);
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            let out: Vec<_> = arr.iter().map(sanitize_schema_metadata).collect();
            serde_json::Value::Array(out)
        }
        other => other.clone(),
    }
}

fn is_schema_metadata_key(k: &str) -> bool {
    matches!(
        k,
        "title" | "description" | "$comment" | "examples" | "enum"
    )
}

fn sanitize_metadata_value(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(sanitize_upstream_text(s)),
        serde_json::Value::Array(arr) => {
            // examples / enum: sanitize any string members; recurse non-strings
            let out: Vec<_> = arr
                .iter()
                .map(|item| {
                    if let serde_json::Value::String(s) = item {
                        serde_json::Value::String(sanitize_upstream_text(s))
                    } else {
                        sanitize_schema_metadata(item)
                    }
                })
                .collect();
            serde_json::Value::Array(out)
        }
        other => sanitize_schema_metadata(other),
    }
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

/// Build a JSON-schema object with optional or required string properties.
fn string_object_schema(props: &[&str], required: bool) -> Arc<JsonObject> {
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
    if required {
        let req: Vec<_> = props
            .iter()
            .map(|p| serde_json::Value::String((*p).into()))
            .collect();
        schema.insert("required".to_string(), serde_json::Value::Array(req));
    }
    Arc::new(schema)
}

fn optional_string_object_schema(props: &[&str]) -> Arc<JsonObject> {
    string_object_schema(props, false)
}

fn required_string_object_schema(props: &[&str]) -> Arc<JsonObject> {
    string_object_schema(props, true)
}

/// Render the structured not-implemented tool result.
///
/// Returns `Ok(CallToolResult::error(...))` — a tool-level result the caller
/// sees and can reason about, never a JSON-RPC error (D-005).
fn not_implemented_result(tool: String) -> CallToolResult {
    let message = ToolError::NotImplemented { tool }.message();
    CallToolResult::error(vec![Content::text(message)])
}
