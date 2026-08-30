//! Configuration — the server config file and CLI flag plumbing.
//!
//! Phase 1: a TOML config model for stdio upstreams and namespaces, a loader
//! that reads the `--config` file path, and fail-fast startup validation.
//!
//! GOTCHA #1: nothing here may print to stdout — config diagnostics go to
//! stderr via `tracing`. A config-validation failure must exit the process
//! BEFORE `serve(stdio())` begins, so no bytes ever reach the JSON-RPC stream.
//!
//! Schema (binding — `tests/common/fixtures.rs` encodes the exact shape):
//!
//! ```toml
//! [servers.<name>]
//! transport = "stdio"          # optional; defaults to "stdio"
//! command = '<path>'           # required for stdio
//! args = []                    # optional; default empty
//! description = "..."          # optional human-readable capability summary
//! cwd = '<path>'               # optional; stdio child working directory; may contain ${VAR}
//! timeout_secs = 60            # optional; default 60 (Phase 2 parses, Phase 3 wraps)
//! log_file = '<path>'          # optional
//! [servers.<name>.env]         # optional; values may contain ${VAR} (interpolated at spawn)
//! # or:
//! transport = "streamable-http"
//! endpoint = "http://127.0.0.1:8080/mcp"
//! [servers.<name>.headers]      # optional; values may contain ${VAR}
//!
//! [namespaces.<name>]
//! servers = ["<name>"]         # the servers visible in this namespace
//!
//! [namespaces.<name>.tools]    # optional per-server name-level allow-list
//! <server> = ["<tool>", ...]   # absent entry for an allowed server => all its tools visible
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::credentials::CredentialStoreChoice;
use crate::error::StartupError;

/// The selected namespace name when `--namespace` is omitted.
///
/// Phase 1: a config that declares `[namespaces.default]` works with the flag
/// omitted. This is the only namespace-name default Phase 1 synthesizes.
pub const DEFAULT_NAMESPACE: &str = "default";

/// Maximum number of `extends` links resolved for one namespace.
///
/// This bounds recursive resolution of hostile acyclic configurations before
/// they can exhaust the process stack.
const MAX_NAMESPACE_INHERITANCE_DEPTH: usize = 64;

/// The resolved CLI configuration for a `serve` invocation.
///
/// Carries the global flags verbatim. The config file is loaded and validated
/// separately in `run_serve` (see [`load_and_validate`]) so a startup failure
/// exits before the aggregator is constructed.
#[derive(Debug, Clone, Default)]
pub struct CliConfig {
    /// The selected namespace for this session.
    pub namespace: String,
    /// The path to the server config file (`--config`).
    pub config_path: Option<PathBuf>,
    /// Preferred credential backend for `${VAR}` resolution (Phase 2+).
    /// Env fallback is always tried after the preferred backend.
    pub credential_store: CredentialStoreChoice,
}

impl CliConfig {
    /// Build a [`CliConfig`] from the parsed global flags.
    pub fn from_flags(
        namespace: Option<String>,
        config_path: Option<PathBuf>,
        credential_store: CredentialStoreChoice,
    ) -> Self {
        Self {
            namespace: namespace.unwrap_or_default(),
            config_path,
            credential_store,
        }
    }
}

/// A Phase 1 TOML config: named stdio upstreams and namespaces.
///
/// Both maps are optional at the TOML level; Phase 1 validation enforces the
/// semantic rules (server names, namespace existence, command presence).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TomlConfig {
    /// `[servers.<name>]` — named stdio upstreams.
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
    /// `[namespaces.<name>]` — named namespaces.
    #[serde(default)]
    pub namespaces: HashMap<String, NamespaceConfig>,
}

