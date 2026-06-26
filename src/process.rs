//! Upstream process lifecycle — spawning and stderr capture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

use rmcp::transport::child_process::TokioChildProcess;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::ServerConfig;

const LOG_CHANNEL_CAPACITY: usize = 256;
const MAX_LOG_LINE_BYTES: usize = 8 * 1024;
const LOG_LINE_TRUNCATED_MARKER: &str = "... [truncated]";

type LogKey = (PathBuf, String);

static LOG_WRITERS: OnceLock<Mutex<HashMap<LogKey, mpsc::Sender<String>>>> = OnceLock::new();

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
        let mut stderr = stderr;
        let mut chunk = [0_u8; 1024];
        let mut line = Vec::with_capacity(1024);
        let mut truncated = false;

        loop {
            match stderr.read(&mut chunk).await {
                Ok(0) => {
                    if !line.is_empty() || truncated {
                        emit_stderr_line(&log_file, &server_name, &mut line, truncated).await;
                    }
                    break;
                }
                Ok(n) => {
                    for byte in &chunk[..n] {
                        if *byte == b'\n' {
                            emit_stderr_line(&log_file, &server_name, &mut line, truncated).await;
                            truncated = false;
                        } else if line.len() < MAX_LOG_LINE_BYTES {
                            line.push(*byte);
                        } else {
                            truncated = true;
                        }
                    }
                }
                Err(e) => {
                    append_log_line(
                        log_file.clone(),
                        server_name.clone(),
                        format!("stderr read error: {e}"),
                    )
                    .await;
                    break;
                }
            }
        }
    });
}

async fn emit_stderr_line(log_file: &Path, server_name: &str, line: &mut Vec<u8>, truncated: bool) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }

    let mut text = String::from_utf8_lossy(line).into_owned();
    if truncated {
        text.push_str(LOG_LINE_TRUNCATED_MARKER);
    }
    append_log_line(log_file.to_path_buf(), server_name.to_string(), text).await;
    line.clear();
}

/// Append one server-prefixed line to a log file.
pub async fn append_log_line(log_file: PathBuf, server_name: String, line: String) {
    let sender = log_sender(log_file, server_name.clone());
    if let Err(e) = sender.send(line).await {
        tracing::warn!(server = %server_name, error = %e, "failed to enqueue upstream log line");
    }
}

fn log_sender(log_file: PathBuf, server_name: String) -> mpsc::Sender<String> {
    let key = (log_file.clone(), server_name.clone());
    let writers = LOG_WRITERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut writers = writers.lock().expect("log writer registry poisoned");
    if let Some(sender) = writers.get(&key) {
        return sender.clone();
    }

    let (sender, receiver) = mpsc::channel(LOG_CHANNEL_CAPACITY);
    spawn_log_writer(log_file, server_name, receiver);
    writers.insert(key, sender.clone());
    sender
}

fn spawn_log_writer(log_file: PathBuf, server_name: String, mut receiver: mpsc::Receiver<String>) {
    tokio::spawn(async move {
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .await
        {
            Ok(file) => file,
            Err(e) => {
                tracing::warn!(server = %server_name, path = %log_file.display(), error = %e, "failed to open upstream log file");
                return;
            }
        };

        while let Some(line) = receiver.recv().await {
            if let Err(e) = file
                .write_all(format!("[{server_name}] {line}\n").as_bytes())
                .await
            {
                tracing::warn!(server = %server_name, path = %log_file.display(), error = %e, "failed to write upstream log line");
                continue;
            }
            if let Err(e) = file.flush().await {
                tracing::warn!(server = %server_name, path = %log_file.display(), error = %e, "failed to flush upstream log file");
            }
        }
    });
}
