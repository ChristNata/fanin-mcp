//! Phase 1 config-fixture helpers.
//!
//! Builds the TOML config files the Phase 1 tests spawn `fanin-mcp` against.
//! The exact TOML schema encoded here is the binding contract the implementer
//! must parse — see `tests.md` §Config schema. If ARCHITECTURE.md left a
//! detail open, the simplest reasonable shape was chosen and recorded there.
//!
//! All config files are written to the OS temp dir; the caller owns the
//! [`NamedTempFile`] (keep it alive for the duration of the spawned child so
//! the path stays valid on Windows, where the file is opened by the child).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use tempfile::NamedTempFile;

/// Resolve the probe-server binary path from the cargo-injected env var.
///
/// Mirrors `common::spawn_bin("probe-server")` but returns the path for
/// embedding in a config file's `command` field rather than spawning it.
/// The env var name uses the bin name EXACTLY as-declared in `[[bin]]`
/// (dashes and case preserved) — cargo does NOT uppercase or transform it.
/// See the Phase 0 `spawn_bin` comment: uppercasing breaks resolution on
/// every platform.
pub fn probe_bin_path() -> String {
    std::env::var("CARGO_BIN_EXE_probe-server").unwrap_or_else(|_| {
        panic!(
            "cargo did not inject CARGO_BIN_EXE_probe-server; the Phase 1 \
             config fixtures rely on the `probe-server` [[bin]] target being \
             built before the test binary runs"
        )
    })
}

/// A written config file plus its on-disk path. Keep the [`NamedTempFile`]
/// alive for the lifetime of the child that reads it.
pub struct ConfigFile {
    /// The temp-file handle. Held (not dropped) so the path stays valid on
    /// Windows while the aggregator child opens it.
    _file: NamedTempFile,
    /// The absolute path to pass to `--config`.
    pub path: PathBuf,
}

impl ConfigFile {
    /// The absolute path as a string suitable for `--config`.
    pub fn path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// A builder for a Phase 1 TOML config.
///
/// Defaults to one stdio upstream named `probe` pointing at the in-repo
/// probe-server binary, and a `default` namespace containing `["probe"]`.
/// The builder methods let a test mutate the minimal set of fields Phase 1
/// exercises: server name, command, args, namespace name, namespace servers,
/// and an optional log file.
#[allow(dead_code)] // pub fixture API: not every test exercises every field.
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    server_name: String,
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    namespace_name: String,
    namespace_servers: Vec<String>,
    log_file: Option<String>,
    /// Extra raw TOML lines appended verbatim (for negative-schema tests:
    /// e.g. adding a second server the plan does not forbid but Phase 1 does
    /// not need). Use sparingly — prefer typed fields.
    extra: Vec<String>,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            server_name: "probe".to_string(),
            command: probe_bin_path(),
            args: Vec::new(),
            env: Vec::new(),
            namespace_name: "default".to_string(),
            namespace_servers: vec!["probe".to_string()],
            log_file: None,
            extra: Vec::new(),
        }
    }
}

#[allow(dead_code)] // pub fixture API: not every test exercises every method/field.
impl ConfigBuilder {
    /// Start a fresh builder with the canonical Phase 1 defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the server name (the `[servers.<name>]` key).
    pub fn server_name(mut self, name: impl Into<String>) -> Self {
        self.server_name = name.into();
        self
    }

    /// Override the spawn command.
    pub fn command(mut self, cmd: impl Into<String>) -> Self {
        self.command = cmd.into();
        self
    }

    /// Override the spawn args.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Add a literal env var (key, value). Phase 1 does NOT resolve `${VAR}`;
    /// these are passed verbatim. Used to prove child stderr / env isolation
    /// is observable without touching the credential path (out of scope).
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Override the namespace name (the `[namespaces.<name>]` key).
    pub fn namespace_name(mut self, name: impl Into<String>) -> Self {
        self.namespace_name = name.into();
        self
    }

