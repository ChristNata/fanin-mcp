//! Forward path — upstream client handler and byte-faithful forwarding.

use std::path::PathBuf;

use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientInfo, CreateElicitationRequestParams, CreateElicitationResult,
    CreateMessageRequestParams, CreateMessageResult, Implementation, ListRootsResult,
    LoggingMessageNotificationParam, ProgressNotificationParam,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData as McpError, RoleClient};
use serde_json::json;

use crate::process::append_log_line;

/// Client-side handler installed for every upstream stdio connection.
#[derive(Debug, Clone)]
pub struct UpstreamClientHandler {
    server: String,
    log_file: Option<PathBuf>,
}

impl UpstreamClientHandler {
    /// Creates a handler for one upstream server.
    pub fn new(server: impl Into<String>, log_file: Option<PathBuf>) -> Self {
        Self {
            server: server.into(),
            log_file,
        }
    }
}

impl ClientHandler for UpstreamClientHandler {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::builder().build(),
            Implementation::new("fanin-mcp", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn create_message(
        &self,
        _params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, McpError> {
        Err(McpError::invalid_request(
            "fanin-mcp does not support upstream sampling requests in MVP",
            Some(json!({
                "server": self.server,
                "code": "sampling_rejected",
                "recoverable": false,
            })),
        ))
    }

    async fn create_elicitation(
        &self,
        _request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, McpError> {
        Err(McpError::invalid_request(
            "fanin-mcp does not support upstream elicitation requests in MVP",
            Some(json!({
                "server": self.server,
                "code": "elicitation_rejected",
                "recoverable": false,
            })),
        ))
    }

    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, McpError> {
        Ok(ListRootsResult::new(Vec::new()))
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let raw = format!("logging {:?}: {}", params.level, params.data);
        self.append_redacted(raw).await;
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let raw = format!("progress {:?}", params);
        self.append_redacted(raw).await;
    }
}

impl UpstreamClientHandler {
    /// Redact then append to the per-server log file if one is configured.
    /// Centralizes the redaction + append pattern used by log and progress handlers.
    async fn append_redacted(&self, raw: String) {
        if let Some(path) = &self.log_file {
            let line = crate::process::redact(&raw);
            append_log_line(path.clone(), self.server.clone(), line).await;
        }
    }
}
