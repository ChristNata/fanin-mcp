//! Credentials — OS keychain storage and the hidden stdin prompt.
//!
//! P0.1: stub. `cred` is a CLI stub only in P0.2; real keyring calls, the
//! hidden prompt, and credential persistence arrive in later phases.
//! D-010 / GOTCHA #18/#19/#22: secrets never on argv, never in logs; each
//! upstream gets only its own vars.
