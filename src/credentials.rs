//! Credentials — OS keychain storage and the hidden stdin prompt.
//!
//! Phase 1 of the credential plan:
//! - `CredentialStore` trait (get/set/delete/list_names) server-scoped.
//! - `KeyringStore` backed by the `keyring` crate (service = "fanin-mcp/<server>").
//! - `EnvStore` read-only process-env fallback (flat, server ignored on get).
//! - Resolution chain for get: preferred store → env fallback → None.
//! - `cred set` always uses hidden prompt (`rpassword`); never accepts value on argv.
//! - `cred list` emits names only (never values).
//! - `cred rm` deletes from the mutable backend (keyring when selected; env set is no-op).
//!
//! `--credential-store` selects preferred (keyring|env); env fallback is always
//! tried for reads. D-010, GOTCHA #18/#19/#22. GOTCHA #1 stdout discipline
//! observed (cred paths are outside serve).

use std::collections::HashSet;
use std::io::BufRead;

use keyring::Entry;
use serde_json::json;

/// Selectable preferred backend for the credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum CredentialStoreChoice {
    #[default]
    Keyring,
    Env,
}

/// Abstraction over a credential backend.
/// All operations are scoped to a server (the service id is "fanin-mcp/<server>").
///
/// `get` is invoked at spawn time by the registry to resolve `${VAR}` env
/// placeholders (preferred backend → process-env fallback → structured error);
/// `set` / `delete` / `list_names` back the `cred set | rm | list` subcommands.
#[allow(dead_code)]
pub trait CredentialStore: Send + Sync {
    /// Retrieve a credential value if present.
    fn get(&self, server: &str, key: &str) -> Result<Option<String>, String>;

    /// Store or overwrite a credential value.
    fn set(&self, server: &str, key: &str, value: &str) -> Result<(), String>;

    /// Delete a credential if present. Idempotent success if absent.
    fn delete(&self, server: &str, key: &str) -> Result<(), String>;

    /// List known credential names for the server (never values).
    fn list_names(&self, server: &str) -> Result<Vec<String>, String>;
}

/// Keyring-backed implementation.
/// Uses `fanin-mcp/<server>` as the service, `<KEY>` as the user name.
#[derive(Debug, Default)]
pub struct KeyringStore;

impl KeyringStore {
    fn service(server: &str) -> String {
        format!("fanin-mcp/{}", server)
    }

    fn entry(server: &str, key: &str) -> Result<Entry, String> {
        Entry::new(&Self::service(server), key)
            .map_err(|e| format!("keyring entry error for {}/{}: {}", server, key, e))
    }

    /// Index entry that stores a JSON array of known keys for the server.
    fn index_entry(server: &str) -> Result<Entry, String> {
        Entry::new(&Self::service(server), "__CRED_KEYS__")
            .map_err(|e| format!("keyring index error for {}: {}", server, e))
    }

    fn load_index(&self, server: &str) -> HashSet<String> {
        match Self::index_entry(server).and_then(|e| e.get_password().map_err(|e| e.to_string())) {
            Ok(s) => {
                if let Ok(arr) = serde_json::from_str::<Vec<String>>(&s) {
                    arr.into_iter().collect()
                } else {
                    HashSet::new()
                }
            }
            Err(_) => HashSet::new(),
        }
    }

    fn save_index(&self, server: &str, keys: &HashSet<String>) -> Result<(), String> {
        let e = Self::index_entry(server)?;
        let arr: Vec<&String> = keys.iter().collect();
        let s = json!(arr).to_string();
        e.set_password(&s)
            .map_err(|e| format!("failed to save keyring index for {}: {}", server, e))
    }
}

impl CredentialStore for KeyringStore {
    fn get(&self, server: &str, key: &str) -> Result<Option<String>, String> {
        let e = Self::entry(server, key)?;
        match e.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keyring get failed for {}/{}: {}", server, key, e)),
        }
    }

    fn set(&self, server: &str, key: &str, value: &str) -> Result<(), String> {
        let e = Self::entry(server, key)?;
        e.set_password(value)
            .map_err(|e| format!("keyring set failed for {}/{}: {}", server, key, e))?;

        let mut idx = self.load_index(server);
        idx.insert(key.to_string());
        self.save_index(server, &idx)?;
        Ok(())
    }

    fn delete(&self, server: &str, key: &str) -> Result<(), String> {
        if let Ok(e) = Self::entry(server, key) {
            let _ = e.delete_credential();
        }
        let mut idx = self.load_index(server);
        idx.remove(key);
        let _ = self.save_index(server, &idx);
        Ok(())
    }

    fn list_names(&self, server: &str) -> Result<Vec<String>, String> {
        let idx = self.load_index(server);
        let mut v: Vec<String> = idx.into_iter().collect();
        v.sort();
        Ok(v)
    }
}

/// Read-only env fallback.
/// get(server, key) == std::env::var(key).ok()
/// set/delete/list are no-ops.
#[derive(Debug, Default)]
pub struct EnvStore;

impl CredentialStore for EnvStore {
    fn get(&self, _server: &str, key: &str) -> Result<Option<String>, String> {
        match std::env::var(key) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Ok(None),
        }
    }

    fn set(&self, _server: &str, _key: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete(&self, _server: &str, _key: &str) -> Result<(), String> {
        Ok(())
    }

    fn list_names(&self, _server: &str) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
}

/// Build the preferred store for the given choice.
pub fn build_store(choice: CredentialStoreChoice) -> Box<dyn CredentialStore> {
    match choice {
        CredentialStoreChoice::Keyring => Box::new(KeyringStore),
        CredentialStoreChoice::Env => Box::new(EnvStore),
    }
}

/// Read a secret from a hidden prompt on stdin (no echo).
///
/// - If stdin is a terminal (interactive), use rpassword for hidden input.
/// - Otherwise (piped stdin in tests/CI), read a single line directly.
///   The secret is never echoed to stdout/stderr.
pub fn prompt_for_secret(prompt: &str) -> Result<String, String> {
    // Bring the IsTerminal trait into scope so is_terminal() is available.
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        // Interactive: truly hidden prompt.
        rpassword::prompt_password(prompt)
            .map_err(|e| format!("failed to read secret from prompt: {}", e))
            .map(|s| s.trim_end_matches(&['\r', '\n'][..]).to_string())
    } else {
        // Piped / non-interactive: read the provided line. Do not print prompt
        // to avoid polluting captured stderr in tests; secret itself must not leak.
        let mut line = String::new();
        let mut r = stdin.lock();
        r.read_line(&mut line)
            .map_err(|e| format!("failed to read secret from stdin: {}", e))?;
        Ok(line.trim_end_matches(&['\r', '\n'][..]).to_string())
    }
}
