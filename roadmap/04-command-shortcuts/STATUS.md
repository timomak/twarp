# 04 — Custom command shortcuts

**Phase:** impl-in-review
**Spec PR:** [#51](https://github.com/timomak/twarp/pull/51)
**Impl PR(s):**
- 4a — runtime: [#52](https://github.com/timomak/twarp/pull/52) (merged)
- 4b — hot reload + GUI data layer: [#53](https://github.com/timomak/twarp/pull/53)
- 4c — side-panel tab + stub view: [#54](https://github.com/timomak/twarp/pull/54)
- 4d — inline GUI features: —

## Scope

Declarative keybindings → action sequence (`new_tab`, `new_pane`, `type`, `press`, `wait`) plus a side-panel GUI for create/preview/edit/delete next to "Global search". See [PRODUCT.md](PRODUCT.md) for behavior invariants and the two pinned driving examples; [TECH.md](TECH.md) for the implementation plan.

## Sub-phases

Split into three sub-PRs (refined from TECH §"Sub-phase split" — the GUI half of the original 4b proved larger than one PR could carry, so the file-watch reload landed as its own increment in 4b with the GUI moving to 4c):

- [x] **4a — Runtime.** Parser + executor + bindings + Escape-cancel + startup toast. Hand-edit `shortcuts.yaml`, restart to apply. Covers PRODUCT §§1–23, §25.
- [ ] **4b — Hot reload + GUI data layer.** File watcher → registry reload (PRODUCT §24), `serialize_shortcuts`/`save_to_disk` (PRODUCT §§36-37), conflict detection (PRODUCT §38), shortcut-summary helper. Covers PRODUCT §24 and primes 4c's data needs. Users hand-edit `shortcuts.yaml`; edits take effect immediately without restart.
- [ ] **4c — Side-panel tab + interactive list view.** `ToolPanelView::Shortcuts` registered next to Global Search with a keyboard-icon toolbelt button. Panel content: header with "+ New shortcut" link + one row per registered shortcut showing the shortcut's name (or arrow-form action summary as fallback) on the left and a styled chord pill (boxed per-modifier glyphs) on the right; long names truncate with `…`. Empty-state hint when nothing is configured. "+ New shortcut" appends a placeholder bound to the first unused chord from `cmd-shift-J..Z` with a `new_tab` action and a default `New shortcut` name, saves via 4b's `save_to_disk`, and 4b's hot reload picks up the change. **Left-click a row** dispatches `WorkspaceAction::OpenFileInNewTab` for `shortcuts.yaml` — opens the YAML in a new twarp tab, not the OS editor. **Right-click a row** reveals an inline Delete affordance beneath it; clicking elsewhere closes it. `Shortcut` gains an optional `name: Option<String>` field that the parser and serializer round-trip. Covers PRODUCT §§3 (name field), 26, 27, 28, 29, 30 (open-in-tab variant), 31.
- [ ] **4d — Inline detail editor + conflict UI + errors banner.** Detail editor (keystroke-capture widget, action editor with type dropdowns and per-row reorder), inline validation surfacing per-field errors, conflict warnings inline beneath the chord field, errors banner for malformed YAML, and a proper deregister/replace API for the keymap so hot reload stops leaking duplicate bindings. Covers the rest of PRODUCT §§30, 32-38. Consumes 4b's serialize/save/conflict/summary scaffolding.

The feature reaches `merged` only when all sub-PRs ship.

## Notes

- `cmdorctrl-shift-D` (the first driving example) conflicts with twarp's built-in **Split pane right** by design; the custom shortcut shadows the built-in and reproduces the split as the first action of the sequence. See PRODUCT §16 and TECH §1.
- **Known 4b limitation:** each hot reload calls `workspace::register_shortcut_bindings` again, which appends to the keymap's editable-binding vector (warpui has no public unregister API). Functionally fine — reverse-iteration in the matcher means the latest set wins — but memory grows by ~N bindings per reload. A proper deregister/replace API is a 4c follow-up.
