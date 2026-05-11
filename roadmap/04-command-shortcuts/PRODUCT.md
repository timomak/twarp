---
name: 04 — Custom command shortcuts
status: draft
---

# Custom command shortcuts — PRODUCT

## Summary

A declarative way to bind a keyboard shortcut to a sequence of terminal actions: open a new tab, split the active pane, type literal text, press named keys, wait. Lets users compress frequent multi-step workflows ("split right and start tool X", "open a pane, run a known command sequence after a delay") into a single keystroke. Shortcuts live in a YAML file alongside twarp's existing settings. A side-panel GUI — slotted into the left panel next to global search — lets users create, preview, edit, and delete shortcuts without hand-editing YAML; both surfaces share one in-memory model so changes from either side take effect immediately. v1 ships with no built-in shortcuts.

## Goals / Non-goals

**Goals**

- Declarative YAML config: a list of `(keys, actions)` pairs that a user can hand-edit.
- A minimal but complete v1 action vocabulary — `new_tab`, `new_pane`, `type`, `press`, `wait` — sufficient for the two driving examples below.
- Platform-neutral key normalization using the same `cmdorctrl-shift-<key>` convention twarp's existing `keybindings.yaml` uses (matches `Keystroke::parse` exactly).
- A clean cancellation hatch: Escape aborts an in-flight sequence.
- Clear, actionable user-facing error messages when the config is malformed — every error names the offending shortcut and the expected shape.
- Invalid entries are skipped, not fatal: one bad entry never disables the feature or blocks twarp from launching.
- A side-panel GUI for create / preview / edit / delete of shortcuts, sitting next to "Global search" in the left panel. The GUI and the YAML file stay in sync: a change made through either takes effect immediately, no restart.

**Non-goals**

- Variables, conditionals, loops, parameter prompts, or any form of programmable control flow. v1 sequences are literal lists of actions.
- Per-tab, per-pane, or context-scoped shortcuts. v1 keybindings are global to twarp.
- Reading terminal output, conditioning behavior on prompt state, or any form of "wait until prompt is ready". v1 only knows wall-clock time.
- Modifier keys inside `press` (e.g. `press: ctrl-c`). v1 `press` is a named key only. Sending control characters is a follow-up.
- Shortcuts that trigger other twarp keybindings. `type` and `press` write to the active pane's PTY only — they do not dispatch through twarp's own keybinding handler.
- Shortcut presets beyond the two driving examples. v1's default `shortcuts.yaml` writes those two entries on first launch (§1); larger bundled or themed shortcut packs are not part of v1.
- Preserving comments / hand-formatting in `shortcuts.yaml` when the GUI rewrites it. Hand-edits round-tripped through the GUI lose comments and may be reformatted.
- Drag-to-reorder in the GUI's action editor. v1 reorders via up/down buttons.
- Multi-keystroke chord sequences (e.g. `cmd-k cmd-s`). v1 is single-chord only.

## Driving examples

These two examples are the v1 acceptance bar: PRODUCT and TECH must support them exactly, and the smoke test below exercises both.

```yaml
shortcuts:
  - keys: cmdorctrl-shift-D
    actions:
      - new_pane: right
      - wait: 1500ms
      - type: "claude"
      - press: enter

  - keys: cmdorctrl-shift-A
    actions:
      - new_pane: right
      - wait: 1500ms
      - type: "claude"
      - press: enter
      - wait: 3s
      - type: "/address-code-review-comments"
      - press: enter
```

- `⌘⇧D` (macOS) / `Ctrl+Shift+D` (Linux/Windows) opens a new pane to the right of the active pane, types `claude`, submits.
- `⌘⇧A` / `Ctrl+Shift+A` does the same, then waits three seconds, types `/address-code-review-comments`, submits.

The `1500ms` wait between `new_pane` and `type` gives the new pane's shell time to finish bootstrapping. Warp's shell-bootstrap script typically completes ~900ms–1s after spawn; the 1500ms default leaves headroom on slower machines. Without the wait, the typed input arrives before bootstrap completes, the shell's prompt-state detection misses the bootstrap signal, and a "Bootstrapping slow" toast appears. Tune the value per machine if you want a snappier sequence.

