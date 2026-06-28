# Contributing to fanin-mcp

## Build + Run
```bash
cargo build --release
```
Produces a single static binary (`target/release/fanin-mcp`) with zero runtime dependencies.

## Gate — every PR must pass
```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all          # 100 % pass, no thresholds
```

## rmcp exact-pin discipline
`rmcp` is pinned `=1.8.0`; `Cargo.lock` is committed. Do not bump casually. See the `rmcp-general` skill and `docs/DECISIONS.md` (D-015).

## Binding canon
Read `docs/DECISIONS.md` (ADRs) and `docs/GOTCHA.md` (trap list) before touching any subsystem. They encode decisions already made, not suggestions.

## Identity constraints (non-goals)
Single static binary. **No** web framework, HTTP server, database/ORM, plugin loader, Node, or Docker at runtime. A PR adding any of these contradicts the design — flag it.

## CI
Every PR runs the full 3-OS matrix. PRs must be green on all three.

## Security reporting
Report vulnerabilities via the repository **Security** tab → **Report a vulnerability** (GitHub private vulnerability reporting). Never open a public issue.
