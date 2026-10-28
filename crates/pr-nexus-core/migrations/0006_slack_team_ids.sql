-- Identify Slack workspaces by team ID, not domain (§7.11).
--
-- In an Enterprise Grid, every workspace can report the org host, so two
-- workspaces can share a domain. `accounts.slack_workspace` was UNIQUE, which
-- SQLite cannot drop in place: move the values to a new, non-unique column
-- and empty the old one (NULLs never collide).
ALTER TABLE accounts RENAME COLUMN slack_workspace TO slack_workspace_unique_v1;
ALTER TABLE accounts ADD COLUMN slack_workspace TEXT;
UPDATE accounts SET slack_workspace = slack_workspace_unique_v1, slack_workspace_unique_v1 = NULL;

ALTER TABLE review_requests ADD COLUMN slack_team_id TEXT;
ALTER TABLE team_channel_mappings ADD COLUMN slack_team_id TEXT;

-- Backfill requests: a domain held by exactly one Slack account gets that
-- account's team (`external_id` is `team:user`); the request was posted
-- through it. Ambiguous rows stay NULL and are resolved at send time.
-- Mappings are not backfilled: their domain may come from a pasted channel
-- link, whose host is the org's in a grid, not the posting workspace's.
CREATE TEMP TABLE slack_domain_team AS
  SELECT lower(slack_workspace) AS domain,
         min(substr(external_id, 1, instr(external_id, ':') - 1)) AS team_id
  FROM accounts
  WHERE kind = 'slack' AND slack_workspace IS NOT NULL AND instr(external_id, ':') > 1
  GROUP BY lower(slack_workspace)
  HAVING count(*) = 1;
UPDATE review_requests SET slack_team_id = (
  SELECT team_id FROM slack_domain_team WHERE domain = lower(review_requests.slack_workspace));
DROP TABLE slack_domain_team;
