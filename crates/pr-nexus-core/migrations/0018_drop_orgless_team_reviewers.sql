-- List sync used to store a requested team with an empty org when GitHub
-- left out `organization`, so two orgs' teams with one slug read as the
-- same `/slug`. Drop those rows; clearing detail_fetched_at lets the next
-- list sync write the pull request's requests again.
UPDATE pull_requests SET detail_fetched_at = NULL
WHERE id IN (
  SELECT pr_id FROM pr_requested_reviewers
  WHERE kind = 'team' AND ifnull(org, '') = ''
);
DELETE FROM pr_requested_reviewers WHERE kind = 'team' AND ifnull(org, '') = '';
