---
name: 04 — Custom command shortcuts
status: draft
---

# Custom command shortcuts — PRODUCT

## Summary

A declarative way to bind a keyboard shortcut to a sequence of terminal actions: open a new tab, type literal text into the active pane, press named keys (enter, tab, …), wait. Lets users compress frequent multi-step workflows ("open a tab and start tool X", "switch to a pane and run a known command sequence") into a single keystroke. Shortcuts are declared in a YAML file that lives alongside twarp's existing settings; v1 ships with no built-in shortcuts, so the feature has no observable effect until the user adds entries.

## Goals / Non-goals

**Goals**

- Declarative YAML config: a list of `(keys, actions)` pairs that a user can hand-edit.
- A minimal but complete v1 action vocabulary — `new_tab`, `type`, `press`, `wait` — sufficient for the two driving examples in README §4 and for most "kick off tool X in a new tab" workflows.
- Platform-neutral key normalization using the same `cmdorctrl-shift-<key>` convention twarp's existing `keybindings.yaml` uses (matches `Keystroke::parse` exactly).
- A clean cancellation hatch: Escape aborts an in-flight sequence.
- Clear, actionable user-facing error messages when the config is malformed — every error names the offending shortcut and the expected shape.
- Invalid entries are skipped, not fatal: one bad entry never disables the feature or blocks twarp from launching.

**Non-goals**

- A GUI / settings-page editor for shortcuts. v1 is config-file only.
- Variables, conditionals, loops, parameter prompts, or any form of programmable control flow. v1 sequences are literal lists of actions.
- Per-tab, per-pane, or context-scoped shortcuts. v1 keybindings are global to twarp.
- Reading terminal output, conditioning behavior on prompt state, or any form of "wait until prompt is ready". v1 only knows wall-clock time.
- Modifier keys inside `press` (e.g. `press: ctrl+c`). v1 `press` is a named key only. Sending control characters is a follow-up.
- Shortcuts that trigger other twarp keybindings. `type` and `press` write to the active pane's PTY only — they do not dispatch through twarp's own keybinding handler.
- Hot reload of the config file at runtime. v1 reads `shortcuts.yaml` at startup; edits require a restart.
- Built-in / bundled shortcut presets. v1 ships empty; the user authors every shortcut.

## Behavior

### Config

1. **Config file.** Custom shortcuts are declared in `shortcuts.yaml`, located in the same directory as twarp's existing `keybindings.yaml` and `settings.toml` (twarp's per-user config directory). If the file does not exist, no shortcuts are loaded and no error is surfaced — an absent config is the default state for a fresh install.

2. **Top-level shape.** `shortcuts.yaml` is a YAML map with a single top-level key, `shortcuts:`, whose value is a list of shortcut entries:

    ```yaml
    shortcuts:
      - keys: cmdorctrl-shift-9
        actions:
          - new_tab
          - type: "git status"
          - press: enter
    ```

    An empty `shortcuts:` list (or a totally empty file) loads zero shortcuts with no error. Any other top-level shape (top-level list, top-level scalar, an unexpected top-level key) is a config error (§19).

3. **Shortcut entry shape.** Each entry is a map with two required fields: `keys` (the chord string) and `actions` (a non-empty list of actions). Unknown fields on an entry are an error (§19). Either required field being absent or empty is an error.

4. **Key chord normalization.** The `keys` value is a single chord string: zero or more modifier names joined by `-`, followed by `-<key>` — the same syntax twarp's built-in `keybindings.yaml` already uses, and the exact syntax accepted by `Keystroke::parse`. Recognized modifiers: `cmd`, `ctrl`, `cmdorctrl` (resolves to ⌘ on macOS, Ctrl on Linux/Windows — matches the convention used by built-in twarp bindings), `alt`, `shift`, `meta`. The key portion is a single printable character (`0`–`9`, `a`–`z` / `A`–`Z`, common punctuation) or one of the named keys listed in §10. Order of modifiers does not matter (`shift-cmd-D` and `cmd-shift-D` parse to the same chord). When `shift` is in the modifier set together with a letter, the letter must be uppercase (`cmd-shift-D`, not `cmd-shift-d`); this matches the existing `Keystroke::parse` rule. Multi-stroke chords (e.g. `cmd-k cmd-s`) are **not** supported in v1; the entire `keys` string is a single chord.

