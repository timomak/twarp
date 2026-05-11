# 04 — Custom command shortcuts

**Phase:** spec-in-review
**Spec PR:** [#51](https://github.com/timomak/twarp/pull/51)
**Impl PR(s):**
- 4a — runtime: —
- 4b — side-panel GUI + hot reload: —

## Scope

Declarative keybindings → action sequence (`new_tab`, `new_pane`, `type`, `press`, `wait`) plus a side-panel GUI for create/preview/edit/delete next to "Global search". See [PRODUCT.md](PRODUCT.md) for behavior invariants and the two pinned driving examples; [TECH.md](TECH.md) for the implementation plan.

## Sub-phases

Confirmed split into two sub-PRs (TECH §"Sub-phase split"):

- [ ] **4a — Runtime.** Parser + executor + bindings + Escape-cancel + startup toast. Hand-edit `shortcuts.yaml`, restart to apply. Covers PRODUCT §§1–23, §25.
- [ ] **4b — GUI + hot reload.** `ToolPanelView::Shortcuts` view (list + detail editor + keystroke capture + validation + conflict warnings) + file-watch reload. Covers PRODUCT §24 and §§26–38.

The feature reaches `merged` only when both sub-PRs ship.

## Notes

- `cmdorctrl-shift-D` (the first driving example) conflicts with twarp's built-in **Split pane right** by design; the custom shortcut shadows the built-in and reproduces the split as the first action of the sequence. See PRODUCT §16 and TECH §1.
