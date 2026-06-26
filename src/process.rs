//! Upstream process lifecycle — spawning, the process-tree death guarantee,
//! and stderr capture.
//!
//! P0.1: stub. Later phases implement the D-009 / GOTCHA #11/#14 process-tree
//! lifetime: every upstream lives in a Windows Job Object / Unix process
//! group so hard-kill leaves zero orphans.