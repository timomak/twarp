# 24 — Plugin auth: remote-first add flow + MCP OAuth (TECH)

Behavior: see `PRODUCT.md` (invariant numbers referenced as P1–P16).

## Context

Upstream Warp shipped exactly this feature and the ai-removal pass (02c-d,
`6d1ff624b`) deleted its core while leaving the plumbing:

- **Deleted:** `app/src/ai/mcp/templatable_manager/oauth.rs` — drove rmcp's
  OAuth state machine (dynamic client registration, CSRF `state` routing)
  with redirect URI `{scheme}://mcp/oauth2callback?server_id={uuid}`. The
  retained upstream spec `specs/APP-4099/TECH.md` documents it.
- **Survives, dead:** `FeatureFlag::McpOauth`
  (`crates/twarp_features/src/lib.rs:425`), cargo feature
  (`app/Cargo.toml:711`), static-provider config for issuers without DCR —
  `McpOAuthProviderConfig` (`crates/twarp_core/src/channel/config.rs:116-135`)
  and `ChannelState::mcp_oauth_provider_by_{client_id,issuer}`
  (`crates/twarp_core/src/channel/state.rs:271-292`).
- **Survives, live:** the `twarp://` URL scheme end to end —
  `CFBundleURLSchemes` (`app/src/bin/oss.rs:71`), AppKit `on_open_urls` →
  `uri::handle_incoming_uri` (`app/src/lib.rs:2028`), and `UriHost::Mcp`
  parses but hits a log-only stub (`app/src/uri/mod.rs:403-406`).
- **Ready dependencies:** the rmcp fork is already built with `auth` +
  `transport-streamable-http-client-reqwest` + `transport-sse-client-reqwest`
  (`app/Cargo.toml:302-308`); `oauth2` v5 is in the workspace
  (`Cargo.toml:193`) with an `AsyncHttpClient` adapter
  (`crates/http_client/src/lib.rs:653-695`).

Other load-bearing facts:

- **No in-app MCP client exists.** rmcp is only used as a *server*
  (`app/src/browser_mcp.rs`, `app/src/computer_control/mcp.rs`); the
  mcp-viewer (13) derives tools from Claude's stream-json init event, not a
  connection. Connect/probe (P5) needs the first in-app rmcp client.
- Registry: `McpServerEntry { transport: Stdio|Http, command, args, url,
  env, enabled_claude, enabled_codex, plugin_id }`
  (`app/src/mcp_registry.rs:23-73`), persisted whole-table via
  `ModelEvent::ReplaceMcpServers` → `replace_mcp_servers`
  (`app/src/persistence/sqlite.rs:3742`), table from migrations
  `2026-07-28-000000_add_mcp_servers` + `...000005_add_plugins`.
- Agent injection: `claude_config_value` (`mcp_registry.rs:135-152`) emits
  `{"type":"http","url"}` with **no headers**; Claude's `--mcp-config`
  accepts a `headers` map on http servers, so this is additive.
  `codex_config_value` (`:154-168`) emits `{url}`; Codex's `mcp_servers`
  config supports `bearer_token` / `bearer_token_env_var` for streamable
  HTTP. Merge/spawn path: `merge_mcp_servers`
  (`app/src/claude_code_view.rs:12288`) → `SpawnOptions.mcp_config`
  (`crates/claude_code/src/driver.rs:245,343`).
- Keychain: `twarpui_extras::secure_storage` trait + self-healing macOS impl
  (`crates/twarpui_extras/src/secure_storage/mac.rs:29-110`), registered
  with service = data domain (`app/src/lib.rs:977-990`). The agent API-key
  pattern (secret in keychain, presence-boolean in settings —
  `app/src/settings/agent.rs:588`, `app/src/settings_view/agent_page.rs:900`)
  is the model to copy.
- Form UI: `ServerForm` / `PluginEditor`
  (`app/src/automation/plugins_page.rs:209-262`), server sub-form render at
  `:1637-1700`, save-time validation `:820-865`, compiled-in gallery presets
  `:52-158`. Loopback-server precedent exists (axum in `browser_mcp.rs:305`)
  but is unnecessary — the custom scheme is the upstream-proven callback.

## Proposed changes

**Decision: revive, don't replace.** Rebuild the deleted oauth module on the
rmcp `auth` machinery, reusing the live `twarp://mcp/oauth2callback` scheme
and the dead-but-correct static-provider lookups. Delete nothing except the
stub comment. Keep `FeatureFlag::McpOauth` compiled-on (features are never
flag-gated per repo policy; the flag stays only because removing an upstream
flag invites merge conflicts — note this in code).

### 24a — data model + auth passthrough (no OAuth yet)

1. `McpServerEntry` gains `headers: BTreeMap<String, String>` +
   `auth: McpAuth` where `McpAuth = None | Headers | Oauth` (persisted as a
   string column; `Oauth` unused until 24b). New migration
   `add_mcp_server_headers` (two nullable TEXT columns, JSON + string);
   whole-table replace path and sqlite load extended.
2. `claude_config_value` emits `"headers": {...}` when non-empty;
   `codex_config_value` maps a sole `Authorization: Bearer <t>` header to
   `bearer_token` and logs a one-line warning for other headers (Codex can't
   express arbitrary headers — document as a known limit).
3. Form: `ServerForm` gains `headers_editor`; parse/validate `Name: value`
   lines at save (P4). Headers render inside the new Advanced disclosure.

