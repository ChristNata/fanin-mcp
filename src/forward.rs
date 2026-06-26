//! Forward path — translates a downstream meta-tool call into an upstream
//! `call_tool` and returns the result byte-faithfully.
//!
//! Stub module. Future forwarding must preserve the D-004 / GOTCHA #4
//! byte-faithful content-array pass-through: never `to_string()` a content
//! array because it corrupts images/resources.
