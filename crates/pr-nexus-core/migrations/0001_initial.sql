CREATE TABLE accounts (
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN ('github','slack')),
  display_name TEXT NOT NULL, host TEXT NOT NULL,
  external_id TEXT NOT NULL,
  slack_workspace TEXT UNIQUE,
  login TEXT NOT NULL,
  auth_mode TEXT NOT NULL CHECK (auth_mode IN ('gh_cli','pat','oauth_device','slack_mcp_oauth','slack_user_token')),
  keychain_key TEXT,
  created_at TEXT NOT NULL, UNIQUE (kind, external_id));

CREATE TABLE repositories (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  owner TEXT NOT NULL, name TEXT NOT NULL, node_id TEXT, default_branch TEXT,
  archived INTEGER NOT NULL DEFAULT 0, UNIQUE (account_id, owner, name));

CREATE TABLE pull_requests (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
  number INTEGER NOT NULL, node_id TEXT NOT NULL UNIQUE, title TEXT NOT NULL, url TEXT NOT NULL,
  author_login TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', state TEXT NOT NULL, is_draft INTEGER NOT NULL,
  head_ref TEXT NOT NULL, base_ref TEXT NOT NULL, head_sha TEXT NOT NULL,
  review_decision TEXT, checks_state TEXT NOT NULL, mergeable TEXT NOT NULL, merge_state TEXT NOT NULL,
  unresolved_threads INTEGER NOT NULL, total_threads INTEGER NOT NULL, involvement INTEGER NOT NULL,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_activity_at TEXT NOT NULL, synced_at TEXT NOT NULL,
  hidden INTEGER NOT NULL DEFAULT 0, UNIQUE (repo_id, number));
CREATE TABLE pr_labels (pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  name TEXT NOT NULL, PRIMARY KEY (pr_id, name));
CREATE TABLE pr_requested_reviewers (pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('user','team')), org TEXT, login_or_slug TEXT NOT NULL,
  PRIMARY KEY (pr_id, kind, login_or_slug));

CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT NOT NULL DEFAULT '',
  color TEXT NOT NULL DEFAULT '#64748b', archived INTEGER NOT NULL DEFAULT 0, sort_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE project_rules (id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, pattern TEXT NOT NULL, priority INTEGER NOT NULL DEFAULT 0);
CREATE TABLE pr_projects (
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK (source IN ('manual','rule','excluded')),
  rule_id INTEGER REFERENCES project_rules(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL, PRIMARY KEY (pr_id, project_id));

CREATE TABLE team_channel_mappings (
  id INTEGER PRIMARY KEY, layer TEXT NOT NULL CHECK (layer IN ('imported','user')),
  github_org TEXT NOT NULL, team_slug TEXT NOT NULL, team_name TEXT NOT NULL DEFAULT '',
  slack_workspace TEXT NOT NULL,
  slack_channel_id TEXT NOT NULL DEFAULT '', slack_channel_name TEXT NOT NULL DEFAULT '',
  disabled INTEGER NOT NULL DEFAULT 0,
  note TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL,
  UNIQUE (layer, github_org, team_slug));
CREATE UNIQUE INDEX tcm_name ON team_channel_mappings(layer, github_org, team_name) WHERE team_name <> '';

CREATE TABLE slack_channels_cache (slack_workspace TEXT NOT NULL, channel_id TEXT NOT NULL,
  name TEXT NOT NULL, is_private INTEGER NOT NULL, is_archived INTEGER NOT NULL, is_member INTEGER NOT NULL,
  fetched_at TEXT NOT NULL, PRIMARY KEY (slack_workspace, channel_id));

CREATE TABLE ownership_cache (pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  head_sha TEXT NOT NULL, base_ref TEXT NOT NULL, codeowners_oid TEXT,
  decision_json TEXT NOT NULL, computed_at TEXT NOT NULL);
CREATE TABLE team_members_cache (github_org TEXT NOT NULL, team_slug TEXT NOT NULL,
  members_json TEXT NOT NULL, complete INTEGER NOT NULL, fetched_at TEXT NOT NULL,
  PRIMARY KEY (github_org, team_slug));

CREATE TABLE review_messages (
  id INTEGER PRIMARY KEY, slack_account_id INTEGER NOT NULL REFERENCES accounts(id),
  channel_id TEXT NOT NULL, channel_name TEXT NOT NULL, ts TEXT NOT NULL,
  thread_root_ts TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('request','bump')),
  origin TEXT NOT NULL CHECK (origin IN ('posted','imported')),
  permalink TEXT, text TEXT NOT NULL, batch_id TEXT, posted_at TEXT NOT NULL,
  UNIQUE (slack_account_id, channel_id, ts));
CREATE TABLE review_message_prs (
  message_id INTEGER NOT NULL REFERENCES review_messages(id) ON DELETE CASCADE,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  head_sha TEXT, teams_json TEXT NOT NULL DEFAULT '[]', PRIMARY KEY (message_id, pr_id));

CREATE TABLE review_requests (
  id INTEGER PRIMARY KEY,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  slack_workspace TEXT NOT NULL, channel_id TEXT NOT NULL, channel_name TEXT NOT NULL,
  root_ts TEXT NOT NULL, permalink TEXT,
  requested_at TEXT NOT NULL, last_bumped_at TEXT, bump_count INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL CHECK (status IN ('active','closed')),
  close_reason TEXT CHECK (close_reason IN ('pr_merged','pr_closed','dismissed','superseded','channel_archived')),
  closed_at TEXT,
  source TEXT NOT NULL CHECK (source IN ('app','discovered_via_search')),
  CHECK ((status = 'active') = (close_reason IS NULL)),
  UNIQUE (pr_id, slack_workspace, channel_id, root_ts));
CREATE UNIQUE INDEX rr_one_active ON review_requests(pr_id, slack_workspace, channel_id) WHERE status = 'active';
CREATE TABLE discovery_runs (pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  slack_workspace TEXT NOT NULL, ran_at TEXT NOT NULL, outcome TEXT NOT NULL,
  PRIMARY KEY (pr_id, slack_workspace));
CREATE TABLE send_attempts (id INTEGER PRIMARY KEY, batch_id TEXT NOT NULL, pr_id INTEGER, channel_id TEXT,
  origin TEXT NOT NULL DEFAULT 'ui', outcome TEXT NOT NULL CHECK (outcome IN ('sent','skipped','failed')),
  detail TEXT NOT NULL, at TEXT NOT NULL);

CREATE TABLE chat_thread_cache (slack_account_id INTEGER NOT NULL, channel_id TEXT NOT NULL,
  thread_root_ts TEXT NOT NULL, messages_json TEXT NOT NULL, fetched_at TEXT NOT NULL,
  PRIMARY KEY (slack_account_id, channel_id, thread_root_ts));

CREATE TABLE sync_state (key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value_json TEXT NOT NULL);
CREATE TABLE view_state (view_key TEXT PRIMARY KEY, state_json TEXT NOT NULL, updated_at TEXT NOT NULL);
