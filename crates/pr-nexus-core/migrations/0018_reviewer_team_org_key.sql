-- Requested teams from different orgs can share a slug. The original
-- primary key was (pr_id, kind, login_or_slug), so a second team with the
-- same slug on one pull request could not be stored. Users have an empty
-- org; teams always have one.
CREATE TABLE pr_requested_reviewers_new (
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('user','team')),
  org TEXT NOT NULL DEFAULT '',
  login_or_slug TEXT NOT NULL,
  name TEXT,
  avatar_url TEXT,
  PRIMARY KEY (pr_id, kind, org, login_or_slug)
);
INSERT INTO pr_requested_reviewers_new
  (pr_id, kind, org, login_or_slug, name, avatar_url)
SELECT pr_id, kind, ifnull(org, ''), login_or_slug, name, avatar_url
FROM pr_requested_reviewers;
DROP TABLE pr_requested_reviewers;
ALTER TABLE pr_requested_reviewers_new RENAME TO pr_requested_reviewers;
