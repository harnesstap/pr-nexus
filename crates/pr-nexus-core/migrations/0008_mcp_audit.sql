-- MCP audit log (§9.4): one row per tool call an agent made, allowed or not.
--
-- No argument values that may be private (note text, queries): `fields` lists
-- the argument names, `target_ids` the ids the call touched, and `summary` is
-- a short line written by the app ("starred #482"). Bounded on insert.
CREATE TABLE mcp_audit (
  id INTEGER PRIMARY KEY,
  at TEXT NOT NULL,
  client_name TEXT NOT NULL DEFAULT '',
  tool TEXT NOT NULL,
  capability TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('allowed', 'denied', 'error')),
  fields TEXT NOT NULL DEFAULT '',
  target_ids TEXT NOT NULL DEFAULT '',
  summary TEXT NOT NULL DEFAULT '',
  error TEXT,
  duration_ms INTEGER NOT NULL DEFAULT 0);
CREATE INDEX mcp_audit_by_time ON mcp_audit(at);
