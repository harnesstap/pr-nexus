//! Profiles (§4.5): independent configurations, one active at a time.
//!
//! Layout under the app data directory:
//!
//! ```text
//! pr-nexus.db                       the default profile (unchanged path)
//! pr-nexus.pre-profiles.db          one-off backup taken by the migration
//! profiles.json                     the registry: names, ids, active id
//! profiles/<id>/pr-nexus.db         every other profile
//! ```
//!
//! The default profile keeps the database it always had, so upgrading moves
//! no data. Settings shared by every profile (the MCP server switch, port,
//! capabilities and token) stay in the default profile's database.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::store::db;

/// Id of the profile migrated from the single configuration.
pub const DEFAULT_ID: &str = "default";
pub const DEFAULT_NAME: &str = "Default";
pub const REGISTRY_FILE: &str = "profiles.json";
pub const BACKUP_FILE: &str = "pr-nexus.pre-profiles.db";
pub const MAX_NAME: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub active: String,
    pub profiles: Vec<Profile>,
    /// Profile directories whose removal failed (a file still open on
    /// Windows); retried at the next launch.
    #[serde(default)]
    pub pending_removal: Vec<String>,
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn active_profile(&self) -> &Profile {
        self.get(&self.active)
            .or_else(|| self.profiles.first())
            .expect("a registry always has a profile")
    }

    /// A profile by id, or by name ignoring case.
    pub fn resolve(&self, key: &str) -> Option<&Profile> {
        let key = key.trim();
        self.get(key).or_else(|| {
            self.profiles
                .iter()
                .find(|profile| profile.name.eq_ignore_ascii_case(key))
        })
    }
}

/// Where profiles live: `dir` is the app data directory, `default_db` the
/// default profile's database (`dir/pr-nexus.db` unless overridden).
#[derive(Debug, Clone)]
pub struct ProfileDirs {
    pub dir: PathBuf,
    pub default_db: PathBuf,
}

impl ProfileDirs {
    pub fn new(default_db: impl Into<PathBuf>) -> Self {
        let default_db = default_db.into();
        let dir = default_db
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self { dir, default_db }
    }

    pub fn registry_path(&self) -> PathBuf {
        self.dir.join(REGISTRY_FILE)
    }

    pub fn backup_path(&self) -> PathBuf {
        self.dir.join(BACKUP_FILE)
    }

    pub fn profile_dir(&self, id: &str) -> PathBuf {
        self.dir.join("profiles").join(id)
    }

    pub fn database(&self, id: &str) -> PathBuf {
        if id == DEFAULT_ID {
            self.default_db.clone()
        } else {
            self.profile_dir(id).join("pr-nexus.db")
        }
    }

    /// Loads the registry, creating it on first run. Idempotent: an
    /// existing registry is read as is, and the backup is taken once, only
    /// when a pre-profiles database exists and no backup does.
    pub fn migrate(&self, now: DateTime<Utc>) -> Result<Registry, CoreError> {
        std::fs::create_dir_all(&self.dir).map_err(db)?;
        if let Some(mut registry) = self.read()? {
            if registry.profiles.is_empty() {
                registry.profiles.push(default_profile(now));
            }
            if registry.get(&registry.active).is_none() {
                registry.active = registry.profiles[0].id.clone();
            }
            self.retry_removals(&mut registry)?;
            return Ok(registry);
        }
        if self.default_db.exists() && !self.backup_path().exists() {
            backup(&self.default_db, &self.backup_path())?;
        }
        let registry = Registry {
            version: 1,
            active: DEFAULT_ID.into(),
            profiles: vec![default_profile(now)],
            pending_removal: Vec::new(),
        };
        self.save(&registry)?;
        Ok(registry)
    }

