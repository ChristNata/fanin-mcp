//! Upstream registry — maps server names to lazy `RunningService`s.

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use tokio::sync::{Mutex, RwLock};

use crate::config::{ServerConfig, TomlConfig};
use crate::error::ToolError;
use crate::forward::UpstreamClientHandler;

/// Running upstream service type.
pub type UpstreamService = RunningService<RoleClient, UpstreamClientHandler>;

/// Cached upstream connection and inventory.
#[derive(Debug, Clone)]
pub struct UpstreamEntry {
    /// Live rmcp client service.
    pub service: Arc<UpstreamService>,
    /// Cached tools/list inventory for the session.
    pub tools: Vec<Tool>,
}

/// Lazy upstream registry with per-server initialization guards.
#[derive(Debug)]
pub struct Registry {
    config: Arc<TomlConfig>,
    entries: RwLock<HashMap<String, Arc<UpstreamEntry>>>,
    init_guards: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Registry {
    /// Creates a registry from a validated config.
    pub fn new(config: TomlConfig) -> Self {
        Self {
            config: Arc::new(config),
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

        let server_config =
            self.config
                .servers
                .get(server)
                .cloned()
                .ok_or_else(|| ToolError::UnknownServer {
                    server: server.to_string(),
                })?;

        let entry = Arc::new(connect(server, &server_config).await?);
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

        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = arguments;
        entry
            .service
            .peer()
            .call_tool(params)
            .await
            .map_err(|e| ToolError::UpstreamCall {
                server: server.to_string(),
                tool: tool.to_string(),
                message: e.to_string(),
            })
    }
}

async fn connect(server: &str, config: &ServerConfig) -> Result<UpstreamEntry, ToolError> {
    let log_file = config.log_file.as_ref().map(std::path::PathBuf::from);
    let transport = crate::process::spawn_stdio_transport(server, config).map_err(|e| {
        ToolError::UpstreamConnect {
            server: server.to_string(),
            message: e.to_string(),
        }
    })?;
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

    Ok(UpstreamEntry { service, tools })
}
