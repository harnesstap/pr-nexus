use rusqlite::{OptionalExtension, params};

use crate::domain::{
    AccountId, ChatThreadCacheEntry, OwnershipCacheEntry, PrId, SettingEntry, SlackChannel,
    SyncStateEntry, TeamMembersCacheEntry, ViewState,
};
use crate::error::CoreError;

use super::time::{format_ts, parse_bool, sql_bool, ts_col};
use super::{Store, db};

impl Store {
    pub fn upsert_slack_channel(&self, channel: SlackChannel) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO slack_channels_cache (
                    slack_workspace, channel_id, name, is_private, is_archived, is_member, fetched_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(slack_workspace, channel_id) DO UPDATE SET
                    name = excluded.name,
                    is_private = excluded.is_private,
                    is_archived = excluded.is_archived,
                    is_member = excluded.is_member,
                    fetched_at = excluded.fetched_at",
                params![
                    channel.slack_workspace,
                    channel.channel_id,
                    channel.name,
                    sql_bool(channel.is_private),
                    sql_bool(channel.is_archived),
                    sql_bool(channel.is_member),
                    format_ts(channel.fetched_at),
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_slack_channel(
        &self,
        workspace: String,
        channel_id: String,
    ) -> Result<Option<SlackChannel>, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT * FROM slack_channels_cache WHERE slack_workspace = ?1 AND channel_id = ?2",
                params![workspace, channel_id],
                map_channel,
            )
            .optional()
            .map_err(db)
        })
    }

    pub fn upsert_ownership_cache(&self, entry: OwnershipCacheEntry) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO ownership_cache (
                    pr_id, head_sha, base_ref, codeowners_oid, decision_json, computed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(pr_id) DO UPDATE SET
                    head_sha = excluded.head_sha,
                    base_ref = excluded.base_ref,
                    codeowners_oid = excluded.codeowners_oid,
                    decision_json = excluded.decision_json,
                    computed_at = excluded.computed_at",
                params![
                    entry.pr_id.0,
                    entry.head_sha,
                    entry.base_ref,
                    entry.codeowners_oid,
                    entry.decision_json,
                    format_ts(entry.computed_at),
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_ownership_cache(&self, pr: PrId) -> Result<Option<OwnershipCacheEntry>, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT * FROM ownership_cache WHERE pr_id = ?1",
                [pr.0],
                map_ownership,
            )
            .optional()
            .map_err(db)
        })
    }

    pub fn upsert_team_members(&self, entry: TeamMembersCacheEntry) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO team_members_cache (
                    github_org, team_slug, members_json, complete, fetched_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(github_org, team_slug) DO UPDATE SET
                    members_json = excluded.members_json,
                    complete = excluded.complete,
                    fetched_at = excluded.fetched_at",
                params![
                    entry.github_org,
                    entry.team_slug,
                    entry.members_json,
                    sql_bool(entry.complete),
                    format_ts(entry.fetched_at),
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_team_members(
        &self,
        org: String,
        slug: String,
    ) -> Result<Option<TeamMembersCacheEntry>, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT * FROM team_members_cache WHERE github_org = ?1 AND team_slug = ?2",
                params![org, slug],
                map_members,
            )
            .optional()
            .map_err(db)
        })
    }

    pub fn upsert_chat_thread(&self, entry: ChatThreadCacheEntry) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO chat_thread_cache (
                    slack_account_id, channel_id, thread_root_ts, messages_json, fetched_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(slack_account_id, channel_id, thread_root_ts) DO UPDATE SET
                    messages_json = excluded.messages_json,
                    fetched_at = excluded.fetched_at",
                params![
                    entry.slack_account_id.0,
                    entry.channel_id,
                    entry.thread_root_ts,
                    entry.messages_json,
                    format_ts(entry.fetched_at),
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_chat_thread(
        &self,
        account: AccountId,
        channel_id: String,
        thread_root_ts: String,
    ) -> Result<Option<ChatThreadCacheEntry>, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT * FROM chat_thread_cache
                 WHERE slack_account_id = ?1 AND channel_id = ?2 AND thread_root_ts = ?3",
                params![account.0, channel_id, thread_root_ts],
                map_thread,
            )
            .optional()
            .map_err(db)
        })
    }

    pub fn put_sync_state(&self, entry: SyncStateEntry) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO sync_state (key, value_json, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at",
                params![entry.key, entry.value_json, format_ts(entry.updated_at)],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_sync_state(&self, key: String) -> Result<Option<SyncStateEntry>, CoreError> {
        self.read(move |conn| {
            conn.query_row("SELECT * FROM sync_state WHERE key = ?1", [key], map_sync)
                .optional()
                .map_err(db)
        })
    }

    pub fn put_setting(&self, entry: SettingEntry) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                params![entry.key, entry.value_json],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_setting(&self, key: String) -> Result<Option<SettingEntry>, CoreError> {
        self.read(move |conn| {
            conn.query_row("SELECT * FROM settings WHERE key = ?1", [key], map_setting)
                .optional()
                .map_err(db)
        })
    }

    pub fn put_view_state(&self, state: ViewState) -> Result<(), CoreError> {
        self.write(move |conn| {
            conn.execute(
                "INSERT INTO view_state (view_key, state_json, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(view_key) DO UPDATE SET
                    state_json = excluded.state_json,
                    updated_at = excluded.updated_at",
                params![
                    state.view_key,
                    state.state_json,
                    format_ts(state.updated_at)
                ],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    pub fn get_view_state(&self, view_key: String) -> Result<Option<ViewState>, CoreError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT * FROM view_state WHERE view_key = ?1",
                [view_key],
                map_view,
            )
            .optional()
            .map_err(db)
        })
    }
}

