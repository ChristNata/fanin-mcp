# Release Checklist

Run these checks before tagging a release. CI covers format, lint, integration
tests, supply-chain policy, release build, stripped binary size, and the token
benchmark on Windows, macOS, and Linux. This checklist covers the manual gates
that need real hosts or credentials.

## Real Streamable-HTTP upstream

- Use a disposable public MCP-compatible Streamable-HTTP upstream.
- Configure a static auth header through a placeholder, not a literal secret:

```toml
[servers.remote]
transport = "streamable-http"
endpoint = "https://example.invalid/mcp"

[servers.remote.headers]
Authorization = "Bearer ${FANIN_RELEASE_REMOTE_TOKEN}"
```

- Run `fanin-mcp --config <config> --log-file <log> --log-level info` and invoke
  one harmless remote tool through `invoke_tool`.
- Verify the remote received the header and the token does not appear in stderr
  or the JSON log file.

## Memory budgets

Thresholds are release blockers:

- Idle fanin-mcp process: `<15MB` RSS.
- Five active upstreams: `<50MB` RSS for fanin-mcp itself.

Use a release build and a config with five loopback probe upstreams. Measure the
fanin-mcp process after initialization and again after invoking one tool on each
upstream.

Linux:

```bash
cargo build --release
target/release/fanin-mcp --config /tmp/fanin-five-upstreams.toml --log-file /tmp/fanin.log &
pid=$!
ps -o rss= -p "$pid" # KiB; must be <15360 idle and <51200 after five upstreams
```

macOS:

```bash
cargo build --release
target/release/fanin-mcp --config /tmp/fanin-five-upstreams.toml --log-file /tmp/fanin.log &
pid=$!
ps -o rss= -p "$pid" # KiB; must be <15360 idle and <51200 after five upstreams
```

Windows PowerShell:

```powershell
cargo build --release
$p = Start-Process -FilePath target\release\fanin-mcp.exe -ArgumentList '--config', $env:TEMP\fanin-five-upstreams.toml, '--log-file', $env:TEMP\fanin.log -PassThru
(Get-Process -Id $p.Id).WorkingSet64 # bytes; must be <15728640 idle and <52428800 after five upstreams
```

## Platform verification

- Windows: run CI green on `windows-latest`; verify Job Object hard-kill tests
  pass and no probe descendants remain after forced termination.
- Linux: run CI green on `ubuntu-latest`; verify the PDEATHSIG hard-kill test
  passes and no probe descendants remain after forced termination.
- macOS: run CI green on `macos-latest`; verify graceful process-group teardown
  passes. Do not claim zero-orphan protection for SIGKILL on macOS.
- Record stripped binary sizes from CI artifacts/logs for each OS; each must be
  `<10MB`.
