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

mod aggregator;
mod probe;
mod pinning;
mod manual_e2e;