-- Users assigned to a pull request. The list query reads them with the
-- other cheap fields. Empty until the next sync of an older row.
CREATE TABLE pr_assignees (
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  login TEXT NOT NULL,
  name TEXT,
  PRIMARY KEY (pr_id, login)
);
