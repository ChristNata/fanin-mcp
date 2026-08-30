//! Namespace ACL — scopes a session's visible tools.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::config::{resolve_namespace, TomlConfig, DEFAULT_NAMESPACE};

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
    /// Stored in sorted maps and sets so consumers can deterministically
    /// inspect every present filter, including a present-empty deny-all filter.
    tools: BTreeMap<String, BTreeSet<String>>,
}

impl ActiveNamespace {
    /// Builds an ACL from the validated config and selected namespace.
    pub fn new(config: &TomlConfig, selected: &str) -> Self {
        let name = if selected.is_empty() {
            DEFAULT_NAMESPACE.to_string()
        } else {
            selected.to_string()
        };
        let resolved = match resolve_namespace(config, &name) {
            Ok(resolved) => resolved,
            Err(error) => panic!("ActiveNamespace requires a validated config: {error}"),
        };
        Self {
            name,
            servers: resolved.servers,
            tools: resolved
                .tools
                .into_iter()
                .map(|(server, tools)| (server, tools.into_iter().collect()))
                .collect(),
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
            .map_or(true, |list| list.contains(tool))
    }

    /// Lists visible configured servers in deterministic order.
    pub fn allowed_servers(&self) -> Vec<String> {
        let mut servers: Vec<String> = self.servers.iter().cloned().collect();
        servers.sort();
        servers
    }

    /// Returns the resolved per-server tool filters in deterministic order.
    ///
    /// An absent server key permits all of that server's tools. A present empty
    /// set denies every tool and remains visible to callers of this accessor.
    pub fn effective_tool_filters(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.tools
    }
}