    pub fn read(&self) -> Result<Option<Registry>, CoreError> {
        match std::fs::read_to_string(self.registry_path()) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|err| CoreError::Store(format!("{REGISTRY_FILE}: {err}"))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(db(err)),
        }
    }

    /// Writes through a temporary file and a rename, so a crash never
    /// leaves half a registry.
    pub fn save(&self, registry: &Registry) -> Result<(), CoreError> {
        let text = serde_json::to_string_pretty(registry).map_err(db)?;
        let tmp = self.dir.join(format!("{REGISTRY_FILE}.tmp"));
        std::fs::write(&tmp, text).map_err(db)?;
        std::fs::rename(&tmp, self.registry_path()).map_err(db)
    }

    /// Removes a deleted profile's directory, or remembers to retry.
    pub fn remove_files(&self, registry: &mut Registry, id: &str) {
        if id == DEFAULT_ID {
            return;
        }
        let dir = self.profile_dir(id);
        if dir.exists() && std::fs::remove_dir_all(&dir).is_err() {
            registry.pending_removal.push(id.to_owned());
        }
    }

    fn retry_removals(&self, registry: &mut Registry) -> Result<(), CoreError> {
        if registry.pending_removal.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut registry.pending_removal);
        for id in pending {
            if registry.get(&id).is_none() {
                self.remove_files(registry, &id);
            }
        }
        self.save(registry)
    }
}

fn default_profile(now: DateTime<Utc>) -> Profile {
    Profile {
        id: DEFAULT_ID.into(),
        name: DEFAULT_NAME.into(),
        created_at: now.to_rfc3339(),
    }
}

/// A consistent copy of `source` (WAL included) at `target`.
fn backup(source: &Path, target: &Path) -> Result<(), CoreError> {
    let conn = Connection::open(source).map_err(db)?;
    conn.pragma_update(None, "busy_timeout", 5000).map_err(db)?;
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
        .map_err(db)?;
    Ok(())
}

/// A trimmed, non-empty name of at most [`MAX_NAME`] characters, unique
/// (ignoring case) among `others`.
pub fn validate_name<'a>(
    name: &str,
    others: impl IntoIterator<Item = &'a Profile>,
) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Validation("Name the profile".into()));
    }
    if name.chars().count() > MAX_NAME {
        return Err(CoreError::Validation(format!(
            "Keep the name under {MAX_NAME} characters"
        )));
    }
    if others
        .into_iter()
        .any(|profile| profile.name.eq_ignore_ascii_case(name))
    {
        return Err(CoreError::Validation(format!(
            "A profile named {name} already exists"
        )));
    }
    Ok(name.to_owned())
}

/// A new id: a slug of `name` plus a short suffix, never `default`.
pub fn new_id(name: &str, taken: &[&str], now: DateTime<Utc>) -> String {
    let mut slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    slug.truncate(24);
    if slug.is_empty() || slug == DEFAULT_ID {
        slug = "profile".into();
    }
    let mut stamp = now.timestamp_millis().unsigned_abs() % 0x10_0000;
    loop {
        let id = format!("{slug}-{stamp:05x}");
        if !taken.contains(&id.as_str()) {
            return id;
        }
        stamp = (stamp + 1) % 0x10_0000;
    }
}

/// What the Add-a-profile wizard copies from one existing profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReuseItems {
    /// The GitHub account, its tracked orgs and the personal-repos switch.
    pub github: bool,
    /// Slack workspaces by team id (`T…`), with their channel mappings.
    pub slack_teams: Vec<String>,
    pub templates: bool,
    pub send_policy: bool,
    pub projects: bool,
}

impl ReuseItems {
    pub fn is_empty(&self) -> bool {
        !self.github
            && self.slack_teams.is_empty()
            && !self.templates
            && !self.send_policy
            && !self.projects
    }
}

/// Settings key of the per-profile Slack send policy (§10.4).
pub const SEND_POLICY_KEY: &str = "mcp.send_policy";

const GITHUB_SETTINGS: &[&str] = &[
    crate::settings::VCS,
    crate::settings::CHAT,
    crate::settings::TRACKED_ORGS,
    crate::settings::INCLUDE_PERSONAL,
    crate::settings::VIEWER_ORGS,
    crate::settings::SSO_ORGS,
];

/// Copies `items` from the database at `source` into `target` (a fresh or
/// existing profile database). Rows are copied, never shared: later edits
/// on either side stay there. Returns the copied accounts' keychain keys,
/// whose secrets the caller copies between the profiles' vault scopes.
pub fn copy_items(
    target: &Path,
    source: &Path,
    items: &ReuseItems,
) -> Result<Vec<String>, CoreError> {
    let mut conn = Connection::open(target).map_err(db)?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(db)?;
    conn.pragma_update(None, "busy_timeout", 5000).map_err(db)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS src",
        [source.to_string_lossy().as_ref()],
    )
    .map_err(db)?;
    let keys = copy_in(&mut conn, items);
    conn.execute("DETACH DATABASE src", []).map_err(db)?;
    keys
}

