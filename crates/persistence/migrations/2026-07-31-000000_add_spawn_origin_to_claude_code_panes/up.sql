-- twarp 26d: spawn provenance for agent panes created via the sessions MCP
-- create_chat tool (PRODUCT 26 P#22: the badge survives restore). JSON-encoded
-- SpawnOrigin; NULL for user-opened panes.
ALTER TABLE claude_code_panes ADD COLUMN spawn_origin TEXT;
