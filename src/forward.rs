//! Forward path — upstream client handler and byte-faithful forwarding.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    ClientCapabilities, ClientInfo, CreateElicitationRequestParams, CreateElicitationResult,
    CreateMessageRequestParams, CreateMessageResult, Implementation, ListRootsResult,
    LoggingMessageNotificationParam, ProgressNotificationParam,
};
use rmcp::service::{MaybeSendFuture, NotificationContext, Peer, RequestContext};
use rmcp::{ErrorData as McpError, RoleClient, RoleServer};
use serde_json::json;

use crate::process::append_log_line;

/// Shared downstream peer cell — the single source of truth for downstream
/// elicitation capability (D-008 / GP-1 / GOTCHA #8).
///
/// Captured once in `main.rs` after `aggregator.serve(stdio())` completes the
/// initialize handshake and BEFORE any upstream can be lazily spawned by a
/// downstream `invoke_tool` (DELTA-3: the peer race is structurally dead —
/// `serve()` populates `peer_info()` before returning `running`). The cell
/// threads through `Registry` → `connect()` → `UpstreamClientHandler::new`
/// as a self-contained `Arc` (no back-reference into the registry map:
/// D-007 / GOTCHA #16). It is read live at forward time so capability
/// advertisement and the forwarding gate are the SAME condition (GP-1).
pub type DownstreamPeerCell = Arc<OnceLock<Peer<RoleServer>>>;

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
    /// Shared downstream peer cell — the single source of truth for whether
    /// the downstream client declared elicitation and the handle via which a
    /// forwarded `elicitation/create` is sent back to that client. Read live
    /// at both `get_info()` (capability advertisement) and
    /// `create_elicitation()` (the forwarding gate) so they are the SAME
    /// condition (GP-1). Self-contained `Arc`; no registry back-reference
    /// (D-007 / GOTCHA #16). The peer handle is reusable — two upstreams may
    /// forward concurrently (GP-9). This is NOT a single-slot current-elicitation
    /// holder; rmcp owns id correlation on both hops.
    downstream_peer: DownstreamPeerCell,
    /// Effective per-server tool-call timeout bounding a forwarded elicitation
    /// so it cannot outlive the enclosing tool-call deadline (GP-3). The
    /// forward handler runs on the upstream service receive task, concurrent
    /// with the downstream `call_tool` timeout wrapper in `registry.rs`; a
    /// never-answering downstream client would dangle the forward await
    /// forever without an independent bound.
    effective_timeout: Duration,
}

impl UpstreamClientHandler {
    /// Creates a handler for one upstream server.
    ///
    /// `dirty` is a per-server atomic flag (shared with the registry entry)
    /// that is set to `true` on receipt of `notifications/tools/list_changed`
    /// for this upstream only. The caller (registry) owns the Arc and clones
    /// it into the handler; no registry map lock is touched inside the handler.
    ///
    /// `downstream_peer` is the shared cell captured in `main.rs` after the
    /// downstream initialize handshake (single source of truth for elicitation
    /// capability — GP-1). `effective_timeout` is the per-server tool-call
    /// deadline bounding any forwarded elicitation (GP-3).
    pub fn new(
        server: impl Into<String>,
        log_file: Option<PathBuf>,
        dirty: Arc<AtomicBool>,
        downstream_peer: DownstreamPeerCell,
        effective_timeout: Duration,
    ) -> Self {
        Self {
            server: server.into(),
            log_file,
            dirty,
            downstream_peer,
            effective_timeout,
        }
    }

    /// Read the downstream peer and its elicitation capability from the cell.
    /// Returns `Some((peer, true))` when the downstream client declared
    /// elicitation and the peer is available — the single condition that
    /// gates BOTH capability advertisement (`get_info`) and forwarding
    /// (`create_elicitation`), so they cannot drift (GP-1).
    fn downstream_elicitation_available(&self) -> Option<Peer<RoleServer>> {
        let peer = self.downstream_peer.get()?;
        let declared = peer
            .peer_info()
            .map(|info| info.capabilities.elicitation.is_some())
            .unwrap_or(false);
        if declared {
            Some(peer.clone())
        } else {
            None
        }
    }
}

impl ClientHandler for UpstreamClientHandler {
    fn get_info(&self) -> ClientInfo {
        // Capability honesty (GP-1 / GOTCHA #8): advertise elicitation to the
        // upstream IFF the downstream peer is captured AND it declared
        // elicitation at initialize. The SAME condition gates the forwarding
        // branch in `create_elicitation` — one source of truth (the peer cell).
        let capabilities = if self.downstream_elicitation_available().is_some() {
            ClientCapabilities::builder().enable_elicitation().build()
        } else {
            ClientCapabilities::builder().build()
        };
        ClientInfo::new(
            capabilities,
            Implementation::new("fanin-mcp", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn create_message(
        &self,
        _params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, McpError> {
        // GP-10: sampling stays rejected. No sampling forwarding path.
        Err(McpError::invalid_request(
            "fanin-mcp does not support upstream sampling requests",
            Some(json!({
                "server": self.server,
                "code": "sampling_rejected",
                "recoverable": false,
            })),
        ))
    }

    async fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, McpError> {
        // GP-1 / GP-3: forward IFF the downstream peer is captured and it
        // declared elicitation. The forward is independently bounded by the
        // effective per-server timeout so it cannot outlive the enclosing
        // tool-call deadline (the forward handler runs on the upstream
        // service receive task, concurrent with the downstream call_tool
        // timeout wrapper). rmcp's create_elicitation_with_timeout sends a
        // `notifications/cancelled` to the downstream client on timeout, so
        // the pending prompt is NOT left dangling.
        let peer_opt = self.downstream_elicitation_available();
        tracing::debug!(
            server = %self.server,
            peer_cell_set = self.downstream_peer.get().is_some(),
            elicitation_available = peer_opt.is_some(),
            effective_timeout_secs = self.effective_timeout.as_secs(),
            "create_elicitation invoked"
        );
        if let Some(peer) = peer_opt {
            match peer
                .create_elicitation_with_timeout(request, Some(self.effective_timeout))
                .await
            {
                // D-004: relay the result VERBATIM. Accept, Decline, and Cancel
                // all pass through as-is — no normalization, no re-serialization.
                Ok(result) => Ok(result),
                // Default-deny (SC10): a timeout, disconnect, drop, malformed
                // response, or downstream JSON-RPC error is NEVER an accept.
                // Map to a structured non-accept McpError; the probe surfaces
                // `non_accept: true` for its tool result so the test asserts
                // the direct non-accept outcome.
                Err(e) => {
                    tracing::warn!(
                        server = %self.server,
                        error = %e,
                        "forwarded elicitation failed (non-accept default-deny)"
                    );
                    Err(McpError::invalid_request(
                        "fanin-mcp forwarded elicitation resolved to non-accept",
                        Some(json!({
                            "server": self.server,
                            "code": "elicitation_non_accept",
                            "recoverable": true,
                        })),
                    ))
                }
            }
        } else {
            // No downstream peer OR client did not declare elicitation: keep
            // the EXISTING structured rejection VERBATIM (GOTCHA #8 honesty).
            Err(McpError::invalid_request(
                "fanin-mcp does not support upstream elicitation requests in this session",
                Some(json!({
                    "server": self.server,
                    "code": "elicitation_rejected",
                    "recoverable": false,
                })),
            ))
        }
    }

    async fn list_roots(
        &self,
        _context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, McpError> {
        // GP-10: roots stays empty. No roots forwarding path.
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