/// A single upstream server (`[servers.<name>]`).
///
/// Fields are read by later phases (registry/process forward path); Phase 1
/// validates `transport`, `command` presence, and server names.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Transport kind. Optional; defaults to `"stdio"`.
    #[serde(default)]
    pub transport: Option<String>,
    /// The spawn command. Required for stdio servers; a missing `command`
    /// fails startup (see [`TomlConfig::validate`]).
    pub command: Option<String>,
    /// Streamable-HTTP endpoint URL. Required for `transport = "streamable-http"`.
    pub endpoint: Option<String>,
    /// Spawn args. Optional; defaults to an empty vector.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional human-readable capability description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional stdio child working directory.
    ///
    /// When present for stdio upstreams this is resolved at connect time using
    /// the same `${VAR}` credential/env path as env and headers, then applied
    /// with `Command::current_dir`. When absent, the child inherits fanin-mcp's
    /// process working directory. Streamable-HTTP accepts but ignores this field.
    /// Empty or whitespace-only values are rejected before spawn, including after `${VAR}` resolution.
    pub cwd: Option<String>,
    /// Per-server env vars (`[servers.<name>.env]`). Optional.
    /// Values may contain `${VAR}` placeholders; these are resolved at spawn
    /// time (preferred credential store → process env fallback → structured error).
    /// Literal (non-secret) values pass through unchanged.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Static HTTP headers for Streamable-HTTP upstreams.
    /// Values may contain `${VAR}` placeholders resolved through the same
    /// credential chain as env vars.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Optional per-server log sink for child stderr + upstream log
    /// notifications (Phase 2+ asserts against it).
    pub log_file: Option<String>,
    /// Per-server upstream call timeout in seconds. Optional; defaults to 60.
    /// Parsing + default lands in Phase 2; the actual timeout wrapping is Phase 3.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// Default for `timeout_secs` (60 seconds).
fn default_timeout_secs() -> u64 {
    60
}

/// A single namespace (`[namespaces.<name>]`).
///
/// `servers` is the server allow-list.
/// `tools` is the optional per-server name-level tool allow-list:
///   [namespaces.<name>.tools]
///   alpha = ["echo_ok"]
/// A server in `servers` with no `tools` entry exposes all its tools.
/// A present list is an exact allow-list of tool names.
/// Tool names are not validated at startup (tools known only after discovery).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceConfig {
    /// The servers visible in this namespace.
    #[serde(default)]
    pub servers: Vec<String>,
    /// Parent namespaces whose permissions this namespace composes.
    #[serde(default)]
    pub extends: Vec<String>,
    /// Optional per-server tool allow-lists (name-level only).
    /// Keys are server names (should be in `servers`); values are exact tool
    /// name lists. Absent entry for an allowed server => all tools visible.
    #[serde(default)]
    pub tools: HashMap<String, Vec<String>>,
}

