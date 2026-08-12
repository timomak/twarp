# 27 — Plugin gallery UX: connector-style add flow (TECH)

## Context

Feature 24 shipped the OAuth machinery; the gallery still funnels every
preset through the generic inline editor. Everything this feature touches
lives in one view plus two models it already talks to:

- `app/src/automation/plugins_page.rs` — the whole Plugins page.
  `PRESETS` (92–163) are transport/command/URL templates;
  `OpenPreset` (435) prefills the editor; `save()` (911) validates and
  commits plugin + servers, then optionally kicks `McpOauthModel::connect`
  for one sub-form (1131–1142); `render()` (1192) lays out header →
  Quick add gallery → editor → cards; `render_preset_card` (1401) is the
  clickable gallery card; `render_status_chip` / `render_connect_row`
  (2286 / 1815) already render per-status chips and state-dependent
  buttons for editor sub-forms.
- `app/src/mcp_oauth.rs` — `McpAuthStatus` (66) with `Connecting` /
  `WaitingForBrowser` / `Connected` / `NeedsStaticAuth` / `Error`;
  `McpOauthModel::{connect,cancel,disconnect,status}` drive the flow and
  `ctx.notify()` on transitions. Untouched by this feature.
- `crates/twarp_core/src/ui/external_product_icon.rs` — brand icons for
  every current preset key. Untouched.

Key facts the plan leans on: a `ConnectServer` press already saves first
because the callback routes by persisted server id; plugin + server
creation without the editor is exactly what `adopt_skill` (607) and
`orphan_server_into_plugin` (1147) already do for skills/servers; and
`ActionButton` labels are fixed at construction, so each distinct label
needs its own `ViewHandle` (established pattern: `ServerForm`'s
show/hide/connect/disconnect/cancel handles).

## Proposed changes

All in `plugins_page.rs` except copy-level tweaks; no model, registry,
schema, or OAuth changes.

1. **Preset kinds.** Extend `PluginPreset` with
   `kind: PresetKind { OneClick, SetupUrl, Credentials }`, a `doc_link:
   Option<&'static str>`, an optional `setup_note: &'static str`, and for
   `Credentials` a `fields: &'static [CredentialField]` (`env_key`,
   `label`, `help`). Reclassify: Notion/Linear/GitHub/Cloudflare →
   `OneClick`; Composio → `SetupUrl` (drop the placeholder URL from the
   preset; keep `mcp.composio.dev` only in the guidance copy); Slack →
   `Credentials` (bot token + team ID fields, archived-server caveat as
   `setup_note`); Gmail → `Credentials` with zero fields (its npx server
   self-authorizes), which degenerates to add-on-click.

2. **Direct creation, no editor.** New helper
   `create_preset_plugin(preset, url_override, env, ctx) -> server_id`:
   builds the `McpServerEntry` (id = new UUID, name =
   `unique_name(preset.key)`) and a single-server `PluginEntry`, upserts
   both (mirroring `orphan_server_into_plugin`), returns the server id.
   New actions replace `OpenPreset`:
   - `ConnectPreset(key)` — `OneClick`: create if absent, then
     `McpOauthModel::connect` with the committed entry.
   - `OpenPresetSetup(key)` / `CancelPresetSetup` — open/close the setup
     panel for `SetupUrl`/`Credentials` kinds.
   - `SubmitPresetSetup(key)` — validate (http(s) URL for `SetupUrl`,
     non-empty required fields for `Credentials`), create, then connect
     (SetupUrl) or stop (Credentials).
   - `CancelPresetConnect(key)` / retry maps onto `ConnectPreset`.
   `OpenPreset` and its editor-prefill path are deleted.

3. **Installed-detection.** `preset_plugin(preset, ctx) -> Option<(plugin,
   server)>`: a registry server matches when its URL equals the preset URL
   (OneClick), its URL host equals the preset's known host (`SetupUrl`:
   `mcp.composio.dev`), or its args contain the preset's npm package
   (`Credentials`). Drives invariant 7: card state + Manage. **Spec
   deviation, recorded:** Manage opens the plugin's Edit form
   (`OpenEdit`) rather than scrolling to the card — the scrollable has no
   anchor API, and the editor renders directly under the gallery, which
   satisfies the intent (get to the entry in one click). PRODUCT.md §7
   updated to match.

4. **Card rendering.** `render_preset_card` gains the status- and
   kind-dependent right side: primary `ActionButton` (Connect / Set up… /
   Add / Manage / Cancel / Retry — one pre-built handle per label per
   preset, stored in a new `PresetUi` struct replacing the bare
   `preset_hover` map), plus the existing `render_status_chip` for
   connecting/connected/error states, and `status.detail()` as a caption
   line for errors. The whole-card click target is removed (the button is
   the action); hover highlight stays.

5. **Setup panel.** `PresetSetup { key, url_editor?, field_editors:
   Vec<(env_key, ViewHandle<EditorView>)>, error, submit/cancel buttons }`
   on `PluginsPageState`, rendered as an inline card directly under the
   gallery (same visual class as the editor — the page's "no modals"
   precedent). Guidance line + `doc_link` rendered as a clickable
   sub-text (opens via `ctx.open_url`, the same call the OAuth consent
   uses at `mcp_oauth.rs:1048`). Submit disabled-state mirrors the
   existing pattern of validating on press and surfacing `error` (warpui
   ActionButton has no disabled affordance in this page today; pressing
   with invalid input shows the inline error instead — PRODUCT §4/§5's
   "disabled until valid" is implemented as validate-on-press, recorded
   as a second small deviation in PRODUCT.md).

6. **Copy.** "QUICK ADD" section header stays; add-custom affordance is
   the existing header "Add plugin" button (unchanged, already secondary
   relative to the gallery).

## Testing and validation

- **Unit (in-file `#[cfg(test)]`, extending the existing suite):**
  - preset classification: every `PRESETS` entry's kind ↔
    required-input matrix (P2), Composio preset carries no persistable
    placeholder URL (P4).
  - `preset_plugin` matching: URL match, host match, npm-arg match, and
    no-match → None (P7's no-duplicates guarantee).
  - setup validation: `SetupUrl` rejects non-http(s); `Credentials`
    rejects empty required fields (P4, P5).
- **Manual smoke (owner):** Notion card → Connect → browser consent →
  Connected chip on card, no form shown (P3); deny → Retry on card;
  Composio Set up… → URL guidance + dashboard link → Connect (P4);
  Slack Set up… → labeled token fields, Add → Installed, no consent
  claimed (P5); second press on an installed card → Manage opens Edit,
  no duplicate (P7); Add plugin still opens the 24c editor (P8);
  existing plugins untouched after upgrade (P13). Launch the app to
  verify — UI changes are never assumed correct from compilation alone
  (repo invariant).
- P9–P12 (installed cards, static-auth fallback, Codex header warning,
  relaunch mid-consent) are feature-24 behavior this change must not
  regress; covered by the same smoke.

## Parallelization

None — the work is one file plus spec text, tightly coupled around
`PluginsPageState`. A single sequential implementation is faster than
coordinating agents.
