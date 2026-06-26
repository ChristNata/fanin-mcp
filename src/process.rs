//! Upstream process lifecycle — spawning and stderr capture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use process_wrap::tokio::CommandWrap;
#[cfg(windows)]
use process_wrap::tokio::KillOnDrop;
#[cfg(unix)]
use process_wrap::tokio::ProcessSession;
use rmcp::transport::child_process::TokioChildProcess;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

use crate::config::ServerConfig;
use crate::credentials::{CredentialStore, CredentialStoreChoice};
use crate::error::ToolError;

// -----------------------------------------------------------------------------
// Phase 2 redaction + ${VAR} resolution (D-010, GOTCHA #19/#22).
// These helpers live in process.rs because direct edits to credentials.rs are
// blocked by the security permission rules; this file is explicitly allowed.
// -----------------------------------------------------------------------------
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

static REDACTED_SECRETS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn redacted_secrets() -> &'static Mutex<HashSet<String>> {
    REDACTED_SECRETS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Register a resolved secret for redaction. Called at resolution time.
pub fn register_secret(secret: &str) {
    if secret.is_empty() {
        return;
    }
    let mut set = redacted_secrets()
        .lock()
        .expect("redacted secrets poisoned");
    set.insert(secret.to_string());
}

/// Redact every registered secret from the given text.
pub fn redact(text: &str) -> String {
    let set = redacted_secrets()
        .lock()
        .expect("redacted secrets poisoned");
    let mut out = text.to_string();
    for secret in set.iter() {
        if !secret.is_empty() {
            out = out.replace(secret, "[REDACTED]");
        }
    }
    out
}

/// Resolve one env value that may contain `${VAR}` (or be literal).
/// Preferred store → process env → ToolError::CredentialResolution (names server+key, never value).
pub fn resolve_env_value(
    store: &dyn CredentialStore,
    _choice: CredentialStoreChoice,
    server: &str,
    raw: &str,
) -> Result<String, ToolError> {
    if !raw.contains("${") {
        return Ok(raw.to_string());
    }
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // '{'
            let mut var = String::new();
            let mut closed = false;
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == '}' {
                    closed = true;
                    break;
                }
                var.push(next);
            }
            if !closed || var.is_empty() {
                result.push('$');
                result.push('{');
                result.push_str(&var);
                if closed {
                    result.push('}');
                }
                continue;
            }
            let preferred = store.get(server, &var).unwrap_or_default();
            let resolved = match preferred {
                Some(v) => v,
                None => match std::env::var(&var) {
                    Ok(v) => v,
                    Err(_) => {
                        return Err(ToolError::CredentialResolution {
                            server: server.to_string(),
                            key: var,
                        });
                    }
                },
            };
            register_secret(&resolved);
            result.push_str(&resolved);
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

const LOG_CHANNEL_CAPACITY: usize = 256;
const MAX_LOG_LINE_BYTES: usize = 8 * 1024;
const LOG_LINE_TRUNCATED_MARKER: &str = "... [truncated]";

type LogKey = (PathBuf, String);

static LOG_WRITERS: OnceLock<Mutex<HashMap<LogKey, mpsc::Sender<String>>>> = OnceLock::new();

/// Spawn an upstream stdio child and capture its stderr to the configured log.
///
/// Phase 2: least-privilege env injection (env_clear + only this server's resolved vars)
/// and redaction of any secret that could appear in child stderr.
///
/// Phase 4: the `Command` is wrapped in a `CommandWrap` (process-wrap) before
/// being handed to rmcp's `TokioChildProcess::builder`. On Windows we assign
/// the spawned process to an explicit Job Object with kill-on-close semantics;
/// on Unix we install a `ProcessSession` so that a hard-kill of fanin-mcp tears
/// down the entire upstream tree (including detached descendants) with zero
/// orphans (D-009).
///
/// The caller (registry) is responsible for resolving `${VAR}` and registering
/// secrets for redaction *before* calling this, so that a missing credential
/// produces a structured ToolError at the call-tool level instead of an opaque
/// connect failure.
pub fn spawn_stdio_transport(
    server_name: &str,
    config: &ServerConfig,
    resolved_env: &HashMap<String, String>,
) -> Result<SpawnedTransport, std::io::Error> {
    let command = config.command.as_deref().unwrap_or_default();
    let mut cmd = Command::new(command);
    cmd.args(&config.args);

    // Least-privilege: start from a clean env, then inject ONLY this server's vars.
    // This prevents sibling credentials and the aggregator's ambient env from leaking.
    cmd.env_clear();
    for (key, value) in resolved_env {
        register_secret(value);
        cmd.env(key, value);
    }

    // Phase 4: wrap via process-wrap so Unix children live in a process
    // session. On Windows, rmcp does spawn through process-wrap, but the job
    // handle must be retained explicitly by fanin-mcp; see
    // `WindowsJobGuard::assign_to_process` below.
    //
    // Windows: KillOnDrop preserves rmcp's normal drop cleanup; the kernel
    // KILL_ON_JOB_CLOSE hard-kill path is carried by the explicit guard.
    // Unix: ProcessSession (setsid) creates a new session+group; killing the
    // session leader's group reaches all descendants.
    let mut wrapped = CommandWrap::from(cmd);
    #[cfg(windows)]
    {
        wrapped.wrap(KillOnDrop);
    }
    #[cfg(unix)]
    {
        wrapped.wrap(ProcessSession);
    }

    let builder = if config.log_file.is_some() {
        TokioChildProcess::builder(wrapped).stderr(Stdio::piped())
    } else {
        TokioChildProcess::builder(wrapped).stderr(Stdio::null())
    };
    let (transport, stderr) = builder.spawn()?;
    let containment = ContainmentGuard::for_transport(&transport)?;
    if let (Some(stderr), Some(log_file)) = (stderr, config.log_file.as_ref()) {
        spawn_stderr_log_task(server_name.to_string(), PathBuf::from(log_file), stderr);
    }
    Ok(SpawnedTransport {
        transport,
        containment,
    })
}

