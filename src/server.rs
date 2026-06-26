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
}

impl Aggregator {
    /// Build a new aggregator from the resolved CLI configuration.
    pub fn new(config: CliConfig) -> Self {
        Self { config }
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
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let tool = request.name.to_string();
        std::future::ready(Ok(not_implemented_result(tool)))
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
