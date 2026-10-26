use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{Account, AccountId};
use crate::error::CoreError;

use super::row::{conv, parse_enum};
use super::time::{format_ts, parse_ts};
use super::{Store, db};

impl Store {
    pub fn insert_account(&self, account: Account) -> Result<Account, CoreError> {
        self.write(move |conn| insert_account(conn, &account))
    }

    pub fn get_account(&self, id: AccountId) -> Result<Option<Account>, CoreError> {
        self.read(move |conn| load_account(conn, id))
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>, CoreError> {
        self.read(|conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM accounts ORDER BY id")
                .map_err(db)?;
            let rows = stmt
                .query_map([], map_account)
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok(rows)
        })
    }

    pub fn upsert_account(&self, account: Account) -> Result<Account, CoreError> {
        self.write(move |conn| upsert_account(conn, &account))
    }

    pub fn delete_account(&self, id: AccountId) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute("DELETE FROM accounts WHERE id = ?1", [id.0])
                .map_err(db)?;
            Ok(())
        })
    }
}

fn upsert_account(conn: &Connection, account: &Account) -> Result<Account, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO accounts (
                kind, display_name, host, external_id, slack_workspace, login,
                auth_mode, keychain_key, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(kind, external_id) DO UPDATE SET
                display_name = excluded.display_name,
                host = excluded.host,
                slack_workspace = excluded.slack_workspace,
                login = excluded.login,
                auth_mode = excluded.auth_mode,
                keychain_key = excluded.keychain_key
             RETURNING id",
            params![
                account.kind.as_str(),
                account.display_name,
                account.host,
                account.external_id,
                account.slack_workspace,
                account.login,
                account.auth_mode.as_str(),
                account.keychain_key,
                format_ts(account.created_at),
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_account(conn, AccountId(id))?
        .ok_or_else(|| CoreError::Store("upserted account disappeared".to_owned()))
}

fn insert_account(conn: &Connection, account: &Account) -> Result<Account, CoreError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO accounts (
                kind, display_name, host, external_id, slack_workspace, login,
                auth_mode, keychain_key, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             RETURNING id",
            params![
                account.kind.as_str(),
                account.display_name,
                account.host,
                account.external_id,
                account.slack_workspace,
                account.login,
                account.auth_mode.as_str(),
                account.keychain_key,
                format_ts(account.created_at),
            ],
            |row| row.get(0),
        )
        .map_err(db)?;
    load_account(conn, AccountId(id))?
        .ok_or_else(|| CoreError::Store("inserted account disappeared".to_owned()))
}

pub(super) fn load_account(conn: &Connection, id: AccountId) -> Result<Option<Account>, CoreError> {
    conn.query_row("SELECT * FROM accounts WHERE id = ?1", [id.0], map_account)
        .optional()
        .map_err(db)
}

fn map_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let created_at: String = row.get("created_at")?;
    Ok(Account {
        id: AccountId(row.get("id")?),
        kind: parse_enum(row.get("kind")?)?,
        display_name: row.get("display_name")?,
        host: row.get("host")?,
        external_id: row.get("external_id")?,
        slack_workspace: row.get("slack_workspace")?,
        login: row.get("login")?,
        auth_mode: parse_enum(row.get("auth_mode")?)?,
        keychain_key: row.get("keychain_key")?,
        created_at: parse_ts(&created_at).map_err(conv)?,
    })
}