Note: `⌘⇧D` is the default chord for twarp's built-in **Split pane right** action. Custom shortcuts shadow built-ins (§16), so this binding overrides the built-in while reproducing its split as the first action of the sequence. Users who want to keep the built-in unchanged should pick a different chord.

## Behavior

### Config

1. **Config file.** Custom shortcuts are declared in `shortcuts.yaml`, located in the same directory as twarp's existing `keybindings.yaml` and `settings.toml` (twarp's per-user config directory). **If the file does not exist at startup, twarp writes a default `shortcuts.yaml` containing the two driving examples (see §Driving examples) and loads it.** Subsequent launches find the file and load it without rewriting; any user edits are preserved. If the bootstrap write fails (read-only filesystem, permission denied), a single warning is logged and no shortcuts are loaded — twarp still launches normally.

2. **Top-level shape.** `shortcuts.yaml` is a YAML map with a single top-level key, `shortcuts:`, whose value is a list of shortcut entries (see §Driving examples for the canonical shape). An empty `shortcuts:` list (or a totally empty file) loads zero shortcuts with no error. Any other top-level shape (top-level list, top-level scalar, an unexpected top-level key) is a config error (§20).

3. **Shortcut entry shape.** Each entry is a map with two required fields: `keys` (the chord string) and `actions` (a non-empty list of actions). Unknown fields on an entry are an error (§20). Either required field being absent or empty is an error.

4. **Key chord normalization.** The `keys` value is a single chord string: zero or more modifier names joined by `-`, followed by `-<key>` — the same syntax twarp's built-in `keybindings.yaml` already uses, and the exact syntax accepted by `Keystroke::parse`. Recognized modifiers: `cmd`, `ctrl`, `cmdorctrl` (resolves to ⌘ on macOS, Ctrl on Linux/Windows — matches the convention used by built-in twarp bindings), `alt`, `shift`, `meta`. The key portion is a single printable character (`0`–`9`, `a`–`z` / `A`–`Z`, common punctuation) or one of the named keys listed in §10. Order of modifiers does not matter (`shift-cmd-D` and `cmd-shift-D` parse to the same chord). When `shift` is in the modifier set together with a letter, the letter must be uppercase (`cmd-shift-D`, not `cmd-shift-d`); this matches the existing `Keystroke::parse` rule. The YAML loader auto-uppercases `shift-<lowercase-letter>` so users can spell either way. Multi-stroke chords (e.g. `cmd-k cmd-s`) are **not** supported in v1.

5. **Action vocabulary.** v1 supports exactly five actions:

    | Action      | Shape                          | Effect                                                                                                  |
    |-------------|--------------------------------|---------------------------------------------------------------------------------------------------------|
    | `new_tab`   | bare string `new_tab`          | Opens a new tab in the focused window and updates the sequence's target tab to the new tab. (§6, §8)    |
    | `new_pane`  | map `new_pane: <direction>`    | Splits the target tab's active pane (`right` or `down`); the new pane becomes the target's active pane. (§7) |
    | `type`      | map `type: "<text>"`           | Writes the literal text to the target tab's active pane's PTY, as if the user had typed/pasted it. (§9) |
    | `press`     | map `press: <named-key>`       | Writes the byte sequence for the named key to the target tab's active pane's PTY. (§10)                 |
    | `wait`      | map `wait: <duration>`         | Pauses the sequence for the given duration before running the next action. (§11)                       |

    Any other action token is a config error (§20). Action items are either a bare string (for parameterless actions, currently only `new_tab`) or a single-key map (for parameterized actions). A multi-key map on a single action item is an error.

### Action semantics

6. **`new_tab`.** Opens a new tab in the focused twarp window — the same operation the user's default "new tab" keybinding triggers — and makes it the sequence's target tab. If twarp has no open windows when the action runs, the sequence aborts (§17). If twarp is open but unfocused, `new_tab` operates on the most recently focused twarp window.

