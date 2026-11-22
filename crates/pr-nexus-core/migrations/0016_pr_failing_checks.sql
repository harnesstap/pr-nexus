-- Failing check names from the status rollup. No row means not read yet,
-- or the rollup is not failing. `checks` is a JSON array of
-- {name, conclusion, url}.
CREATE TABLE pr_failing_checks (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  checks TEXT NOT NULL
);
