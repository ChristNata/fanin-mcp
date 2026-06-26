//! Canonical Phase 0 expectations shared across aggregator tests.
//!
//! The static meta-tool descriptions are FINAL design, not a temporary stub
//! (master.md §Required Pattern, D-003). Asserting them in one place keeps the
//! contract visible and prevents a later implementation from silently
//! substituting placeholder text — any divergence here is a spec conflict to
//! surface, not a silent rewrite.

/// Exact descriptions from master.md §Required Pattern. Changing these is a
/// SemVer-major break (ARCHITECTURE.md §Versioning).
pub const LIST_TOOLS_DESC: &str = "Lists the tools available through this aggregator, grouped by server, with one-line descriptions. Call this once to see what's connected; pass server to fetch a single server's tools.";
pub const GET_TOOL_SCHEMA_DESC: &str =
    "Get the full input schema for a tool. Format: server__tool (e.g. postgres__query).";
pub const INVOKE_TOOL_DESC: &str = "Call a tool by server__tool name with arguments.";

/// The exact, ordered set of meta-tool names the aggregator exposes.
pub const META_TOOL_NAMES: [&str; 3] = ["list_tools", "get_tool_schema", "invoke_tool"];

/// Look up a tool definition in a tools/list result by name.
pub fn find_tool<'a>(tools: &'a [serde_json::Value], name: &str) -> Option<&'a serde_json::Value> {
    tools
        .iter()
        .find(|t| t.get("name").and_then(|n| n.as_str()) == Some(name))
}

/// Assert the three meta-tools are present exactly, by name, in any order.
/// Returns the tools slice for further per-tool assertions.
pub fn assert_exact_meta_tools(tools: &[serde_json::Value]) {
    assert_eq!(
        tools.len(),
        3,
        "tools/list must return exactly 3 meta-tools, got {}: {tools:?}",
        tools.len()
    );
    for name in META_TOOL_NAMES {
        assert!(
            find_tool(tools, name).is_some(),
            "tools/list is missing the `{name}` meta-tool"
        );
    }
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    let mut expected = META_TOOL_NAMES.to_vec();
    expected.sort();
    assert_eq!(
        sorted, expected,
        "tools/list returned unexpected tool names: {names:?}"
    );
}

/// Assert a tool's `description` field matches the expected string exactly.
pub fn assert_desc(tool: &serde_json::Value, expected: &str) {
    let desc = tool
        .get("description")
        .unwrap_or_else(|| panic!("tool `{}` missing description", tool_name(tool)));
    let got = desc
        .as_str()
        .unwrap_or_else(|| panic!("tool description is not a string: {desc}"));
    assert_eq!(
        got,
        expected,
        "tool `{}` description mismatch",
        tool_name(tool)
    );
}

fn tool_name(tool: &serde_json::Value) -> String {
    tool.get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("<unknown>")
        .to_string()
}
