-- twarp 23a: Plugins — a named grouping layer above the MCP-server registry
-- and the shared-skills store. One row per plugin; `id` is a UUID string.
-- Components reference their owning plugin via the nullable `plugin_id`
-- columns added below; NULL means "not yet migrated" (auto-adopted into a
-- single-component plugin on the next load).
CREATE TABLE plugins (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled_claude BOOL NOT NULL DEFAULT 1,
    enabled_codex BOOL NOT NULL DEFAULT 1
);
ALTER TABLE mcp_servers ADD COLUMN plugin_id TEXT;
ALTER TABLE shared_skills ADD COLUMN plugin_id TEXT;
