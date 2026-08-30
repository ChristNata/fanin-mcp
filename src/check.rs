//! CLI preflight for configured upstream capabilities.
//!
//! Unlike `serve`, this command never starts the downstream stdio transport, so
//! `--json` may write its bounded result to stdout. It owns the registry for the
//! whole preflight and drops it before emitting a result, which releases every
//! upstream containment guard before the process returns.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{CliConfig, ServerConfig, TomlConfig, DEFAULT_NAMESPACE};
use crate::error::ToolError;
use crate::namespace::ActiveNamespace;
use crate::registry::Registry;

/// Runs the eager namespace preflight without entering the MCP stdio server.
pub async fn run(
    config: CliConfig,
    json: bool,
    server_filter: Option<String>,
    refresh_cache: bool,
    no_cache_write: bool,
) -> ExitCode {
    let namespace_name = selected_namespace(&config.namespace);
    let Some(config_path) = config.config_path.as_ref() else {
        return emit(
            CheckResult::failure(
                namespace_name,
                CheckError::new(
                    None,
                    "config_required",
                    None,
                    "--config is required for check",
                ),
            ),
            json,
        );
    };

    let loaded = match crate::config::load_and_validate(config_path, &config.namespace) {
        Ok(loaded) => loaded,
        Err(_) => {
            return emit(
                CheckResult::failure(
                    namespace_name,
                    CheckError::new(
                        None,
                        "config_validation_failed",
                        None,
                        "config could not be loaded and validated",
                    ),
                ),
                json,
            );
        }
    };

    let namespace = ActiveNamespace::new(&loaded, &config.namespace);
    let writes_full_namespace_cache = server_filter.is_none();
    let servers = match server_filter {
        Some(server) if namespace.is_server_allowed(&server) => vec![server],
        Some(server) => {
            return emit(
                CheckResult::failure(
                    namespace_name,
                    CheckError::new(
                        Some(server),
                        "server_not_allowed",
                        None,
                        "server is not visible in the selected namespace",
                    ),
                ),
                json,
            );
        }
        None => namespace.allowed_servers(),
    };

    let registry = Registry::new(loaded, config.credential_store, Arc::new(OnceLock::new()));
    let mut result = CheckResult::new(namespace.name().to_string());

    for server in servers {
        match registry.inventory(&server).await {
            Ok(inventory) => {
                let tools = inventory
                    .into_iter()
                    .map(|tool| CachedTool {
                        name: tool.name.to_string(),
                        description: tool
                            .description
                            .map(|description| description.to_string())
                            .unwrap_or_default(),
                    })
                    .collect::<Vec<_>>();
                let tool_names: BTreeSet<String> =
                    tools.iter().map(|tool| tool.name.clone()).collect();
                let missing_tools = explicit_tools(&namespace, &server)
                    .into_iter()
                    .filter(|tool| !tool_names.contains(tool))
                    .collect::<Vec<_>>();
                let status = if missing_tools.is_empty() {
                    "ok"
                } else {
                    "error"
                };

                result.servers.push(CheckServer {
                    name: server.clone(),
                    status,
                    tools: tool_names
                        .into_iter()
                        .map(|name| CheckTool { name })
                        .collect(),
                });
                result.cache_servers.push(CachedServer {
                    name: server.clone(),
                    description: registry
                        .toml_config()
                        .servers
                        .get(&server)
                        .and_then(|server_config| server_config.description.clone())
                        .unwrap_or_default(),
                    tools,
                });
                for tool in missing_tools {
                    result.errors.push(CheckError::new(
                        Some(server.clone()),
                        "configured_tool_missing",
                        Some(tool),
                        "allowlisted tool absent from live inventory",
                    ));
                }
            }
            Err(error) => {
                result.servers.push(CheckServer {
                    name: server.clone(),
                    status: "error",
                    tools: Vec::new(),
                });
                result.errors.push(map_tool_error(error));
            }
        }
    }

    result.ok = result.errors.is_empty();
    if result.ok && writes_full_namespace_cache && !no_cache_write {
        // Check always inventories live upstreams and never consumes a cache.
        // Therefore `--refresh-cache` has no stale-read path to bypass here;
        // it still reaches this successful write path and replaces any old body.
        let _refreshes_without_reading = refresh_cache;
        if let Err(error) = write_capability_cache(registry.toml_config(), &namespace, &result) {
            // The cache is advisory. A filesystem failure cannot turn a healthy
            // live preflight into a failed availability result.
            tracing::warn!(error = %error, "capability cache write failed");
        }
    }
    // `Registry` owns every `UpstreamEntry`. Dropping it releases each entry's
    // containment guard (Unix killpg; Windows job/transport teardown) before
    // stdout is written and before this command returns.
    drop(registry);
    emit(result, json)
}

