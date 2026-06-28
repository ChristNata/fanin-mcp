## LENS: ADVERSARIAL — write review-adv.md

Try to break the hardening and find anything the green gate hides:

- **H-3 (unconditional header redaction, registry.rs).** Does removing the
  `if raw.contains("${")` guard genuinely close the literal-secret leak for ALL
  header paths? Any remaining path where a header value (literal or resolved)
  reaches a log/error/trace WITHOUT being registered first? Does registering
  every header value (even non-secret ones like `Content-Type: application/json`)
  cause over-redaction that could corrupt logs (e.g. a common substring redacted
  everywhere)? Check `register_secret`/`redact` interaction.
- **H-1 (mutex poison recovery, process.rs).** `.unwrap_or_else(|p|
  p.into_inner())` recovers the poisoned guard — is the recovered `HashSet` /
  writers map ever left in a torn/inconsistent state that a later reader trusts?
  Any panic path still remaining on those globals?
- **H-2 (length cap, server.rs).** Is the 200-char cap applied on a CHAR boundary
  (no panic on a multibyte codepoint at the boundary)? Can a crafted identifier
  still bloat context below 200, or bypass the cap?
- **H-7 (cfg(debug_assertions) gating).** THE risk item. Confirm: the test-only
  spawn hooks (`__fanin_immediate_descendant__` sentinel, `--spawn-immediate-
  descendant` scan, marker writer, `spawn_immediate_descendant`,
  `ImmediateDescendantGuard`, and the now-cfg'd imports) are FULLY excluded from a
  release build — no reachable remnant, no behavioral divergence beyond the hooks.
  Does the debug build still expose them ONLY as before (no new surface)? Could a
  release user reach any of this? (Release build is verified warning-free; look
  for logic the gate boundary split incorrectly.)
- **H-6.** The comment-only change must not have altered startup behavior. Confirm
  no functional change to the eprintln/tracing paths.
- **Secrets/D-010 + D-005.** No secret newly logged; public error shape unchanged.
- **Docs accuracy as an attack surface:** does CONTRIBUTING.md or SECURITY.md make
  a claim the code does NOT honor (e.g. a gate command that doesn't exist, a
  redaction guarantee that's stronger than reality)? An over-claim in a security
  doc is a real finding.
