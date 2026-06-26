//! Configuration — the server config file and CLI flag plumbing.
//!
//! P0.2: only the CLI flag values are carried. No file parsing, no validation,
//! no server-config loading (that is Phase 1). [`CliConfig`] is the minimal
//! struct the `serve` path receives so later phases can thread it through
//! without changing the CLI shape.
//!
//! GOTCHA #1: nothing here may print to stdout — config diagnostics go to
//! stderr via `tracing`.

use std::path::PathBuf;

/// The resolved CLI configuration for a `serve` invocation.
///
/// Phase 0 carries the global flags verbatim; it does not read the config
/// file or validate the namespace. [`CliConfig::namespace`] is the selected
/// session namespace (empty = all tools visible; enforcement is a later
/// phase). [`CliConfig::config_path`] is the `--config` path, carried but not
/// yet opened.
///
/// The fields are read by later phases (registry, namespace ACL, config
/// loader). Phase 0 only carries them so the CLI shape is fixed early; the
/// `dead_code` allow is scoped here and removed when a phase starts reading
/// them.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct CliConfig {
    /// The selected namespace for this session. Empty means "no namespace
    /// filter" — Phase 0 does not enforce the ACL (D-006); it only carries
    /// the value so the CLI shape is fixed early.
    pub namespace: String,
    /// The path to the server config file (`--config`). Phase 0 does not
    /// open it; Phase 1 parses and validates it.
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