### 24b — OAuth handshake + token store

4. New module `app/src/mcp_oauth.rs`:
   - `probe(entry) -> ProbeResult { Ok{tool_count} | NeedsOauth{metadata} |
     NeedsStaticAuth | Err }` — rmcp streamable-http client `initialize` +
     `tools/list`, falling back to SSE; a 401 with OAuth
     resource/authorization-server metadata → `NeedsOauth`.
   - `begin_oauth(server_id, metadata)` — rmcp `auth` flow: discovery, DCR
     (falling back to `mcp_oauth_provider_by_issuer` static credentials —
     this revives the dead lookups), PKCE, then
     `cx.open_url(authorize_url)` (pattern:
     `app/src/auth/auth_manager.rs:79`). Pending flows keyed by CSRF `state`
     in a singleton `McpOauthModel`; starting a second flow for the same
     server cancels the first (P13); 2-minute timeout task (P6).
   - `complete(callback_uri)` — token exchange, persist, notify UI.
5. Replace the stub at `app/src/uri/mod.rs:403` with dispatch to
   `McpOauthModel::complete`, matching upstream's
   `?server_id=&state=&code=` shape.
6. Token storage: keychain key `mcp.oauth.{server_id}` holding the
   serialized token set (access+refresh+expiry). Presence/status mirrored
   in-memory only (`McpAuthStatus: NotConnected | WaitingForBrowser |
   Connected{tool_count} | Error{msg}` on the registry model, not
   persisted). Refresh: on read, if expired and refresh token present,
   refresh synchronously via `oauth2` before returning; failures flip status
   to NeedsAuthorization (P7, P10). Delete keychain item on server/plugin
   delete and on Disconnect (P10, P16).
7. Agent injection: at spawn, for `auth == Oauth` servers, read/refresh the
   token and inject `Authorization: Bearer` into the emitted
   headers/bearer_token (P11). Token is minted per-spawn; mid-session expiry
   is a documented limitation (agent sees the provider's 401 and the user
   reconnects — acceptable v1, matches P11's error-surfacing clause).

### 24c — remote-first form + status chips

8. `ServerForm` rework per P1–P3: default paint = Name + URL + Connect;
   Advanced disclosure (a bool on `ServerForm`) contains transport dropdown,
   stdio fields, headers, env. Both field sets retained across transport
   flips (already true — editors persist in the struct). Remote gallery
   presets unchanged except they now land in the collapsed form; a stdio
   preset sets `advanced_open = true`.
9. Connect button drives `mcp_oauth::probe`/`begin_oauth` via a spawned
   task; status chip component rendered in both the editor row and the
   plugin card row (P9), driven by `McpAuthStatus` on the shared model with
   `notify()` on transitions. `NeedsStaticAuth` opens Advanced and focuses
   headers (P5c).
10. Save never blocks on auth state (P15); migration is a no-op for existing
    rows (`auth = None`, empty headers → P12).

Sub-phase → PR mapping: 24a is independently smoke-testable (add a
bearer-token remote server, agent calls its tools), so it can be its own PR;
24b+24c bundle into one PR since the handshake is only exercisable through
the reworked UI.

## Testing and validation

- **Unit** (`mcp_registry.rs` tests, per rust-unit-tests skill):
  claude/codex config emission with headers/bearer mapping (24a-2); header
  line parsing incl. invalid lines (P4); entry serde round-trip with new
  columns.
- **Unit** (`mcp_oauth.rs`): probe classification from canned responses
  (200-init vs 401+metadata vs 401-bare vs network error → P5's three
  branches + P15); callback URI parsing incl. unknown `state` and denied
  consent (P6); concurrent-flow cancellation (P13).
- **Manual smoke** (owner, per invariant): Composio and Notion hosted MCP as
  real OAuth targets — gallery card → Connect → browser consent → Connected
  chip (P1, P3, P5, P6, P9); deny consent + timeout paths (P6); quit
  mid-consent, relaunch, reconnect (P14); Claude session calls a Composio
  tool with no extra step (P11); pre-existing URL-with-key server still
  Connected after upgrade (P12); Disconnect then keychain inspection shows
  the item gone (P10, P16). Launch the app to verify — UI changes are never
  assumed correct from compilation alone.
- Keychain behavior must be tested with the signed run script
  (`./script/run`) so ACLs match (`WARP_SIGNING_TEAM` pinning).

## Risks and mitigations

- **rmcp fork's `auth` API drift** vs upstream's deleted oauth.rs — mitigate
  by keeping `specs/APP-4099/TECH.md` as the reference and porting, not
  rebuilding (same lesson as feature 07).
- **Providers without DCR** — static `McpOAuthProviderConfig` path exists
  but twarp ships no static credentials; such providers fall back to
  `NeedsStaticAuth` + headers. Acceptable: Composio/Notion/Linear support
  DCR.
- **Mid-session token expiry** (see 24b-7) — documented v1 limitation;
  follow-up: a twarp-local authenticating proxy would fix it properly but is
  out of scope.
- **Codex header limitation** — only bearer tokens map; arbitrary-header
  servers are Claude-only. Warn in UI copy if `enabled_codex` is on with
  non-bearer headers.

## Follow-ups

- Local authenticating proxy for long-lived sessions (removes per-spawn
  token minting and the Codex header limit).
- Ship static client credentials for popular non-DCR providers if any
  gallery preset needs one.
