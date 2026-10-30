//! SQLite store: one writer thread, a small read pool, WAL.
//!
//! Open one [`Store`] per database file. Writes are serialized on a dedicated
//! thread. Reads borrow a connection from a pool of four. `foreign_keys` and
//! `busy_timeout` are set on every connection.

mod accounts;
mod cache;
mod listing;
mod mappings;
mod mcp_audit;
pub use mcp_audit::{AUDIT_KEEP_DAYS, AUDIT_KEEP_ROWS, AuditEntry, AuditOutcome};
mod migrate;
mod project_info;
mod projects;
pub use project_info::MAX_TEXT;
mod pull_requests;
pub(crate) use pull_requests::cap_body;
mod reviews;
mod row;
mod time;

pub use listing::ListFacts;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use rusqlite::Connection;

use crate::error::CoreError;

const READ_POOL_SIZE: usize = 4;

enum WriterMsg {
    Job(Box<dyn FnOnce(&mut Connection) + Send>),
    Shutdown,
}

/// Process-local access to `pr-nexus.db`.
pub struct Store {
    path: PathBuf,
    writer_tx: Sender<WriterMsg>,
    writer: Mutex<Option<JoinHandle<()>>>,
    reads: Mutex<Vec<Connection>>,
}

impl Store {
    /// Open or create the database, apply migrations, and start the writer.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(db)?;
        }

        let mut writer_conn = open_connection(&path, false)?;
        migrate::migrate(&mut writer_conn)?;
        pull_requests::backfill_search_text(&mut writer_conn)?;

        let (tx, rx) = mpsc::channel();
        let writer = thread::Builder::new()
            .name("pr-nexus-store".to_owned())
            .spawn(move || writer_loop(writer_conn, rx))
            .map_err(db)?;

        let mut reads = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            reads.push(open_connection(&path, true)?);
        }

        Ok(Self {
            path,
            writer_tx: tx,
            writer: Mutex::new(Some(writer)),
            reads: Mutex::new(reads),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, CoreError>) -> Result<T, CoreError> {
        let conn = {
            let mut guard = self
                .reads
                .lock()
                .map_err(|_| CoreError::Store("read pool poisoned".to_owned()))?;
            match guard.pop() {
                Some(conn) => conn,
                None => open_connection(&self.path, true)?,
            }
        };
        let result = f(&conn);
        if let Ok(mut guard) = self.reads.lock()
            && guard.len() < READ_POOL_SIZE
        {
            guard.push(conn);
        }
        result
    }

    fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, CoreError> + Send + 'static,
    ) -> Result<T, CoreError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.writer_tx
            .send(WriterMsg::Job(Box::new(move |conn| {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(conn)));
                let result = match result {
                    Ok(value) => value,
                    Err(_) => Err(CoreError::Store("store write panicked".to_owned())),
                };
                let _ = reply_tx.send(result);
            })))
            .map_err(|_| CoreError::Store("store writer is stopped".to_owned()))?;
        reply_rx
            .recv()
            .map_err(|_| CoreError::Store("store writer dropped the reply".to_owned()))?
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = self.writer_tx.send(WriterMsg::Shutdown);
        if let Ok(mut slot) = self.writer.lock()
            && let Some(handle) = slot.take()
        {
            let _ = handle.join();
        }
    }
}

fn writer_loop(mut conn: Connection, rx: Receiver<WriterMsg>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            WriterMsg::Shutdown => break,
            WriterMsg::Job(job) => job(&mut conn),
        }
    }
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection, CoreError> {
    let conn = Connection::open(path).map_err(db)?;
    conn.pragma_update(None, "foreign_keys", "ON").map_err(db)?;
    conn.pragma_update(None, "busy_timeout", 5000).map_err(db)?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(db)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(CoreError::Store(format!(
            "journal_mode is {mode}, want wal"
        )));
    }
    if read_only {
        conn.pragma_update(None, "query_only", "ON").map_err(db)?;
    }
    Ok(conn)
}

pub(crate) fn db(err: impl std::fmt::Display) -> CoreError {
    CoreError::Store(err.to_string())
}

