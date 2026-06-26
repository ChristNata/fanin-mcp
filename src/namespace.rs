//! Namespace ACL — scopes a session's visible tools.

use std::collections::{HashMap, HashSet};

use crate::config::{TomlConfig, DEFAULT_NAMESPACE};

/// The active namespace ACL for this session.
///
/// Stores the server allow-list and the optional per-server tool allow-lists
/// (name-level only). An absent entry for an allowed server means all its
/// tools are visible. A present list is an exact name-level allow-list.
#[derive(Debug, Clone)]
pub struct ActiveNamespace {
    name: String,
    servers: HashSet<String>,
    /// Per-server tool allow-lists. Key present => exact allow-list.
    /// Key absent for an allowed server => all tools on that server visible.
    tools: HashMap<String, Vec<String>>,
}

impl ActiveNamespace {
    /// Builds an ACL from the validated config and selected namespace.
    pub fn new(config: &TomlConfig, selected: &str) -> Self {
        let name = if selected.is_empty() {
            DEFAULT_NAMESPACE.to_string()
        } else {
            selected.to_string()
        };
        let (servers, tools) = config
            .namespaces
            .get(&name)
            .map(|ns| (ns.servers.iter().cloned().collect(), ns.tools.clone()))
            .unwrap_or_default();
        Self {
            name,
            servers,
            tools,
        }
    }

    /// Returns the active namespace name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns true when the server is visible in this namespace.
    pub fn is_server_allowed(&self, server: &str) -> bool {
        self.servers.contains(server)
    }

    /// Returns true when a tool is visible in this namespace.
    ///
    /// A server must be allowed. If no tool list exists for that server,
    /// all tools are visible. If a list is present, the tool name must be
    /// exactly in the list (name-level only).
    pub fn is_tool_allowed(&self, server: &str, tool: &str) -> bool {
        if !self.is_server_allowed(server) {
            return false;
        }
        self.tools
            .get(server)
            .map_or(true, |list| list.iter().any(|t| t == tool))
    }

    /// Lists visible configured servers in deterministic order.
    pub fn allowed_servers(&self) -> Vec<String> {
        let mut servers: Vec<String> = self.servers.iter().cloned().collect();
        servers.sort();
        servers
    }
}
