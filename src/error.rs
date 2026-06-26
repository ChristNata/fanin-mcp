//! Error model — the structured-error JSON shape that is public API.
//!
//! P0.2: a single error variant for the not-implemented tool result. Later
//! phases add upstream-routing, transport, and config error variants.
//!
//! D-005 / GOTCHA #3: tool-level errors are `CallToolResult { isError: true }`,
//! never JSON-RPC errors; the structured-error JSON shape is public API. The
//! `NotImplemented` text is the readable content the LLM reasons about — it is
//! not part of the public wire contract (the exact wording is free to change),
//! but the *shape* (a text content block + `isError: true`) is fixed.

/// A tool-level failure surfaced to the caller as structured content.
///
/// Phase 0 has exactly one failure mode: every meta-tool call (and any unknown
/// tool name) returns [`ToolError::NotImplemented`] wrapped in a
/// `CallToolResult { isError: true }`. The variant carries a human-readable
/// message that becomes a text content block; the model reads it and can
/// decide to stop or retry. This is deliberately not a `thiserror` enum yet —
/// there is no source error to chain and no caller that pattern-matches on
/// the variant. Later phases replace this with the structured error JSON
/// (server / tool / code / message / recoverable) per D-005.
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