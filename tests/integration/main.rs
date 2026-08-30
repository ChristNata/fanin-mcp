//! Phase 0 integration test binary.
//!
//! One top-level integration crate (per rust-test layout) with submodules
//! declared here. Each submodule maps to a subset of the master Success
//! Criteria — see `tests.md` for the coverage map.
//!
//! Wire-level tests spawn the built binary and speak raw JSON-RPC over stdio
//! (D-015). Unit-level tests are avoided where a wire check suffices; the
//! only pure file-content test is the pinning gate (criterion 9).

// `common` lives at `tests/common/mod.rs` (the rust-test convention), one
// level up from this crate root at `tests/integration/main.rs`. The `#[path]`
// attribute resolves it; without it, `mod common;` would look for
// `tests/integration/common.rs` and fail to compile.
#[path = "../common/mod.rs"]
mod common;

// Compile the production config model into this integration crate so config
// data-model tests can assert deserialized fields directly. This binary-only
// package has no library target to import.
#[path = "../../src/config.rs"]
mod config_model;
#[path = "../../src/credentials.rs"]
mod credentials;
#[path = "../../src/error.rs"]
mod error;

mod advertisement;
mod aggregator;
mod capability_cache;
mod check;
mod config;
mod cred_store;
mod discovery;
mod error_hardening;
mod gate;
mod http_upstream;
mod invoke;
mod list_changed;
mod literal_header_redaction;
mod manual_e2e;
mod multi_upstream;
mod namespace_acl;
mod namespace_compose;
mod observability;
mod phase4_guard;
mod pinning;
mod probe;
mod process_lifetime;
mod registry;
mod regression_guard;
mod remediation_s1_d1;
mod reverse_traffic;
mod sanitization;
mod timeout_cancellation;
mod token_figures;
mod tool_search;