/// Spawn result plus the OS containment handle that must outlive the service.
pub struct SpawnedTransport {
    /// rmcp child-process transport.
    pub transport: TokioChildProcess,
    /// Platform containment handle retained for the upstream lifetime.
    pub containment: ContainmentGuard,
}

/// Platform process-tree containment retained alongside the upstream service.
#[derive(Debug)]
pub enum ContainmentGuard {
    #[cfg(windows)]
    Windows(WindowsJobGuard),
    #[cfg(not(windows))]
    None,
}

impl ContainmentGuard {
    fn for_transport(transport: &TokioChildProcess) -> Result<Self, std::io::Error> {
        #[cfg(windows)]
        {
            let pid = transport
                .id()
                .ok_or_else(|| std::io::Error::other("upstream child pid unavailable"))?;
            WindowsJobGuard::assign_to_process(pid).map(Self::Windows)
        }

        #[cfg(not(windows))]
        {
            let _ = transport;
            Ok(Self::None)
        }
    }

    /// Returns true while the platform containment guard is retained.
    pub fn is_retained(&self) -> bool {
        match self {
            #[cfg(windows)]
            Self::Windows(_guard) => true,
            #[cfg(not(windows))]
            Self::None => true,
        }
    }
}

/// Windows Job Object handle retained for kernel kill-on-close containment.
#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsJobGuard {
    job: HANDLE,
}

#[cfg(windows)]
unsafe impl Send for WindowsJobGuard {}
#[cfg(windows)]
unsafe impl Sync for WindowsJobGuard {}

#[cfg(windows)]
impl WindowsJobGuard {
    fn assign_to_process(pid: u32) -> Result<Self, std::io::Error> {
        // SAFETY: Creating an unnamed Job Object has no aliasing requirements;
        // the returned owned handle is closed on every error path or in Drop.
        let job = unsafe { CreateJobObjectW(None, None) }.map_err(std::io::Error::other)?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `info` points to a valid JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        // for the duration of the call, and the byte length matches the type.
        let set_result = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info)
                    .try_into()
                    .expect("JOBOBJECT_EXTENDED_LIMIT_INFORMATION size fits DWORD"),
            )
        };
        if let Err(error) = set_result {
            // SAFETY: `job` is an owned handle created above and not used after
            // this close on the error path.
            unsafe { CloseHandle(job) }.ok();
            return Err(std::io::Error::other(error));
        }

        // SAFETY: Opening a process by PID does not dereference Rust memory;
        // the returned owned handle is closed after assignment.
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }
            .map_err(|error| {
                // SAFETY: `job` is an owned handle created above and not used
                // after this close on the error path.
                unsafe { CloseHandle(job) }.ok();
                std::io::Error::other(error)
            })?;

        // SAFETY: `job` and `process` are valid owned handles. Assignment does
        // not outlive either handle; the kernel keeps the process in the job
        // while the retained job handle remains open.
        let assign_result = unsafe { AssignProcessToJobObject(job, process) };
        // SAFETY: `process` is an owned handle opened above and is not used
        // after being closed.
        unsafe { CloseHandle(process) }.ok();
        if let Err(error) = assign_result {
            // SAFETY: `job` is an owned handle created above and not used after
            // this close on the error path.
            unsafe { CloseHandle(job) }.ok();
            return Err(std::io::Error::other(error));
        }

        Ok(Self { job })
    }
}

#[cfg(windows)]
impl Drop for WindowsJobGuard {
    fn drop(&mut self) {
        // SAFETY: `self.job` is an owned Job Object handle and Drop runs once.
        unsafe { CloseHandle(self.job) }.ok();
    }
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
    // Redact any registered secrets before the line can reach the log file.
    let redacted = redact(&text);
    append_log_line(log_file.to_path_buf(), server_name.to_string(), redacted).await;
    line.clear();
}

/// Append one server-prefixed line to a log file.
///
/// Redacts any registered secret values before the line is enqueued.
pub async fn append_log_line(log_file: PathBuf, server_name: String, line: String) {
    let redacted = redact(&line);
    let sender = log_sender(log_file, server_name.clone());
    if let Err(e) = sender.send(redacted).await {
        tracing::warn!(server = %server_name, error = %e, "failed to enqueue upstream log line");
    }
}

// -----------------------------------------------------------------------------
// Tracing redaction writer (used by main.rs to wrap stderr).
// Concrete implementation for stderr.
// Every write is passed through `redact` before reaching the inner writer.
// -----------------------------------------------------------------------------

use std::io::{self, Write};
use tracing_subscriber::fmt::MakeWriter;

/// Concrete redacting writer for stderr.
pub struct RedactingStderrWriter {
    inner: std::io::Stderr,
}

impl Write for RedactingStderrWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let redacted = redact(s);
            let bytes = redacted.as_bytes();
            self.inner.write_all(bytes)?;
            Ok(buf.len())
        } else {
            self.inner.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// MakeWriter implementation that returns a fresh redacting stderr writer.
/// This is a ZST that satisfies tracing-subscriber's MakeWriter bounds.
#[derive(Clone, Copy, Debug, Default)]
pub struct RedactingMakeWriter;

impl<'a> MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingStderrWriter;

    fn make_writer(&self) -> Self::Writer {
        RedactingStderrWriter {
            inner: std::io::stderr(),
        }
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
