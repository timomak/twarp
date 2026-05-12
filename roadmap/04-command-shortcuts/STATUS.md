# 04 — Custom command shortcuts

**Phase:** merged
**Spec PR:** [#51](https://github.com/timomak/twarp/pull/51)
**Impl PR(s):**
- 4a — runtime: [#52](https://github.com/timomak/twarp/pull/52) (merged)
- 4b — hot reload + GUI data layer: [#53](https://github.com/timomak/twarp/pull/53) (closed; content shipped via #54's squash)
- 4c — side-panel tab + stub view: [#54](https://github.com/timomak/twarp/pull/54) (merged, includes 4b)
- 4d — inline detail editor + keymap unregister API: [#55](https://github.com/timomak/twarp/pull/55) (merged)

## Scope

Declarative keybindings → action sequence (`new_tab`, `new_pane`, `type`, `press`, `wait`) plus a side-panel GUI for create/preview/edit/delete next to "Global search". See [PRODUCT.md](PRODUCT.md) for behavior invariants and the two pinned driving examples; [TECH.md](TECH.md) for the implementation plan.

## Sub-phases

Split into three sub-PRs (refined from TECH §"Sub-phase split" — the GUI half of the original 4b proved larger than one PR could carry, so the file-watch reload landed as its own increment in 4b with the GUI moving to 4c):

- [x] **4a — Runtime.** Parser + executor + bindings + Escape-cancel + startup toast. Hand-edit `shortcuts.yaml`, restart to apply. Covers PRODUCT §§1–23, §25.
- [x] **4b — Hot reload + GUI data layer.** File watcher → registry reload (PRODUCT §24), `serialize_shortcuts`/`save_to_disk` (PRODUCT §§36-37), conflict detection (PRODUCT §38), shortcut-summary helper. Covers PRODUCT §24 and primes 4c's data needs. Users hand-edit `shortcuts.yaml`; edits take effect immediately without restart. *Shipped via #54's squash (4c branched off 4b); standalone PR #53 closed as redundant.*
- [x] **4c — Side-panel tab + interactive list view.** `ToolPanelView::Shortcuts` registered next to Global Search with a keyboard-icon toolbelt button. Panel content: header with "+ New shortcut" link + one row per registered shortcut showing the shortcut's name (or arrow-form action summary as fallback) on the left and a styled chord pill (boxed per-modifier glyphs) on the right; long names truncate with `…`. Empty-state hint when nothing is configured. "+ New shortcut" appends a placeholder bound to the first unused chord from `cmd-shift-J..Z` with a `new_tab` action and a default `New shortcut` name, saves via 4b's `save_to_disk`, and 4b's hot reload picks up the change. **Left-click a row** dispatches `WorkspaceAction::OpenFileInNewTab` for `shortcuts.yaml` — opens the YAML in a new twarp tab, not the OS editor. **Right-click a row** reveals an inline Delete affordance beneath it; clicking elsewhere closes it. `Shortcut` gains an optional `name: Option<String>` field that the parser and serializer round-trip. Covers PRODUCT §§3 (name field), 26, 27, 28, 29, 30 (open-in-tab variant), 31.
- [x] **4d — Inline detail editor + keymap unregister API.** Side-panel detail editor: name (`ClickableTextInput`), chord (keystroke-capture widget via `EventHandler::on_keydown` + `disable_key_bindings_dispatching`), action editor with cycle-buttons for the enum-valued shapes (action kind, `new_pane` direction, `press` key) and `ClickableTextInput` for free-text shapes (`type` text, `wait` duration), per-row reorder/remove, [+ Add action], Save/Cancel with inline validation banner. "+ New shortcut" now opens the editor in create mode (replacing 4c's placeholder-append); right-click row menu gains `[Edit]` next to `[Delete]`. Plus `Keymap::unregister_editable_bindings_with_name_prefix` in `warpui_core` + Matcher/AppContext wrappers, used at the top of `register_shortcut_bindings` so hot reload replaces the previous shortcut binding generation instead of appending. Covers PRODUCT §§30 (inline edit), 32 (keystroke capture), 33 (action editor — dropdowns implemented as cycle-buttons to avoid a `FilterableDropdown` View handle per row), 36 (save + rollback on disk failure). Known deferred-to-follow-up: PRODUCT §38 conflict warnings shown beneath the chord field, PRODUCT §34's per-field-highlighting (current implementation surfaces validation as a single inline banner above Save).

The feature reaches `merged` only when all sub-PRs ship.

## Notes

- `cmdorctrl-shift-D` (the first driving example) conflicts with twarp's built-in **Split pane right** by design; the custom shortcut shadows the built-in and reproduces the split as the first action of the sequence. See PRODUCT §16 and TECH §1.
- **4b binding leak fixed in 4d.** `warpui_core::Keymap::unregister_editable_bindings_with_name_prefix` lets `register_shortcut_bindings` drop the previous generation before the new one registers, so the keymap's editable-binding vector no longer grows by ~N entries per reload.
