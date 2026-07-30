# 24 — Plugin auth: remote-first add flow + MCP OAuth — STATUS

**Phase:** impl-in-review

Rework the Plugins add-integration flow so hosted MCP servers (Composio,
Notion, Linear, …) are "paste URL / pick preset → Connect → approve in
browser → connected", with keychain-stored OAuth tokens and headers/bearer
support. Stdio stays as the Advanced path. Motivated by the Composio add
experience (2026-07-30 owner feedback: too many options, no way to auth).

Spec merged in [#297](https://github.com/timomak/twarp/pull/297).

## Sub-phases

All three shipped in one PR — the 24a/24b seam didn't cut cleanly (see
TECH.md "Amended during implementation").

- [x] **24a — data model + auth passthrough** — `headers`/`auth` on
  `McpServerEntry` + migration, Claude `headers` / Codex `bearer_token`
  emission, headers field in Advanced.
- [x] **24b — OAuth handshake + token store** — `app/src/mcp_oauth.rs`
  (probe / start_authorization / complete on rmcp `OAuthState`), revived
  `twarp://mcp/oauth2callback` handler, keychain token storage + background
  refresh, spawn-time bearer injection.
- [x] **24c — remote-first form + status chips** — collapsed Name + Server URL
  + Connect form, Advanced disclosure, per-server status chips in the editor
  and on plugin cards, Connect / Cancel / Disconnect.

## Verification

- 20 unit tests pass (`cargo test -p twarp --lib mcp_`, `… plugins_page`):
  Claude header + Codex bearer emission, typed-header precedence, pre-24a
  migration defaults, persisted round-trip, callback-URL parsing (granted /
  denied / incomplete), header-line and URL validation.
- Builds and launches clean (`./script/run`), no panic in
  `~/Library/Logs/twarp-oss.log`.
- **Owner smoke test pending** — the reworked form was never seen rendered
  (computer control wasn't available against the dev build). Worth walking
  P1 / P3 / P5 / P6 / P9 against a real Composio server, plus P12 (an existing
  URL-with-key server must still work untouched).

## Known limitations (recorded in TECH.md)

- The static-provider lookups stay dead: rmcp exposes no way to set discovered
  metadata from outside the crate, so only dynamic client registration is
  reachable. twarp ships no static providers, so nothing regresses.
- Codex can only carry a bearer `Authorization`; other headers are Claude-only
  and the form warns when that applies.
- Tokens are minted per session spawn, so a grant expiring mid-session
  surfaces as the provider's 401 until the user reconnects.