7. **`new_pane`.** Splits the target tab's currently active pane and focuses the new pane. The `direction` value is one of:
    - `right` — split horizontally; new pane to the right of the existing one.
    - `down` — split vertically; new pane below the existing one.

    Any other value (or omission) is a config error (§20). The new pane becomes the target tab's active pane for subsequent `type` and `press` actions. `new_pane` does not change the target tab — only the active pane within it. If the target tab has been closed before `new_pane` runs, the sequence aborts (§17). `new_pane` operates regardless of any feature-flag gates that the built-in "Split pane" UI carries: a custom shortcut authored by the user is treated as user intent to split, even in contexts where the built-in split is suppressed.

8. **Sequence target.** Every sequence has a single "target tab": the tab into which `type` and `press` actions write. The target tab is set when the sequence starts to the active tab of the focused twarp window. Each `new_tab` action updates the target tab to the newly created tab; `new_pane` does not change the target tab. Subsequent user focus changes (clicking another tab, alt-tabbing to another app) do **not** change the sequence's target tab — a running sequence delivers its remaining actions to the tab it captured, not to whatever the user is currently looking at. Within the target tab, `type` and `press` target the tab's currently active pane, and `new_pane` switches that active pane to the newly created one — so consecutive `new_pane` → `type` actions place text in the new pane, not the old one.

9. **`type`.** Writes the literal UTF-8 text of the `type:` value to the target tab's active pane's PTY. The text is treated as if pasted: no shell expansion happens client-side, no keyboard-shortcut interpretation by twarp, no triggering of twarp keybindings even if the typed text contains a chord-shaped substring. Leading and trailing whitespace are preserved exactly as written. Newline characters (`\n`, `\r`, `\r\n`) inside a `type` value are a config error (§20) — to submit input, use `press: enter`. Tab characters are allowed but discouraged; prefer `press: tab` for clarity.

10. **`press`.** Writes the byte sequence corresponding to the named key to the target tab's active pane's PTY. v1 supports these key names: `enter` (CR), `tab` (HT), `escape`, `backspace`, `space`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `delete`, `insert`, `numpadenter`, `f1`–`f12`. Modifier keys inside `press` are not supported in v1 — `press: ctrl-c` is a config error (§20). `press` does not dispatch through twarp's keybinding handler; it only writes bytes to the PTY. Whether the running program does anything with a given key sequence (e.g. `press: f5`) is up to that program.

11. **`wait`.** Pauses sequence execution for the duration named in the `wait:` value. Accepted units: `ms` (milliseconds), `s` (seconds), `m` (minutes), written without spaces (`500ms`, `2s`, `1m`). The duration must be ≥ 1ms and ≤ 60s; values outside that range are a config error (§20). During a `wait`, the user retains full control of twarp — they can type into any pane, switch tabs, switch windows, switch apps. User input during a `wait` is not blocked, not captured by the sequence, and not buffered.

### Running a sequence

12. **Trigger.** Pressing a chord that matches a shortcut entry's `keys` field starts that sequence. Sequences run asynchronously: the chord press returns immediately, and actions dispatch one after another per the rules above.

13. **Single in-flight sequence.** Only one custom shortcut sequence runs at a time. If a sequence is in flight and the user presses a chord bound to another (or the same) shortcut, the new trigger is **ignored** — the running sequence continues to completion. Chaining shortcuts via synthetic key presses is out of scope; queueing would create surprising delays.

14. **Cancellation via Escape.** Pressing the bare `escape` key (no modifiers) anywhere in twarp while a sequence is in flight aborts the rest of the sequence: no `type`, `press`, `wait`, `new_pane`, or `new_tab` actions run after the in-flight one finishes. A `wait` is interrupted immediately; a `type` or `press` that has already started writing is not rolled back. The aborting Escape press is consumed by the abort handler and is **not** also passed through to the active pane.

15. **No abort on focus loss or other input.** Alt-tabbing away, clicking another tab, scrolling, or typing normal keys (anything other than bare Escape) does not abort a running sequence. The sequence keeps delivering actions to its sticky target tab (§8) and the active pane inside it.

