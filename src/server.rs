//! Downstream MCP server surface.
//!
//! GOTCHA #1: stdout is the MCP transport. Nothing in this module writes to
//! stdout; all diagnostics go to stderr via `tracing`.
//!
//! The static description prefixes below are FINAL design (D-003), copied
//! verbatim from `tests/common/expectations.rs` / `master.md` §Required
//! Pattern. `list_tools` may append a configured, advisory ToC suffix.

use std::future::Future;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, InitializeResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

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
    registry: Option<Arc<Registry>>,
    namespace: Option<ActiveNamespace>,
}

impl Aggregator {
    /// Build a new aggregator from the resolved CLI configuration.
    pub fn new() -> Self {
        Self {
            registry: None,
            namespace: None,
        }
    }

    /// Build a new aggregator with a live upstream registry.
    pub fn with_registry(registry: Arc<Registry>, namespace: ActiveNamespace) -> Self {
        Self {
            registry: Some(registry),
            namespace: Some(namespace),
        }
    }

    /// Builds the three meta-tools, adding a configured table of contents when available.
    fn meta_tools(&self) -> Vec<Tool> {
        let toc = self.configured_toc();
        vec![
            list_tools_tool(toc.as_deref()),
            get_tool_schema_tool(),
            invoke_tool_tool(),
        ]
    }

    /// Builds the configured server table of contents without inventorying upstreams.
    fn configured_toc(&self) -> Option<String> {
        const INSTRUCTIONS_BUDGET: usize = 3_000;
        const TOC_HEADER: &str = "\n\nConfigured servers:\n";

        let registry = self.registry.as_ref()?;
        let namespace = self.namespace.as_ref()?;
        let config = registry.toml_config();
        let cache_summaries = crate::check::matching_cache_tool_summaries(config, namespace);
        let servers = namespace.allowed_servers();
        if servers.is_empty() {
            return None;
        }

        // Server lines are the durable part of the advertisement. Render every
        // one before spending the bounded space on optional cache-derived hints.
        let entries = servers
            .iter()
            .map(|server| {
                let description = config
                    .servers
                    .get(server)
                    .and_then(|server_config| server_config.description.as_deref())
                    .map(sanitize_list_row_description)
                    .filter(|description| !description.is_empty());
                match description {
                    Some(description) => format!("- {server}: {description}\n"),
                    None => format!("- {server}\n"),
                }
            })
            .collect::<Vec<_>>();

        let base_len = TOC_HEADER.len() + entries.iter().map(String::len).sum::<usize>();
        let mut remaining_budget = INSTRUCTIONS_BUDGET.saturating_sub(base_len);
        let mut hints = Vec::with_capacity(servers.len());

        for server in &servers {
            let names = cache_summaries
                .get(server)
                .into_iter()
                .flatten()
                // Authorization remains the namespace query, never the cache.
                .filter(|(name, _)| namespace.is_tool_allowed(server, name))
                .map(|(name, _)| sanitize_upstream_identifier(name))
                .collect::<Vec<_>>();

            let hint = (1..=names.len()).rev().find_map(|shown| {
                let omitted = names.len() - shown;
                let more = (omitted > 0).then(|| format!(" +{omitted} more"));
                let hint = format!(
                    " (tools: {}{})",
                    names[..shown].join(", "),
                    more.unwrap_or_default()
                );
                (hint.len() <= remaining_budget).then_some(hint)
            });
            if let Some(hint) = hint {
                remaining_budget -= hint.len();
                hints.push(hint);
            } else {
                hints.push(String::new());
            }
        }

        let mut toc = String::with_capacity(base_len + INSTRUCTIONS_BUDGET - remaining_budget);
        toc.push_str(TOC_HEADER);
        for (entry, hint) in entries.into_iter().zip(hints) {
            toc.push_str(entry.trim_end());
            toc.push_str(&hint);
            toc.push('\n');
        }
        Some(toc)
    }
}

