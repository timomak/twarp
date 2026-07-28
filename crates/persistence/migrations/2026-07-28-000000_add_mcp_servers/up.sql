-- twarp 20b: user-managed shared MCP-server registry, edited on the
-- Automation > MCPs page and injected into both providers (Claude / Codex)
-- at session spawn. One row per server; `id` is a UUID string.
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    -- 'stdio' | 'http'
    transport TEXT NOT NULL,
    command TEXT,
    -- JSON array of strings
    args TEXT,
    url TEXT,
    -- JSON object of string -> string
    env TEXT,
    enabled_claude BOOL NOT NULL DEFAULT 1,
    enabled_codex BOOL NOT NULL DEFAULT 1
);
