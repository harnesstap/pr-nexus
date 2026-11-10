-- Last-seen activity, snooze, and mute, keyed by the GitHub node id so they
-- survive the local pull request row being replaced (same idea as pr_stars).
CREATE TABLE pr_attention (
  node_id TEXT PRIMARY KEY,
  seen_sha TEXT NOT NULL,
  seen_checks TEXT NOT NULL,
  seen_review TEXT,
  seen_threads INTEGER NOT NULL,
  seen_unresolved INTEGER NOT NULL,
  seen_updated_at TEXT NOT NULL,
  muted INTEGER NOT NULL DEFAULT 0,
  snooze_kind TEXT CHECK (snooze_kind IS NULL OR snooze_kind IN ('until', 'activity')),
  snooze_until TEXT,
  snooze_sha TEXT,
  snooze_checks TEXT,
  snooze_review TEXT,
  snooze_threads INTEGER,
  snooze_unresolved INTEGER,
  snooze_updated_at TEXT,
  updated_at TEXT NOT NULL
);
