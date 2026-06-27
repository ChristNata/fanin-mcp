WRITE THE TEST CONTRACT — oss-readiness cycle. NARROW scope.

You are the test-creator, sole author of test files. Read
`docs/master-plans/oss-readiness/master.md` and `state.json` (the `decisions`
block — D-2 is strike-from-docs, so NO test for it). Tier thorough, stack rust.

Most findings in this cycle are docs/metadata (O-1/O-2/O-3, H-8), pure refactors
(H-4 dead_code removal, H-5 meta_tools→assoc-fn), a startup tweak (H-6), or
defensive/structural changes verified by code-inspection. **Do NOT fabricate
tests for mechanical/doc findings** — a padded test is worse than none. The
existing 134-test suite staying green is the contract for those.

Write only the genuinely-new BEHAVIORAL tests below, plus `tests.md`. You may
edit only `tests/**`. The suite must COMPILE, be `cargo fmt`-clean and
`clippy -D warnings`-clean; the NEW tests are expected RED until the implementer
lands the code; existing tests stay green.

## H-3 — literal HTTP header values must be redacted (REAL new test)

Today `registry.rs` registers a resolved header for redaction only when the raw
template contains `${...}`. A LITERAL secret in a server's `headers` table (e.g.
`Authorization = "Bearer sk-LITERALSECRET123"`, no `${VAR}`) is never added to the
redaction set, so it can appear un-redacted if logged. The implementer will make
header-value registration unconditional.

Write a test that: configures a Streamable-HTTP (or stdio, whichever the existing
redaction tests use) upstream with a LITERAL secret header value containing a
unique sentinel; drives a code path that would log header values (follow how the
existing redaction / sentinel test exercises the log file —
`tests/integration/` has a redaction test already); asserts the sentinel never
appears un-redacted in the log/stderr output (it must be `[REDACTED]`). Mirror the
structure of the existing secret-sentinel redaction test. This test should FAIL
today (literal value leaks) and pass once registration is unconditional.

## H-2 — identifier length cap (test ONLY if feasible)

The implementer will cap `sanitize_upstream_identifier` (server.rs) at a generous
length (e.g. 200) — note: a malicious upstream can put an over-long tool NAME in
its raw `tools/list` JSON regardless of rmcp's own registration limits.

Assess feasibility: can the probe-server (`tests/probe-server/main.rs`) advertise
a tool whose NAME exceeds the cap (i.e. emit an over-length identifier in its
tools/list)? If rmcp's server API lets you register/return such a name, write a
test asserting the identifier surfaced in `list_tools` is capped. If rmcp's API
prevents emitting an over-cap name (so the cap is unreachable via a real probe),
do NOT force it — record in tests.md that H-2 is a defense-in-depth cap verified
by code-inspection only, and why the probe can't exercise it.

## Everything else (no new tests — state the verification approach in tests.md)

- H-1 (mutex poison `.expect()`→recover): verified by code-inspection (grep: no
  `.expect()` on the two globals). Poisoning a global mutex from an integration
  test is not worth a fixture.
- H-4 / H-5 / H-6: refactors / startup tweak — covered by existing tests staying
  green (meta-tools test, startup/observability tests). H-6 must not change the
  `cred list` raw-stderr output the existing tests rely on — note that.
- D-2 (strike passthrough-stderr from docs), O-1/O-2/O-3, H-8: no code / docs only.

## tests.md

Per plan-format: files created (path + criteria); coverage map (which Success
Criteria get a test vs. which are inspection/existing-green-verified); deferred
tests; side-effect assertions. Be explicit that the cycle is mostly
inspection/existing-green-gated, and list exactly which criteria each new test
covers.

Return as data for the orchestrator: the test(s) you wrote, the H-2 feasibility
verdict, and any criterion you believe needs a test but you could not write.
