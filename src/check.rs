//! CLI preflight for configured upstream capabilities.
//!
//! Unlike `serve`, this command never starts the downstream stdio transport, so
//! `--json` may write its bounded result to stdout. It owns the registry for the
//! whole preflight and drops it before emitting a result, which releases every
//! upstream containment guard before the process returns.

use std::collections::BTreeSet;
use std::io::Write;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

use crate::config::{CliConfig, DEFAULT_NAMESPACE};
use crate::error::ToolError;
use crate::namespace::ActiveNamespace;
use crate::registry::Registry;

/// Runs the eager namespace preflight without entering the MCP stdio server.
pub async fn run(config: CliConfig, json: bool, server_filter: Option<String>) -> ExitCode {
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
                let tool_names: BTreeSet<String> = inventory
                    .into_iter()
                    .map(|tool| tool.name.to_string())
                    .collect();
                let missing_tools = explicit_tools(&registry, namespace.name(), &server)
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
    // `Registry` owns every `UpstreamEntry`. Dropping it releases each entry's
    // containment guard (Unix killpg; Windows job/transport teardown) before
    // stdout is written and before this command returns.
    drop(registry);
    emit(result, json)
}

fn selected_namespace(namespace: &str) -> String {
    if namespace.is_empty() {
        DEFAULT_NAMESPACE.to_string()
    } else {
        namespace.to_string()
    }
}

fn explicit_tools(registry: &Registry, namespace: &str, server: &str) -> Vec<String> {
    registry
        .toml_config()
        .namespaces
        .get(namespace)
        .and_then(|config| config.tools.get(server))
        .map(|tools| {
            tools
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
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
}

impl CheckResult {
    fn new(namespace: String) -> Self {
        Self {
            namespace,
            ok: true,
            servers: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn failure(namespace: String, error: CheckError) -> Self {
        Self {
            namespace,
            ok: false,
            servers: Vec::new(),
            errors: vec![error],
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
