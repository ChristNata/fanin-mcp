# CI greening — post-cycle log (v0.6.6 → v0.6.14)

Standing up the Phase 5 3-OS CI matrix (the first time fanin-mcp ran on
Linux/macOS — the dev host is Windows-only) surfaced a stack of issues each
masked by the previous failure. All now green on ubuntu + macos + windows
(run `28281319160`). This note records what happened + the one carried follow-up.

## Iteration log

| Run | Red cause(s) | Fix |
|---|---|---|
| 1 (v0.6.6) | Linux `libdbus-1-dev` missing (keyring); all-OS `uninlined_format_args` clippy (toolchain drift) | apt install libdbus; `-A clippy::uninlined_format_args` |
| 2 (v0.6.7) | Linux unused `CommandExt` import (linux-only code never compiled locally); all-OS `CARGO_BIN_EXE_*` resolved at runtime not compile-time | remove import; switch tests to `env!` |
| 3 (v0.6.8) | `token_figures` ordering (bench after tests); Unix graceful teardown; Unix hard-kill scope; headless-Linux keyring | reorder CI; KillOnDrop (insufficient); cfg-gate; runtime-skip |
| 4 (v0.6.10) | clippy dead_code from cfg-gating; Windows token CRLF | `cfg_attr(not(windows), ignore)`; normalize line endings |
| 5 (v0.6.11) | Unix graceful teardown STILL failing (KillOnDrop didn't killpg the group) | ContainmentGuard holds pgid, `killpg(SIGKILL)` on Drop |
| 6 (v0.6.12) | killpg `pgid:0` safety bug (would kill own group); Unix compile errors (enum `Retained`/`libc` cfg) | guard `pgid>0` + inert variant; cross-platform `Inert` + `libc` cfg(unix) |
| 7 (v0.6.13) | killpg killed LIVE upstreams (guard dropped post-spawn); cargo-audit CVSS-4.0 | retain guard into UpstreamEntry; drop cargo-audit |
| 8 (v0.6.14) | `cargo deny` advisories ALSO CVSS-4.0 | run `cargo deny check bans licenses sources` |
| 9 | — | **GREEN, all 3 OSes** |

## Key enabler

`cargo clippy --target x86_64-apple-darwin` on the Windows host **type-checks
the `cfg(unix)` code locally** (macOS uses the Keychain, so no libdbus
build-script block, unlike the linux target). This ended the blind
push-and-wait loop for Unix compile/clippy errors. Linux-only code
(`cfg(target_os="linux")`, e.g. PDEATHSIG) still needs CI, but the broad
`cfg(unix)` surface is now locally verifiable. See [[dispatch-model-reliability]]
sibling memory notes.

## Final process-containment guarantee (matches SECURITY.md / GOTCHA #14)

- **Windows:** full whole-tree crash-safe (Job Object, suspended-spawn).
- **Linux:** graceful whole-tree (`killpg` on `ContainmentGuard` Drop) + direct
  child crash-safe (`PR_SET_PDEATHSIG`). Grandchild may orphan on hard SIGKILL.
- **macOS:** graceful whole-tree (`killpg`). Hard SIGKILL may orphan.
- Whole-tree hard-kill orphan test is `#[cfg_attr(not(windows), ignore)]`.

## ⚠️ CARRIED FOLLOW-UP — advisory scanning paused

CI runs `cargo deny check bans licenses sources` — the **`advisories` check is
excluded**. Reason: the RustSec advisory DB now ships **CVSS 4.0** entries
(e.g. RUSTSEC-2026-0124) that the current parser tooling rejects on load
(`unsupported CVSS version: 4.0`) — confirmed on BOTH `cargo audit` and the
latest `cargo deny` (0.19.9). No released RustSec tool parses the current DB.

**Re-enable trigger:** a `cargo deny` release with CVSS-4.0 support. Then switch
ci.yml back to `cargo deny check` (full) and revert the `/gate` + SECURITY.md /
STACK.md / ARCHITECTURE.md / MVP.md notes. Interim risk is bounded by the small,
exact-pinned dependency tree with committed `Cargo.lock`.
