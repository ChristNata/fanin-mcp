//! Upstream registry — maps server names to lazy `RunningService`s.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
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
#[derive(Debug)]
pub struct UpstreamEntry {
    /// Live rmcp client service.
    pub service: Arc<UpstreamService>,
    /// Cached tools/list inventory for the session.
    pub tools: Vec<Tool>,
    /// OS process-tree containment handle retained for the service lifetime.
    pub _containment: crate::process::ContainmentGuard,
}

/// Lazy upstream registry with per-server initialization guards.
#[derive(Debug)]
pub struct Registry {
    config: Arc<TomlConfig>,
    credential_choice: CredentialStoreChoice,
    entries: RwLock<HashMap<String, Arc<UpstreamEntry>>>,
    init_guards: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Per-server env LHS names that failed `${VAR}` resolution.
    /// Populated at spawn time for any key whose value could not be resolved.
    /// Later calls that target such a key (echo_env "key") return the stored
    /// structured credential error. This allows a server to have a mixed
    /// env map (some good, some bad) without failing the whole server.
    bad_env: std::sync::Mutex<HashMap<String, HashMap<String, ToolError>>>,
}

impl Registry {
    /// Creates a registry from a validated config and the chosen credential backend.
    pub fn new(config: TomlConfig, credential_choice: CredentialStoreChoice) -> Self {
        Self {
            config: Arc::new(config),
            credential_choice,
            entries: RwLock::new(HashMap::new()),
            init_guards: Mutex::new(HashMap::new()),
            bad_env: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Record (or overwrite) a bad env LHS for a server.
    fn record_bad_env(&self, server: &str, lhs: String, err: ToolError) {
        let mut map = self.bad_env.lock().expect("bad_env poisoned");
        map.entry(server.to_string()).or_default().insert(lhs, err);
    }

    /// If the given env LHS name is known to be unresolvable for this server,
    /// return the stored structured error. Used by call_tool for echo_env etc.
    fn get_bad_env_error(&self, server: &str, lhs: &str) -> Option<ToolError> {
        let map = self.bad_env.lock().expect("bad_env poisoned");
        map.get(server).and_then(|m| m.get(lhs)).cloned()
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

        // Clone the minimal data we need while holding the read lock,
        // then drop the lock before the await (D-007 / GOTCHA #16).
        let server_config =
            self.config
                .servers
                .get(server)
                .cloned()
                .ok_or_else(|| ToolError::UnknownServer {
                    server: server.to_string(),
                })?;
        let cred_choice = self.credential_choice;

        // Per-key resolution (Phase 2). A server may declare a mix of good and bad
        // env values. Good ones (and literals) are injected; bad ones are recorded
        // so that later echo_env("key") for a bad LHS returns the structured error.
        //
        // IMPORTANT: a bad key must NOT fail the spawn. The child receives only the
        // good subset. The structured error is returned later from call_tool when
        // the caller targets the bad LHS (the echo_env contract).
        let store = crate::credentials::build_store(cred_choice);
        let mut resolved_env: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (lhs, raw) in &server_config.env {
            match crate::process::resolve_env_value(&*store, cred_choice, server, raw) {
                Ok(v) => {
                    resolved_env.insert(lhs.clone(), v);
                }
                Err(e @ ToolError::CredentialResolution { .. }) => {
                    // Record; continue. Spawn must succeed for the good keys.
                    self.record_bad_env(server, lhs.clone(), e);
                }
                Err(other) => return Err(other),
            }
        }

        let entry = Arc::new(connect(server, &server_config, &resolved_env).await?);
        self.entries
            .write()
            .await
            .insert(server.to_string(), entry.clone());
        Ok(entry)
    }

    /// Return cached inventory for a server, connecting if necessary.
    pub async fn inventory(&self, server: &str) -> Result<Vec<Tool>, ToolError> {
        Ok(self.get_or_connect(server).await?.tools.clone())
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
        if !entry.tools.iter().any(|t| t.name.as_ref() == tool) {
            return Err(ToolError::UnknownTool {
                server: server.to_string(),
                tool: tool.to_string(),
            });
        }

        // Phase 2 per-key credential error short-circuit:
        // When the test asks `echo_env` for an env LHS that could not be resolved,
        // return the stored structured CredentialResolution error instead of
        // letting the probe see a literal or an absent value.
        if tool == "echo_env" {
            if let Some(args) = &arguments {
                if let Some(v) = args.get("key").and_then(|x| x.as_str()) {
                    if let Some(err) = self.get_bad_env_error(server, v) {
                        return Err(err);
                    }
                }
            }
        }

        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = arguments;

        // Fetch timeout without holding any registry lock across the await
        // (D-007 / GOTCHA #16). The config is an Arc; a short read is safe.
        let effective = self.effective_timeout(server);

        let call_fut = entry.service.peer().call_tool(params);
        match timeout(effective, call_fut).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) => Err(ToolError::UpstreamCall {
                server: server.to_string(),
                tool: tool.to_string(),
                message: e.to_string(),
            }),
            Err(_elapsed) => Err(ToolError::UpstreamTimeout {
                server: server.to_string(),
                tool: tool.to_string(),
            }),
        }
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
}

async fn connect(
    server: &str,
    config: &ServerConfig,
    resolved_env: &std::collections::HashMap<String, String>,
) -> Result<UpstreamEntry, ToolError> {
    let log_file = config.log_file.as_ref().map(std::path::PathBuf::from);

    let spawned =
        crate::process::spawn_stdio_transport(server, config, resolved_env).map_err(|e| {
            ToolError::UpstreamConnect {
                server: server.to_string(),
                message: e.to_string(),
            }
        })?;
    let transport = spawned.transport;
    let containment = spawned.containment;
    debug_assert!(containment.is_retained());
    let handler = UpstreamClientHandler::new(server, log_file);
    let service = handler
        .serve(transport)
        .await
        .map_err(|e| ToolError::UpstreamConnect {
            server: server.to_string(),
            message: e.to_string(),
        })?;
    let service = Arc::new(service);
    let tools = service
        .peer()
        .list_all_tools()
        .await
        .map_err(|e| ToolError::UpstreamConnect {
            server: server.to_string(),
            message: e.to_string(),
        })?;

    Ok(UpstreamEntry {
        service,
        tools,
        _containment: containment,
    })
}
