//! Forward path — upstream client handler and byte-faithful forwarding.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientInfo, CreateElicitationRequestParams, CreateElicitationResult,
    CreateMessageRequestParams, CreateMessageResult, Implementation, ListRootsResult,
    LoggingMessageNotificationParam, ProgressNotificationParam,
};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext};
use rmcp::{ErrorData as McpError, RoleClient};
use serde_json::json;

use crate::process::append_log_line;

/// Client-side handler installed for every upstream stdio connection.
#[derive(Debug, Clone)]
pub struct UpstreamClientHandler {
    server: String,
    log_file: Option<PathBuf>,
    /// Per-server dirty flag for `notifications/tools/list_changed` cache
    /// invalidation. Shared with the corresponding `UpstreamEntry` so the
    /// registry can observe it on the next read path (lazy refetch).
    /// No back-reference to the entry is stored here — no Arc cycle.
    dirty: Arc<AtomicBool>,
}

impl UpstreamClientHandler {
    /// Creates a handler for one upstream server.
    ///
    /// `dirty` is a per-server atomic flag (shared with the registry entry)
    /// that is set to `true` on receipt of `notifications/tools/list_changed`
    /// for this upstream only. The caller (registry) owns the Arc and clones
    /// it into the handler; no registry map lock is touched inside the handler.
    pub fn new(
        server: impl Into<String>,
        log_file: Option<PathBuf>,
        dirty: Arc<AtomicBool>,
    ) -> Self {
        Self {
            server: server.into(),
            log_file,
            dirty,
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

    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        // Per design (state.json decisions.cache-shape + tests.md "Notes"):
        // Mark ONLY this server's cache dirty. Do NOT refetch here (would risk
        // blocking rmcp's notification path). Do NOT touch the registry map.
        // Lazy refetch happens on the next inventory()/call_tool() read path.
        // Per-server scope: each handler owns only its own dirty flag (SC 10).
        self.dirty.store(true, Ordering::Relaxed);
        std::future::ready(())
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
