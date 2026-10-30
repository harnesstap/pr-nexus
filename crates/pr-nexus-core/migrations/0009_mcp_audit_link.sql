-- Link to the Slack message a successful agent write touched (§10.1.3).
-- NULL for every other call and for rows logged before it existed.
ALTER TABLE mcp_audit ADD COLUMN link TEXT;