#[cfg(test)]
mod acceptance;

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn migration_is_wal_and_idempotent() {
        let (dir, store) = open_temp();
        let (mode, foreign_keys, busy, version, tables) = store
            .read(|conn| {
                let mode: String = conn
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .map_err(db)?;
                let foreign_keys: i64 = conn
                    .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                    .map_err(db)?;
                let busy: i64 = conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                    .map_err(db)?;
                let version: i64 = conn
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .map_err(db)?;
                let mut stmt = conn
                    .prepare(
                        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    )
                    .map_err(db)?;
                let names = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(db)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db)?;
                Ok((mode, foreign_keys, busy, version, names))
            })
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");
        assert_eq!(foreign_keys, 1);
        assert_eq!(busy, 5000);
        assert_eq!(version, 9);
        assert!(tables.contains(&"pull_requests".to_owned()));
        assert!(tables.contains(&"review_requests".to_owned()));
        assert!(tables.contains(&"view_state".to_owned()));
        assert!(tables.contains(&"team_channel_mappings".to_owned()));
        drop(store);

        let again = Store::open(dir.path().join("pr-nexus.db")).unwrap();
        let version = again
            .read(|conn| {
                conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .map_err(db)
            })
            .unwrap();
        assert_eq!(version, 9);
    }

    #[test]
    fn create_project_trims_appends_last_and_rejects_duplicates() {
        let (_dir, store) = open_temp();
        let first = store.create_project("  Auth  ".into()).unwrap();
        assert_eq!(first.name, "Auth");
        let second = store.create_project("Billing".into()).unwrap();
        assert!(second.sort_order > first.sort_order);
        let dup = store.create_project("Auth".into()).unwrap_err().to_string();
        assert!(dup.contains("already exists"), "{dup}");
        let empty = store.create_project(" ".into()).unwrap_err().to_string();
        assert!(empty.contains("empty"), "{empty}");
        assert_eq!(store.list_projects().unwrap().len(), 2);
    }

    #[test]
    fn rename_rejects_empty_and_duplicate_names_and_archive_round_trips() {
        let (_dir, store) = open_temp();
        let auth = store
            .insert_project(crate::domain::Project {
                id: crate::domain::ProjectId(0),
                name: "Auth".into(),
                description: String::new(),
                color: "#7c5cbf".into(),
                archived: false,
                sort_order: 0,
            })
            .unwrap();
        let renamed = store.rename_project(auth.id, "  Auth v2  ".into()).unwrap();
        assert_eq!(renamed.name, "Auth v2");
        let err = store
            .rename_project(auth.id, "   ".into())
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty"), "{err}");
        let other = store
            .insert_project(crate::domain::Project {
                id: crate::domain::ProjectId(0),
                name: "Billing".into(),
                description: String::new(),
                color: "#3f6f8f".into(),
                archived: false,
                sort_order: 1,
            })
            .unwrap();
        let dup = store
            .rename_project(other.id, "Auth v2".into())
            .unwrap_err()
            .to_string();
        assert!(dup.contains("already exists"), "{dup}");
        store.set_project_archived(other.id, true).unwrap();
        let loaded = store.get_project(other.id).unwrap().unwrap();
        assert!(loaded.archived);
    }

    #[test]
    fn no_column_stores_a_secret() {
        let (_dir, store) = open_temp();
        let columns = store
            .read(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                    )
                    .map_err(db)?;
                let tables = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(db)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db)?;
                drop(stmt);
                let mut columns = Vec::new();
                for table in tables {
                    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
                    let mut info = conn
                        .prepare(&format!("PRAGMA table_info({quoted})"))
                        .map_err(db)?;
                    let names = info
                        .query_map([], |row| row.get::<_, String>(1))
                        .map_err(db)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(db)?;
                    columns.extend(names);
                }
                Ok(columns)
            })
            .unwrap();
        assert!(!columns.is_empty());
        for name in columns {
            let lower = name.to_ascii_lowercase();
            assert!(
                !lower.contains("token")
                    && !lower.contains("secret")
                    && !lower.contains("password"),
                "column {name} looks like a secret"
            );
        }
    }
}
