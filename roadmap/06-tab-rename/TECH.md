---
name: 06 — Tab rename shortcut
status: draft
---

# Tab rename shortcut — TECH

Companion to [PRODUCT.md](PRODUCT.md). Section numbers below refer to PRODUCT.md.

## Context

The entire tab-rename interaction already exists in this checkout — action, dispatch handler, inline editor, commit/cancel, and even a registered remappable binding. **The only gap is that the binding ships with no default key chord.** This feature attaches `cmdorctrl-alt-r` to that existing binding. There is no new action, no new handler, no new UI, and no new behavior.

The double-click path and the (currently unbound) keyboard path converge on the same inline-edit flow; this feature simply lights up the keyboard path.

Relevant files on master (all verified):

- `app/src/workspace/action.rs:110` — `WorkspaceAction::RenameActiveTab` already exists (no parameters; resolves the active tab in the handler). The sibling `RenameActivePane` at the following lines is the pane equivalent and is **out of scope**.
- `app/src/workspace/view.rs:18211` — dispatch handler already wired: `RenameActiveTab => self.rename_tab(self.active_tab_index, ctx)`.
- `app/src/workspace/view.rs:~4420` — `Workspace::rename_tab(index, ctx)` reads the tab's current committed title and delegates to `rename_tab_internal`.
- `app/src/workspace/view.rs:4396-4418` — `rename_tab_internal(index, title, ctx)`: activates the tab, sets `tab_being_renamed`, clears + pre-fills `tab_rename_editor` with the title via `insert_selected_text` (so the text is selected — PRODUCT §1), and focuses the editor.
- `app/src/workspace/view.rs:1254-1270` — `handle_tab_rename_editor_event`: `Enter | Blurred → finish_tab_rename` (commit), `Escape → cancel_tab_rename` (PRODUCT §2). `finish_tab_rename` at ~1290, `cancel_tab_rename` at ~1326.
- `app/src/workspace/util.rs:138, 233-247` — `WorkspaceState::tab_being_renamed: Option<usize>` plus `is_tab_being_renamed` / `set_tab_being_renamed` / `tab_being_renamed`.
- `app/src/tab.rs:1109-1151` — `render_tab_content` renders the inline `TextInput` when `is_tab_being_renamed()`, else the static title. `app/src/tab.rs:1805-1807` — the **double-click** handler dispatches `WorkspaceAction::RenameTab(tab_index)` (the per-index variant); the keyboard path uses the active-tab variant instead. Both reach `rename_tab` → `rename_tab_internal`.
- `app/src/workspace/view/vertical_tabs.rs:457-458, 2209-2210` — vertical-tabs double-click also dispatches `RenameTab(tab_index)`; the vertical layout renders the same rename editor (PRODUCT §9).
- **`app/src/workspace/mod.rs:1014-1021` — the change site.** The existing `EditableBinding`:
  ```rust
  app.register_editable_bindings([EditableBinding::new(
      "workspace:rename_active_tab",
      "Rename the current tab",
      WorkspaceAction::RenameActiveTab,
  )
  .with_group(bindings::BindingGroup::Settings.as_str())
  .with_custom_action(CustomAction::RenameTab)
  .with_context_predicate(id!("Workspace"))]);
  ```
  Note: no `.with_key_binding(...)` today — that is exactly what we add.
- `app/src/util/bindings.rs:69` — `CustomAction::RenameTab`. `app/src/app_menus.rs:509` — the application menu's **Rename Tab** entry is built from `CustomAction::RenameTab`, so binding a key surfaces ⌘⌥R in the menu automatically (PRODUCT §10).

## Proposed changes

### 1. Attach the default keybinding (the entire feature)

In `app/src/workspace/mod.rs`, add one builder call to the existing binding at lines 1014-1021:

