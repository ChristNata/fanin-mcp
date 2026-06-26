//! Error model.
//!
//! D-005 / GOTCHA #3: tool-level errors are `CallToolResult { isError: true }`,
//! never JSON-RPC errors; the structured-error JSON shape is public API. The
//! `NotImplemented` text is the readable content the LLM reasons about — it is
//! not part of the public wire contract (the exact wording is free to change),
//! but the *shape* (a text content block + `isError: true`) is fixed.
//!
//! [`StartupError`] is the typed error for config load + startup validation. It
//! is rendered to stderr via `tracing` and never to stdout (GOTCHA #1): a
//! startup failure exits the process BEFORE `serve(stdio())` begins, so the
//! JSON-RPC stream is never corrupted.

use std::io;

use rmcp::model::{CallToolResult, Content};
use serde_json::json;

/// A tool-level failure surfaced to the caller as structured content.
///
/// Produces a text content block inside `CallToolResult { isError: true }`.
#[derive(Debug, Clone)]
pub enum ToolError {
    /// The meta-tool surface is declared but the forward path is not wired yet.
    ///
    /// Phase 0 ships the three static meta-tools and returns this for every
    /// `tools/call`. Real proxying (registry, forward, process) lands in
    /// Phase 1.
    NotImplemented { tool: String },
    /// A meta-tool request was malformed.
    InvalidRequest { tool: String, message: String },
    /// A requested upstream server is unknown or not visible in the namespace.
    UnknownServer { server: String },
    /// A requested upstream tool is unknown.
    UnknownTool { server: String, tool: String },
    /// The namespace ACL denied a server/tool.
    NamespaceDenied {
        server: String,
        tool: Option<String>,
    },
    /// Spawning or initializing the upstream failed.
    UpstreamConnect { server: String, message: String },
    /// The upstream tool call itself failed before producing a tool result.
    UpstreamCall {
        server: String,
        tool: String,
        message: String,
    },
}

impl ToolError {
    /// Render the error as the human-readable text a content block carries.
    pub fn message(&self) -> String {
        match self {
            ToolError::NotImplemented { tool } => {
                format!(
                    "tool `{tool}` is not implemented in this build of fanin-mcp; \
                     upstream proxying is not wired yet"
                )
            }
            ToolError::InvalidRequest { tool, message } => {
                structured_error(None, Some(tool), "invalid_request", message, true)
            }
            ToolError::UnknownServer { server } => structured_error(
                Some(server),
                None,
                "unknown_server",
                "server is not configured or not visible in the active namespace",
                true,
            ),
            ToolError::UnknownTool { server, tool } => structured_error(
                Some(server),
                Some(tool),
                "unknown_tool",
                &format!("unknown upstream tool `{tool}` on server `{server}`"),
                true,
            ),
            ToolError::NamespaceDenied { server, tool } => structured_error(
                Some(server),
                tool.as_deref(),
                "namespace_denied",
                "server or tool is denied by the active namespace",
                true,
            ),
            ToolError::UpstreamConnect { server, message } => {
                structured_error(Some(server), None, "upstream_connect_failed", message, true)
            }
            ToolError::UpstreamCall {
                server,
                tool,
                message,
            } => structured_error(
                Some(server),
                Some(tool),
                "upstream_call_failed",
                message,
                true,
            ),
        }
    }

    /// Render the error as a tool-level `CallToolResult`.
    pub fn as_result(&self) -> CallToolResult {
        CallToolResult::error(vec![Content::text(self.message())])
    }
}

fn structured_error(
    server: Option<&str>,
    tool: Option<&str>,
    code: &str,
    message: &str,
    recoverable: bool,
) -> String {
    json!({
        "server": server,
        "tool": tool,
        "code": code,
        "message": message,
        "recoverable": recoverable,
    })
    .to_string()
}

/// A startup / config-validation failure.
///
/// Returned by `config::load_and_validate`; rendered to stderr by `run_serve`
/// via `tracing`. Never written to stdout (GOTCHA #1).
#[derive(Debug)]
pub enum StartupError {
    /// The config file could not be read.
    ReadConfig(io::Error),
    /// The config file could not be parsed as TOML / deserialized.
    ParseConfig(toml::de::Error),
    /// A server name is outside `[a-z0-9-]+` (rejects uppercase, underscore,
    /// `__`, spaces, and any other disallowed character — GOTCHA #15).
    InvalidServerName { server: String, reason: String },
    /// A server transport is outside the Phase 1 stdio-only contract.
    UnsupportedTransport { server: String, transport: String },
    /// A stdio server table is missing the `command` field.
    StdioServerMissingCommand { server: String },
    /// The resolved `--namespace` is not present in `[namespaces]`.
    UnknownNamespace { namespace: String },
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupError::ReadConfig(e) => {
                write!(f, "failed to read config file: {e}")
            }
            StartupError::ParseConfig(e) => {
                write!(f, "failed to parse config file as TOML: {e}")
            }
            StartupError::InvalidServerName { server, reason } => {
                write!(f, "invalid server name `{server}`: {reason}")
            }
            StartupError::UnsupportedTransport { server, transport } => {
                write!(
                    f,
                    "unsupported transport `{transport}` for server `{server}`; Phase 1 supports only `stdio`"
                )
            }
            StartupError::StdioServerMissingCommand { server } => {
                write!(
                    f,
                    "stdio server `{server}` is missing the required `command` field"
                )
            }
            StartupError::UnknownNamespace { namespace } => {
                write!(
                    f,
                    "unknown namespace `{namespace}`; not present in [namespaces]"
                )
            }
        }
    }
}

impl std::error::Error for StartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StartupError::ReadConfig(e) => Some(e),
            StartupError::ParseConfig(e) => Some(e),
            _ => None,
        }
    }
}
