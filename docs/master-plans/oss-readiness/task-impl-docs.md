IMPLEMENT Phase A (docs/metadata) + D-2 doc strike — oss-readiness. DOCS ONLY.

You are the implementer for the documentation/metadata phase. Edit ONLY:
`Cargo.toml`, `SECURITY.md`, `ARCHITECTURE.md` (under docs/), `STACK.md`,
`docs/GOTCHA.md`, and a NEW `CONTRIBUTING.md` at repo root. Do NOT touch `src/`
or `tests/`. Read `docs/master-plans/oss-readiness/master.md` and the `decisions`
block in `state.json`. The repo git remote is
`https://github.com/ChristNata/fanin-mcp`.

## O-1 — Cargo.toml [package] metadata + publish

Add to the `[package]` table (keep existing keys; version stays `0.1.0`):
- `repository = "https://github.com/ChristNata/fanin-mcp"`
- `homepage = "https://github.com/ChristNata/fanin-mcp"`
- `readme = "README.md"`
- `keywords = ["mcp", "proxy", "llm", "stdio", "aggregator"]`  (max 5, each ≤20 chars)
- `categories = ["command-line-utilities", "development-tools"]`  (must be valid
  crates.io categories — these two are valid)
- change `publish = false` → `publish = true`

Do not reorder or remove existing keys; insert the new ones logically (after
`description`). Leave the `autobins`/`autotests` comment block intact.

## O-2 — SECURITY.md contact = GitHub Security Advisories

Replace the placeholder line (currently `Please report security issues privately
to <SECURITY_CONTACT_EMAIL> rather than opening a public issue. We aim to
acknowledge within 72 hours.`) with a GitHub Security Advisories instruction:
report privately via the repository's **Security** tab → **Report a
vulnerability** (GitHub private vulnerability reporting), no public issue; keep
the 72-hour acknowledgement intent. No email address.

## H-8 — SECURITY.md redaction-scope note

Add a short paragraph (near the existing secret-redaction discussion, or a new
"Log redaction scope" line) stating: log redaction is exact-substring matching of
registered secret values — whole-secret appearances are caught and replaced with
`[REDACTED]`; a secret that appears perturbed/partial (e.g. truncated by an
upstream) is out of scope. Honest scoping, no over-claim.

## D-2 — strike `--passthrough-stderr` from the docs

The flag is documented but unimplemented; the decision is to STRIKE it (child
stderr is already captured to the log file). Remove its references cleanly,
leaving surrounding prose correct:
- `docs/ARCHITECTURE.md` (~line 163) — remove the `--passthrough-stderr`
  sentence/clause.
- `STACK.md` (~line 27) — remove the `--passthrough-stderr` reference.
- `docs/GOTCHA.md` #29 — the entry ends `…write to the log file;
  \`--passthrough-stderr\` for debugging only. ✅`. Strike the
  "; \`--passthrough-stderr\` for debugging only" clause so it reads
  `…write to the log file. ✅`.
Grep `passthrough` across docs first to catch every reference; remove them all.

## O-3 — new CONTRIBUTING.md (repo root)

Write a concise one-page guide in the project's voice (capital-style: sharp,
decision-first, no filler). Cover:
- Build + run: `cargo build --release` → single static binary.
- The gate every PR must pass: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test --all` (100% pass).
- rmcp exact-pin discipline: `rmcp` is pinned `=1.8.0`, `Cargo.lock` committed;
  do not bump casually (see the `rmcp-general` skill / docs).
- Read `docs/DECISIONS.md` (binding ADRs) and `docs/GOTCHA.md` (the trap list)
  before touching a subsystem — they encode decisions, not suggestions.
- Identity constraints: single static binary, NO runtime deps; the anti-stack
  (no web framework, HTTP server, database/ORM, plugin loader, Node/Docker at
  runtime) — a PR adding one contradicts the design.
- CI runs the full 3-OS matrix; PRs must be green there.
- Security: report vulnerabilities via GitHub Security Advisories (Security tab),
  never a public issue.
Keep it to ~one page. Match the markdown style of the existing root docs.

## Finish

No code, no tests — so the suite is unaffected (still 134 passed / 1 failed — the
failing test is the src phase's H-3, not yours). Run nothing heavier than a
`cargo metadata` sanity check if you wish. Return as data for the orchestrator:
the files changed, the exact Cargo.toml keys added, confirmation every
`passthrough` reference is gone from docs, and the CONTRIBUTING.md outline.
