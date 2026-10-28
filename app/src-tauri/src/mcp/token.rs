//! The bearer token MCP clients send. Lives in the OS keyring (the app's
//! [`Vault`]), never in the database or logs.

use pr_nexus_providers::vault::Vault;
use subtle::ConstantTimeEq;

use crate::error::AppError;

/// Vault key of the token.
pub const VAULT_KEY: &str = "mcp.token";
const PREFIX: &str = "pnx_";

/// A fresh token: `pnx_` and 64 hex digits (256 random bits).
pub fn generate() -> String {
    let bytes: [u8; 32] = rand::random();
    let mut out = String::with_capacity(PREFIX.len() + 64);
    out.push_str(PREFIX);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The saved token, creating one on first use.
pub fn ensure(vault: &Vault) -> Result<String, AppError> {
    if let Some(token) = vault.get(VAULT_KEY)?.filter(|token| !token.is_empty()) {
        return Ok(token);
    }
    regenerate(vault)
}

/// Replaces the token: clients using the old one are rejected from now on.
pub fn regenerate(vault: &Vault) -> Result<String, AppError> {
    let token = generate();
    vault.set(VAULT_KEY, &token)?;
    Ok(token)
}

/// What Settings shows: the prefix and the last four characters.
pub fn masked(token: &str) -> String {
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{PREFIX}{}{tail}", "•".repeat(12))
}

/// Constant-time check of an `Authorization` header value.
pub fn authorized(header: Option<&str>, expected: &str) -> bool {
    let Some(given) = header.and_then(|value| value.strip_prefix("Bearer ")) else {
        return false;
    };
    !expected.is_empty() && bool::from(given.trim().as_bytes().ct_eq(expected.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_random_and_masked() {
        let token = generate();
        assert_eq!(token.len(), 68);
        assert_ne!(token, generate());
        let shown = masked(&token);
        assert!(shown.starts_with("pnx_"));
        assert!(shown.ends_with(&token[64..]));
        assert!(!shown.contains(&token[4..60]));
    }

    #[test]
    fn only_the_exact_bearer_token_is_accepted() {
        assert!(authorized(Some("Bearer pnx_abc"), "pnx_abc"));
        assert!(!authorized(Some("Bearer pnx_abd"), "pnx_abc"));
        assert!(!authorized(Some("Bearer pnx_ab"), "pnx_abc"));
        assert!(!authorized(Some("pnx_abc"), "pnx_abc"));
        assert!(!authorized(None, "pnx_abc"));
        assert!(!authorized(Some("Bearer "), ""));
    }

    #[test]
    fn regenerate_replaces_the_saved_token() {
        let vault = Vault::memory("test");
        let first = ensure(&vault).unwrap();
        assert_eq!(ensure(&vault).unwrap(), first);
        let second = regenerate(&vault).unwrap();
        assert_ne!(first, second);
        assert_eq!(ensure(&vault).unwrap(), second);
    }

    /// The token is one more key in the app's single vault entry, so turning
    /// MCP on adds no keychain item and no extra prompt.
    #[test]
    fn the_token_lives_in_the_existing_vault_entry() {
        use pr_nexus_providers::vault::{KeyringBackend, KeyringError};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        #[derive(Clone, Default)]
        struct OneEntry(Arc<Mutex<Option<String>>>);
        impl KeyringBackend for OneEntry {
            fn read(&self) -> Result<Option<String>, KeyringError> {
                Ok(self.0.lock().unwrap().clone())
            }
            fn write(&self, json: &str) -> Result<(), KeyringError> {
                *self.0.lock().unwrap() = Some(json.to_owned());
                Ok(())
            }
        }

        let entry = OneEntry(Arc::new(Mutex::new(Some(
            r#"{"github.token":"gho_acme"}"#.into(),
        ))));
        let vault = Vault::with_backend(Box::new(entry.clone()), Duration::from_secs(5));
        let first = ensure(&vault).unwrap();
        let second = regenerate(&vault).unwrap();
        let wait = std::time::Instant::now();
        while !entry
            .0
            .lock()
            .unwrap()
            .as_deref()
            .unwrap_or("")
            .contains(&second)
        {
            assert!(wait.elapsed() < Duration::from_secs(5), "token never saved");
            std::thread::sleep(Duration::from_millis(10));
        }
        let stored: serde_json::Value =
            serde_json::from_str(entry.0.lock().unwrap().as_deref().unwrap()).unwrap();
        assert_eq!(stored["github.token"], "gho_acme");
        assert_eq!(stored[VAULT_KEY], second.as_str());
        assert!(!stored.to_string().contains(&first));
    }
}
