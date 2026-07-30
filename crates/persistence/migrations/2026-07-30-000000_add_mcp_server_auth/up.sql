-- twarp 24a: per-server auth for the MCP registry.
--
-- `headers` is a JSON string->string object of extra HTTP headers sent to
-- remote (http transport) servers — the escape hatch for services that want a
-- static `Authorization: Bearer <token>` or an `X-Api-Key`.
--
-- `auth` records how the server authenticates: 'none' (or NULL, for rows
-- written before this migration) | 'headers' | 'oauth'. OAuth tokens
-- themselves are never stored here — they live in the OS keychain under
-- `mcp.oauth.<server id>` (twarp 24b).
ALTER TABLE mcp_servers ADD COLUMN headers TEXT;
ALTER TABLE mcp_servers ADD COLUMN auth TEXT;