    /// Override the namespace's server list.
    pub fn namespace_servers(mut self, servers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.namespace_servers = servers.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Set a log file path the aggregator should write diagnostics to.
    /// Phase 1 stderr-capture / log-notification routing asserts against this.
    pub fn log_file(mut self, path: impl Into<String>) -> Self {
        self.log_file = Some(path.into());
        self
    }

    /// Append raw TOML lines verbatim. For negative tests only.
    pub fn extra_raw(mut self, line: impl Into<String>) -> Self {
        self.extra.push(line.into());
        self
    }

    /// Render the TOML to a string.
    pub fn to_toml(&self) -> String {
        let mut s = String::new();

        // [servers.<name>]
        s.push_str(&format!("[servers.{}]\n", self.server_name));
        s.push_str("transport = \"stdio\"\n");
        // command is a path; quote it. TOML basic strings escape backslashes
        // on Windows, so use a literal string for safety.
        s.push_str(&format!(
            "command = '{}'\n",
            escape_literal(&self.command)
        ));
        if self.args.is_empty() {
            s.push_str("args = []\n");
        } else {
            let quoted: Vec<String> =
                self.args.iter().map(|a| format!("'{}'", escape_literal(a))).collect();
            s.push_str(&format!("args = [{}]\n", quoted.join(", ")));
        }
        if !self.env.is_empty() {
            let pairs: Vec<String> = self
                .env
                .iter()
                .map(|(k, v)| format!("{k} = '{}'", escape_literal(v)))
                .collect();
            s.push_str(&format!("[servers.{}.env]\n", self.server_name));
            s.push_str(&format!("{}\n", pairs.join("\n")));
        }
        if let Some(log) = &self.log_file {
            s.push_str(&format!("log_file = '{}'\n", escape_literal(log)));
        }
        s.push('\n');

        // [namespaces.<name>]
        s.push_str(&format!("[namespaces.{}]\n", self.namespace_name));
        let quoted: Vec<String> = self
            .namespace_servers
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect();
        s.push_str(&format!("servers = [{}]\n", quoted.join(", ")));

        for line in &self.extra {
            s.push_str(line);
            s.push('\n');
        }

        s
    }

    /// Write the config to a temp file and return the path. The temp file
    /// stays alive for the lifetime of the returned [`ConfigFile`].
    pub fn write(self) -> ConfigFile {
        let toml = self.to_toml();
        let mut tmp = NamedTempFile::new()
            .expect("failed to create temp config file in OS tmp dir");
        tmp.write_all(toml.as_bytes())
            .expect("failed to write Phase 1 config to temp file");
        // Flush + keep the handle; the path is valid while the handle lives.
        tmp.as_file().sync_all().ok();
        let path = tmp.path().to_path_buf();
        ConfigFile { _file: tmp, path }
    }
}

/// Escape a string for a TOML literal string (single-quoted). Literal strings
/// forbid only the single quote and the NUL byte; backslashes pass through
/// verbatim (critical for Windows paths).
fn escape_literal(s: &str) -> String {
    s.replace('\u{0000}', "").replace('\'', "\\'")
}

/// Create an empty log file at a unique temp path and return its absolute
/// path. The aggregator writes `[server]`-prefixed lines here; the test reads
/// it back to assert on stderr capture / log notification routing.
///
/// Returns the path; the file is NOT held open by this helper (the aggregator
/// opens it for appending). On Windows a file can be opened by one writer
/// while another reads, so this is safe.
pub fn empty_log_file_path() -> String {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "fanin-mcp-phase1-log-{}.log",
        std::process::id()
    ));
    // Create / truncate so a stale file from a previous run does not pollute
    // the assertions.
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&dir)
        .unwrap_or_else(|e| panic!("failed to create log file at {}: {e}", dir.display()));
    dir.to_string_lossy().into_owned()
}