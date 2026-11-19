-- When a live detail overview last wrote the list-row fields. A later sync
-- whose document is not newer (updated_at) leaves those fields in place.
ALTER TABLE pull_requests ADD COLUMN detail_fetched_at TEXT;
