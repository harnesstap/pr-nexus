//! One keychain entry, `dev.pr-nexus` / `vault`, holding a JSON map.
//!
//! If the secret service is missing, the vault stays in memory and exposes a
//! warning. It does not panic.

use std::collections::BTreeMap;
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use pr_nexus_core::error::CoreError;

const SERVICE: &str = "dev.pr-nexus";
const ACCOUNT: &str = "vault";

enum Backend {
    Keyring,
    Memory,
}

pub struct Vault {
    backend: Backend,
    warning: Mutex<Option<String>>,
    secrets: Mutex<BTreeMap<String, String>>,
}

impl Vault {
    pub fn open() -> Self {
        if std::env::var("PR_NEXUS_DISABLE_KEYRING")
            .ok()
            .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        {
            return Self::memory(
                "Keyring disabled via PR_NEXUS_DISABLE_KEYRING. Tokens stay in memory for this session.",
            );
        }
        match probe_keyring() {
            Ok(secrets) => Self {
                backend: Backend::Keyring,
                warning: Mutex::new(None),
                secrets: Mutex::new(secrets),
            },
            Err(reason) => Self::memory(&format!(
                "Keyring unavailable ({reason}). Tokens stay in memory for this session and are lost when the app quits."
            )),
        }
    }

    pub fn memory(warning: &str) -> Self {
        Self {
            backend: Backend::Memory,
            warning: Mutex::new(Some(warning.to_owned())),
            secrets: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn warning(&self) -> Option<String> {
        self.warning.lock().ok().and_then(|guard| guard.clone())
    }

    pub fn available(&self) -> bool {
        matches!(self.backend, Backend::Keyring)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.secrets()?.get(key).cloned())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        {
            let mut secrets = self.secrets()?;
            secrets.insert(key.to_owned(), value.to_owned());
        }
        self.persist()
    }

    pub fn remove(&self, key: &str) -> Result<(), CoreError> {
        {
            let mut secrets = self.secrets()?;
            secrets.remove(key);
        }
        self.persist()
    }

    fn secrets(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, String>>, CoreError> {
        self.secrets
            .lock()
            .map_err(|_| CoreError::Store("vault lock poisoned".into()))
    }

    fn persist(&self) -> Result<(), CoreError> {
        if !matches!(self.backend, Backend::Keyring) {
            return Ok(());
        }
        let json = {
            let secrets = self.secrets()?;
            serde_json::to_string(&*secrets)
                .map_err(|err| CoreError::Store(format!("vault encode: {err}")))?
        };
        match write_keyring(&json) {
            Ok(()) => Ok(()),
            Err(reason) => {
                if let Ok(mut warning) = self.warning.lock() {
                    *warning = Some(format!(
                        "Keyring write failed ({reason}). The token is only in memory."
                    ));
                }
                Ok(())
            }
        }
    }
}

fn probe_keyring() -> Result<BTreeMap<String, String>, String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(read_keyring());
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(_) => Err("timed out after 2s".into()),
    }
}

fn read_keyring() -> Result<BTreeMap<String, String>, String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|err| err.to_string())?;
    match entry.get_password() {
        Ok(json) => serde_json::from_str(&json).map_err(|err| err.to_string()),
        Err(keyring::Error::NoEntry) => Ok(BTreeMap::new()),
        Err(err) => Err(err.to_string()),
    }
}

fn write_keyring(json: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|err| err.to_string())?;
    entry.set_password(json).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_vault_round_trips_without_a_keyring() {
        let vault = Vault::memory("test");
        assert!(!vault.available());
        assert!(vault.warning().unwrap().contains("test"));
        vault.set("github:U_1", "secret").unwrap();
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("secret"));
        vault.remove("github:U_1").unwrap();
        assert_eq!(vault.get("github:U_1").unwrap(), None);
    }
}
