//! fanin-mcp — a standalone, stdio-native MCP proxy that federates many
//! upstream MCP servers behind a single endpoint.
//!
//! P0.2 ships the real CLI (`serve` default + `cred` stubs + global
//! `--namespace` / `--config`) and the stdio `serve(stdio())` loop. The
//! three static meta-tools and the not-implemented `call_tool` live in
//! [`server`].
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

use clap::{Parser, Subcommand};

use rmcp::ServiceExt;

use crate::config::CliConfig;
use crate::server::Aggregator;

/// The top-level CLI. `serve` is the default command path; `cred` is a
/// subcommand stub surface that later phases fill with keyring calls.
#[derive(Debug, Parser)]
#[command(
    name = "fanin-mcp",
    version,
    about = "A standalone, stdio-native MCP proxy that federates many upstream MCP servers behind a single endpoint.",
    long_about = None,
)]
struct Cli {
    /// The selected namespace for this session. Empty means "no namespace
    /// filter". Phase 0 carries the value; namespace ACL enforcement is a
    /// later phase (D-006: annotations are conservative, not a security
    /// boundary).
    #[arg(long, global = true)]
    namespace: Option<String>,

    /// Path to the server config file. Phase 0 carries the value; config
    /// parsing and validation arrive in Phase 1.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// The subcommand surface. `serve` is the default; `cred` is a stub.
#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the aggregator over stdio (default when no subcommand is given).
    Serve,

    /// Credential management — stubs in Phase 0. Never touch keyring or
    /// secrets here yet (D-010, GOTCHA #18/#19/#22).
    Cred {
        #[command(subcommand)]
        action: CredAction,
    },
}

/// `cred` subcommand surface (stubs).
#[derive(Debug, Subcommand)]
enum CredAction {
    /// Store a secret for an upstream. STUB in Phase 0 — does not read the
        /// keyring or prompt. Implemented in a later phase.
    Set,
    /// List stored credential names only (never values). STUB in Phase 0.
    List,
    /// Remove a stored secret. STUB in Phase 0.
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
/// Initializes the [`Aggregator`] with the CLI config and drives the rmcp
/// stdio transport. `serve(stdio())` hands the JSON-RPC stream; `.waiting()`
/// blocks until the client disconnects. Errors go to stderr via `tracing`.
async fn run_serve(config: CliConfig) -> ExitCode {
    let aggregator = Aggregator::new(config);

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
/// Phase 0 does not implement credential storage. The stubs print a
/// not-implemented notice to **stderr** (never stdout — GOTCHA #1) and return
/// a failure exit so a caller relying on `cred` fails fast rather than
/// silently no-op'ing.
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
/// Everything — the serve path, rmcp internals, diagnostics — writes to
/// stderr so the JSON-RPC stream on stdout stays clean (GOTCHA #1). The
/// subscriber is installed before any serve logic runs.
///
/// Note: `EnvFilter` is gated behind the `env-filter` feature of
/// `tracing-subscriber`, which the frozen `Cargo.toml` does not enable. We
/// use the static `LevelFilter` instead — good enough for Phase 0
/// diagnostics. A later phase can add the feature and switch to env-driven
/// filtering.
fn init_tracing() {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    fmt()
        .with_max_level(LevelFilter::INFO)
        .with_writer(std::io::stderr)
        .init();
}