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
//! transport = "stdio"          # optional; defaults to "stdio" in Phase 1
//! command = '<path>'           # required for stdio
//! args = []                    # optional; default empty
//! log_file = '<path>'          # optional
//! [servers.<name>.env]         # optional; literal KEY = 'value' (NO ${VAR})
//!
//! [namespaces.<name>]
//! servers = ["<name>"]         # the servers visible in this namespace
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::StartupError;

/// The selected namespace name when `--namespace` is omitted.
///
/// Phase 1: a config that declares `[namespaces.default]` works with the flag
/// omitted. This is the only namespace-name default Phase 1 synthesizes.
pub const DEFAULT_NAMESPACE: &str = "default";

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
}

impl CliConfig {
    /// Build a [`CliConfig`] from the parsed global flags.
    pub fn from_flags(namespace: Option<String>, config_path: Option<PathBuf>) -> Self {
        Self {
            namespace: namespace.unwrap_or_default(),
            config_path,
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

/// A single stdio upstream server (`[servers.<name>]`).
///
/// Fields are read by later phases (registry/process forward path); Phase 1
/// only validates `command` presence and server names.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Transport kind. Optional in Phase 1; defaults to `"stdio"`. Any other
    /// value is out of scope for Phase 1 and is accepted-but-ignored here
    /// (HTTP/remote transports are later phases); the field exists so a
    /// config that omits it deserializes cleanly.
    #[serde(default)]
    pub transport: Option<String>,
    /// The spawn command. Required for stdio servers; a missing `command`
    /// fails startup (see [`TomlConfig::validate`]).
    pub command: Option<String>,
    /// Spawn args. Optional; defaults to an empty vector.
    #[serde(default)]
    pub args: Vec<String>,
    /// Literal env vars (`[servers.<name>.env]`). Optional. Phase 1 does NOT
    /// resolve `${VAR}` placeholders — values are stored verbatim for the
    /// later forward path. No secret ever enters this map in Phase 1.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional per-server log sink for child stderr + upstream log
    /// notifications (Phase 2+ asserts against it).
    pub log_file: Option<String>,
}

/// A single namespace (`[namespaces.<name>]`).
///
/// The `servers` field is read by later phases (namespace ACL / discovery);
/// Phase 1 only checks namespace existence.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceConfig {
    /// The servers visible in this namespace. Phase 1 reads only this field;
    /// the `tools.<server>` filter (ARCHITECTURE) is Phase 2 and ignored.
    #[serde(default)]
    pub servers: Vec<String>,
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
pub fn load_and_validate(
    config_path: &Path,
    namespace: &str,
) -> Result<TomlConfig, StartupError> {
    let contents = std::fs::read_to_string(config_path).map_err(StartupError::ReadConfig)?;
    let config: TomlConfig =
        toml::from_str(&contents).map_err(StartupError::ParseConfig)?;
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

        // 2. stdio servers must declare a `command`. A missing command fails
        //    startup (a server table without the spawn entry is malformed).
        for (name, server) in &self.servers {
            if server.command.as_ref().map(|c| c.trim()).is_none()
                || server
                    .command
                    .as_ref()
                    .map(|c| c.trim().is_empty())
                    .unwrap_or(true)
            {
                return Err(StartupError::StdioServerMissingCommand {
                    server: name.clone(),
                });
            }
        }

        // 3. Resolve the active namespace. Empty `--namespace` selects the
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

        Ok(())
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
        let bad: Vec<char> = name
            .chars()
            .filter(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
            .collect();
        return Err(StartupError::InvalidServerName {
            server: name.to_string(),
            reason: format!(
                "server name must match [a-z0-9-]+; found disallowed character(s): {}",
                bad.iter().collect::<String>()
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