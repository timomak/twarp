# 06 — Tab rename shortcut

**Phase:** not-started
**Spec PR:** —
**Impl PR:** —

## Scope

Bind `⌘⌥R` to the same rename flow that double-clicking a tab title invokes — focus the active tab's title, enter inline edit mode, commit on Enter / cancel on Escape. No new UI, just an additional input path into the existing rename action.

## Sub-phases

Single impl PR expected. The rename interaction already exists (double-click); this only wires a keybinding to it.

## Notes

- If the existing rename action isn't trivially callable from a keybinding, the spec phase needs to identify the cleanest seam (action enum, command-palette entry, etc.).
- Default `⌘⌥R` should be remappable via the same config surface as feature 01's tab-color shortcuts and feature 04's command shortcuts.
- No conflict expected with upstream cherry-picks — the tab title and rename codepaths are stable and this only adds a keybinding entry.