```rust
app.register_editable_bindings([EditableBinding::new(
    "workspace:rename_active_tab",
    "Rename the current tab",
    WorkspaceAction::RenameActiveTab,
)
.with_group(bindings::BindingGroup::Settings.as_str())
.with_custom_action(CustomAction::RenameTab)
.with_context_predicate(id!("Workspace"))
.with_key_binding("cmdorctrl-alt-r")]);
```

`cmdorctrl-alt-r` resolves to **⌘⌥R on macOS** (the README contract and the primary target) and **Ctrl+Alt+R on Linux/Windows**. The `id!("Workspace")` predicate is unchanged and already gives PRODUCT §6's focus behavior: the shortcut is inactive when a modal/palette/settings-editor owns focus, and active when a terminal pane has focus (terminal panes don't push a competing context for this chord on macOS). Because the binding stays an `EditableBinding`, it remains remappable on the keybindings settings page (PRODUCT §10) with no extra work.

That is the whole change. No new action, handler, state, render path, persistence, telemetry, or feature flag.

### 2. Resolve / document the `alt-r` conflict (Linux/Windows)

`cmd-alt-r` is **free on macOS** — confirmed by grep: the only `*-alt-r` binding in the tree is `TerminalAction::ResumeConversation` (`app/src/terminal/view/init.rs:177-184`), which on macOS uses `cmd-shift-R`, not `cmd-alt-r`. So the macOS default has no conflict.

On **Linux/Windows**, that same `ResumeConversation` binding uses `ctrl-alt-r` — the same chord `cmdorctrl-alt-r` produces there. It is a `FixedBinding` scoped to `id!("Terminal") & !id!("IMEOpen") & id!(CAN_RESUME_CONVERSATION_KEY)`. Precedence:

- When a terminal pane is focused **and** a resumable conversation exists (`CAN_RESUME_CONVERSATION_KEY` set at `app/src/terminal/view.rs:25956`), the deeper terminal-context fixed binding wins → ⌃⌥R would resume, not rename.
- Otherwise (flag not set — i.e. no resumable conversation), that predicate fails and the `Workspace` editable rename binding wins → rename.

In twarp, AI was removed in feature 02, so a "resumable conversation" should never come into existence and `CAN_RESUME_CONVERSATION_KEY` should never be set — making the conflict effectively dead on Linux/Windows too. **The impl must verify this empirically** (build, focus a terminal on Linux/Windows or reason from `view.rs:25956` whether the flag can be set post-AI-removal). Decision tree for the impl:

- **If the flag is provably never set in twarp** → ship `cmdorctrl-alt-r` as-is; the conflict is theoretical. Note it in the PR description.
- **If the flag can still be set** → keep `cmdorctrl-alt-r` (mac is the primary, reviewed target and is clean; the binding is remappable as the escape hatch) and document the narrow Linux/Windows shadow in the PR. A `PerPlatformKeystroke` (mac `cmd-alt-r`, a non-conflicting Linux/Windows chord) is the fallback only if the owner wants zero Linux/Windows ambiguity; it diverges from the natural ⌘⌥R → Ctrl+Alt+R mapping, so don't take it unprompted.

Removing the now-defunct `ResumeConversation` binding is **feature-02 cleanup, out of scope here.**

Re-grep `cmdorctrl-alt-r`, `cmd-alt-r`, and `ctrl-alt-r` against `app/` and `crates/` immediately before adding the binding (same mitigation feature 01 used for `alt-[0-9]`), in case a conflict lands upstream between spec and impl.

### 3. Verify the existing flow's edge cases (no code expected)

These are guarantees the existing flow should already provide; the impl confirms them rather than building them. If any fails, the fix is small and localized:

