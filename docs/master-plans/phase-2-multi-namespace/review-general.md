# General Review: phase-2-multi-namespace

Found 0 blocker, 0 structural, 1 targeted, 2 trivial.

Suite: `cargo test --workspace` passed (67 passed, 2 ignored); `cargo clippy
--workspace --all-targets -- -D warnings` passed; `cargo fmt --all -- --check`
passed.

## Findings

- File: `src/config.rs:145`
  Severity: targeted
  Pass: general
  What: `TomlConfig::validate` accepts `[namespaces.<name>.tools]` keys that are
  not present in that namespace's `servers` allow-list.
  Why: A misspelled tool-filter key is silently ignored by `ActiveNamespace`.
  Because an allowed server with no matching `tools` entry exposes all tools,
  this turns a config typo into a silent broadening of visibility and leaves a
  future maintainer with no startup signal.
  Cite: project rule: fail-fast startup validation; rust-general §Error
  handling (surface invalid state instead of silently recovering).
  Fix: During namespace validation, reject each `tools` key that is not also in
  `NamespaceConfig.servers` and include the namespace/server name in the startup
  error. Keep tool-name validation deferred until discovery.

- File: `src/config.rs:10`
  Severity: trivial
  Pass: general
  What: The module-level "binding" TOML schema omits the new
  `[namespaces.<name>.tools]` table.
  Why: The lower struct docs describe the tool allow-list, but the top schema is
  the first maintenance anchor and says the fixture encodes the exact shape. A
  reader can wrongly conclude Phase 2 did not add the table.
  Cite: md-authoring §First line / docs as retrieval anchors; rust-general §Doc
  conventions.
  Fix: Add the `tools` table example next to `servers = [...]`.

- File: `src/namespace.rs:18`
  Severity: trivial
  Pass: general
  What: `ActiveNamespace` stores exact-match tool allow-lists as `Vec<String>`
  and scans them on every `is_tool_allowed` call.
  Why: The config shape naturally deserializes as lists, and the per-session
  clone is acceptable for expected config sizes. Once inside the active ACL,
  though, the data is used only for membership. A `HashSet<String>` better
  communicates exact allow-list semantics and avoids repeated linear scans while
  `list_tools` filters discovered tools.
  Cite: rust-general §Type-level patterns / collection choice by use; rust-review
  §Type and lifetime drift.
  Fix: Keep `NamespaceConfig.tools` as `HashMap<String, Vec<String>>` for TOML,
  but convert to `HashMap<String, HashSet<String>>` in `ActiveNamespace::new`.

## Verdict

PASS with targeted cleanup recommended.