16. **Built-in binding conflict.** If a custom shortcut's `keys` matches a chord already bound to a built-in twarp action, the custom shortcut wins — the built-in is shadowed for as long as the custom shortcut is loaded. The driving example `cmdorctrl-shift-D` is an intentional demonstration: it overrides the built-in "Split pane down" while reproducing a (right-side) split as the first action of the sequence. The user remains free to rebind the built-in elsewhere via twarp's standard keybindings settings page. Shadowing a built-in is not a config error; it is a supported use of the feature.

    On macOS, several built-in actions are exposed as NSMenu items with key equivalents (e.g. `cmd-shift-D` for "Split Pane Down"). Without intervention, the OS-level menu intercepts the chord before the app's keymap matcher sees it. When a custom shortcut claims a chord that matches a menu item's key equivalent, twarp removes that key equivalent at startup — the menu entry remains and is still clickable, but its keyboard shortcut belongs to the custom shortcut. Adding or removing a custom shortcut takes effect on menu items at the next twarp restart (in 4a). Once 4b ships hot reload, the menu also refreshes on save.

17. **Target lost.** If the sequence's target tab is closed mid-sequence (by the user, by an OS event, or by another twarp action), the sequence aborts at that point with no error notification. If the target pane within the target tab is closed mid-sequence, the sequence aborts (the executor does not "fall back" to a sibling pane). If the focused twarp window is closed mid-sequence and no twarp window remains, the sequence aborts.

### Errors and edge cases

18. **Invalid entries are skipped, not fatal.** When `shortcuts.yaml` is parsed, every entry that fails validation (§20) is dropped from the loaded set with its error message recorded. The remaining valid entries are loaded and become active. A malformed config never prevents twarp from launching and never disables shortcuts wholesale.

19. **Error surfacing.** When `shortcuts.yaml` produces any errors — at startup or on any subsequent reload — twarp surfaces a single non-modal toast notification of the form `shortcuts.yaml has N error(s) — see logs or open Custom shortcuts`. The full error list is also written to twarp's log file and shown inline as a banner in the Custom shortcuts side panel (§35). The toast is dismissable and shown at most once per load event; it does not steal focus.

20. **Config validation error messages.** Each validation error produces a log line and panel-banner entry of the form `shortcuts.yaml: <message>`. The message identifies the offending entry by its 1-indexed position in the `shortcuts:` list and (where helpful) its `keys` value. The full set of recognized errors and their exact message templates — these are part of the user-facing contract:

    | Condition | Message |
    |---|---|
    | Top-level is not a `shortcuts:` map | `expected top-level 'shortcuts:' key with a list value` |
    | `shortcuts:` value is not a list | `'shortcuts:' must be a list, got <YAML type>` |
    | Entry missing `keys` | `entry #<n>: missing required field 'keys'` |
    | Entry missing or empty `actions` | `entry #<n> ('<keys>'): missing required field 'actions' (must have at least one action)` |
    | Unknown field on entry | `entry #<n> ('<keys>'): unknown field '<field>'; expected 'keys' and 'actions'` |
    | Malformed key chord | `entry #<n>: invalid key chord '<chord>'; expected modifiers (cmdorctrl/cmd/ctrl/alt/shift/meta) joined by '-' with a key, e.g. 'cmdorctrl-shift-D'` |
    | Action is not a string or single-key map | `entry #<n> ('<keys>'), action #<m>: expected a bare action name or a single-key map` |
    | Unknown action token | `entry #<n> ('<keys>'), action #<m>: unknown action '<token>'; expected one of new_tab, new_pane, type, press, wait` |
    | Missing `new_pane` direction | `entry #<n> ('<keys>'), action #<m>: 'new_pane' requires a direction; expected 'right' or 'down'` |
    | Invalid `new_pane` direction | `entry #<n> ('<keys>'), action #<m>: invalid 'new_pane' direction '<value>'; expected 'right' or 'down'` |
    | `type` value is not a string | `entry #<n> ('<keys>'), action #<m>: 'type' expects a string value` |
    | Newline in `type` | `entry #<n> ('<keys>'), action #<m>: 'type' value contains a newline; use 'press: enter' to submit input` |
    | Unknown press key | `entry #<n> ('<keys>'), action #<m>: unknown key '<key>' in 'press'; expected one of enter, tab, escape, backspace, space, up, down, left, right, home, end, pageup, pagedown, delete, insert, numpadenter, f1-f12` |
    | Modifier in `press` | `entry #<n> ('<keys>'), action #<m>: 'press' does not support modifiers in v1 (got '<value>')` |
    | Invalid `wait` value | `entry #<n> ('<keys>'), action #<m>: invalid 'wait' value '<value>'; expected a duration like '500ms', '2s', '1m' (1ms–60s)` |
    | Duplicate `keys` (later wins) | `entry #<n> ('<keys>') is a duplicate of an earlier entry; using the last definition` |

    Messages contain only YAML the user wrote — no Rust types, no internal error chain, no stack traces. The GUI's inline validation uses the same message text so users see one consistent vocabulary across surfaces.

