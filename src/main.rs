//! fanin-mcp — the stdio-native MCP proxy entry point.
//!
//! GOTCHA #1: stdout is the MCP transport once `serve(stdio())` runs. No
//! `println!` / `print!` / `dbg!` exists in this crate. Tracing is initialized
//! to a **stderr** writer before serving starts so diagnostics never corrupt
//! the JSON-RPC stream.

mod config;
mod credentials;
mod error;
mod forward;
mod namespace;
mod process;
mod registry;
mod server;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use rmcp::ServiceExt;

use crate::config::CliConfig;
use crate::namespace::ActiveNamespace;
use crate::registry::Registry;
use crate::server::Aggregator;

/// The top-level CLI.
#[derive(Debug, Parser)]
#[command(
    name = "fanin-mcp",
    version,
    about = "A standalone, stdio-native MCP proxy that federates many upstream MCP servers behind a single endpoint.",
    long_about = None,
)]
struct Cli {
    /// The selected namespace for this session.
    #[arg(long, global = true)]
    namespace: Option<String>,

    /// Path to the server config file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// The subcommand surface.
#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the aggregator over stdio (default when no subcommand is given).
    Serve,

    /// Credential management stub. Never touch keyring or secrets here yet
    /// (D-010, GOTCHA #18/#19/#22).
    Cred {
        #[command(subcommand)]
        action: CredAction,
    },
}

/// `cred` subcommand surface (stubs).
#[derive(Debug, Subcommand)]
enum CredAction {
    /// Store a secret for an upstream. Stub only.
    Set,
    /// List stored credential names only. Stub only.
    List,
    /// Remove a stored secret. Stub only.
    Rm,
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let cli = Cli::parse();
    let config = CliConfig::from_flags(cli.namespace.clone(), cli.config.clone());

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => run_serve(config).await,
        Command::Cred { action } => run_cred(action),
    }
}

/// Run the stdio MCP server.
///
/// Phase 1: load + validate the TOML config (when `--config` is given) BEFORE
/// constructing or serving the aggregator. A validation failure is logged to
/// stderr via `tracing` and returns `ExitCode::FAILURE` — it never reaches
/// `serve(stdio())`, so no bytes are written to stdout (GOTCHA #1).
///
/// When `--config` is omitted, Phase 0 behavior is preserved: the aggregator
/// serves the three static meta-tools with no upstream config. This keeps the
/// Phase 0 aggregator tests (which spawn the binary with no flags) green.
///
/// All diagnostics go to stderr via `tracing`.
async fn run_serve(config: CliConfig) -> ExitCode {
    // Load + validate the config BEFORE serving. A failure here must exit
    // before `serve(stdio())` begins so the JSON-RPC stream is never corrupted
    // (GOTCHA #1). The loaded config is not yet wired into the aggregator —
    // registry/forward/invoke are later sub-phases; Phase 1 config sub-phase
    // only needs startup validation to gate serving.
    let loaded = if let Some(path) = config.config_path.as_ref() {
        match crate::config::load_and_validate(path, &config.namespace) {
            Ok(loaded) => Some(loaded),
            Err(e) => {
                tracing::error!(error = %e, "startup config validation failed");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    let aggregator = if let Some(loaded) = loaded {
        let namespace = ActiveNamespace::new(&loaded, &config.namespace);
        tracing::debug!(namespace = namespace.name(), "active namespace selected");
        let registry = Arc::new(Registry::new(loaded));
        Aggregator::with_registry(config, registry, namespace)
    } else {
        Aggregator::new(config)
    };

    // `stdio()` returns `(tokio::io::Stdin, tokio::io::Stdout)`. Once the serve
    // future starts, stdout is the JSON-RPC transport — no further stdout
    // writes are permitted (GOTCHA #1). All diagnostics already route to
    // stderr via `tracing` (see `init_tracing`).
    let running = match aggregator.serve(rmcp::transport::stdio()).await {
        Ok(running) => running,
        Err(e) => {
            tracing::error!(error = %e, "failed to start stdio MCP server");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = running.waiting().await {
        tracing::error!(error = %e, "stdio MCP server task failed");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Run a `cred` subcommand stub.
///
/// Emits a not-implemented warning to **stderr** and fails fast.
fn run_cred(action: CredAction) -> ExitCode {
    let name = match action {
        CredAction::Set => "cred set",
        CredAction::List => "cred list",
        CredAction::Rm => "cred rm",
    };
    tracing::warn!(
        subcommand = name,
        "credential management is not implemented in this build of fanin-mcp; \
         keyring calls arrive in a later phase"
    );
    ExitCode::FAILURE
}

/// Initialize `tracing` with a stderr writer.
///
/// The subscriber is installed before any serve logic runs.
fn init_tracing() {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    fmt()
        .with_max_level(LevelFilter::INFO)
        .with_writer(std::io::stderr)
        .init();
}
