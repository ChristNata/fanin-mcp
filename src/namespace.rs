//! Namespace ACL — scopes a session's visible tools.

use std::collections::HashSet;

use crate::config::{TomlConfig, DEFAULT_NAMESPACE};

/// The active namespace ACL for this session.
#[derive(Debug, Clone)]
pub struct ActiveNamespace {
    name: String,
    servers: HashSet<String>,
}

impl ActiveNamespace {
    /// Builds an ACL from the validated config and selected namespace.
    pub fn new(config: &TomlConfig, selected: &str) -> Self {
        let name = if selected.is_empty() {
            DEFAULT_NAMESPACE.to_string()
        } else {
            selected.to_string()
        };
        let servers = config
            .namespaces
            .get(&name)
            .map(|namespace| namespace.servers.iter().cloned().collect())
            .unwrap_or_default();
        Self { name, servers }
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
    pub fn is_tool_allowed(&self, server: &str, _tool: &str) -> bool {
        self.is_server_allowed(server)
    }

    /// Lists visible configured servers in deterministic order.
    pub fn allowed_servers(&self) -> Vec<String> {
        let mut servers: Vec<String> = self.servers.iter().cloned().collect();
        servers.sort();
        servers
    }
}
