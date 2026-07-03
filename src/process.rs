//! Upstream process lifecycle — spawning and stderr capture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[cfg(debug_assertions)]
use process_wrap::tokio::ChildWrapper;
use process_wrap::tokio::CommandWrap;
#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(all(debug_assertions, any(windows, unix)))]
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
use windows::Win32::System::Threading::GetCurrentProcess;

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
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    set.insert(secret.to_string());
}

/// Redact every registered secret from the given text.
pub fn redact(text: &str) -> String {
    let set = redacted_secrets()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// Outer process-tree containment for fanin-mcp itself.
/// Upstream containment is provided by the process-wrap `JobObject` wrapper
/// (suspended-spawn + KILL_ON_JOB_CLOSE); this is an additional outer guard.
#[allow(dead_code)]
// retained solely for Drop (KILL_ON_JOB_CLOSE on self)
#[derive(Debug)]
pub enum ProcessTreeGuard {
    #[cfg(windows)]
    Windows(WindowsSelfJobGuard),
    #[cfg(not(windows))]
    None,
}

/// Installs process-tree containment for descendants of fanin-mcp itself.
pub fn contain_current_process_tree() -> Result<ProcessTreeGuard, std::io::Error> {
    #[cfg(windows)]
    {
        WindowsSelfJobGuard::assign_current_process().map(ProcessTreeGuard::Windows)
    }
    #[cfg(not(windows))]
    {
        Ok(ProcessTreeGuard::None)
    }
}

/// Windows Job Object handle retained for fanin-mcp's whole process tree.
#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsSelfJobGuard {
    job: HANDLE,
}

#[cfg(windows)]
unsafe impl Send for WindowsSelfJobGuard {}
#[cfg(windows)]
unsafe impl Sync for WindowsSelfJobGuard {}