impl ServerHandler for Aggregator {
    /// Advertise server info and the `tools` capability (GOTCHA #8).
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        let info = InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")));
        match self.configured_toc() {
            Some(toc) => info.with_instructions(toc),
            None => info,
        }
    }

    /// Returns exactly the three meta-tools with static prefixes and a ToC suffix.
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
                let disp_name = sanitize_upstream_identifier(&tool.name);
                let disp_desc =
                    sanitize_list_row_description(&tool.description.unwrap_or_default());
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
        // Sanitize only upstream-authored annotation strings inside the schema
        // (title/description/$comment/markdownDescription). Validation values
        // (enum, const, default, examples, pattern, etc.) are left untouched so
        // the returned JSON remains semantically faithful to the upstream schema.
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

/// Sanitize upstream-authored display text for LLM-visible display.
///
/// Control-neutralization is DISPLAY-WIDE: it applies to both `list_tools` row
/// descriptions and `get_tool_schema` annotation strings (`title`/`description`/
/// `$comment`/`markdownDescription`). It replaces C0, C1, DEL, Unicode
/// separators, bidi controls, BOM, and common zero-width format chars with a
/// single ASCII space, then trims to a single logical line. It does NOT
/// length-cap.
///
/// The ~100-char length cap is a `list_tools` ROW control ONLY — see
/// `sanitize_list_row_description`. It does NOT apply to `get_tool_schema`
/// annotations, which are relayed full-length after neutralization.
///
/// This is DISPLAY-ONLY. It is never applied to `invoke_tool` arguments or
/// result content (see D-004 / GOTCHA #4), nor to schema validation values
/// (`enum`/`const`/`default`/`pattern`/`examples`).
fn neutralize_upstream_display(s: &str) -> String {
    s.chars()
        .map(|c| {
            if should_neutralize_upstream_char(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Sanitize a `list_tools` row description: control-neutralize, trim to a single
/// line, then cap at ~100 Unicode scalars (the row summary cap). The cap is a
/// row control ONLY; `get_tool_schema` annotations use `neutralize_upstream_display`
/// and are relayed full-length.
fn sanitize_list_row_description(s: &str) -> String {
    const CAP: usize = 100;
    // Cap AFTER strip; char iterator never splits multibyte.
    neutralize_upstream_display(s).chars().take(CAP).collect()
}

/// Sanitize an upstream identifier for LLM-visible row keys without changing
/// dispatch identity by length-capping it.
fn sanitize_upstream_identifier(s: &str) -> String {
    const CAP: usize = 200;
    // Defense-in-depth against a non-rmcp upstream sending an over-long raw tool name.
    s.chars()
        .map(|c| {
            if should_neutralize_upstream_char(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(CAP)
        .collect()
}

fn should_neutralize_upstream_char(c: char) -> bool {
    let u = c as u32;
    matches!(
        u,
        0x0000..=0x001F // C0 controls
            | 0x007F // DEL
            | 0x0080..=0x009F // C1 controls
            | 0x200B..=0x200D // zero-width space/joiners
            | 0x2028..=0x2029 // line / paragraph separators
            | 0x202A..=0x202E // bidi embeddings / overrides
            | 0x2066..=0x2069 // bidi isolates
            | 0xFEFF // BOM / zero-width no-break space
    )
}

/// Recursively sanitize string values that appear under upstream-authored
/// schema metadata keys, while preserving the JSON structure required by
/// JSON Schema consumers (type, properties, required, property keys, etc.).
///
/// Targeted annotation keys: "title", "description", "$comment", and
/// "markdownDescription". Validation and structural values are left verbatim;
/// only the contents of those annotation strings change.
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
        "title" | "description" | "$comment" | "markdownDescription"
    )
}

fn sanitize_metadata_value(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(neutralize_upstream_display(s)),
        other => sanitize_schema_metadata(other),
    }
}

/// `list_tools` — optional `server` string filter.
fn list_tools_tool(toc: Option<&str>) -> Tool {
    let description = match toc {
        Some(toc) => format!("{LIST_TOOLS_DESC}{toc}"),
        None => LIST_TOOLS_DESC.to_string(),
    };
    Tool::new(
        "list_tools",
        description,
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
