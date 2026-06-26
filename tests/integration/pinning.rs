//! Pinning gate — master Success Criterion 9 (P0.1).
//!
//! Asserts `Cargo.lock` exists and `Cargo.toml` pins `rmcp` with exact
//! `=x.y.z` syntax (D-015, STACK.md). The exact-pin policy prevents
//! implementers from fighting the compiler with stale rmcp signatures and
//! makes the lockfile part of the deliverable.
//!
//! This is a pure file-content test — no process spawn — so it can run even
//! before the binary builds, but it still requires the implementer to have
//! written `Cargo.toml` and `Cargo.lock`. Until then it fails cleanly (file
//! not found) rather than erroring on a missing symbol.

use std::path::PathBuf;

/// Resolve the repo root from CARGO_MANIFEST_DIR (the crate root). Phase 0
/// is a flat single-crate project, so the crate root is the repo root.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Master criterion 9 (Pinning gate): `Cargo.lock` exists and `Cargo.toml`
/// pins `rmcp` with exact `=x.y.z` syntax. A caret/tilde/range pin or an
/// unpinned dependency fails this test.
#[test]
fn cargo_toml_pins_rmcp_exactly_and_lockfile_exists() {
    let root = repo_root();
    let cargo_toml = root.join("Cargo.toml");
    let cargo_lock = root.join("Cargo.lock");

    let toml_text = std::fs::read_to_string(&cargo_toml).unwrap_or_else(|e| {
        panic!("Cargo.toml must exist at {} (P0.1 produces it): {e}", cargo_toml.display())
    });
    assert!(
        cargo_lock.exists(),
        "Cargo.lock must exist and be committed (D-015, master criterion 9): {}",
        cargo_lock.display()
    );

    // The pin may appear in either form:
    //   rmcp = "=0.1.0"
    //   rmcp = { version = "=0.1.0", features = [...] }
    //
    // We accept either, but the version string MUST start with `=` and contain
    // at least major.minor (the convention is major.minor.patch). A bare
    // `rmcp = "0.1.0"` (caret-default) or `rmcp = "^0.1.0"` fails.
    assert!(
        contains_exact_raincp_pin(&toml_text),
        "Cargo.toml must pin rmcp with exact `=x.y.z` syntax (D-015). \
         Expected a `rmcp` dependency whose version literal starts with `=`, \
         e.g. `rmcp = \"=0.1.0\"` or `rmcp = {{ version = \"=0.1.0\", ... }}`. \
         Cargo.toml contents:\n{toml_text}"
    );
}

/// Scan Cargo.toml text for an exact-pinned rmcp dependency. Looks for either
/// the bare `rmcp = "=..."` form or the table `rmcp = { ... version = "=..." ... }`
/// form, scoped to the `[dependencies]` / `[dependencies.*]` table lines.
fn contains_exact_raincp_pin(toml: &str) -> bool {
    // Strip the cargo feature resolver section noise; we only care about lines
    // that declare the rmcp dependency. A robust-enough scan: find a line whose
    // key (before `=`) is `rmcp`, then inspect its value for an `="x.y.z"` or
    // `version = "x.y.z"` literal starting with `=`.
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Key is the segment before the first `=`, trimmed and bare (table
        // headers like `[dependencies]` have no `=` key-value shape we care
        // about; rmcp may be declared as `rmcp = ...` or quoted `"rmcp" = ...`).
        let key = match trimmed.split_once('=') {
            Some((k, _)) => k.trim().trim_matches('"'),
            None => continue,
        };
        if key != "rmcp" {
            continue;
        }
        // This is the rmcp dependency line. Look for a quoted version literal
        // starting with `=`.
        let value = trimmed.split_once('=').map(|(_, v)| v).unwrap_or("");
        if has_quoted_exact_version(value) {
            return true;
        }
    }
    false
}

/// In a dependency value string, find any quoted literal that starts with `=`
/// followed by digits (e.g. `"=0.1.0"`). Handles both the bare form
/// (`"=0.1.0"`) and the table form (`{ version = "=0.1.0", features = [...] }`).
fn has_quoted_exact_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            // Start of a quoted literal; read until the closing quote.
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j > start {
                let lit = &value[start..j];
                // Exact pin: a literal starting with `=` then a digit. We do not
                // pin a specific version here — the implementer chooses and
                // commits the lockfile; we only assert the *form* is exact.
                if let Some(rest) = lit.strip_prefix('=') {
                    if rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        return true;
                    }
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}