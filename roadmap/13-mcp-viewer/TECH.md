# 13 — MCP viewer in the Claude pane (TECH)

Implements PRODUCT.md. Read-only MCP-server viewer surfaced as a composer pill +
popover in the Claude pane. All references below verified against the tree at spec
time; line numbers are approximate and will drift — match by symbol name.

## Architecture overview

Two data paths feed the viewer; both terminate in the `Transcript` (the pane's
per-session source of truth):

1. **Server list + status** — parsed from the stream-json `system`/`init` event in
   `crates/claude_code/src/driver.rs::parse_system`, carried on
   `TranscriptEvent::SessionInit`, and stored on `Transcript`.
2. **Per-server tools** — derived in `Transcript::apply` from each
   `mcp__<server>__<tool>` tool call already flowing through as `TranscriptItem::Tool`
   / `ToolCall` events. No new parsing; we bucket existing tool names by server.

The view (`app/src/claude_code_view.rs`) reads the assembled list via a getter and
renders a new `ComposerMenu::Mcp` pill + popover, mirroring the existing
permission/model/effort dropdowns exactly.

## Data model (crate `claude_code`)

`crates/claude_code/src/lib.rs`:

- **New struct** near the other transcript types:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct McpServerInfo {
      pub name: String,
      /// Raw status string from the init event ("connected" / "failed" / "pending" /
      /// other), or None if the event listed the server without a status.
      pub status: Option<String>,
      /// Tool names (with the `mcp__<server>__` prefix stripped), in first-seen order.
      pub tools: Vec<String>,
  }
  ```
- **`TranscriptEvent::SessionInit`** (currently ~lines 129–139): add
  `mcp_servers: Vec<McpServerInfo>` (tools empty at init; only name + status known
  there).
- **`Transcript`** (currently ~lines 261–274): add `mcp_servers: Vec<McpServerInfo>`.
  Default empty. Add getter `pub fn mcp_servers(&self) -> &[McpServerInfo]`.

### Parsing (driver)

`crates/claude_code/src/driver.rs::parse_system` (~635–673): the init event JSON
includes `"mcp_servers"` as an array of `{ "name": String, "status": String }`. Parse
defensively — the field may be absent (older CLI), an empty array, or missing
`status`:
```rust
let mcp_servers = value
    .get("mcp_servers")
    .and_then(|v| v.as_array())
    .map(|arr| arr.iter().filter_map(|s| {
        let name = s.get("name")?.as_str()?.to_owned();
        let status = s.get("status").and_then(|v| v.as_str()).map(str::to_owned);
        Some(McpServerInfo { name, status, tools: Vec::new() })
    }).collect())
    .unwrap_or_default();
