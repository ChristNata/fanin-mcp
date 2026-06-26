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

    /// Preferred credential backend for `cred` commands and resolution.
    /// Env fallback is always available for reads regardless of this choice.
    #[arg(long, global = true, value_enum, default_value_t = crate::credentials::CredentialStoreChoice::Keyring)]
    credential_store: crate::credentials::CredentialStoreChoice,

    #[command(subcommand)]
    command: Option<Command>,
}

/// The subcommand surface.
#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the aggregator over stdio (default when no subcommand is given).
    Serve,

    /// Manage credentials with `cred set`, `cred list`, and `cred rm`.
    /// Secrets are read from a hidden prompt and never printed.
    Cred {
        #[command(subcommand)]
        action: CredAction,
    },
}

/// `cred` subcommand surface.
#[derive(Debug, Subcommand)]
enum CredAction {
    /// Store a secret for an upstream (reads value from hidden stdin prompt).
    Set {
        /// Server (service scope) name.
        server: String,
        /// Credential key name.
        key: String,
    },
    /// List stored credential *names* only (never values) for a server.
    List {
        /// Server (service scope) name.
        server: String,
    },
    /// Remove a stored secret.
    Rm {
        /// Server (service scope) name.
        server: String,
        /// Credential key name.
        key: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let cli = Cli::parse();
    let credential_store = cli.credential_store;
    let config = CliConfig::from_flags(cli.namespace, cli.config, credential_store);

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => run_serve(config).await,
        Command::Cred { action } => run_cred(action, credential_store),
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
    // (GOTCHA #1). On success the loaded config is wired into the aggregator
    // below: it builds the active namespace, the lazy upstream `Registry` (with
    // the selected credential store), and the meta-tool dispatch path.
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
        let registry = Arc::new(Registry::new(loaded, config.credential_store));
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

/// Run a `cred` subcommand.
///
/// All output (diagnostics or list results) goes to stderr via tracing.
/// Secrets are never echoed; `cred list` prints names only.
/// `cred set` reads the secret via hidden stdin prompt (rpassword).
fn run_cred(action: CredAction, choice: crate::credentials::CredentialStoreChoice) -> ExitCode {
    use crate::credentials::{build_store, prompt_for_secret};

    let store = build_store(choice);
    let backend = credential_backend_name(choice);

    match action {
        CredAction::Set { server, key } => {
            if choice == crate::credentials::CredentialStoreChoice::Env {
                tracing::error!(
                    backend,
                    server = %server,
                    key = %key,
                    "cred set rejected: env credential store is read-only; set the process environment variable or choose --credential-store keyring"
                );
                return ExitCode::FAILURE;
            }

            // Hidden prompt. Prompt text may go to terminal; secret itself must not.
            let secret = match prompt_for_secret(&format!("Enter secret for {server}/{key}: ")) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, server = %server, key = %key, "cred set failed to read secret");
                    return ExitCode::FAILURE;
                }
            };
            if secret.is_empty() {
                tracing::error!(server = %server, key = %key, "cred set: empty secret is not allowed");
                return ExitCode::FAILURE;
            }
            let set_result = store.set(&server, &key, &secret);
            match set_result {
                Ok(()) => {
                    tracing::info!(server = %server, key = %key, "credential stored");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        backend,
                        server = %server,
                        key = %key,
                        "cred set failed: selected credential store rejected the write; no secret was stored. Use a working keyring backend or set the process environment variable as fallback"
                    );
                    ExitCode::FAILURE
                }
            }
        }
        CredAction::List { server } => {
            let list_result = store.list_names(&server);
            match list_result {
                Ok(names) => {
                    for n in names {
                        // Print names to stderr (via tracing) so they are observable in tests
                        // without ever touching stdout (GOTCHA #1 discipline for non-serve paths).
                        eprintln!("{}", n);
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    tracing::error!(error = %e, server = %server, "cred list failed");
                    ExitCode::FAILURE
                }
            }
        }
        CredAction::Rm { server, key } => {
            if choice == crate::credentials::CredentialStoreChoice::Env {
                tracing::error!(
                    backend,
                    server = %server,
                    key = %key,
                    "cred rm rejected: env credential store is read-only; remove the process environment variable or choose --credential-store keyring"
                );
                ExitCode::FAILURE
            } else {
                match store.delete(&server, &key) {
                    Ok(()) => {
                        tracing::info!(server = %server, key = %key, "credential removed (if present)");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        tracing::error!(error = %e, backend, server = %server, key = %key, "cred rm failed");
                        ExitCode::FAILURE
                    }
                }
            }
        }
    }
}

fn credential_backend_name(choice: crate::credentials::CredentialStoreChoice) -> &'static str {
    match choice {
        crate::credentials::CredentialStoreChoice::Keyring => "keyring",
        crate::credentials::CredentialStoreChoice::Env => "env",
    }
}

/// Initialize `tracing` with a redacting stderr writer.
///
/// The subscriber is installed before any serve logic runs.
/// Phase 2 redaction is applied at tracing, child stderr log, and upstream
/// notification sinks. All diagnostics still go to stderr, never stdout.
fn init_tracing() {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    fmt()
        .with_max_level(LevelFilter::INFO)
        .with_writer(crate::process::RedactingMakeWriter)
        .init();
}
