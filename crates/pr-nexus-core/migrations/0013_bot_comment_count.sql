-- Count of bot comments and reviews in the cached overview.
-- Filled when the overview is written. NULL is a row from before this
-- column: the next read computes it and stores the result.
ALTER TABLE pr_overview_cache ADD COLUMN bot_comment_count INTEGER;
