# Knowledge-Sync: oss-readiness

**Summary:** 2 binding-doc edits applied + staged (GOTCHA #20, DECISIONS D-017).
Most of this cycle's doc work landed IN-CYCLE (the Phase-A docs implementer +
orchestrator already updated SECURITY.md, Cargo.toml, CONTRIBUTING.md, and struck
D-2 across ARCHITECTURE/STACK/GOTCHA/PRD), so the post-push residual is small.
Tier THOROUGH. Reconciled against the pushed diff `d219794..93524c4`.

## Change set (diff truth)
Cargo.toml, SECURITY.md, STACK.md, CONTRIBUTING.md (new), docs/ARCHITECTURE.md,
docs/GOTCHA.md, docs/PRD.md, and src: process.rs, registry.rs, server.rs,
error.rs, credentials.rs, main.rs; test: literal_header_redaction.rs. Matches the
plan's phases (A docs/metadata, B hardening, C hygiene, D test-hooks). No
planned-not-shipped, no shipped-not-planned.

## Residual binding-doc edits (applied + staged)
- **GOTCHA #20** — targeted. "length-cap (~100 chars)" was imprecise after H-2;
  refined to: descriptions/text ~100 chars, tool-name identifiers capped at 200
  (defense-in-depth against a non-rmcp upstream emitting an over-long raw name).
- **DECISIONS D-017** — targeted. Its "Open" line listed exactly the two items
  this cycle closed; updated to "Resolved": `[package]` metadata + `publish=true`
  in place, SECURITY.md contact = GitHub Security Advisories. A `cargo publish
  --dry-run` smoke remains a release-checklist step (not blocking).

## Already reconciled in-cycle (no further edit)
- SECURITY.md — O-2 GitHub-Advisories contact, H-8 exact-substring redaction note,
  and the H-3 over-redaction tradeoff note (A5).
- Cargo.toml — O-1 metadata + publish flag.
- CONTRIBUTING.md — new (O-3).
- ARCHITECTURE/STACK/GOTCHA #29/PRD — D-2 `--passthrough-stderr` struck.

## Stale-doc flags carried forward (NOT this cycle's scope)
- **MVP.md checklist sign-off** — still unchecked boxes despite Phase-3..5 + both
  remediation cycles shipping. A project-wide checklist sign-off pass.
- **ROADMAP v1.0 status** — with S-1/D-1 closed and OSS-readiness done, a ROADMAP
  pass to reflect "v1.0-ready" posture is warranted (user call).
- **T4** — `HeaderSeen`/`start_http_probe` duplicated across two integration test
  files; a future test-cleanup pass.

## Pending
Staged doc edits ride the post-cycle knowledge-sync push.