5. **Action vocabulary.** v1 supports exactly four actions:

    | Action     | Shape                          | Effect                                                                                                  |
    |------------|--------------------------------|---------------------------------------------------------------------------------------------------------|
    | `new_tab`  | bare string `new_tab`          | Opens a new tab in the focused window and updates the sequence's target tab to the new tab. (§7, §8)    |
    | `type`     | map `type: "<text>"`           | Writes the literal text to the target tab's active pane's PTY, as if the user had typed/pasted it. (§9) |
    | `press`    | map `press: <named-key>`       | Writes the byte sequence for the named key to the target tab's active pane's PTY. (§10)                 |
    | `wait`     | map `wait: <duration>`         | Pauses the sequence for the given duration before running the next action. (§11)                        |

    Any other action token is a config error (§19). Action items are either a bare string (for parameterless actions, currently only `new_tab`) or a single-key map (for parameterized actions). A multi-key map on a single action item is an error.

### Action semantics

6. **`new_tab`.** Opens a new tab in the focused twarp window — the same operation the user's default "new tab" keybinding triggers — and makes it the sequence's target tab. If twarp has no open windows at the moment the action runs, the sequence aborts (§16). If twarp is open but unfocused (the user alt-tabbed away after triggering the shortcut), `new_tab` operates on the most recently focused twarp window.

7. **Sequence target.** Every sequence has a single "target tab": the tab into which `type` and `press` actions write. The target tab is set when the sequence starts to the active tab of the focused twarp window. Each `new_tab` action updates the target tab to the newly created tab. **Subsequent user focus changes (clicking another tab, alt-tabbing to another app) do not change the sequence's target tab** — a running sequence delivers its remaining actions to the tab it captured, not to whatever the user is currently looking at. Within the target tab, `type` and `press` always target the tab's currently active pane (a sequence does not pin to a specific pane within a tab).

8. **`type`.** Writes the literal UTF-8 text of the `type:` value to the target tab's active pane's PTY. The text is treated as if the user had pasted it: no shell expansion happens client-side, no keyboard-shortcut interpretation by twarp, no triggering of twarp keybindings even if the typed text happens to contain a chord-shaped substring. Leading and trailing whitespace are preserved exactly as written. Newline characters (`\n`, `\r`, `\r\n`) inside a `type` value are a config error (§19) — to submit input, use `press: enter`. Tab characters are allowed but discouraged; prefer `press: tab` for clarity.

9. **`press`.** Writes the byte sequence corresponding to the named key to the target tab's active pane's PTY. v1 supports these key names: `enter` (CR), `tab` (HT), `escape`, `backspace`, `space`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `delete`, `insert`, `numpadenter`, `f1`–`f12`. Modifier keys inside `press` are not supported in v1 — `press: ctrl-c` is a config error (§19). `press` does not dispatch through twarp's keybinding handler; it only writes bytes to the PTY. Whether the running program does anything with a given key sequence (e.g. `press: f5`) is up to that program.

10. **`wait`.** Pauses sequence execution for the duration named in the `wait:` value. Accepted units: `ms` (milliseconds), `s` (seconds), `m` (minutes), written without spaces (`500ms`, `2s`, `1m`). The duration must be ≥ 1ms and ≤ 60s; values outside that range are a config error (§19). During a `wait`, the user retains full control of twarp — they can type into any pane, switch tabs, switch windows, switch apps. User input during a `wait` is not blocked, not captured by the sequence, and not buffered.

### Running a sequence

11. **Trigger.** Pressing a chord that matches a shortcut entry's `keys` field starts that sequence. Sequences run asynchronously: the chord press returns immediately, and actions dispatch one after another per the rules above.

12. **Single in-flight sequence.** Only one custom shortcut sequence runs at a time. If a sequence is in flight and the user presses a chord bound to another (or the same) shortcut, the new trigger is **ignored** — the running sequence continues to completion. Chaining shortcuts via synthetic key presses is out of scope (§Non-goals); queueing would create surprising delays.