21. **YAML parse errors.** If `shortcuts.yaml` cannot be parsed at all (unterminated string, invalid indentation, etc.), no shortcuts are loaded and a single error is recorded: `shortcuts.yaml: failed to parse: <yaml error message, with line:column when available>`. This is the only case where one bad token disables the whole file.

22. **Empty file.** A zero-byte `shortcuts.yaml` or one whose content is `shortcuts: []` loads zero shortcuts with no error and no toast. (A missing file at startup is bootstrapped with the default per §1, so the "missing" case is normally transient.)

23. **No effect when twarp lacks focus.** A custom shortcut only fires when twarp has keyboard focus, like any other twarp keybinding. The chord does not "leak" to twarp when another app is focused.

24. **Reload.** Edits to `shortcuts.yaml` — from either the side-panel GUI or hand-editing — take effect immediately. twarp watches the file and reloads on save; in addition, the GUI's save path triggers an explicit reload, so GUI edits are guaranteed to apply even on platforms where file-watching is unavailable. Reloading replaces the entire in-memory set; in-flight sequences from the previous set continue to run to completion, then the new set is in effect.

25. **Telemetry.** The feature reuses the existing user-keybinding telemetry path: firing a shortcut emits the same generic "user-defined keybinding triggered" event as any user-bound keybinding, with no per-action breakdown and no PII — in particular, `type` payloads are not telemetry-reported. GUI actions (create / edit / delete) are not telemetry-reported beyond what the existing settings-edit path already records.

### Side panel GUI

26. **Where it lives.** The GUI is a new tool-panel view, "Custom shortcuts", added to the left side panel **next to (immediately right of in the panel switcher) "Global search"**. Opening the left panel and switching to "Custom shortcuts" shows the view; the panel is dismissed and switched by the same gestures that govern any other left-panel view (no new top-level UI). The view is reachable via the same keyboard / menu / mouse paths as the existing tool-panel views.

27. **List view.** When `shortcuts.yaml` has at least one valid entry, the panel shows one row per entry in source order. Each row contains:
    - The chord in display form (e.g. `⌘⇧D`, `Ctrl+Shift+A`).
    - A compressed single-line summary of the action sequence, in arrow form: e.g. `new pane right → type "claude" → enter`. Long sequences truncate with `…`.
    - An [edit] affordance and a [delete] affordance.

    Clicking a row (or its [edit] icon) opens the detail editor (§30). Rows are read-only outside the editor.

28. **Empty state.** When `shortcuts.yaml` has zero valid entries, the panel shows a "+ New shortcut" button and a one-line helper hint ("Custom shortcuts run a sequence of terminal actions when you press a chord."). No list rows are rendered.

29. **Create.** A "+ New shortcut" button is always pinned at the top of the panel. Clicking it opens the detail editor with empty fields and an empty action list. Saving appends the entry to `shortcuts.yaml`. Cancelling discards.

