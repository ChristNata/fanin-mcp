# Review: schema-relay-fidelity

## Verdict

FAIL

Found 1 blocker, 0 structural, 0 targeted, 0 trivial.

## Verification

- `git diff v0.6.31..HEAD` could not resolve because the local repo has no
  `v0.6.31` tag. Reviewed the committed equivalent `bb39cac..HEAD`, where
  `bb39cac` is `v0.6.31 - plan stage - schema-relay-fidelity`.
- `cargo test --all` passed: 5 unit tests and 131 integration tests passed;
  5 integration tests ignored.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- Scope review: product changes are limited to `src/server.rs`,
  `tests/integration/sanitization.rs`, `tests/probe-server/main.rs`,
  `SECURITY.md`, and `docs/GOTCHA.md`. Pipeline artifacts also changed.

## Alignment and adversarial notes

- The new full-length bite test genuinely bites the old cap: it asserts exact
  equality for `properties.long_clean.description` against a clean 235-char
  fixture and separately checks the distinctive tail past `DESC_CAP`.
- The new invoke BEL test genuinely bites D-004 argument mutation: it asserts
  the returned text is exactly `"wei\u{0007}rd"`, not merely success/no-error.
- `src/server.rs` now routes `list_tools` rows through
  `sanitize_list_row_description` and `get_tool_schema` annotation strings
  through `neutralize_upstream_display`, so the row cap is decoupled from
  full schema annotations.
- D-004 is intact in the implementation: `invoke_tool` clones the raw
  arguments object and forwards it to `registry.call_tool`; result content is
  returned without sanitizer/stringification changes. Schema validation values
  are preserved by the annotation-key allowlist.
- `SECURITY.md` and `docs/GOTCHA.md` accurately state the row-only cap,
  full-length `get_tool_schema` annotations, and verbatim invoke channels.

## Findings

- File: `tests/integration/sanitization.rs:136`, `tests/integration/sanitization.rs:463`, `tests/integration/sanitization.rs:470`, `tests/integration/sanitization.rs:477`, `tests/probe-server/main.rs:524`
  Severity: blocker
  Pass:     alignment + adversarial
  What:     The `get_tool_schema` display-safety criterion is not genuinely
            tested for the full forbidden set. The schema annotation tests use
            `assert_no_control_chars`, which checks only C0 plus DEL, and the
            `poison_schema` annotation fixture only carries C0-style controls.
            A schema-specific bug that neutralizes `\n`/C0 but leaks C1,
            U+2028/U+2029, bidi controls, or zero-width characters in
            `get_tool_schema` annotations would keep this suite green.
  Why:      Master SC 2 and Phase 2 SC 2 require `get_tool_schema` annotation
            strings to be single-line and free of the existing forbidden
            display controls, not just C0/DEL. This sanitization area was
            previously test-gamed; list-row F1 coverage proves the broader set
            for `list_tools`, but it does not prove the schema path. The code
            currently appears correct because both paths share
            `neutralize_upstream_display`, but the acceptance evidence does
            not bite if that path diverges.
  Cite:     `master.md` Success Criterion 2; Phase 2 Success Criterion 2;
            reviewer fakery checklist — tests shaped to pass / side effects
            not checked; `rmcp-general` passthrough fidelity and GOTCHA #20.
  Fix:      Add a `get_tool_schema` fixture annotation with C1, Unicode
            separators, bidi, BOM, and zero-width characters in at least one
            annotation key (`title`, `description`, `$comment`, or
            `markdownDescription`) and assert none survive in the returned
            schema. Prefer a shared helper that checks the same forbidden set
            as `F1_FORBIDDEN_CODEPOINTS`, while keeping validation strings and
            invoke arguments verbatim.
  Routing:  Gate. This is an unverified success criterion in a security-adjacent
            sanitization contract.

## Out-of-scope observations

(none)

## Confidence

Not launch-ready until the schema-annotation forbidden-control coverage bites;
the implementation itself looks aligned once that evidence gap is closed.