13. **Cancellation via Escape.** Pressing the bare `escape` key (no modifiers) anywhere in twarp while a sequence is in flight aborts the rest of the sequence: no `type`, `press`, `wait`, or `new_tab` actions run after the in-flight one finishes. A `wait` is interrupted immediately; a `type` or `press` that has already started writing is not rolled back. The aborting Escape press is consumed by the abort handler and is **not** also passed through to the active pane.

14. **No abort on focus loss or other input.** Alt-tabbing away, clicking another tab, scrolling, or typing normal keys (anything other than bare Escape) does not abort a running sequence. The sequence keeps delivering actions to its sticky target tab (§7).

15. **Built-in binding conflict.** If a custom shortcut's `keys` matches a chord already bound to a built-in twarp action, the custom shortcut wins — the built-in is shadowed for as long as the custom shortcut is loaded. The user remains free to rebind the built-in elsewhere via twarp's standard keybindings settings page. Shadowing a built-in is not a config error; it is a supported use of the feature. Built-ins the user has explicitly unbound through the keybindings page stay unbound regardless of this feature.

16. **Target lost.** If the sequence's target tab is closed mid-sequence (by the user, by an OS event, or by another twarp action), the sequence aborts at that point with no error notification. If the focused twarp window is closed mid-sequence and no twarp window remains, the sequence aborts.

### Errors and edge cases

17. **Invalid entries are skipped, not fatal.** When `shortcuts.yaml` is parsed, every entry that fails validation (§19) is dropped from the loaded set with its error message recorded. The remaining valid entries are loaded and become active. A malformed config never prevents twarp from launching and never disables shortcuts wholesale.

18. **Error surfacing.** On twarp startup, if `shortcuts.yaml` produced any errors, twarp surfaces a single non-modal toast notification of the form `shortcuts.yaml has N error(s) — see logs`. The full error list is written to twarp's log file. v1 does not ship a dedicated settings-page surface for shortcut errors; that is a follow-up. The toast is dismissable and shown at most once per launch; it does not steal focus.

19. **Config validation error messages.** Each validation error produces a log line of the form `shortcuts.yaml: <message>`. The message identifies the offending entry by its 1-indexed position in the `shortcuts:` list and (where helpful) its `keys` value. The full set of recognized errors and their exact message templates — these are part of the user-facing contract:

    | Condition | Message |
    |---|---|
    | Top-level is not a `shortcuts:` map | `expected top-level 'shortcuts:' key with a list value` |
    | `shortcuts:` value is not a list | `'shortcuts:' must be a list, got <YAML type>` |
    | Entry missing `keys` | `entry #<n>: missing required field 'keys'` |
    | Entry missing or empty `actions` | `entry #<n> ('<keys>'): missing required field 'actions' (must have at least one action)` |
    | Unknown field on entry | `entry #<n> ('<keys>'): unknown field '<field>'; expected 'keys' and 'actions'` |
    | Malformed key chord | `entry #<n>: invalid key chord '<chord>'; expected modifiers (cmdorctrl/cmd/ctrl/alt/shift/meta) joined by '-' with a key, e.g. 'cmdorctrl-shift-D'` |
    | Action is not a string or single-key map | `entry #<n> ('<keys>'), action #<m>: expected a bare action name or a single-key map` |
    | Unknown action token | `entry #<n> ('<keys>'), action #<m>: unknown action '<token>'; expected one of new_tab, type, press, wait` |
    | `type` value is not a string | `entry #<n> ('<keys>'), action #<m>: 'type' expects a string value` |
    | Newline in `type` | `entry #<n> ('<keys>'), action #<m>: 'type' value contains a newline; use 'press: enter' to submit input` |
    | Unknown press key | `entry #<n> ('<keys>'), action #<m>: unknown key '<key>' in 'press'; expected one of enter, tab, escape, backspace, space, up, down, left, right, home, end, pageup, pagedown, delete, insert, numpadenter, f1-f12` |
    | Modifier in `press` | `entry #<n> ('<keys>'), action #<m>: 'press' does not support modifiers in v1 (got '<value>')` |
    | Invalid `wait` value | `entry #<n> ('<keys>'), action #<m>: invalid 'wait' value '<value>'; expected a duration like '500ms', '2s', '1m' (1ms–60s)` |
    | Duplicate `keys` (later wins) | `entry #<n> ('<keys>') is a duplicate of an earlier entry; using the last definition` |

    Messages contain only YAML the user wrote — no Rust types, no internal error chain, no stack traces.

