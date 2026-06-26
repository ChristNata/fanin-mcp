//! Manual / deferred E2E verification — NOT run in CI.
//!
//! This file holds the live-client E2E that cannot run without a real
//! Claude Code or OpenCode MCP client in the loop. The wire-level
//! `static_discovery_returns_three_meta_tools_with_exact_descriptions` test in
//! `aggregator.rs` covers the same contract programmatically (spawn the
//! binary, speak JSON-RPC, assert exactly 3 meta-tools). This file is the
//! client-side confirmation: that a real CC/OC session discovers exactly 3
//! meta-tools when it spawns fanin-mcp.
//!
//! Re-enable when a CI job with a real CC/OC MCP client harness is wired.
//! Until then, every test here is `#[ignore]` with a concrete unblock trigger.

/// Deferred: live Claude Code E2E — no CC MCP client harness available in CI.
/// The wire-level static_discovery test covers the same server-side contract;
/// this is the client-side confirmation. Run manually via
/// `cargo nextest run -- --ignored live_cc_discovers_exactly_three_meta_tools`
/// against a real CC session configured to spawn fanin-mcp.
#[ignore = "deferred: live CC E2E — no MCP client available in CI; wire-level static_discovery covers the same contract programmatically. Re-enable when a CI job with a real CC MCP client is wired (manual verification gate)."]
#[tokio::test]
async fn live_cc_discovers_exactly_three_meta_tools() {
    // This test is a placeholder for a manual verification procedure:
    // 1. Configure a real Claude Code session to use fanin-mcp as its MCP
    //    server (per CLAUDE.md / the project's MCP wiring).
    // 2. Start the session and trigger tools/list (CC does this at session
    //    start automatically).
    // 3. Capture the discovered tool names and assert they are exactly
    //    list_tools, get_tool_schema, invoke_tool — no more, no fewer.
    //
    // The assertion logic mirrors `exp::assert_exact_meta_tools`; the
    // difference is the *transport* — a real CC client, not the in-test
    // JSON-RPC harness. Because there is no CC client harness in CI, this
    // test body is intentionally a manual-run stub.
    let _ = crate::common::expectations::META_TOOL_NAMES;
    panic!("manual E2E — run manually against a live CC session; not executable in CI");
}

/// Deferred: live OpenCode E2E — no OC MCP client harness available in CI.
/// Same contract as the CC variant; OC ignores annotations (GOTCHA #10) but
/// must still discover the three meta-tools by name.
#[ignore = "deferred: live OC E2E — no MCP client available in CI; wire-level static_discovery covers the same contract programmatically. Re-enable when a CI job with a real OC MCP client is wired (manual verification gate)."]
#[tokio::test]
async fn live_oc_discovers_exactly_three_meta_tools() {
    let _ = crate::common::expectations::META_TOOL_NAMES;
    panic!("manual E2E — run manually against a live OC session; not executable in CI");
}