30. **Edit.** Clicking a row's [edit] (or the row itself) opens the detail editor pre-filled with that entry's `keys` and `actions`. Saving rewrites that entry in place. Cancelling discards.

31. **Delete.** Clicking a row's [delete] removes the entry from `shortcuts.yaml`. No confirmation dialog — the operation is cheap to undo by recreating the entry, and a destructive-confirmation modal for a config-editor surface would be heavyweight. If undo proves necessary post-ship, it is a follow-up.

32. **Keystroke capture (`keys` field).** The `keys` field starts in display mode showing the bound chord (or placeholder text "(no chord set)"). Clicking it switches to capture mode: the field reads "Press a chord…" and the next key press with optional modifiers is captured into the field, which then returns to display mode showing the new chord. Pressing Escape during capture cancels and restores the previous value. Capture listens once per click — to recapture, the user clicks the field again. The captured chord is platform-specific (cmd-shift-D on mac, ctrl-shift-D on Linux/Windows); users who want a `cmdorctrl-` portable chord across OSes should hand-edit the YAML.

33. **Action editor.** The detail editor shows the action list as a vertical, ordered list of rows below the keys field. Each row contains:
    - A type dropdown (`new_tab`, `new_pane`, `type`, `press`, `wait`).
    - A parameter input whose shape depends on the type:
        - `new_tab`: no parameter.
        - `new_pane`: dropdown of `right` / `down`.
        - `type`: single-line text input.
        - `press`: dropdown of supported key names (§10).
        - `wait`: text input accepting `<n>ms` / `<n>s` / `<n>m`, with inline validation.
    - Up and down arrow buttons to reorder (disabled on first/last row).
    - A [×] button to remove the row.

    An "+ Add action" button below the list appends a new row defaulting to `new_tab`. The action list must have at least one action; deleting the last row leaves the [+ Add action] button as the only control until the user adds one.

34. **Validation surfacing.** Saving the detail editor runs the same validation as §20. If any field would produce an error, the offending field is highlighted and the error message (§20) shows inline beneath it. The Save button is disabled while any field has a current error. Inline validation also fires while editing — typing into the `wait` field shows the duration parse result live, for example — so users see problems before pressing Save.

35. **Errors banner.** When the in-memory error list from the most recent load is non-empty (typically from hand-edits introduced outside the GUI), the panel shows a banner at the top: `shortcuts.yaml has N error(s)`. Clicking the banner expands it inline, listing each error message verbatim (§20). Invalid entries do not appear as rows in the list view — only valid entries are listed — but their text remains in `shortcuts.yaml` on disk. Fixing them in YAML (or replacing them via the GUI) clears the banner on the next reload.

36. **Save semantics.** Pressing Save in the detail editor immediately rewrites `shortcuts.yaml` on disk with the full in-memory entry list and refreshes the in-memory shortcut registry (§24). The new or edited shortcut becomes active without restarting twarp. If the disk write fails (permission denied, disk full), the panel surfaces an inline error and the in-memory state is rolled back to match disk.

37. **YAML formatting.** When the GUI writes `shortcuts.yaml`, the file is re-serialized from the in-memory representation. Comments and hand-formatted whitespace in the previous file are **not** preserved. Within each entry, key order is `keys` then `actions`. Within each action item, single-key maps are written compact (`type: "claude"`, not multi-line). Users who want comment-preserving config keep using hand-edits exclusively; mixing hand-edits with GUI saves loses hand-edit comments.

38. **Conflict warnings.** While editing the `keys` field in the detail editor, two non-blocking warnings can appear below the field:
    - If the captured chord matches a currently-loaded built-in twarp binding: `This chord is bound to '<built-in name>' — saving will shadow that binding.`
    - If the captured chord matches another custom shortcut already in `shortcuts.yaml`: `This chord is also bound to shortcut #<n> ('<other-keys>') — saving will shadow that entry.`

    Both are warnings, not errors: Save remains enabled. Conflicts compare normalized chords (modifiers in canonical order). The driving example `⌘⇧D` triggers the first warning; that is intentional.

