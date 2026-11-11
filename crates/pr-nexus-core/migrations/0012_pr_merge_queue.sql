-- Merge queue facts from the status sync. Absent row means not read yet.
-- `required` is whether the base branch requires the queue.
-- `phase` is queued, merging, failed, or removed; null when it is not in the queue.
CREATE TABLE pr_merge_queue (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  required INTEGER NOT NULL,
  phase TEXT,
  position INTEGER,
  eta_seconds INTEGER,
  reason TEXT
);
