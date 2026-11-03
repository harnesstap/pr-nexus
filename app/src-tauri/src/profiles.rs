//! Profiles (§8.4): one active profile at a time, each with its own
//! database and keychain scope. The default profile keeps the database it
//! had before profiles existed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};

use chrono::Utc;
use pr_nexus_core::profiles::{self as core, Profile, ProfileDirs, Registry, ReuseItems};
use pr_nexus_core::store::Store;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// The store of the active profile. Derefs to [`Store`], so code written
/// before profiles keeps reading `state.store`. Stores are opened once per
/// profile and kept for the session.
pub struct StoreSlot {
    current: RwLock<&'static Store>,
    home: &'static Store,
}

impl StoreSlot {
    pub fn new(current: &'static Store, home: &'static Store) -> Self {
        Self {
            current: RwLock::new(current),
            home,
        }
    }

    /// The active store, valid for the whole session.
    pub fn get(&self) -> &'static Store {
        *self.current.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The default profile's store, which also holds app-wide settings
    /// (the MCP server's switch, port and capabilities).
    pub fn home(&self) -> &'static Store {
        self.home
    }

    pub fn is_home(&self) -> bool {
        std::ptr::eq(self.get(), self.home)
    }

    pub(crate) fn set(&self, store: &'static Store) {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = store;
    }
}

impl std::ops::Deref for StoreSlot {
    type Target = Store;

    fn deref(&self) -> &Store {
        self.get()
    }
}

/// The registry, the profile stores, and the switch generation.
pub struct ProfileManager {
    pub dirs: ProfileDirs,
    registry: Mutex<Registry>,
    stores: Mutex<HashMap<PathBuf, &'static Store>>,
    /// Bumped on every switch; a sync started under an older value stops.
    generation: AtomicU64,
}

/// A sync stopped by a profile switch. Unwound with `resume_unwind`, so the
/// panic hook does not log it as a crash.
pub struct Cancelled;

impl ProfileManager {
    /// Reads or creates the registry next to `default_db` (idempotent).
    pub fn open(default_db: PathBuf) -> Result<Self, AppError> {
        let dirs = ProfileDirs::new(default_db);
        let registry = dirs.migrate(Utc::now())?;
        Ok(Self {
            dirs,
            registry: Mutex::new(registry),
            stores: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
        })
    }

    pub fn registry(&self) -> Registry {
        self.registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn bump(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// The store of profile `id`, opened on first use.
    pub fn store(&self, id: &str) -> Result<&'static Store, AppError> {
        self.store_at(&self.dirs.database(id))
    }

    pub fn store_at(&self, path: &Path) -> Result<&'static Store, AppError> {
        let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(store) = stores.get(path) {
            return Ok(store);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::new("internal", format!("create {}: {err}", parent.display()))
            })?;
        }
        let store: &'static Store = Box::leak(Box::new(Store::open(path)?));
        stores.insert(path.to_owned(), store);
        Ok(store)
    }

    /// Finds a profile by id or name (case-insensitive).
    pub fn resolve(&self, key: &str) -> Result<Profile, AppError> {
        self.registry()
            .resolve(key.trim())
            .cloned()
            .ok_or_else(|| AppError::new("not_found", format!("No profile named \"{key}\"")))
    }

    fn update<T>(
        &self,
        change: impl FnOnce(&mut Registry) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut registry = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = registry.clone();
        let out = change(&mut next)?;
        self.dirs.save(&next)?;
        *registry = next;
        Ok(out)
    }

    pub fn set_active(&self, id: &str) -> Result<(), AppError> {
        self.update(|registry| {
            if registry.get(id).is_none() {
                return Err(AppError::new("not_found", "No such profile"));
            }
            registry.active = id.to_owned();
            Ok(())
        })
    }

    /// Adds a profile and copies `reuse` from `sources` into it. Returns the
    /// profile and, per source, the keychain keys to copy.
    pub fn create(
        &self,
        name: &str,
        reuse: &[ReuseFrom],
    ) -> Result<(Profile, SecretCopies), AppError> {
        let profile = self.update(|registry| {
            let name = core::validate_name(name, &registry.profiles)?;
            let taken: Vec<&str> = registry.profiles.iter().map(|p| p.id.as_str()).collect();
            let profile = Profile {
                id: core::new_id(&name, &taken, Utc::now()),
                name,
                created_at: Utc::now().to_rfc3339(),
            };
            registry.profiles.push(profile.clone());
            Ok(profile)
        })?;
        let target = self.dirs.database(&profile.id);
        // Create the schema before copying into it.
        self.store_at(&target)?;
        let mut secrets = Vec::new();
        for from in reuse {
            if from.items.is_empty() {
                continue;
            }
            let source = self.resolve(&from.profile)?;
            let keys = core::copy_items(&target, &self.dirs.database(&source.id), &from.items)?;
            secrets.push((source.id, keys));
        }
        Ok((profile, secrets))
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Profile, AppError> {
        self.update(|registry| {
            let others: Vec<Profile> = registry
                .profiles
                .iter()
                .filter(|p| p.id != id)
                .cloned()
                .collect();
            let name = core::validate_name(name, &others)?;
            let profile = registry
                .profiles
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| AppError::new("not_found", "No such profile"))?;
            profile.name = name;
            Ok(profile.clone())
        })
    }

    /// Removes a profile from the registry and deletes its files. The
    /// default profile holds app-wide settings and the last profile is
    /// needed to run, so neither can go.
    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let path = self.dirs.database(id);
        self.update(|registry| {
            if id == core::DEFAULT_ID {
                return Err(AppError::new(
                    "validation",
                    "The default profile can't be deleted.",
                ));
            }
            if registry.profiles.len() <= 1 {
                return Err(AppError::new(
                    "validation",
                    "The last profile can't be deleted.",
                ));
            }
            if registry.active == id {
                return Err(AppError::new(
                    "validation",
                    "Switch to another profile before deleting this one.",
                ));
            }
            if registry.get(id).is_none() {
                return Err(AppError::new("not_found", "No such profile"));
            }
            registry.profiles.retain(|p| p.id != id);
            Ok(())
        })?;
        // Its open store stays in memory (other code may hold it for the
        // rest of a call), but no new state is built over it.
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&path);
        let mut registry = self.registry();
        self.dirs.remove_files(&mut registry, id);
        self.update(|r| {
            r.pending_removal = registry.pending_removal.clone();
            Ok(())
        })
    }
}

