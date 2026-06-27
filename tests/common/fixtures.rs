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
use std::sync::atomic::{AtomicU64, Ordering};

use tempfile::NamedTempFile;

/// Per-call counter appended to the log-file path so two concurrent tests in
/// the same integration binary (which share one process id) never collide on
/// the same log file. Without this, every test passing a `log_file` writes to
/// the same path and clobbers each other under cargo's default parallel
/// execution — `registry::downstream_tools_list_does_not_spawn_upstream` goes
/// flaky seeing `[probe]` lines written by sibling tests.
static LOG_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Resolve the probe-server binary path from the cargo-injected env var.
///
/// Mirrors `common::spawn_bin("probe-server")` but returns the path for
/// embedding in a config file's `command` field rather than spawning it.
/// The env var name uses the bin name EXACTLY as-declared in `[[bin]]`
/// (dashes and case preserved) — cargo does NOT uppercase or transform it.
/// See the Phase 0 `spawn_bin` comment: uppercasing breaks resolution on
/// every platform.
pub fn probe_bin_path() -> String {
    env!("CARGO_BIN_EXE_probe-server").to_string()
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

/// Write raw TOML to a temp config file. Use only for Phase 5 transport shapes
/// that the older typed builders cannot express yet (Streamable-HTTP).
pub fn raw_config_file(toml: &str) -> ConfigFile {
    let mut tmp = NamedTempFile::new().expect("failed to create raw config temp file");
    tmp.write_all(toml.as_bytes())
        .expect("failed to write raw config TOML");
    tmp.as_file().sync_all().ok();
    let path = tmp.path().to_path_buf();
    ConfigFile { _file: tmp, path }
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
    cwd: Option<String>,
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
            cwd: None,
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
    pub fn namespace_servers(
        mut self,
        servers: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.namespace_servers = servers.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Set a log file path the aggregator should write diagnostics to.
    /// Phase 1 stderr-capture / log-notification routing asserts against this.
    pub fn log_file(mut self, path: impl Into<String>) -> Self {
        self.log_file = Some(path.into());
        self
    }

    /// Set the per-server working directory.
    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
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
        s.push_str(&format!("command = '{}'\n", escape_literal(&self.command)));
        if self.args.is_empty() {
            s.push_str("args = []\n");
        } else {
            let quoted: Vec<String> = self
                .args
                .iter()
                .map(|a| format!("'{}'", escape_literal(a)))
                .collect();
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
        if let Some(cwd) = &self.cwd {
            s.push_str(&format!("cwd = '{}'\n", escape_literal(cwd)));
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
        let mut tmp =
            NamedTempFile::new().expect("failed to create temp config file in OS tmp dir");
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

// ---- Phase 2 multi-upstream + namespace-ACL fixtures -----------------------

/// A server entry for the Phase 2 multi-server config builder.
///
/// Each server is the same `probe-server` binary registered under a distinct
/// configured name (D-016 / plan §Probe fixture decision). No second fixture
/// identity is introduced — the configured name is what the aggregator routes
/// on, so registering the probe under `alpha`, `beta`, `gamma` simulates N
/// distinct upstreams from the proxy's perspective.
#[allow(dead_code)] // pub fixture API: not every test uses every field.
#[derive(Debug, Clone)]
pub struct ServerEntry {
    /// The `[servers.<name>]` key — the configured server name the aggregator
    /// routes on. Must match `[a-z0-9-]+` (GOTCHA #15).
    pub name: String,
    /// Optional per-server log sink. When `None`, no `log_file` is written.
    pub log_file: Option<String>,
    /// Optional per-server working directory.
    pub cwd: Option<String>,
}

impl ServerEntry {
    /// A server entry with no log file.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            log_file: None,
            cwd: None,
        }
    }

    /// Attach a per-server log sink.
    pub fn with_log_file(mut self, log_file: impl Into<String>) -> Self {
        self.log_file = Some(log_file.into());
        self
    }

    /// Attach a per-server working directory.
    #[allow(dead_code)] // fixture API used by remediation builders as needed.
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

/// A namespace entry for the Phase 2 namespace-ACL config builder.
///
/// Encodes the resolved Open Question #1 schema: a `servers` allow-list plus
/// an optional per-server `tools.<server> = [...]` name-level allow-list. A
/// server present in `servers` with NO `tools` entry exposes ALL its tools;
/// a server with a `tools` entry exposes exactly those tool names
/// (D-006, GOTCHA #31, plan §Phase 2).
#[allow(dead_code)] // pub fixture API: not every test uses every field.
#[derive(Debug, Clone)]
pub struct NamespaceEntry {
    /// The `[namespaces.<name>]` key.
    pub name: String,
    /// The server allow-list (`servers = [...]`).
    pub servers: Vec<String>,
    /// Per-server tool allow-lists. `tools.<server> = ["tool", ...]`. A server
    /// in `servers` with no entry here exposes all its tools; a server with an
    /// entry exposes exactly the listed tool names. Name-level only — no
    /// parameter-level ACL (D-006).
    pub tools: Vec<(String, Vec<String>)>,
}

impl NamespaceEntry {
    /// A namespace with a server allow-list and no tool filters (all tools on
    /// each allowed server are visible).
    pub fn new(
        name: impl Into<String>,
        servers: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            servers: servers.into_iter().map(|s| s.into()).collect(),
            tools: Vec::new(),
        }
    }

    /// Add a per-server tool allow-list for `server`. The server must also be
    /// in `servers` for the filter to apply (a filter on a denied server is
    /// moot). Present list => exact name-level allow-list; the server is
    /// already allowed by `servers`.
    pub fn with_tools(
        mut self,
        server: impl Into<String>,
        tools: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.tools
            .push((server.into(), tools.into_iter().map(|t| t.into()).collect()));
        self
    }
}

/// A builder for a Phase 2 multi-upstream + namespace-ACL TOML config.
///
/// Distinct from the Phase 1 [`ConfigBuilder`] (which encodes the single-
/// upstream Phase 1 schema): this builder supports N named servers (each the
/// same probe binary under a distinct configured name) and N namespaces, each
/// with a `servers` allow-list and optional per-server `tools.<server>` name-
/// level allow-lists (the resolved Open Question #1 shape).
///
/// The rendered TOML is the binding Phase 2 config schema the implementer must
/// parse — see `tests.md` §Config schema (Phase 2 extension). Phase 1 fields
/// not exercised by Phase 2 (env, args) are omitted; the implementer's Phase 1
/// parser must still accept this config because it is a strict superset of the
/// Phase 1 shape.
#[allow(dead_code)] // pub fixture API: not every test uses every field.
#[derive(Debug, Clone, Default)]
pub struct MultiConfigBuilder {
    servers: Vec<ServerEntry>,
    namespaces: Vec<NamespaceEntry>,
}

#[allow(dead_code)] // pub fixture API: not every test uses every method.
impl MultiConfigBuilder {
    /// Start a fresh empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a server entry.
    pub fn server(mut self, entry: ServerEntry) -> Self {
        self.servers.push(entry);
        self
    }

    /// Add a namespace entry.
    pub fn namespace(mut self, entry: NamespaceEntry) -> Self {
        self.namespaces.push(entry);
        self
    }

    /// Render the TOML to a string.
    pub fn to_toml(&self) -> String {
        let mut s = String::new();
        let probe = probe_bin_path();

        for entry in &self.servers {
            s.push_str(&format!("[servers.{}]\n", entry.name));
            s.push_str("transport = \"stdio\"\n");
            s.push_str(&format!("command = '{}'\n", escape_literal(&probe)));
            s.push_str("args = []\n");
            if let Some(log) = &entry.log_file {
                s.push_str(&format!("log_file = '{}'\n", escape_literal(log)));
            }
            if let Some(cwd) = &entry.cwd {
                s.push_str(&format!("cwd = '{}'\n", escape_literal(cwd)));
            }
            s.push('\n');
        }

        for ns in &self.namespaces {
            s.push_str(&format!("[namespaces.{}]\n", ns.name));
            let quoted: Vec<String> = ns.servers.iter().map(|n| format!("\"{n}\"")).collect();
            s.push_str(&format!("servers = [{}]\n", quoted.join(", ")));
            // Per-server tool allow-lists. The resolved Open Question #1
            // shape is a single `[namespaces.<name>.tools]` sub-table with
            // each allowed server as a key mapping to an array of tool names:
            //   [namespaces.<name>.tools]
            //   alpha = ["echo_ok", "list_things"]
            // A server in `servers` with NO entry here exposes all its tools;
            // a server with an entry exposes exactly the listed tool names.
            if !ns.tools.is_empty() {
                s.push_str(&format!("[namespaces.{}.tools]\n", ns.name));
                for (server, tools) in &ns.tools {
                    let quoted_tools: Vec<String> =
                        tools.iter().map(|t| format!("\"{t}\"")).collect();
                    s.push_str(&format!("{} = [{}]\n", server, quoted_tools.join(", ")));
                }
            }
            s.push('\n');
        }

        s
    }

    /// Write the config to a temp file and return the path. The temp file
    /// stays alive for the lifetime of the returned [`ConfigFile`].
    pub fn write(self) -> ConfigFile {
        let toml = self.to_toml();
        let mut tmp =
            NamedTempFile::new().expect("failed to create temp config file in OS tmp dir");
        tmp.write_all(toml.as_bytes())
            .expect("failed to write Phase 2 config to temp file");
        tmp.as_file().sync_all().ok();
        let path = tmp.path().to_path_buf();
        ConfigFile { _file: tmp, path }
    }
}

/// Create an empty log file at a unique temp path and return its absolute
/// path. The aggregator writes `[server]`-prefixed lines here; the test reads
/// it back to assert on stderr capture / log notification routing.
///
/// Returns the path; the file is NOT held open by this helper (the aggregator
/// opens it for appending). On Windows a file can be opened by one writer
/// while another reads, so this is safe.
pub fn empty_log_file_path() -> String {
    // Two concurrent tests in the same integration binary share the same
    // `std::process::id()`, so the pid alone is not unique. Append a
    // monotonically increasing per-call counter so every call returns a
    // distinct path — no clobbering under parallel execution.
    let seq = LOG_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "fanin-mcp-phase1-log-{}-{}.log",
        std::process::id(),
        seq
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

// ---- Phase 3 credentials + timeout + lifetime fixtures ---------------------

/// A unique-marker counter for sentinel secret values and grandchild marker
/// paths. Keeps parallel Phase 3 tests from colliding on the same env var
/// name or marker file under cargo's default parallel execution.
static PHASE3_MARKER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A per-call unique integer used to derive unique env-var names, marker
/// paths, and sentinel values so concurrent Phase 3 tests do not collide.
pub fn phase3_unique_seq() -> u64 {
    PHASE3_MARKER_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// A Phase 3 server entry — extends the Phase 2 [`ServerEntry`] with the
/// `timeout_secs`, `env`, and `env_keys` fields Phase 3 exercises.
///
/// `timeout_secs` is the per-server upstream call timeout (SC12/13).
/// `env` is a list of `(key, value)` pairs rendered as a
/// `[servers.<name>.env]` sub-table; values may carry `${VAR}` placeholders
/// for the interpolation proof (SC8) or be literal non-secrets (SC10).
/// `env_keys` is an explicit allow-list of env var names the server should
/// receive — when non-empty, the rendered config uses the
/// `[servers.<name>.env_keys]` array form so the implementer can inject ONLY
/// those keys from the resolved credential/env store (SC9 isolation). When
/// empty, the rendered config uses the literal `env` map form.
#[allow(dead_code)] // pub fixture API: not every test uses every field.
#[derive(Debug, Clone)]
pub struct Phase3ServerEntry {
    /// The `[servers.<name>]` key.
    pub name: String,
    /// Per-server call timeout in seconds. `None` => omit (default 60).
    pub timeout_secs: Option<u64>,
    /// Literal env vars rendered as `[servers.<name>.env]`. Values may carry
    /// `${VAR}` placeholders.
    pub env: Vec<(String, String)>,
    /// Optional per-server log sink.
    pub log_file: Option<String>,
    /// Optional per-server working directory.
    pub cwd: Option<String>,
}

impl Phase3ServerEntry {
    /// A server entry with no env, no timeout, no log file.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            timeout_secs: None,
            env: Vec::new(),
            log_file: None,
            cwd: None,
        }
    }

    /// Set the per-server `timeout_secs`.
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Attach a per-server log sink.
    pub fn with_log_file(mut self, log_file: impl Into<String>) -> Self {
        self.log_file = Some(log_file.into());
        self
    }

    /// Set the per-server working directory.
    #[allow(dead_code)] // fixture API used by future cwd cases as needed.
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Add a literal env var (key, value). Values may carry `${VAR}`.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// A builder for a Phase 3 TOML config. Extends the Phase 2
/// [`MultiConfigBuilder`] surface with `timeout_secs` and `env` per server.
/// Namespaces reuse the Phase 2 [`NamespaceEntry`].
///
/// The rendered TOML is a strict superset of the Phase 2 shape; the
/// implementer's Phase 2 parser must already accept it, and Phase 3 adds
/// parsing of `timeout_secs` and the interpolation-aware `env` values.
#[allow(dead_code)] // pub fixture API: not every test uses every field.
#[derive(Debug, Clone, Default)]
pub struct Phase3ConfigBuilder {
    servers: Vec<Phase3ServerEntry>,
    namespaces: Vec<NamespaceEntry>,
}

#[allow(dead_code)] // pub fixture API: not every test uses every method.
impl Phase3ConfigBuilder {
    /// Start a fresh empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a Phase 3 server entry.
    pub fn server(mut self, entry: Phase3ServerEntry) -> Self {
        self.servers.push(entry);
        self
    }

    /// Add a namespace entry (Phase 2 shape, reused).
    pub fn namespace(mut self, entry: NamespaceEntry) -> Self {
        self.namespaces.push(entry);
        self
    }

    /// Render the TOML to a string.
    pub fn to_toml(&self) -> String {
        let mut s = String::new();
        let probe = probe_bin_path();

        for entry in &self.servers {
            s.push_str(&format!("[servers.{}]\n", entry.name));
            s.push_str("transport = \"stdio\"\n");
            s.push_str(&format!("command = '{}'\n", escape_literal(&probe)));
            s.push_str("args = []\n");
            if let Some(secs) = entry.timeout_secs {
                s.push_str(&format!("timeout_secs = {secs}\n"));
            }
            if !entry.env.is_empty() {
                s.push_str(&format!("[servers.{}.env]\n", entry.name));
                for (k, v) in &entry.env {
                    // Basic-string escape: backslashes and double quotes.
                    let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
                    s.push_str(&format!("{k} = \"{escaped}\"\n"));
                }
            }
            if let Some(log) = &entry.log_file {
                s.push_str(&format!("log_file = '{}'\n", escape_literal(log)));
            }
            if let Some(cwd) = &entry.cwd {
                s.push_str(&format!("cwd = '{}'\n", escape_literal(cwd)));
            }
            s.push('\n');
        }

        for ns in &self.namespaces {
            s.push_str(&format!("[namespaces.{}]\n", ns.name));
            let quoted: Vec<String> = ns.servers.iter().map(|n| format!("\"{n}\"")).collect();
            s.push_str(&format!("servers = [{}]\n", quoted.join(", ")));
            if !ns.tools.is_empty() {
                s.push_str(&format!("[namespaces.{}.tools]\n", ns.name));
                for (server, tools) in &ns.tools {
                    let quoted_tools: Vec<String> =
                        tools.iter().map(|t| format!("\"{t}\"")).collect();
                    s.push_str(&format!("{} = [{}]\n", server, quoted_tools.join(", ")));
                }
            }
            s.push('\n');
        }

        s
    }

    /// Write the config to a temp file and return the path. The temp file
    /// stays alive for the lifetime of the returned [`ConfigFile`].
    pub fn write(self) -> ConfigFile {
        let toml = self.to_toml();
        let mut tmp =
            NamedTempFile::new().expect("failed to create temp config file in OS tmp dir");
        tmp.write_all(toml.as_bytes())
            .expect("failed to write Phase 3 config to temp file");
        tmp.as_file().sync_all().ok();
        let path = tmp.path().to_path_buf();
        ConfigFile { _file: tmp, path }
    }
}

/// A unique marker file path for the Phase 3 hard-kill orphan proof. Each
/// call returns a distinct path under the OS temp dir; the path is created
/// (truncated) so a stale file from a previous run does not pollute the
/// assertion. The grandchild process writes its PID here; a contained tree
/// removes it (kill before lifetime), an uncontained tree leaves it.
pub fn grandchild_marker_path() -> String {
    let seq = phase3_unique_seq();
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "fanin-mcp-phase3-grandchild-{}-{}.marker",
        std::process::id(),
        seq
    ));
    // Remove any stale marker so the test starts from a known-clean state.
    let _ = std::fs::remove_file(&dir);
    dir.to_string_lossy().into_owned()
}

/// A unique env-var name for the Phase 3 per-upstream isolation proof. Each
/// call returns a distinct name so concurrent tests do not collide on the
/// same env var.
pub fn phase3_env_var_name(prefix: &str) -> String {
    let seq = phase3_unique_seq();
    format!("FANIN_TEST_{prefix}_{seq}")
}

/// A unique sentinel secret value for the Phase 3 redaction proof. The
/// sentinel is distinctive enough that a substring search of any log sink
/// is unambiguous, and unique per call so parallel tests do not confuse
/// each other's sentinel.
pub fn phase3_sentinel_value() -> String {
    let seq = phase3_unique_seq();
    format!("SENTINEL-SECRET-DO-NOT-LEAK-{seq}-ZXCV")
}
