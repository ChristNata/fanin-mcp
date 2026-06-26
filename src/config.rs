//! Configuration — the server config file and CLI flag plumbing.
//!
//! GOTCHA #1: nothing here may print to stdout — config diagnostics go to
//! stderr via `tracing`.

use std::path::PathBuf;

/// The resolved CLI configuration for a `serve` invocation.
///
/// Carries the global flags verbatim; parsing and validation are later work.
/// The `dead_code` allow is scoped here and removed when a phase starts reading
/// the fields.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
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
