## LENS: GENERAL — write review-gen.md

Code/doc quality and OSS-reader polish on the change set only:

- **CONTRIBUTING.md** — is it accurate, complete, and in the project's voice? Are
  the gate commands exactly right (`cargo fmt --all -- --check`, `cargo clippy
  --all-targets -- -D warnings`, `cargo test --all`)? Anything a first-time
  external contributor would be confused or misled by? Missing anything essential
  (e.g. how to run, where the config lives)?
- **SECURITY.md** — the GitHub-Advisories instruction + the H-8 redaction-scope
  paragraph: clear and correct? Well-placed?
- **Cargo.toml** — metadata tidy, keys ordered sensibly, no typo in keywords/
  categories.
- **Src idiom** — H-1 `unwrap_or_else(|p| p.into_inner())` applied consistently
  at all sites? H-2 cap readable with a clear comment? H-5 `meta_tools()` clean
  and the `config`-field resolution sensible (no awkward `#[allow]` left)? H-6
  comment clear? H-7 cfg attributes applied tidily (not scattered/duplicated),
  imports gated cleanly?
- **Consistency** — naming, comment density matches surrounding code; no leftover
  debug, no stray TODO, no commented-out code introduced.
- **D-2 strikes** — did removing the references leave any awkward sentence,
  double space, or dangling list comma in the docs?
