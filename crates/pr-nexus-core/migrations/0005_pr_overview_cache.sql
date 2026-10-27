-- The PR detail panel's at-a-glance facts (§7.2), one JSON blob per PR.
CREATE TABLE IF NOT EXISTS pr_overview_cache (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  overview_json TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);