/// Items to copy from one existing profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReuseFrom {
    pub profile: String,
    pub items: ReuseItems,
}

/// Keychain keys to copy, by source profile id.
pub type SecretCopies = Vec<(String, Vec<String>)>;

/// Tauri commands for the account menu, the wizard and Settings.
pub mod commands;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pr_nexus_core::domain::AuthMode;
    use pr_nexus_core::profiles::{DEFAULT_ID, ReuseItems};

    use super::ReuseFrom;
    use super::commands;
    use crate::mcp::config::{self, SlackSendMode};
    use crate::state::AppState;

    fn state(dir: &tempfile::TempDir) -> Arc<AppState> {
        let state = Arc::new(AppState::for_tests(&dir.path().join("pr-nexus.db"), true));
        crate::demo::seed_if_empty(&state.store).unwrap();
        state
    }

    #[test]
    fn switching_swaps_store_and_secrets_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        state.vault.set("github:1", "default-secret").unwrap();
        assert!(!state.store.list_projects().unwrap().is_empty());

        let created = commands::create(&state, "Acme Labs", &[]).unwrap();
        assert_eq!(state.profile(), created.id);
        assert!(state.store.list_projects().unwrap().is_empty());
        assert_eq!(state.vault.get("github:1").unwrap(), None);
        state.vault.set("github:1", "labs-secret").unwrap();

        state.switch_profile(DEFAULT_ID).unwrap();
        assert!(!state.store.list_projects().unwrap().is_empty());
        assert_eq!(
            state.vault.get("github:1").unwrap().as_deref(),
            Some("default-secret")
        );

        let listed = commands::list(&state).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|p| p.id == DEFAULT_ID && p.active));

        commands::delete(&state, &created.id).unwrap();
        assert_eq!(commands::list(&state).unwrap().len(), 1);
        assert_eq!(
            state
                .vault
                .global()
                .get(&format!("profile/{}/github:1", created.id))
                .unwrap(),
            None
        );
        assert_eq!(
            state.vault.get("github:1").unwrap().as_deref(),
            Some("default-secret")
        );
    }

    #[test]
    fn the_last_and_the_default_profile_stay() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        assert!(commands::delete(&state, DEFAULT_ID).is_err());
        let created = commands::create(&state, "Acme Labs", &[]).unwrap();
        // The active profile can't be deleted from under the window.
        assert!(commands::delete(&state, &created.id).is_err());
    }

    #[test]
    fn reused_projects_are_copies() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        let before = state.store.list_projects().unwrap().len();
        let reuse = [ReuseFrom {
            profile: DEFAULT_ID.into(),
            items: ReuseItems {
                projects: true,
                ..ReuseItems::default()
            },
        }];
        let options = commands::reuse_options(&state, DEFAULT_ID).unwrap();
        assert_eq!(options.projects, before);
        commands::create(&state, "Acme Labs", &reuse).unwrap();
        assert_eq!(state.store.list_projects().unwrap().len(), before);
        let first = state.store.list_projects().unwrap()[0].id;
        state.store.delete_project(first).unwrap();
        state.switch_profile(DEFAULT_ID).unwrap();
        assert_eq!(state.store.list_projects().unwrap().len(), before);
    }

    #[test]
    fn send_policy_is_per_profile_and_server_settings_are_shared() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        let mut settings = config::load(&state.store).unwrap();
        settings.slack_send_mode = SlackSendMode::Automatic;
        settings.enabled = true;
        config::save(&state.store, &settings).unwrap();

        commands::create(&state, "Acme Labs", &[]).unwrap();
        let other = config::load(&state.store).unwrap();
        assert!(other.enabled, "server switch is shared");
        assert_ne!(other.slack_send_mode, SlackSendMode::Automatic);

        state.switch_profile(DEFAULT_ID).unwrap();
        assert_eq!(
            config::load(&state.store).unwrap().slack_send_mode,
            SlackSendMode::Automatic
        );
    }

    fn follow_gh(state: &AppState) -> String {
        let mut account = crate::state::github_account(&state.store).unwrap().unwrap();
        account.auth_mode = AuthMode::GhCli;
        account.keychain_key = None;
        state.store.upsert_account(account.clone()).unwrap();
        account.login
    }

    fn reuse_github() -> [ReuseFrom; 1] {
        [ReuseFrom {
            profile: DEFAULT_ID.into(),
            items: ReuseItems {
                github: true,
                ..ReuseItems::default()
            },
        }]
    }

    #[test]
    fn a_copied_gh_account_gets_a_fixed_token() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        let login = follow_gh(&state);
        let asked = std::cell::RefCell::new(Vec::new());
        let created = commands::create_with(&state, "Acme Labs", &reuse_github(), &|user| {
            asked.borrow_mut().push(user.to_owned());
            Ok("acme-fixed".into())
        })
        .unwrap();
        assert_eq!(asked.into_inner(), vec![login.clone()]);

        let account = crate::state::github_account(&state.store).unwrap().unwrap();
        assert_eq!(account.login, login);
        assert_eq!(account.auth_mode, AuthMode::Pat);
        let key = format!("github:{}", account.external_id);
        assert_eq!(account.keychain_key.as_deref(), Some(key.as_str()));
        assert_eq!(
            state
                .vault
                .global()
                .get(&format!("profile/{}/{key}", created.id))
                .unwrap()
                .as_deref(),
            Some("acme-fixed")
        );
        assert_eq!(
            state.vault.get(&key).unwrap().as_deref(),
            Some("acme-fixed")
        );

        // The default profile keeps following gh's active account.
        state.switch_profile(DEFAULT_ID).unwrap();
        let default = crate::state::github_account(&state.store).unwrap().unwrap();
        assert_eq!(default.auth_mode, AuthMode::GhCli);
        assert_eq!(default.keychain_key, None);
        assert_eq!(state.vault.get(&key).unwrap(), None);
    }

    #[test]
    fn a_gh_failure_creates_no_profile() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        follow_gh(&state);
        let err = commands::create_with(&state, "Acme Labs", &reuse_github(), &|_| {
            Err(crate::error::AppError::new(
                "unauthorized",
                "gh is not logged in",
            ))
        });
        assert!(err.is_err());
        assert_eq!(commands::list(&state).unwrap().len(), 1);
        assert_eq!(state.profile(), DEFAULT_ID);
    }

    #[test]
    fn a_copied_stored_token_is_not_read_from_gh() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir);
        commands::create_with(&state, "Acme Labs", &reuse_github(), &|_| {
            panic!("gh must not be asked for an account with its own token")
        })
        .unwrap();
        let account = crate::state::github_account(&state.store).unwrap().unwrap();
        assert_eq!(account.auth_mode, AuthMode::Pat);
    }
}