## Smoke test

Run against a freshly built twarp binary. Chord names below are macOS; substitute Ctrl for ⌘ on Linux/Windows.

1. With twarp closed, verify `shortcuts.yaml` does **not** exist in twarp's config directory (the directory containing `keybindings.yaml` / `settings.toml`). If it does, delete it for this test.

2. Launch twarp. No error toast appears. Quit twarp and confirm a new `shortcuts.yaml` was written in the config directory containing the two driving examples from §Driving examples. Re-launch twarp.

3. Focus any tab with a shell prompt and a single pane. Press `⌘⇧D`. A new pane opens to the right; `claude` is typed and submitted in the new pane. (If `claude` is not installed, the shell reports "command not found" — that is still proof the sequence ran.)

4. Close the new pane to return to a single-pane tab. Press `⌘⇧A`. A new pane opens right; `claude` is typed and submitted; three seconds later, `/address-code-review-comments` is typed and submitted in the same new pane.

5. Trigger `⌘⇧A` again. Mid-sequence — between the `enter` after `claude` and the `wait` expiring — click back to the previously focused pane (the original one, not the new pane). The new pane still receives `/address-code-review-comments` when the wait elapses (sticky target).

6. Trigger `⌘⇧A` again. During the `wait`, press Escape. The second `type` never runs; only `claude` shows in the new pane. Escape is not echoed.

7. Trigger `⌘⇧A`. While it is mid-wait, press `⌘⇧D`. The second trigger is ignored — the running sequence completes; no extra new pane opens for `⌘⇧D` during this window.

8. Open the left panel and switch to "Custom shortcuts" (the entry next to "Global search"). The list shows two rows:
    - `⌘⇧D — new pane right → type "claude" → enter`
    - `⌘⇧A — new pane right → type "claude" → enter → wait 3s → type "/address-code-review-comments" → enter`

9. Click [edit] on the `⌘⇧D` row. In the action editor, change the `type` action's value from `"claude"` to `"echo hi"`. Press Save. Press `⌘⇧D` — a new pane opens with `echo hi` typed.

10. Click "+ New shortcut". Click the `keys` field; press `⌘⇧9`; the field captures and displays `⌘⇧9`. Add two actions: `type: "echo from gui"`, then `press: enter`. Press Save. Press `⌘⇧9` — `echo from gui` is typed and submitted in the active pane.

11. Click [delete] on the `⌘⇧9` row. The row disappears. Press `⌘⇧9` — nothing happens.

12. Quit twarp. Hand-edit `shortcuts.yaml` to add an invalid entry:

    ```yaml
    shortcuts:
      ...existing entries...
      - keys: cmdorctrl-shift-7
        actions:
          - frobnicate    # unknown action
    ```

    Launch twarp. A toast appears: `shortcuts.yaml has 1 error(s) — see logs or open Custom shortcuts`. Open the Custom shortcuts panel: a banner shows "1 error". Click it; the inline message reads `shortcuts.yaml: entry #3 ('cmdorctrl-shift-7'), action #1: unknown action 'frobnicate'; expected one of new_tab, new_pane, type, press, wait`. Press `⌘⇧7` — nothing happens (invalid entry was skipped).

13. With twarp still running, hand-edit `shortcuts.yaml` and change `frobnicate` to `new_tab`. Save the file. Within a moment, the banner clears on its own (file-watch reload), the new entry appears in the list, and `⌘⇧7` opens a new tab.

14. Quit twarp. Delete `shortcuts.yaml`. Launch twarp. Open the Custom shortcuts panel: it shows the empty state with the "+ New shortcut" button and helper text.

15. Click "+ New shortcut". Set keys = `⌘T` (via capture). Add one action `type: "overridden"`. Press Save. Press `⌘T` — `overridden` is typed into the active pane; no new tab is created (custom shadows built-in). In the detail editor for this row, the chord field shows a non-blocking warning: `This chord is bound to 'workspace:new_tab' — saving will shadow that binding.`
