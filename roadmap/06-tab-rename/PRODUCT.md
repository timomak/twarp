---
name: 06 — Tab rename shortcut
status: draft
---

# Tab rename shortcut — PRODUCT

## Summary

A keyboard shortcut — **⌘⌥R** — that puts the active tab's title into inline edit mode, exactly as double-clicking a tab title does today. The title text is pre-selected so typing replaces it; **Enter** (or clicking away) commits the new name, **Escape** cancels. This feature adds no new UI and no new rename behavior — it is a second input path into twarp's existing tab-rename interaction, so users can rename a tab without reaching for the trackpad. The default chord is remappable through the existing keybindings settings page.

## Goals / Non-goals

**Goals**
- Bind ⌘⌥R to the existing "rename the active tab" action so the keyboard reaches the same inline-edit flow double-click already triggers.
- Produce a result identical to double-clicking the tab title: same inline editor, same pre-selected text, same Enter-commits / Escape-cancels semantics.
- Surface the shortcut where it's already discoverable — the keybindings settings page and the existing "Rename Tab" menu entry — and keep it remappable there.

**Non-goals (deferred / out of scope)**
- Any change to the rename interaction itself (the inline editor, commit/cancel rules, title validation, persistence). This feature only adds a keystroke that enters the existing flow.
- A new rename UI, dialog, or command-palette-specific affordance beyond what already exists.
- Renaming panes (a separate `workspace:rename_active_pane` action already exists and is intentionally untouched here) or renaming non-active tabs by keyboard.
- Bulk rename, rename templates, or auto-naming rules.
- Changing what an empty or whitespace-only committed title does — the keyboard path inherits whatever the double-click path does today.

## Behavior

1. While twarp is the focused application and a tab is active, pressing **⌘⌥R** (Ctrl+Alt+R on Linux/Windows) puts the **active tab's** title into inline edit mode: the tab title is replaced by an inline text editor pre-filled with the tab's current title, with that text **selected** so the first keystroke replaces it. This is the same editor and the same entry point that double-clicking the tab title uses.

2. **Commit / cancel** reuse the existing rename mechanics:
   - **Enter** commits the edited title and exits edit mode.
   - **Escape** cancels: the title reverts to its pre-edit value and edit mode exits.
   - **Clicking away / losing focus** commits the current editor contents (same as the existing flow's blur behavior).

3. "Active tab" means the single tab that currently owns workspace focus in the focused window. Tabs in other windows, and inactive tabs in the focused window, are never affected by the shortcut.

4. The result of pressing ⌘⌥R is **identical** to double-clicking the active tab's title. There is exactly one tab-rename interaction in twarp; this feature adds a keyboard input path to it, not a parallel flow.

5. **Re-pressing while already renaming** is benign: pressing ⌘⌥R while a tab is already in inline-edit mode re-arms the rename on the active tab — the editor is reset to the tab's current committed title with that text selected. No second editor opens; no crash.

6. **Focus rules** follow normal keybinding precedence, consistent with twarp's other ⌘⌥ shortcuts (e.g. the tab-color shortcuts from feature 01):
   - When focus is in a surface that captures the keystroke (an open modal, command palette, or settings-page text editor), that surface handles the key and no rename starts.
   - When the user is typing in a terminal pane (the normal case), ⌘⌥R still starts the rename of the active tab — terminal panes do not swallow it.

7. **Zero tabs:** with no tab to rename (an unusual state), the shortcut is a no-op — no error, no crash.

8. **Multiple windows:** each window has its own active tab; ⌘⌥R in window A renames only window A's active tab.

9. **Tab layouts:** the shortcut works in both the horizontal tab bar and the vertical-tabs layout — both render the same inline rename editor for the tab being renamed.

10. **Discoverability and remapping:** the action already appears in twarp as "Rename the current tab" in the keybindings settings page and as a **Rename Tab** entry in the application menu. After this feature, both surfaces show **⌘⌥R** as the bound shortcut. Users can rebind, unbind, or reassign it through the existing keybindings settings page — no special UI is added for this feature.

11. **No feature flag.** Like other built-in default keybindings, the shortcut ships unconditionally.

12. **Persistence and telemetry are unchanged.** A committed rename updates the tab title and persists exactly as a double-click rename does today; the shortcut emits no telemetry distinct from the existing rename flow.

## Smoke test

Run against a freshly built twarp binary.

1. Open twarp. Open three tabs. Focus tab 2.
2. Press **⌘⌥R**. Tab 2's title becomes an inline editor with the current title selected.
3. Type `prod` and press **Enter**. Tab 2's title is now `prod`; edit mode exits. Tabs 1 and 3 are unchanged.
4. Press **⌘⌥R** on tab 2 again, type `staging`, then press **Escape**. Tab 2's title reverts to `prod` (the edit is discarded).
5. Press **⌘⌥R**, then press **⌘⌥R** again while still editing. A single inline editor remains, reset to `prod` with the text selected — no second editor, no crash. Press **Escape**.
6. Focus a terminal pane and run a foreground process (e.g. `top`). Press **⌘⌥R**. The active tab enters rename mode and the running process is unaffected. Press **Escape**.
7. Open the keybindings settings page. Confirm "Rename the current tab" lists **⌘⌥R** and is rebindable. Open the application menu and confirm the **Rename Tab** entry shows **⌘⌥R**.
8. Switch to the vertical-tabs layout (if enabled in this build). Focus a tab, press **⌘⌥R**, rename it, press **Enter** — the rename works the same as in the horizontal tab bar.
9. (macOS) Confirm ⌘⌥R does not collide with an existing shortcut: with a terminal focused, ⌘⌥R renames the tab rather than triggering any other action.
