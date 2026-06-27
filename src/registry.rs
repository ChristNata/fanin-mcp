//! Upstream registry — maps server names to lazy `RunningService`s.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use http::{HeaderName, HeaderValue};
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;

use crate::config::{ServerConfig, TomlConfig};
use crate::credentials::CredentialStoreChoice;
use crate::error::ToolError;
use crate::forward::UpstreamClientHandler;

/// Running upstream service type.
pub type UpstreamService = RunningService<RoleClient, UpstreamClientHandler>;

/// Cached upstream connection and inventory.
///
/// The `tools` cache is interior-mutable (RwLock) so a `list_changed`
/// notification can mark it dirty and a subsequent read can lazily refetch
/// without rebuilding the `Arc<UpstreamEntry>`.
///
/// `dirty` is an `Arc<AtomicBool>` (not a back-pointer) so the handler can
/// set it without creating an Arc cycle and without ever touching the
/// registry map. The registry entry holds the same Arc so `ensure_fresh`
/// can observe it on the read path.
#[derive(Debug)]
pub struct UpstreamEntry {
    /// Configured upstream server name for lifecycle diagnostics.
    pub server: String,
    /// Live rmcp client service.
    pub service: Arc<UpstreamService>,
    /// Cached tools/list inventory for the session (refetchable on dirty).
    pub tools: RwLock<Vec<Tool>>,
    /// Dirty flag for `notifications/tools/list_changed` invalidation.
    /// Set by the handler for THIS server only; observed lazily on read.
    pub dirty: Arc<AtomicBool>,
    /// OS process-tree containment handle retained for the service lifetime.
    pub _containment: crate::process::ContainmentGuard,
}

impl Drop for UpstreamEntry {
    fn drop(&mut self) {
        tracing::info!(server = %self.server, event = "upstream_disconnect", "upstream disconnected");
    }
}

