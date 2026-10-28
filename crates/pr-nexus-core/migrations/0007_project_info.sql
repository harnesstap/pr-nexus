-- Project info and per-PR marks (§8.2).
--
-- JIRA links belong to a project and go with it. A link has a key, a URL,
-- or both; a bare key becomes a link only once `jira.base_url` is set.
CREATE TABLE project_jira_links (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  key TEXT NOT NULL DEFAULT '',
  url TEXT NOT NULL DEFAULT '',
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  UNIQUE (project_id, key, url),
  CHECK (key <> '' OR url <> ''));
CREATE INDEX project_jira_links_by_project ON project_jira_links(project_id, position);

-- A free-text note per pull request, removed with it.
CREATE TABLE pr_notes (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  note TEXT NOT NULL,
  updated_at TEXT NOT NULL);

-- Stars are keyed by the GitHub node id, not the local row id: a PR that
-- leaves the list (closed and pruned, or out of scope) and comes back gets
-- a new row but keeps its star.
CREATE TABLE pr_stars (
  node_id TEXT PRIMARY KEY,
  starred_at TEXT NOT NULL);
