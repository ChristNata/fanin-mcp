//! Upstream registry — maps a server name to its lazy `RunningService`.
//!
//! P0.1: stub. Later phases implement `Registry::get_or_connect()` with the
//! D-007 / GOTCHA #16 lock discipline: lock the map only to get/insert an
//! `Arc<RunningService>`, clone the Arc, drop the lock, then await the
//! upstream call. A lock held across `call_tool` serializes the session.