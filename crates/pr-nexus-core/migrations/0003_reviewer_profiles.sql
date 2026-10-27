-- Name and avatar of each pending requested reviewer, shown in the list.
-- Nullable: rows from before this migration get them on the next sync.
ALTER TABLE pr_requested_reviewers ADD COLUMN name TEXT;
ALTER TABLE pr_requested_reviewers ADD COLUMN avatar_url TEXT;