20. **YAML parse errors.** If `shortcuts.yaml` cannot be parsed at all (unterminated string, invalid indentation, etc.), no shortcuts are loaded and a single error is recorded: `shortcuts.yaml: failed to parse: <yaml error message, with line:column when available>`. This is the only case where one bad token disables the whole file.

21. **Empty / missing file.** A missing `shortcuts.yaml`, a zero-byte `shortcuts.yaml`, and a `shortcuts.yaml` whose content is `shortcuts: []` all load zero shortcuts with no error and no toast.

22. **No effect when twarp lacks focus.** A custom shortcut only fires when twarp has keyboard focus, like any other twarp keybinding. The chord does not "leak" to twarp when another app is focused.

23. **Reload.** Edits to `shortcuts.yaml` take effect on the next twarp launch. v1 does not watch the file at runtime; hot reload is a follow-up.

24. **Telemetry.** The feature reuses the existing user-keybinding telemetry path: firing a shortcut emits the same generic "user-defined keybinding triggered" event as any user-bound keybinding, with no per-action breakdown and no PII — in particular, `type` payloads are not telemetry-reported.

## Smoke test

Run against a freshly built twarp binary. Chord names below are macOS; substitute Ctrl for ⌘ on Linux/Windows.

1. With twarp closed, locate twarp's config directory (the directory containing `keybindings.yaml` / `settings.toml`) and create `shortcuts.yaml` there with:

    ```yaml
    shortcuts:
      - keys: cmdorctrl-shift-9
        actions:
          - type: "echo hello"
          - press: enter
      - keys: cmdorctrl-shift-8
        actions:
          - new_tab
          - type: "pwd"
          - press: enter
          - wait: 1s
          - type: "ls"
          - press: enter
    ```

2. Launch twarp. No error toast appears.

3. In any tab with a shell prompt, press `⌘⇧9`. The prompt receives `echo hello`, submits it, and prints `hello`.

4. Press `⌘⇧8`. A new tab opens and becomes active. `pwd` runs in the new tab; one second later, `ls` runs in the same new tab. The previously focused tab is untouched.

5. Press `⌘⇧8` again. Mid-sequence — between the `pwd`/`enter` and the `wait` expiring — click back to the original tab. The new tab still receives `ls` when the wait elapses (sticky target).

6. Press `⌘⇧8` again. During the `wait`, press Escape. The second `type "ls"` never runs; the new tab shows only `pwd` output. Escape is not echoed into the pane.

7. Press `⌘⇧8`. While it is still mid-wait, press `⌘⇧9`. The second trigger is ignored — the running `wait → ls` portion completes; `echo hello` does not run during this window.

8. Quit twarp. Edit `shortcuts.yaml` to add an invalid entry:

    ```yaml
    shortcuts:
      - keys: cmdorctrl-shift-9
        actions:
          - type: "echo hello"
          - press: enter
      - keys: cmdorctrl-shift-7
        actions:
          - frobnicate    # unknown action
    ```

    Launch twarp. A toast appears: `shortcuts.yaml has 1 error(s) — see logs`. Press `⌘⇧9` — still works. Press `⌘⇧7` — nothing happens (invalid entry was skipped).

9. Open twarp's log file. Locate the line `shortcuts.yaml: entry #2 ('cmdorctrl-shift-7'), action #1: unknown action 'frobnicate'; expected one of new_tab, type, press, wait`.

10. Quit twarp. Delete `shortcuts.yaml`. Launch twarp. No toast, no shortcuts. Press `⌘⇧9` — nothing happens.

11. Quit twarp. Bind a chord that conflicts with a built-in (e.g. `cmdorctrl-t`, twarp's default "new tab"):

    ```yaml
    shortcuts:
      - keys: cmdorctrl-t
        actions:
          - type: "overridden"
    ```

    Launch twarp. Press `⌘T`. `overridden` is typed into the active pane; no new tab is created (custom shortcut shadows the built-in).
