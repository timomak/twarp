# 24 — Plugin auth: remote-first add flow + MCP OAuth — STATUS

**Phase:** spec-in-review

Rework the Plugins add-integration flow so hosted MCP servers (Composio,
Notion, Linear, …) are "paste URL / pick preset → Connect → approve in
browser → connected", with keychain-stored OAuth tokens and headers/bearer
support. Stdio stays as the Advanced path. Motivated by the Composio add
experience (2026-07-30 owner feedback: too many options, no way to auth).

## Sub-phases

- [ ] **24a — data model + auth passthrough** — `headers`/`auth` on
  `McpServerEntry` + migration, Claude `headers` / Codex `bearer_token`
  emission, headers field in Advanced. Independently smoke-testable → own PR.
- [ ] **24b — OAuth handshake + token store** — `app/src/mcp_oauth.rs`
  (probe / begin / complete on rmcp `auth`), revive `twarp://mcp/oauth2callback`
  handler, keychain token storage + refresh, spawn-time bearer injection.
- [ ] **24c — remote-first form + status chips** — collapsed Name+URL+Connect
  form, Advanced disclosure, per-server status chips + Connect/Disconnect.

24b+24c bundle into one PR (handshake only exercisable through the reworked
UI, per the bundle-when-not-testable rule).
