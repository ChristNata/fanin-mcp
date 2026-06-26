//! Upstream process lifecycle — spawning and stderr capture.

use std::path::PathBuf;
use std::process::Stdio;

use rmcp::transport::child_process::TokioChildProcess;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::config::ServerConfig;

/// Spawn an upstream stdio child and capture its stderr to the configured log.
pub fn spawn_stdio_transport(
    server_name: &str,
    config: &ServerConfig,
) -> Result<TokioChildProcess, std::io::Error> {
    let command = config.command.as_deref().unwrap_or_default();
    let mut cmd = Command::new(command);
    cmd.args(&config.args);
    for (key, value) in &config.env {
        cmd.env(key, value);
    }

    let builder = if config.log_file.is_some() {
        TokioChildProcess::builder(cmd).stderr(Stdio::piped())
    } else {
        TokioChildProcess::builder(cmd).stderr(Stdio::null())
    };
    let (transport, stderr) = builder.spawn()?;
    if let (Some(stderr), Some(log_file)) = (stderr, config.log_file.as_ref()) {
        spawn_stderr_log_task(server_name.to_string(), PathBuf::from(log_file), stderr);
    }
    Ok(transport)
}

fn spawn_stderr_log_task(
    server_name: String,
    log_file: PathBuf,
    stderr: tokio::process::ChildStderr,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => append_log_line(log_file.clone(), server_name.clone(), line),
                Ok(None) => break,
                Err(e) => {
                    append_log_line(
                        log_file.clone(),
                        server_name.clone(),
                        format!("stderr read error: {e}"),
                    );
                    break;
                }
            }
        }
    });
}

/// Append one server-prefixed line to a log file.
pub fn append_log_line(log_file: PathBuf, server_name: String, line: String) {
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;

        match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .await
        {
            Ok(mut file) => {
                let _ = file
                    .write_all(format!("[{server_name}] {line}\n").as_bytes())
                    .await;
                let _ = file.flush().await;
            }
            Err(e) => {
                tracing::warn!(server = %server_name, path = %log_file.display(), error = %e, "failed to open upstream log file");
            }
        }
    });
}