/// The effective ACL after resolving a namespace's inheritance graph.
///
/// An absent `tools` key means all tools are permitted for its server. A
/// present empty set means no tools are permitted and must never be removed.
#[derive(Debug, Clone)]
pub struct ResolvedNamespace {
    /// The selected namespace's effective visible servers.
    pub servers: HashSet<String>,
    /// The selected namespace's restrictive effective tool filters.
    pub tools: HashMap<String, HashSet<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

/// Resolves the effective ACL for a namespace.
///
/// Servers compose by union. Tool filters compose by restrictive intersection:
/// an absent filter is the `All` identity and a present empty filter is `None`.
/// The DFS state distinguishes an active back-edge from a completed branch, so
/// a diamond inheritance graph remains valid.
pub fn resolve_namespace(
    config: &TomlConfig,
    name: &str,
) -> Result<ResolvedNamespace, StartupError> {
    let mut states = HashMap::new();
    let mut resolved = HashMap::new();
    resolve_namespace_inner(config, name, None, 0, &mut states, &mut resolved)
}

fn resolve_namespace_inner(
    config: &TomlConfig,
    name: &str,
    child: Option<&str>,
    depth: usize,
    states: &mut HashMap<String, VisitState>,
    resolved: &mut HashMap<String, ResolvedNamespace>,
) -> Result<ResolvedNamespace, StartupError> {
    if depth > MAX_NAMESPACE_INHERITANCE_DEPTH {
        return Err(StartupError::NamespaceInheritanceTooDeep {
            namespace: name.to_string(),
            max_depth: MAX_NAMESPACE_INHERITANCE_DEPTH,
        });
    }

    match states.get(name) {
        Some(VisitState::Visiting) => {
            return Err(StartupError::NamespaceExtendsCycle {
                namespace: name.to_string(),
            });
        }
        Some(VisitState::Done) => {
            return resolved.get(name).cloned().ok_or_else(|| {
                StartupError::NamespaceExtendsCycle {
                    namespace: name.to_string(),
                }
            });
        }
        None => {}
    }

    let namespace = config.namespaces.get(name).ok_or_else(|| match child {
        Some(child) => StartupError::UnknownNamespaceParent {
            namespace: child.to_string(),
            parent: name.to_string(),
        },
        None => StartupError::UnknownNamespace {
            namespace: name.to_string(),
        },
    })?;

    states.insert(name.to_string(), VisitState::Visiting);
    let mut effective = ResolvedNamespace {
        servers: HashSet::new(),
        tools: HashMap::new(),
    };

    for parent in &namespace.extends {
        let parent =
            resolve_namespace_inner(config, parent, Some(name), depth + 1, states, resolved)?;
        merge_resolved_namespace(&mut effective, &parent);
    }

    effective.servers.extend(namespace.servers.iter().cloned());
    merge_tool_filters(
        &mut effective.tools,
        namespace
            .tools
            .iter()
            .map(|(server, tools)| (server.clone(), tools.iter().cloned().collect())),
    );

    states.insert(name.to_string(), VisitState::Done);
    resolved.insert(name.to_string(), effective.clone());
    Ok(effective)
}

fn merge_resolved_namespace(target: &mut ResolvedNamespace, incoming: &ResolvedNamespace) {
    target.servers.extend(incoming.servers.iter().cloned());
    merge_tool_filters(&mut target.tools, incoming.tools.clone());
}

fn merge_tool_filters(
    target: &mut HashMap<String, HashSet<String>>,
    incoming: impl IntoIterator<Item = (String, HashSet<String>)>,
) {
    for (server, tools) in incoming {
        if let Some(current) = target.get_mut(&server) {
            current.retain(|tool| tools.contains(tool));
        } else {
            // Inserting an empty set is intentional: present-empty means NONE.
            target.insert(server, tools);
        }
    }
}

/// Load and validate the TOML config file at `config_path`.
///
/// Reads the file, deserializes it into a [`TomlConfig`], and runs the Phase 1
/// startup validations (server names, namespace existence, command presence,
/// default-namespace selection). A failure returns a [`StartupError`] so the
/// caller can route it to stderr/tracing and exit before serving.
///
/// `namespace` is the value of `--namespace` (empty string when the flag was
/// omitted). An empty value selects the `default` namespace (see
/// [`DEFAULT_NAMESPACE`]).
pub fn load_and_validate(config_path: &Path, namespace: &str) -> Result<TomlConfig, StartupError> {
    let contents = std::fs::read_to_string(config_path).map_err(StartupError::ReadConfig)?;
    let config: TomlConfig = toml::from_str(&contents).map_err(StartupError::ParseConfig)?;
    config.validate(namespace)?;
    Ok(config)
}

impl TomlConfig {
    /// Run the Phase 1 startup validations against the resolved namespace.
    ///
    /// `namespace` is the raw `--namespace` value (empty when omitted). Empty
    /// selects [`DEFAULT_NAMESPACE`].
    fn validate(&self, namespace: &str) -> Result<(), StartupError> {
        // 1. Server names must match `[a-z0-9-]+`. This rejects uppercase,
        //    underscore (single or double), spaces, and any other character;
        //    `__` is covered by the same rule (GOTCHA #15).
        for name in self.servers.keys() {
            validate_server_name(name)?;
        }

        // 2. Transport is optional and defaults to stdio. Phase 5 adds the
        //    minimal Streamable-HTTP client transport for remote upstreams.
        for (name, server) in &self.servers {
            match server.transport_kind() {
                "stdio" | "streamable-http" => {}
                transport => {
                    return Err(StartupError::UnsupportedTransport {
                        server: name.clone(),
                        transport: transport.to_string(),
                    });
                }
            }
        }

        // 3. Validate transport-specific required fields.
        for (name, server) in &self.servers {
            if matches!(server.cwd.as_deref().map(str::trim), Some(cwd) if cwd.is_empty()) {
                return Err(StartupError::EmptyCwd {
                    server: name.clone(),
                });
            }

            match server.transport_kind() {
                "stdio" => {
                    if !matches!(server.command.as_deref().map(str::trim), Some(command) if !command.is_empty())
                    {
                        return Err(StartupError::StdioServerMissingCommand {
                            server: name.clone(),
                        });
                    }
                }
                "streamable-http" => {
                    if !matches!(server.endpoint.as_deref().map(str::trim), Some(endpoint) if !endpoint.is_empty())
                    {
                        return Err(StartupError::HttpServerMissingEndpoint {
                            server: name.clone(),
                        });
                    }
                }
                _ => unreachable!("unsupported transports are rejected above"),
            }
        }

        // 4. Resolve the active namespace. Empty `--namespace` selects the
        //    default. An unknown namespace (not present in [namespaces]) fails
        //    startup.
        let active = if namespace.is_empty() {
            DEFAULT_NAMESPACE
        } else {
            namespace
        };
        if !self.namespaces.contains_key(active) {
            return Err(StartupError::UnknownNamespace {
                namespace: active.to_string(),
            });
        }

        // 5. Resolve and validate every namespace, not only the selected one.
        //    Tool-filter keys are checked against the effective server set so a
        //    child can restrict an inherited server without re-listing it.
        //    Tool names themselves remain discovery-time data and are not
        //    validated here.
        for namespace_name in self.namespaces.keys() {
            let resolved = resolve_namespace(self, namespace_name)?;
            for server in &resolved.servers {
                if !self.servers.contains_key(server) {
                    return Err(StartupError::EffectiveServerUnknown {
                        namespace: namespace_name.clone(),
                        server: server.clone(),
                    });
                }
            }
            for server in resolved.tools.keys() {
                if !resolved.servers.contains(server) {
                    return Err(StartupError::ToolFilterUnknownServer {
                        namespace: namespace_name.clone(),
                        server: server.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

impl ServerConfig {
    /// Returns the configured transport kind, defaulting to stdio.
    pub fn transport_kind(&self) -> &str {
        self.transport.as_deref().unwrap_or("stdio")
    }
}

/// Validate a server name against `^[a-z0-9-]+$`.
///
/// Rejects uppercase, underscore (single `_` and double `__`), spaces, and any
/// other character outside the allowed set. `__` is rejected by the same rule
/// (it contains `_`), making `server__tool` parsing unambiguous (GOTCHA #15).
fn validate_server_name(name: &str) -> Result<(), StartupError> {
    if name.is_empty() {
        return Err(StartupError::InvalidServerName {
            server: name.to_string(),
            reason: "server name must not be empty".to_string(),
        });
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        let bad: String = name
            .chars()
            .filter(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
            .collect();
        return Err(StartupError::InvalidServerName {
            server: name.to_string(),
            reason: format!(
                "server name must match [a-z0-9-]+; found disallowed character(s): {}",
                bad
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_pass() {
        assert!(validate_server_name("probe").is_ok());
        assert!(validate_server_name("my-db").is_ok());
        assert!(validate_server_name("a1").is_ok());
    }

    #[test]
    fn uppercase_rejected() {
        let err = validate_server_name("UPPERCASE").unwrap_err();
        assert!(matches!(err, StartupError::InvalidServerName { .. }));
    }

    #[test]
    fn single_underscore_rejected() {
        let err = validate_server_name("my_db").unwrap_err();
        assert!(matches!(err, StartupError::InvalidServerName { .. }));
    }

    #[test]
    fn double_underscore_rejected() {
        let err = validate_server_name("my__db").unwrap_err();
        assert!(matches!(err, StartupError::InvalidServerName { .. }));
    }

    #[test]
    fn spaces_rejected() {
        let err = validate_server_name("Bad Name With Space").unwrap_err();
        assert!(matches!(err, StartupError::InvalidServerName { .. }));
    }
}
