//! Upstream process lifecycle — spawning, the process-tree death guarantee,
//! and stderr capture.
//!
//! Stub module. Future spawning must preserve the D-009 / GOTCHA #11/#14
//! process-tree lifetime: every upstream lives in a Windows Job Object / Unix
//! process group so hard-kill leaves zero orphans.