fn copy_in(conn: &mut Connection, items: &ReuseItems) -> Result<Vec<String>, CoreError> {
    let tx = conn.transaction().map_err(db)?;
    let mut keys = Vec::new();
    let account_cols = columns(&tx, "accounts")?;
    let mut copy_accounts = |filter: &str, value: &str| -> Result<(), CoreError> {
        let sql = format!(
            "INSERT INTO main.accounts ({account_cols})
             SELECT {account_cols} FROM src.accounts s WHERE {filter}
               AND NOT EXISTS (SELECT 1 FROM main.accounts m
                               WHERE m.kind = s.kind AND m.external_id = s.external_id)"
        );
        tx.execute(&sql, [value]).map_err(db)?;
        let mut stmt = tx
            .prepare(&format!(
                "SELECT keychain_key FROM src.accounts WHERE {filter} AND keychain_key IS NOT NULL"
            ))
            .map_err(db)?;
        let found = stmt
            .query_map([value], |row| row.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        keys.extend(found);
        Ok(())
    };
    if items.github {
        copy_accounts("kind = ?1", "github")?;
    }
    for team in &items.slack_teams {
        copy_accounts("kind = 'slack' AND external_id LIKE ?1 || ':%'", team)?;
    }
    let mut settings: Vec<&str> = Vec::new();
    if items.github {
        settings.extend(GITHUB_SETTINGS);
    }
    if items.templates {
        settings.extend(
            crate::review::message_template::TemplateKind::ALL
                .iter()
                .map(|kind| kind.setting_key()),
        );
    }
    if items.send_policy {
        settings.push(SEND_POLICY_KEY);
        settings.push(crate::settings::BUMP_BROADCAST);
        settings.push(crate::settings::MIN_BUMP_INTERVAL);
    }
    for key in settings {
        tx.execute(
            "INSERT OR REPLACE INTO main.settings (key, value_json)
             SELECT key, value_json FROM src.settings WHERE key = ?1",
            [key],
        )
        .map_err(db)?;
    }
    if !items.slack_teams.is_empty() {
        let cols = columns(&tx, "team_channel_mappings")?;
        for team in &items.slack_teams {
            tx.execute(
                &format!(
                    "INSERT OR IGNORE INTO main.team_channel_mappings ({cols})
                     SELECT {cols} FROM src.team_channel_mappings
                     WHERE slack_workspace IN (SELECT slack_workspace FROM src.accounts
                         WHERE kind = 'slack' AND external_id LIKE ?1 || ':%')"
                ),
                [team],
            )
            .map_err(db)?;
        }
    }
    if items.projects {
        let cols = columns(&tx, "projects")?;
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO main.projects ({cols}) SELECT {cols} FROM src.projects"
            ),
            [],
        )
        .map_err(db)?;
        let map = "(SELECT m.id FROM main.projects m JOIN src.projects s ON s.name = m.name
                    WHERE s.id = r.project_id)";
        tx.execute(
            &format!(
                "INSERT INTO main.project_rules (project_id, kind, pattern, priority)
                 SELECT {map}, r.kind, r.pattern, r.priority FROM src.project_rules r
                 WHERE NOT EXISTS (SELECT 1 FROM main.project_rules x
                     WHERE x.project_id = {map} AND x.kind = r.kind AND x.pattern = r.pattern)"
            ),
            [],
        )
        .map_err(db)?;
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO main.project_jira_links (project_id, key, url, position, created_at)
                 SELECT {map}, r.key, r.url, r.position, r.created_at FROM src.project_jira_links r"
            ),
            [],
        )
        .map_err(db)?;
    }
    tx.commit().map_err(db)?;
    keys.sort();
    keys.dedup();
    Ok(keys)
}

/// Every column of `table` but `id`, comma-separated.
fn columns(conn: &Connection, table: &str) -> Result<String, CoreError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA main.table_info({table})"))
        .map_err(db)?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?;
    Ok(names
        .into_iter()
        .filter(|name| name != "id")
        .collect::<Vec<_>>()
        .join(", "))
}

#[cfg(test)]
mod tests;
