-- A close started in the app.
-- `closed_at` keeps a lagging search from reopening the row.
-- `comment_body` is the comment already posted for that close, so a retry
-- does not post the same text again.
CREATE TABLE pr_close_notes (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  closed_at TEXT,
  comment_body TEXT
);