- **Zero tabs (PRODUCT §7).** `RenameActiveTab → rename_tab(self.active_tab_index, ctx)` must not panic when there are no tabs. The action is already reachable today via the **Rename Tab** menu item (`app_menus.rs:509`), so the normal path is exercised; confirm `rename_tab` bounds-checks the index and returns cleanly on an empty/out-of-range tab list. Add a guard only if missing.
- **Re-press while renaming (PRODUCT §5).** Pressing ⌘⌥R while `is_tab_being_renamed()` is true re-enters `rename_tab_internal`, which re-sets the same `tab_being_renamed` index, clears the editor, and re-inserts the committed title selected. Confirm this is benign (single editor, title re-selected) — expected from reading the code; verify in the smoke test (step 5).
- **Vertical tabs (PRODUCT §9).** `rename_tab_internal` focuses the shared `tab_rename_editor` and sets `tab_being_renamed`; both the horizontal (`tab.rs`) and vertical (`vertical_tabs.rs`) render paths key off `is_tab_being_renamed()`. Confirm the inline editor renders in the vertical layout (smoke step 8).

## Testing and validation

| PRODUCT § | Verification |
|-----------|--------------|
| §1 (⌘⌥R → inline edit, text selected) | Smoke step 2. The binding routes to the existing `rename_tab_internal`, whose `insert_selected_text(title)` pre-selects the text — already covered by the double-click path. |
| §2 (Enter commits / Escape cancels / blur commits) | Smoke steps 3-4. Reuses `handle_tab_rename_editor_event`; no new logic. |
| §3 (only the active tab) | Smoke step 3 (tabs 1/3 unchanged). Handler renames `self.active_tab_index` only. |
| §4 (identical to double-click) | Both paths converge on `rename_tab` → `rename_tab_internal`; verified by inspection + smoke. |
| §5 (re-press while renaming is benign) | Smoke step 5. |
| §6 (focus rules) | Smoke step 6 (terminal running `top`). The `id!("Workspace")` predicate matches the established tab-keybinding behavior (same as feature 01). |
| §7 (zero-tab no-op) | Verify `rename_tab` bounds-checks (already reachable via the menu); no panic. |
| §8 (multiple windows) | Manual: state is per-`Workspace` (per-window); the action dispatches per-window via `id!("Workspace")`. |
| §9 (horizontal + vertical layouts) | Smoke step 8. |
| §10 (discoverability + remap) | Smoke step 7 — settings page lists "Rename the current tab" = ⌘⌥R; the menu shows ⌘⌥R via `CustomAction::RenameTab`. |
| §11 (no feature flag) | The binding is registered unconditionally alongside the existing one. |
| §12 (persistence/telemetry unchanged) | No code touches persistence or telemetry; commit goes through the existing `finish_tab_rename`. |
| macOS conflict-free | Smoke step 9. Plus the grep mitigation in change §2. |

No new unit tests are warranted — the change adds zero new logic; it attaches a key chord to an already-tested action and flow. The keymap/binding layer is config-shaped and has its own coverage (consistent with feature 01's "integration test: skip" stance). The manual smoke test is the canonical pre-merge check.

Run `./script/presubmit` until green before opening the impl PR (twarp-next workflow rule).

## Risks and mitigations

- **Risk: Linux/Windows `ctrl-alt-r` collides with `ResumeConversation` in a focused terminal with a resumable conversation.** Mitigation: see change §2 — verify the `CAN_RESUME_CONVERSATION_KEY` flag is dead in twarp post-AI-removal (expected), keep the mac-clean `cmdorctrl-alt-r`, document the narrow edge, and rely on remappability. Per-platform split is the fallback only if the owner asks.
- **Risk: a new `alt-r` binding lands upstream between spec and impl.** Mitigation: re-grep `cmdorctrl-alt-r` / `cmd-alt-r` / `ctrl-alt-r` immediately before adding the binding.
- **Risk: `rename_tab` panics on an empty tab list.** Mitigation: change §3 — confirm the bounds check; the action is already menu-reachable, so this path is exercised today.

## Follow-ups

- Removing the defunct `TerminalAction::ResumeConversation` keybinding (feature-02 AI-removal cleanup) — out of scope for 06; would eliminate the theoretical Linux/Windows conflict entirely.
- Persisting user remaps is already handled by the existing keybindings settings surface; no follow-up needed.
