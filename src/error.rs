//! Error model.
//!
//! D-005 / GOTCHA #3: tool-level errors are `CallToolResult { isError: true }`,
//! never JSON-RPC errors; the structured-error JSON shape is public API. The
//! `NotImplemented` text is the readable content the LLM reasons about — it is
//! not part of the public wire contract (the exact wording is free to change),
//! but the *shape* (a text content block + `isError: true`) is fixed.

/// A tool-level failure surfaced to the caller as structured content.
///
/// Produces a text content block inside `CallToolResult { isError: true }`.
#[derive(Debug, Clone)]
pub enum ToolError {
    /// The meta-tool surface is declared but the forward path is not wired yet.
    ///
    /// Phase 0 ships the three static meta-tools and returns this for every
    /// `tools/call`. Real proxying (registry, forward, process) lands in
    /// Phase 1.
    NotImplemented { tool: String },
}

impl ToolError {
    /// Render the error as the human-readable text a content block carries.
    pub fn message(&self) -> String {
        match self {
            ToolError::NotImplemented { tool } => {
                format!(
                    "tool `{tool}` is not implemented in this build of fanin-mcp; \
                     upstream proxying is not wired yet"
                )
            }
        }
    }
}
