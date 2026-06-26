# Fix Summary — phase-2-multi-namespace review findings

**Plan:** `docs/master-plans/phase-2-multi-namespace/master.md`
**Source review:** `docs/master-plans/phase-2-multi-namespace/review-general.md`
(1 targeted, 2 trivial; alignment + adversarial passes at 0 findings.)
**Scope:** `src/config.rs`, `src/error.rs`, `src/namespace.rs`. No registry,
forward, process, credential, server, or test changes. No Phase 3/4 surface.

## Finding 1 — fail-fast on a typo'd `tools.<server>` key

**Defect.** `TomlConfig::validate` (the existing 4-step startup validator)
never looked at the `[namespaces.<name>.tools]` map. A misspelled
`tools.<server>` key — one not present in that namespace's `servers`
allow-list — was silently ignored, and because an allowed server with no
matching `tools` entry exposes *all* its tools, the intended restriction
failed open with zero startup signal.

**Root cause.** Validate step (4) only confirmed the *active* namespace
exists; nothing checked the structural relationship between each
namespace's `servers` allow-list and its `tools` map. There was no
`StartupError` variant for this either, so the failure mode had no
typed representation.

**Fix applied.**

- `src/error.rs`: added `StartupError::ToolFilterUnknownServer
  { namespace, server }` with a `Display` impl that names both the
  namespace and the offending server key and explains the silent-fail-
  open risk that motivates fail-fast.
- `src/config.rs::TomlConfig::validate`: added a new step (5) that
  iterates every `NamespaceConfig` (not just the active one — a
  malformed config must fail regardless of which namespace is selected)
  and rejects any `ns.tools` key that is not in that namespace's
  `ns.servers` list. Tool *names* remain unvalidated at startup —
  discovery is lazy and the deferred-validation choice is documented
  in the comment and is consistent with `tests.md` ("MAY validate
  that tool-filter server keys reference a server in `servers`").
- `NamespaceConfig.tools` keeps the `HashMap<String, Vec<String>>`
  shape for TOML deserialization; the runtime conversion happens
  in `ActiveNamespace::new` (Finding 3).

No suggested-fix divergence from the reviewer: the suggested remedy
("reject each `tools` key not in `servers`; include namespace and
server in the error; defer tool-name validation") is what was applied.

## Finding 2 — module schema doc omits the tools table

**Defect.** The module-level "binding" TOML schema comment at the top
of `src/config.rs` showed `[namespaces.<name>] servers = [...]` but
omitted the new `[namespaces.<name>.tools]` sub-table, leaving a
reader (and a future reviewer) with the impression Phase 2 did not add
it.

**Fix applied.** Added the `tools` sub-table example right next to
`servers = [...]` with a one-line note that an absent entry for an
allowed server means all its tools are visible (the contract the
runtime enforces):

```toml
[namespaces.<name>.tools]    # optional per-server name-level allow-list
<server> = ["<tool>", ...]   # absent entry for an allowed server => all its tools visible
```

## Finding 3 — allow-list membership type

**Defect.** `ActiveNamespace::tools` was
`HashMap<String, Vec<String>>` and `is_tool_allowed` linear-scanned the
`Vec` on every call. The data is only used for membership; the linear
shape communicates the wrong semantics and costs repeated scans during
`list_tools` filtering.

**Fix applied.**

- `src/namespace.rs::ActiveNamespace::tools` is now
  `HashMap<String, HashSet<String>>`. Documented as O(1) membership
  for `list_tools` filtering.
- `ActiveNamespace::new` converts `NamespaceConfig.tools` (still
  `HashMap<String, Vec<String>>` for TOML) into the HashSet shape on
  construction.
- `is_tool_allowed` now uses `HashSet::contains` instead of
  `Vec::iter().any(|t| t == tool)`. The `Option::map_or(true, …)`
  shape (no tool entry => all visible) is preserved.
- The `HashSet` import was already in scope in `namespace.rs`; no new
  imports needed.

No suggested-fix divergence.

## Verification

All three plan-scope files modified, no test files touched (`tests/`
untouched; `cargo test` results identical to the pre-fix baseline).
Run from `C:\Users\Chrisyian\RustroverProjects\fanin-mcp`:

- `cargo build --all-targets` — clean.
- `cargo test` — **67 passed; 0 failed; 2 ignored** (matches pre-fix
  baseline: `tests/integration/namespace_acl.rs` tool-level ACL test
  uses `with_tools("alpha", …)` after `NamespaceEntry::new("filtered",
  ["alpha", "beta"])`, so the new fail-fast validator accepts every
  committed test config).
- `cargo fmt --all -- --check` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.

## Out-of-scope issues spotted (surfaced, not fixed)

None that are in-scope for this fix run. Two adjacent observations
worth flagging upstream for the orchestrator to route, not for a
silent edit:

- **`tests.md` language drift (trivial, doc-only).** `tests.md` line 88
  reads "The implementer MAY validate that tool-filter server keys
  reference a server in `servers` (fail-fast on a typo), but the
  tests do not require it." Now that the validator is implemented and
  in the binding contract, that line is stale. It is in the tests
  document, not in source, so it does not affect the gate; but a
  future reader will be told the validator is optional when it is
  actually mandatory. Recommend a one-line update in a docs phase.
- **`NamespaceConfig.tools` doc comment (trivial, docs-only).** The
  per-struct doc at `src/config.rs` line 116–119 says "Keys are server
  names (should be in `servers`)". With fail-fast validation now in
  place, "should be" understates the contract — keys *must* be in
  `servers`. Recommend a follow-up docs pass (Phase 3 or a Phase 4
  scope-cleanup touch) to tighten the wording; not fixed here because
  the change crosses the boundary into docs and the failing behavior
  was already impossible after Finding 1.

Neither item blocks the gate; both are surfaced for routing.