```
Set it on the emitted `SessionInit`.

> **Verify at impl time:** confirm the exact init JSON shape against a live `claude`
> run (the field name and whether `status` is present). The memory note
> `twarp_07_claude_stream_json` documents the init event; cross-check it. If the field
> name differs, adjust the key — do not fake the data.

### Assembling tools (`Transcript::apply`)

`crates/claude_code/src/lib.rs::Transcript::apply` (~343–504):

- **`SessionInit` arm** (~345–360): store `event.mcp_servers` into
  `self.mcp_servers`. On resume the init replays, so overwrite (not append), but
  **preserve already-derived tools** if a server with the same name already has a
  non-empty `tools` vec (resume replays init before the tool history re-streams, so
  prefer keeping whatever is richer; simplest correct approach: when overwriting, carry
  over `tools` from any existing same-named entry).
- **Tool-call arm** (wherever `TranscriptItem::Tool` / a `ToolCall` is recorded): for a
  tool `name` starting with `mcp__`, split `mcp__<server>__<tool>` → `(server, tool)`,
  find-or-insert that server in `self.mcp_servers` (insert with `status: None` if the
  init event never listed it — defensive; normally it will exist), and push `tool` to
  its `tools` if not already present.
  - Reuse the exact prefix-stripping logic that `tool_cards.rs::tool_display_name`
    (~78–85) uses so parsing stays consistent: strip `mcp__`, then `split("__")`; first
    segment = server, remainder joined = tool.

Unit tests (`crates/claude_code`, mirror existing driver/transcript tests):
- `parse_system` extracts `mcp_servers` with name+status; tolerates missing field,
  missing status, empty array.
- `Transcript::apply` buckets `mcp__github__create_issue` under server `github` tool
  `create_issue`; dedups repeated calls; first-seen ordering preserved.
- Resume (replayed `SessionInit`) does not wipe derived tools.

## View (crate `warp`, `app/src/claude_code_view.rs`)

Mirror the **permission dropdown** end to end — it is the closest existing analog.

1. **`ComposerMenu` enum** (~273–278): add `Mcp` variant. Add `Mcp` to
   `render_composer_menu`'s match (~2285–2290) → `self.render_mcp_menu(appearance)`.
2. **State** on `ClaudeCodeView` (~314+, init in `new` ~601):
   - `mcp_pill_mouse: MouseStateHandle`
   - `mcp_menu_row_mouse: RefCell<Vec<MouseStateHandle>>` (pooled, like
     `permission_menu_row_mouse` ~471)
   - `mcp_expanded_server: Option<String>` — which server row is expanded (one at a
     time, per PRODUCT).
3. **Action** `ClaudeCodeViewAction` (~183–236): add
   `ToggleMcpServer(String)` (expand/collapse a server row). The pill itself reuses the
   existing `ToggleComposerMenu(ComposerMenu::Mcp)`.
4. **Pill render** — new `render_mcp_control(&self, appearance)` mirroring the
   permission pill (~2189–2208) via `render_clickable_pill` (~4120–4155). Label
   `format!("MCP · {}", self.transcript.mcp_servers().len())`. Dispatch
   `ToggleComposerMenu(ComposerMenu::Mcp)`. Add it to the controls row alongside the
   other pills.
   - **Streaming exception:** `toggle_composer_menu` (~1541) currently blocks all menus
     except `Context` while streaming. Add `Mcp` to that allow-list so the viewer opens
     mid-stream (read-only, like Context).
5. **Popover render** — new `render_mcp_menu(&self, appearance)` mirroring
   `render_permission_menu` (~2304–2362): a `Flex::column` in a bordered/rounded
   `Container`, header `MCP servers`, one `Hoverable` row per server (name + status
   dot/label + tool count). Expanded server (`mcp_expanded_server == Some(name)`)
   renders its tools as indented muted rows beneath it. Row click dispatches
   `ToggleMcpServer(name)`. Pool row mouse states in `mcp_menu_row_mouse` exactly as the
   permission menu does.
   - **Empty state:** when `mcp_servers().is_empty()`, render the two muted lines from
     PRODUCT (`No MCP servers connected.` + CLI hint) instead of rows.
   - **Status colors:** map `connected`→theme success/green, `failed`→error/red,
     `pending`/`None`→muted. Reuse whatever theme accessors the existing pills use; do
     not hardcode hex.
6. **Handler** `TypedActionView::handle_action` (~2961–3024): add arm for
   `ToggleMcpServer(name)` → toggle `self.mcp_expanded_server` (set to `Some(name)` or
   `None` if already expanded), `ctx.notify()`. `ToggleComposerMenu` is already wired
   (~3005).

No persistence: `mcp_expanded_server` and the menu open-state are ephemeral UI state,
consistent with the other composer menus.

## Per-file change matrix

| File | Change | Risk |
|------|--------|------|
| `crates/claude_code/src/lib.rs` | Add `McpServerInfo`; add `mcp_servers` to `SessionInit` + `Transcript`; getter; `apply` bucketing + resume-merge | Low |
| `crates/claude_code/src/driver.rs` | Parse `mcp_servers` in `parse_system` | Low (defensive, field optional) |
| `crates/claude_code` tests | Unit tests for parse + bucketing + resume | Low |
| `app/src/claude_code_view.rs` | `ComposerMenu::Mcp`, state fields, action variant, pill, popover, handler, streaming allow-list | Medium (UI, manual-verify) |

No changes to `settings_view/mcp_servers_page.rs` (stays an empty placeholder), no new
crates, no new subprocess calls.

## Sub-phasing

Single sub-phase **13a** — the data plumbing and the UI are tightly coupled and the
whole thing is small. The smoke test needs both halves to validate end-to-end, so they
ship together (consistent with the `twarp_bundle_when_not_testable` guidance).

## Risks / caveats

- **Tool list is incremental, not authoritative.** Derived from observed calls, so a
  freshly-started session shows `tools unknown` until Claude uses each server. This is
  by design (PRODUCT) and the only honest option without a separate `claude mcp list`
  call (out of scope). The `tools unknown` state must be shown, not hidden.
- **Init JSON shape is an assumption.** Verify the `mcp_servers` field name/shape
  against a live run before relying on it; the parse is defensive so a wrong key
  degrades to "0 servers", not a crash — but that would silently break the feature, so
  confirm with a real session during impl.
- **UI is a manual-verify surface** (this Mac can't fully run presubmit per
  `twarp_presubmit_tooling`); the smoke test in PRODUCT is the gate. `claude_code`
  crate unit tests give automated coverage of the data layer.
- **Theme/status colors** must come from theme accessors so the pinned-light feature-08
  sidebar and dark terminal themes both read correctly.