#[cfg(windows)]
impl WindowsSelfJobGuard {
    fn assign_current_process() -> Result<Self, std::io::Error> {
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

        // SAFETY: GetCurrentProcess returns a valid pseudo-handle for this
        // process; AssignProcessToJobObject does not take ownership of it.
        let assign_result = unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) };
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
impl Drop for WindowsSelfJobGuard {
    fn drop(&mut self) {
        // SAFETY: `self.job` is an owned Job Object handle and Drop runs once.
        unsafe { CloseHandle(self.job) }.ok();
    }
}

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
    resolved_cwd: Option<&str>,
) -> Result<SpawnedTransport, std::io::Error> {
    let command = config.command.as_deref().unwrap_or_default();
    let mut cmd = Command::new(command);
    cmd.args(&config.args);
    if let Some(cwd) = resolved_cwd {
        cmd.current_dir(cwd);
    }

    // Least-privilege: start from a clean env, then inject ONLY this server's vars.
    // This prevents sibling credentials and the aggregator's ambient env from leaking.
    cmd.env_clear();
    // Windows OS baseline (RCA 2026-07-03): a truly empty environment block
    // breaks the child's network stack — `getaddrinfo` fails with `EAI_FAIL`
    // when `SystemRoot` is absent, which Node/undici surfaces as
    // `TypeError: fetch failed` inside the upstream (observed live with
    // @upstash/context7-mcp; any Node upstream doing outbound fetch hits it).
    // Re-inject the non-sensitive system vars a Windows process cannot
    // function without. This stays least-privilege: no user, credential, or
    // ambient application vars are forwarded, and the server's configured
    // vars (below) still override. Note that naive reproductions miss this —
    // Node's libuv and MSYS `env -i` both silently re-add SYSTEMROOT to
    // children; only a truly empty block (Rust `env_clear`, Python
    // `subprocess(env={})`) exhibits the failure.
    #[cfg(windows)]
    for key in [
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "PROGRAMDATA",
        "NUMBER_OF_PROCESSORS",
    ] {
        if let Ok(value) = std::env::var(key) {
            cmd.env(key, value);
        }
    }
    for (key, value) in resolved_env {
        register_secret(value);
        cmd.env(key, value);
    }

    install_linux_parent_death_signal(&mut cmd)?;

    // Phase 5: process-wrap's Windows JobObject wrapper creates the child
    // suspended, assigns it to a kill-on-close Job Object, then resumes it.
    // That closes the old post-spawn AssignProcessToJobObject race (CARRY-1).
    // The self-Job (above) is an outer containment for fanin itself; upstreams
    // use the per-process JobObject wrapper. Unix keeps ProcessSession for
    // graceful group teardown; Linux additionally installs PR_SET_PDEATHSIG
    // above so a hard-killed parent takes the child with it.
    let mut wrapped = CommandWrap::from(cmd);
    #[cfg(windows)]
    {
        wrapped.wrap(JobObject);
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
    if let (Some(stderr), Some(log_file)) = (stderr, config.log_file.as_ref()) {
        spawn_stderr_log_task(server_name.to_string(), PathBuf::from(log_file), stderr);
    }

    // Capture PID for Unix process-group kill on graceful Drop.
    // ProcessSession guarantees the child is session/group leader → pgid == pid.
    #[cfg(unix)]
    let containment = transport
        .id()
        .map(|pid| ContainmentGuard::Unix { pgid: pid as i32 })
        .unwrap_or(ContainmentGuard::Inert);
    #[cfg(not(unix))]
    let containment = ContainmentGuard::Inert;

    Ok(SpawnedTransport {
        transport,
        containment,
    })
}

/// Spawn a long-lived descendant used by the Phase 5 immediate-startup
/// containment test.
#[cfg(debug_assertions)]
pub fn spawn_immediate_descendant(
    marker_path: &Path,
) -> Result<ImmediateDescendantGuard, std::io::Error> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg(crate::IMMEDIATE_DESCENDANT_SENTINEL)
        .arg(marker_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    install_linux_parent_death_signal(&mut cmd)?;

    let mut wrapped = CommandWrap::from(cmd);
    #[cfg(windows)]
    {
        wrapped.wrap(KillOnDrop);
        wrapped.wrap(JobObject);
    }
    #[cfg(unix)]
    {
        wrapped.wrap(KillOnDrop);
        wrapped.wrap(ProcessSession);
    }

    let child = wrapped.spawn()?;
    Ok(ImmediateDescendantGuard { child })
}

/// Retains the process containment handle for the immediate test descendant.
#[cfg(debug_assertions)]
pub struct ImmediateDescendantGuard {
    child: Box<dyn ChildWrapper>,
}

#[cfg(debug_assertions)]
impl ImmediateDescendantGuard {
    /// Returns the spawned descendant PID.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }
}

#[cfg(target_os = "linux")]
fn install_linux_parent_death_signal(cmd: &mut Command) -> Result<(), std::io::Error> {
    // SAFETY: `pre_exec` runs in the child after fork and before exec. The
    // closure only calls the async-signal-safe `prctl` syscall and constructs
    // an io::Error from errno if it fails; it does not touch shared locks.
    unsafe {
        cmd.pre_exec(|| {
            let rc = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            if rc == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn install_linux_parent_death_signal(_cmd: &mut Command) -> Result<(), std::io::Error> {
    Ok(())
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
    /// No active OS-level teardown by this guard: HTTP upstreams (no process),
    /// Windows stdio (the kill-on-close Job Object lives in the transport
    /// wrapper), and the Unix no-PID fallback. Drop is a no-op.
    Inert,
    /// Unix stdio upstream: the child is its own session/group leader
    /// (`ProcessSession` → `setsid`), so `pgid == pid`. Drop kills the whole
    /// group, reaping grandchildren on graceful teardown.
    #[cfg(unix)]
    Unix { pgid: i32 },
}

#[cfg(unix)]
impl Drop for ContainmentGuard {
    fn drop(&mut self) {
        if let Self::Unix { pgid } = self {
            if *pgid > 0 {
                // SAFETY: killpg is async-signal-safe; ESRCH is expected and ignored.
                unsafe {
                    let _ = libc::killpg(*pgid, libc::SIGKILL);
                }
            }
        }
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
use tracing::field::{Field, Visit};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormattedFields, MakeWriter};
use tracing_subscriber::registry::LookupSpan;

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

/// MakeWriter implementation that returns redacting append writers for one file.
#[derive(Clone, Debug)]
pub struct RedactingFileMakeWriter {
    path: PathBuf,
}

impl RedactingFileMakeWriter {
    /// Creates a redacting writer factory for a structured log file.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

/// Redacting writer for structured file logs.
pub struct RedactingFileWriter {
    path: PathBuf,
}

impl Write for RedactingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        if let Ok(s) = std::str::from_utf8(buf) {
            let redacted = redact(s);
            file.write_all(redacted.as_bytes())?;
            Ok(buf.len())
        } else {
            file.write_all(buf)?;
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RedactingFileMakeWriter {
    type Writer = RedactingFileWriter;

    fn make_writer(&self) -> Self::Writer {
        RedactingFileWriter {
            path: self.path.clone(),
        }
    }
}

/// Minimal NDJSON tracing formatter for the serve log-file sink.
pub struct RedactingJsonFormatter;

impl<S, N> FormatEvent<S, N> for RedactingJsonFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        let mut fields = JsonFieldVisitor::default();
        event.record(&mut fields);

        let mut object = fields.fields;
        object.insert(
            "level".to_string(),
            serde_json::Value::String(meta.level().as_str().to_ascii_lowercase()),
        );
        object.insert(
            "target".to_string(),
            serde_json::Value::String(meta.target().to_string()),
        );

        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<FormattedFields<N>>() {
                    if !fields.is_empty() {
                        object.insert(
                            format!("span.{}", span.name()),
                            serde_json::Value::String(fields.to_string()),
                        );
                    }
                }
            }
        }

        writeln!(writer, "{}", serde_json::Value::Object(object))
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: serde_json::Map<String, serde_json::Value>,
}

impl JsonFieldVisitor {
    fn insert(&mut self, field: &Field, value: serde_json::Value) {
        self.fields.insert(field.name().to_string(), value);
    }
}

impl Visit for JsonFieldVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        let value = serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
        self.insert(field, value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, serde_json::Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, serde_json::Value::Number(value.into()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, serde_json::Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, serde_json::Value::String(value.to_string()));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.insert(field, serde_json::Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        let value = serde_json::from_str::<serde_json::Value>(&rendered)
            .unwrap_or(serde_json::Value::String(rendered));
        self.insert(field, value);
    }
}

fn log_sender(log_file: PathBuf, server_name: String) -> mpsc::Sender<String> {
    let key = (log_file.clone(), server_name.clone());
    let writers = LOG_WRITERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut writers = writers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
