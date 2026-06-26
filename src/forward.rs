//! Forward path — translates a downstream meta-tool call into an upstream
//! `call_tool` and returns the result byte-faithfully.
//!
//! P0.1: stub. Later phases implement the `server__tool` routing and the
//! D-004 / GOTCHA #4 byte-faithful content-array pass-through (never
//! `to_string()` a content array — it corrupts images/resources).