fn map_channel(row: &rusqlite::Row<'_>) -> rusqlite::Result<SlackChannel> {
    Ok(SlackChannel {
        slack_workspace: row.get("slack_workspace")?,
        channel_id: row.get("channel_id")?,
        name: row.get("name")?,
        is_private: parse_bool(row.get("is_private")?),
        is_archived: parse_bool(row.get("is_archived")?),
        is_member: parse_bool(row.get("is_member")?),
        fetched_at: ts_col(row.get("fetched_at")?)?,
    })
}

pub(super) fn map_ownership(row: &rusqlite::Row<'_>) -> rusqlite::Result<OwnershipCacheEntry> {
    Ok(OwnershipCacheEntry {
        pr_id: PrId(row.get("pr_id")?),
        head_sha: row.get("head_sha")?,
        base_ref: row.get("base_ref")?,
        codeowners_oid: row.get("codeowners_oid")?,
        decision_json: row.get("decision_json")?,
        computed_at: ts_col(row.get("computed_at")?)?,
    })
}

fn map_members(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamMembersCacheEntry> {
    Ok(TeamMembersCacheEntry {
        github_org: row.get("github_org")?,
        team_slug: row.get("team_slug")?,
        members_json: row.get("members_json")?,
        complete: parse_bool(row.get("complete")?),
        fetched_at: ts_col(row.get("fetched_at")?)?,
    })
}

fn map_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThreadCacheEntry> {
    Ok(ChatThreadCacheEntry {
        slack_account_id: AccountId(row.get("slack_account_id")?),
        channel_id: row.get("channel_id")?,
        thread_root_ts: row.get("thread_root_ts")?,
        messages_json: row.get("messages_json")?,
        fetched_at: ts_col(row.get("fetched_at")?)?,
    })
}

fn map_sync(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncStateEntry> {
    Ok(SyncStateEntry {
        key: row.get("key")?,
        value_json: row.get("value_json")?,
        updated_at: ts_col(row.get("updated_at")?)?,
    })
}

fn map_setting(row: &rusqlite::Row<'_>) -> rusqlite::Result<SettingEntry> {
    Ok(SettingEntry {
        key: row.get("key")?,
        value_json: row.get("value_json")?,
    })
}

fn map_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<ViewState> {
    Ok(ViewState {
        view_key: row.get("view_key")?,
        state_json: row.get("state_json")?,
        updated_at: ts_col(row.get("updated_at")?)?,
    })
}