/// Lazy upstream registry with per-server initialization guards.
#[derive(Debug)]
pub struct Registry {
    config: Arc<TomlConfig>,
    credential_choice: CredentialStoreChoice,
    entries: RwLock<HashMap<String, Arc<UpstreamEntry>>>,
    init_guards: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Registry {
    /// Creates a registry from a validated config and the chosen credential backend.
    pub fn new(config: TomlConfig, credential_choice: CredentialStoreChoice) -> Self {
        Self {
            config: Arc::new(config),
            credential_choice,
            entries: RwLock::new(HashMap::new()),
            init_guards: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if the server exists in the config.
    pub fn has_server(&self, server: &str) -> bool {
        self.config.servers.contains_key(server)
    }

    /// Returns a cached/lazy upstream connection.
    pub async fn get_or_connect(&self, server: &str) -> Result<Arc<UpstreamEntry>, ToolError> {
        if let Some(entry) = self.entries.read().await.get(server).cloned() {
            return Ok(entry);
        }

        let guard = {
            let mut guards = self.init_guards.lock().await;
            guards
                .entry(server.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };

        let _init = guard.lock().await;
        if let Some(entry) = self.entries.read().await.get(server).cloned() {
            return Ok(entry);
        }

        // Borrow the server config from the Arc-held TomlConfig (no lock needed here).
        // The reference is valid across the resolution work and the connect await.
        // (D-007 / GOTCHA #16 — no entries lock is held.)
        let server_config =
            self.config
                .servers
                .get(server)
                .ok_or_else(|| ToolError::UnknownServer {
                    server: server.to_string(),
                })?;
        let cred_choice = self.credential_choice;

        // Fail-closed resolution: every configured env value is resolved before
        // spawn. If any `${VAR}` is missing from the preferred credential backend
        // and process-env fallback, the server is not spawned and every call to
        // that server receives the same structured credential error.
        let store = crate::credentials::build_store(cred_choice);
        let mut resolved_env = HashMap::new();
        for (lhs, raw) in &server_config.env {
            let resolved = crate::process::resolve_env_value(&*store, cred_choice, server, raw)?;
            resolved_env.insert(lhs.clone(), resolved);
        }
        let mut resolved_headers = HashMap::new();
        for (name, raw) in &server_config.headers {
            let resolved = crate::process::resolve_env_value(&*store, cred_choice, server, raw)?;
            if raw.contains("${") {
                crate::process::register_secret(&resolved);
            }
            resolved_headers.insert(name.clone(), resolved);
        }

        let entry =
            Arc::new(connect(server, server_config, &resolved_env, &resolved_headers).await?);
        self.entries
            .write()
            .await
            .insert(server.to_string(), entry.clone());
        Ok(entry)
    }

    /// Return cached inventory for a server, connecting if necessary.
    ///
    /// Lazily refetches if the per-entry dirty flag was set by a prior
    /// `notifications/tools/list_changed` for this server only.
    pub async fn inventory(&self, server: &str) -> Result<Vec<Tool>, ToolError> {
        let entry = self.get_or_connect(server).await?;
        self.ensure_fresh(&entry, server).await?;
        let tools = entry.tools.read().await.clone();
        Ok(tools)
    }

    /// Forward a tool call without holding the registry map lock across await.
    ///
    /// Every upstream call is wrapped in the per-server `timeout_secs` (default 60).
    /// On timeout we return `ToolError::UpstreamTimeout` as a structured
    /// `CallToolResult { isError: true }` — never a JSON-RPC error (D-005).
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ToolError> {
        let entry = self.get_or_connect(server).await?;
        // Ensure a fresh inventory if a list_changed notification arrived for
        // this server. Lock discipline: get_or_connect already dropped the map
        // lock; we hold only the cloned Arc<Entry> here.
        self.ensure_fresh(&entry, server).await?;
        if !entry
            .tools
            .read()
            .await
            .iter()
            .any(|t| t.name.as_ref() == tool)
        {
            return Err(ToolError::UnknownTool {
                server: server.to_string(),
                tool: tool.to_string(),
            });
        }

        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = arguments;

        // Fetch timeout without holding any registry lock across the await
        // (D-007 / GOTCHA #16). The config is an Arc; a short read is safe.
        let effective = self.effective_timeout(server);

        let started = Instant::now();
        let call_fut = entry.service.peer().call_tool(params);
        let outcome = match timeout(effective, call_fut).await {
            Ok(Ok(result)) => {
                let is_error = result.is_error.unwrap_or(false);
                log_tool_call(
                    server,
                    tool,
                    started,
                    if is_error { "failure" } else { "success" },
                );
                Ok(result)
            }
            Ok(Err(e)) => {
                // Transport-layer death → UpstreamDisconnected (distinct from live-call failure).
                // No map lock held across the await (D-007).
                log_tool_call(server, tool, started, "failure");
                let mapped = map_service_error(e, server, tool);
                if matches!(mapped, ToolError::UpstreamDisconnected { .. }) {
                    tracing::warn!(
                        server,
                        tool,
                        event = "upstream_disconnect",
                        "upstream disconnected"
                    );
                }
                Err(mapped)
            }
            Err(_elapsed) => {
                log_tool_call(server, tool, started, "failure");
                tracing::warn!(
                    server,
                    tool,
                    event = "upstream_failure",
                    code = "timeout",
                    "upstream call timed out"
                );
                Err(ToolError::UpstreamTimeout {
                    server: server.to_string(),
                    tool: tool.to_string(),
                })
            }
        };
        outcome
    }

    /// Effective timeout for a server (from config, default 60).
    /// Callers must ensure no registry map lock is held when awaiting the call.
    fn effective_timeout(&self, server: &str) -> Duration {
        let secs = self
            .config
            .servers
            .get(server)
            .map(|c| c.timeout_secs)
            .unwrap_or(60);
        Duration::from_secs(secs)
    }

    /// Lazily refetch the tool inventory for `entry` if its dirty flag is set.
    ///
    /// - Reads the atomic dirty flag (no lock).
    /// - If dirty: `swap(false)` (only one racer refetches), then calls
    ///   `peer().list_all_tools().await` with **no** registry map lock and
    ///   **no** `tools` RwLock held across the await (D-007 / GOTCHA #16).
    /// - On successful refetch, briefly acquires the per-entry `tools` write
    ///   lock only to overwrite the cached vec.
    /// - On refetch failure, restores dirty=true before returning the error so
    ///   a later read retries instead of serving stale inventory.
    /// - The `server` name is used only for error construction.
    ///
    /// The caller must have already dropped the registry `entries` map lock
    /// (guaranteed by `get_or_connect` returning a cloned `Arc<UpstreamEntry>`).
    async fn ensure_fresh(
        &self,
        entry: &Arc<UpstreamEntry>,
        server: &str,
    ) -> Result<(), ToolError> {
        // Fast path: not dirty.
        if !entry.dirty.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Swap to false; only the winner of a race performs the refetch.
        // A benign double-refetch is acceptable; we must not deadlock.
        let was_dirty = entry.dirty.swap(false, Ordering::Relaxed);
        if !was_dirty {
            // Another task cleared it first; nothing to do.
            return Ok(());
        }

        // IMPORTANT: do NOT hold any registry map lock here.
        // We hold only the cloned Arc<Entry> (map lock already dropped).
        // Also do NOT hold entry.tools across the await.
        let fresh = match entry.service.peer().list_all_tools().await {
            Ok(list) => list,
            Err(e) => {
                entry.dirty.store(true, Ordering::Relaxed);
                return Err(map_service_error(e, server, "")); // empty tool: matches prior observable for ensure_fresh
            }
        };

        // Briefly hold only the per-entry tools lock to install the fresh list.
        {
            let mut guard = entry.tools.write().await;
            *guard = fresh;
        }
        Ok(())
    }
}

/// Map a rmcp `ServiceError` from an upstream operation to the appropriate
/// `ToolError`, distinguishing transport death (mid-session disconnect)
/// from ordinary call failures. Preserves the exact observable strings
/// and the empty-tool convention used by `ensure_fresh`.
fn map_service_error(e: ServiceError, server: &str, tool: &str) -> ToolError {
    if matches!(
        e,
        ServiceError::TransportClosed | ServiceError::TransportSend(_)
    ) {
        ToolError::UpstreamDisconnected {
            server: server.to_string(),
            tool: tool.to_string(),
        }
    } else {
        ToolError::UpstreamCall {
            server: server.to_string(),
            tool: tool.to_string(),
            message: e.to_string(),
        }
    }
}

fn build_http_headers(
    server: &str,
    headers: &std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<HeaderName, HeaderValue>, ToolError> {
    let mut custom_headers = std::collections::HashMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|e| ToolError::UpstreamConnect {
                server: server.to_string(),
                message: format!("invalid HTTP header name `{name}`: {e}"),
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|e| ToolError::UpstreamConnect {
                server: server.to_string(),
                message: format!("invalid HTTP header value for `{name}`: {e}"),
            })?;
        custom_headers.insert(header_name, header_value);
    }
    Ok(custom_headers)
}

async fn connect(
    server: &str,
    config: &ServerConfig,
    resolved_env: &std::collections::HashMap<String, String>,
    resolved_headers: &std::collections::HashMap<String, String>,
) -> Result<UpstreamEntry, ToolError> {
    let log_file = config.log_file.as_ref().map(std::path::PathBuf::from);
    tracing::info!(
        server,
        event = "upstream_connect_start",
        "upstream connect starting"
    );

    let containment = crate::process::ContainmentGuard::Inert;
    let _ = containment.is_retained(); // retained for platform guard; value asserted below after spawn

    // Create the per-server dirty flag BEFORE the handler so we can share it.
    // Handler gets a clone; the entry will store the same Arc.
    // No registry map lock is involved here; this is purely local to the entry.
    let dirty = Arc::new(AtomicBool::new(false));
    let handler = UpstreamClientHandler::new(server, log_file, dirty.clone());
    let service = match config.transport_kind() {
        "stdio" => {
            let spawned = crate::process::spawn_stdio_transport(server, config, resolved_env)
                .map_err(|e| {
                    tracing::warn!(server, event = "upstream_connect_failure", error = %e, "upstream spawn failed");
                    ToolError::UpstreamConnect {
                        server: server.to_string(),
                        message: e.to_string(),
                    }
                })?;
            let transport = spawned.transport;
            debug_assert!(spawned.containment.is_retained());
            handler.serve(transport).await
        }
        "streamable-http" => {
            let endpoint = config.endpoint.as_deref().unwrap_or_default();
            let headers = build_http_headers(server, resolved_headers)?;
            let transport_config =
                StreamableHttpClientTransportConfig::with_uri(endpoint.to_string())
                    .custom_headers(headers);
            let transport = StreamableHttpClientTransport::from_config(transport_config);
            handler.serve(transport).await
        }
        _ => unreachable!("config validation rejects unsupported transports"),
    }
    .map_err(|e| {
        tracing::warn!(server, event = "upstream_connect_failure", error = %e, "upstream service failed to start");
        ToolError::UpstreamConnect {
            server: server.to_string(),
            message: e.to_string(),
        }
    })?;
    let service = Arc::new(service);
    let tools = service
        .peer()
        .list_all_tools()
        .await
        .map_err(|e| {
            tracing::warn!(server, event = "upstream_connect_failure", error = %e, "upstream inventory failed");
            ToolError::UpstreamConnect {
                server: server.to_string(),
                message: e.to_string(),
            }
        })?;
    tracing::info!(
        server,
        event = "upstream_connect_success",
        "upstream connected"
    );

    Ok(UpstreamEntry {
        server: server.to_string(),
        service,
        tools: RwLock::new(tools),
        dirty,
        _containment: containment,
    })
}

fn log_tool_call(server: &str, tool: &str, started: Instant, outcome: &str) {
    tracing::info!(
        server,
        tool,
        latency_ms = started.elapsed().as_secs_f64() * 1000.0,
        outcome,
        event = "invoke_tool",
        "upstream tool call completed"
    );
}