/// Reads compact, matching cache summaries for the configured ToC.
///
/// A malformed, stale, unavailable, or mismatched cache is advisory-only and
/// returns no summaries. This function never contacts an upstream.
pub(crate) fn matching_cache_tool_summaries(
    config: &TomlConfig,
    namespace: &ActiveNamespace,
) -> BTreeMap<String, Vec<(String, String)>> {
    let Some(path) = capability_cache_path(namespace.name()) else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(cache) = serde_json::from_slice::<CapabilityCache>(&raw) else {
        return BTreeMap::new();
    };
    if cache.format_version != 1
        || cache.namespace != namespace.name()
        || cache.config_fingerprint != config_fingerprint(config, namespace)
    {
        return BTreeMap::new();
    }

    cache
        .servers
        .into_iter()
        .filter(|server| namespace.is_server_allowed(&server.name))
        .map(|server| {
            let tools = server
                .tools
                .into_iter()
                // The cache is display-only. Keep the live namespace ACL as the
                // sole authorization source even when the cache was modified.
                .filter(|tool| namespace.is_tool_allowed(&server.name, &tool.name))
                .map(|tool| (tool.name, tool.description))
                .collect();
            (server.name, tools)
        })
        .collect()
}

/// Returns the capability-cache path for a namespace.
fn capability_cache_path(namespace: &str) -> Option<PathBuf> {
    if !is_valid_cache_namespace_stem(namespace) {
        return None;
    }

    std::env::var_os("FANIN_MCP_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(platform_cache_dir)
        .map(|root| {
            root.join("fanin-mcp")
                .join("capabilities")
                .join(format!("{namespace}.json"))
        })
}

/// Returns the platform cache directory using standard environment variables.
#[cfg(target_os = "windows")]
fn platform_cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// Returns the platform cache directory using standard environment variables.
#[cfg(target_os = "macos")]
fn platform_cache_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
}

/// Returns the platform cache directory using standard environment variables.
#[cfg(all(unix, not(target_os = "macos")))]
fn platform_cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

/// Returns no cache directory for unsupported platforms.
#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn platform_cache_dir() -> Option<PathBuf> {
    None
}

/// Returns whether a namespace can safely become one cache filename component.
fn is_valid_cache_namespace_stem(namespace: &str) -> bool {
    let mut components = Path::new(namespace).components();
    matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(namespace)
    ) && components.next().is_none()
}

fn write_capability_cache(
    config: &TomlConfig,
    namespace: &ActiveNamespace,
    result: &CheckResult,
) -> Result<(), String> {
    let Some(path) = capability_cache_path(namespace.name()) else {
        return Ok(());
    };
    let parent = path
        .parent()
        .ok_or_else(|| "capability cache path has no parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let cache = CapabilityCache {
        format_version: 1,
        namespace: namespace.name().to_string(),
        config_fingerprint: config_fingerprint(config, namespace),
        generated_at: rfc3339_utc_now(),
        servers: result.cache_servers.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&cache).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

/// Builds a canonical, non-secret fingerprint of the active namespace.
///
/// Every map and list is sorted before serialization. The fingerprint carries
/// the resolved visible server list and its present tool filters, including an
/// explicitly present empty list; it deliberately omits env, headers, and
/// credential values.
pub(crate) fn config_fingerprint(config: &TomlConfig, namespace: &ActiveNamespace) -> Value {
    let allowed_servers = namespace.allowed_servers();
    let servers = allowed_servers
        .iter()
        .filter_map(|name| {
            config
                .servers
                .get(name)
                .map(|server| FingerprintServer::from_config(name, server))
        })
        .collect::<Vec<_>>();
    let tools = namespace
        .effective_tool_filters()
        .iter()
        .map(|(server, tools)| (server.clone(), tools.iter().cloned().collect()))
        .collect::<BTreeMap<_, _>>();
    serde_json::to_value(ConfigFingerprint {
        namespace: namespace.name(),
        allowed_servers,
        servers,
        tools,
    })
    .expect("capability fingerprint contains only serializable data")
}

fn rfc3339_utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_date_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60,
    )
}

// Howard Hinnant's public-domain civil-date conversion, with days relative to
// 1970-01-01. It avoids adding a time crate solely for a cache timestamp.
fn civil_date_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (
        year + if month <= 2 { 1 } else { 0 },
        month as u32,
        day as u32,
    )
}

fn selected_namespace(namespace: &str) -> String {
    if namespace.is_empty() {
        DEFAULT_NAMESPACE.to_string()
    } else {
        namespace.to_string()
    }
}

fn explicit_tools(namespace: &ActiveNamespace, server: &str) -> Vec<String> {
    namespace
        .effective_tool_filters()
        .get(server)
        .map(|tools| tools.iter().cloned().collect())
        .unwrap_or_default()
}

