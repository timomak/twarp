# 06 — Tab rename shortcut

**Phase:** merged ([#65](https://github.com/timomak/twarp/pull/65) merged 2026-05-26)
**Spec PR:** [#64](https://github.com/timomak/twarp/pull/64) (merged)
**Impl PR:** [#65](https://github.com/timomak/twarp/pull/65)

## Scope

Bind `⌘⌥R` to the same rename flow that double-clicking a tab title invokes — focus the active tab's title, enter inline edit mode, commit on Enter / cancel on Escape. No new UI, just an additional input path into the existing rename action.

## Sub-phases

Single impl PR. The rename interaction already exists (double-click); this only wires a keybinding to it.

- [x] Bind `⌘⌥R` (`cmdorctrl-alt-r`) to the existing `workspace:rename_active_tab` `EditableBinding`; add a zero-tab bounds guard to `rename_tab` (PRODUCT §7). [#65](https://github.com/timomak/twarp/pull/65)

## Notes

- The rename action was already wired (`WorkspaceAction::RenameActiveTab` + dispatch handler + `EditableBinding` with `.with_custom_action(CustomAction::RenameTab)`); it just shipped with no default chord. The default chord is assigned in `custom_tag_to_keystroke` (`app/src/util/bindings.rs`) — the same map every other menu shortcut uses — **not** via `EditableBinding::with_key_binding`. Plus the §7 guard.
- **Runtime-verification catch:** the spec's proposed `.with_key_binding("cmdorctrl-alt-r")` clobbered the binding's `Trigger::Custom(RenameTab)`, which the mac menu's `description_for_custom_action` lookup requires — the app panicked at startup (`app_menus.rs:122`, `default_name`'s `debug_assert`). Compile/clippy/`cargo test` didn't catch it; launching the app did. Fixed by routing the chord through `custom_tag_to_keystroke`, which preserves the custom trigger.
- Default `⌘⌥R` is remappable via the same keybindings settings surface as feature 01's tab-color shortcuts and feature 04's command shortcuts (the binding stays an `EditableBinding`).
- **Conflict check resolved (TECH §2):** macOS ⌘⌥R is conflict-free (no `cmd-alt-r` binding exists). The Linux/Windows ⌃⌥R shadow with `ResumeConversation` is provably dead post-AI-removal — `CAN_RESUME_CONVERSATION_KEY` can never be set (no conversation is ever created; `was_manually_cancelled` is hardcoded `false`).
- No conflict expected with upstream cherry-picks — the tab title and rename codepaths are stable and this only adds a keybinding entry.