fn map_tool_error(error: ToolError) -> CheckError {
    match error {
        ToolError::CredentialResolution { server, key } => CheckError::new_with_key(
            Some(server),
            "credential_resolution_failed",
            None,
            "credential resolution failed",
            key,
        ),
        ToolError::UpstreamTimeout { server, tool } => CheckError::new(
            Some(server),
            "upstream_timeout",
            tool,
            "upstream inventory timed out",
        ),
        ToolError::NotImplemented { tool } => CheckError::new(
            None,
            "upstream_connect_failed",
            Some(tool),
            "upstream connection failed",
        ),
        ToolError::InvalidRequest { tool, .. } => CheckError::new(
            None,
            "upstream_connect_failed",
            Some(tool),
            "upstream connection failed",
        ),
        ToolError::UnknownServer { server } => CheckError::new(
            Some(server),
            "upstream_connect_failed",
            None,
            "upstream connection failed",
        ),
        ToolError::UnknownTool { server, tool } => CheckError::new(
            Some(server),
            "upstream_connect_failed",
            Some(tool),
            "upstream connection failed",
        ),
        ToolError::NamespaceDenied { server, tool } => CheckError::new(
            Some(server),
            "server_not_allowed",
            tool,
            "server is not visible in the selected namespace",
        ),
        ToolError::UpstreamConnect { server, .. } => CheckError::new(
            Some(server),
            "upstream_connect_failed",
            None,
            "upstream connection failed",
        ),
        ToolError::UpstreamCall { server, tool, .. } => CheckError::new(
            Some(server),
            "upstream_connect_failed",
            Some(tool),
            "upstream connection failed",
        ),
        ToolError::UpstreamDisconnected { server, tool } => CheckError::new(
            Some(server),
            "upstream_connect_failed",
            Some(tool),
            "upstream connection failed",
        ),
        ToolError::CallCancelled { server, tool } => CheckError::new(
            Some(server),
            "upstream_connect_failed",
            Some(tool),
            "upstream connection failed",
        ),
    }
}

fn emit(result: CheckResult, json: bool) -> ExitCode {
    let success = result.ok;
    if json {
        let body = match serde_json::to_string(&result) {
            Ok(body) => body,
            Err(_) => return ExitCode::FAILURE,
        };
        let mut stdout = std::io::stdout().lock();
        if stdout
            .write_all(body.as_bytes())
            .and_then(|()| stdout.write_all(b"\n"))
            .is_err()
        {
            return ExitCode::FAILURE;
        }
    } else {
        eprintln!("ok={success} namespace={}", result.namespace);
    }

    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[derive(Serialize)]
struct CheckResult {
    namespace: String,
    ok: bool,
    servers: Vec<CheckServer>,
    errors: Vec<CheckError>,
    #[serde(skip)]
    cache_servers: Vec<CachedServer>,
}

impl CheckResult {
    fn new(namespace: String) -> Self {
        Self {
            namespace,
            ok: true,
            servers: Vec::new(),
            errors: Vec::new(),
            cache_servers: Vec::new(),
        }
    }

    fn failure(namespace: String, error: CheckError) -> Self {
        Self {
            namespace,
            ok: false,
            servers: Vec::new(),
            errors: vec![error],
            cache_servers: Vec::new(),
        }
    }
}

#[derive(Serialize)]
struct CheckServer {
    name: String,
    status: &'static str,
    tools: Vec<CheckTool>,
}

#[derive(Serialize)]
struct CheckTool {
    name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CapabilityCache {
    format_version: u64,
    namespace: String,
    config_fingerprint: Value,
    generated_at: String,
    servers: Vec<CachedServer>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CachedServer {
    name: String,
    description: String,
    tools: Vec<CachedTool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CachedTool {
    name: String,
    description: String,
}

#[derive(Serialize)]
struct ConfigFingerprint<'a> {
    namespace: &'a str,
    allowed_servers: Vec<String>,
    servers: Vec<FingerprintServer>,
    tools: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize)]
struct FingerprintServer {
    name: String,
    transport: String,
    command: Option<String>,
    args: Vec<String>,
    endpoint: Option<String>,
    cwd_template: Option<String>,
    timeout_secs: u64,
    description: Option<String>,
}

impl FingerprintServer {
    fn from_config(name: &str, config: &ServerConfig) -> Self {
        Self {
            name: name.to_string(),
            transport: config.transport_kind().to_string(),
            command: config.command.clone(),
            args: config.args.clone(),
            endpoint: config.endpoint.clone(),
            cwd_template: config.cwd.clone(),
            timeout_secs: config.timeout_secs,
            description: config.description.clone(),
        }
    }
}

#[derive(Serialize)]
struct CheckError {
    server: Option<String>,
    code: &'static str,
    tool: Option<String>,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

impl CheckError {
    fn new(
        server: Option<String>,
        code: &'static str,
        tool: Option<String>,
        message: &'static str,
    ) -> Self {
        Self {
            server,
            code,
            tool,
            message,
            key: None,
        }
    }

    fn new_with_key(
        server: Option<String>,
        code: &'static str,
        tool: Option<String>,
        message: &'static str,
        key: String,
    ) -> Self {
        Self {
            server,
            code,
            tool,
            message,
            key: Some(key),
        }
    }
}
