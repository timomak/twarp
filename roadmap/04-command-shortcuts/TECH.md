---
name: 04 — Custom command shortcuts
status: draft
---

# Custom command shortcuts — TECH

Companion to [PRODUCT.md](PRODUCT.md). Section numbers below refer to PRODUCT.md.

## Context

This feature adds a new user-facing config file (`shortcuts.yaml`) and a runtime executor that interprets sequences of four primitive actions (`new_tab`, `type`, `press`, `wait`) bound to keyboard chords. None of the moving parts exist yet — there is no precedent in the codebase for "one keystroke fires an async sequence of terminal actions". The closest precedent is the existing keybindings system, which dispatches a single synchronous action per chord.

The implementation reuses existing infrastructure for chord parsing, keymap registration, PTY writes, and toast surfacing; the new code is the YAML schema, the action executor, and the cancellation glue.

Relevant files on master:

- `app/src/keyboard.rs:34` — `KEYBINDINGS_FILE_NAME = "keybindings.yaml"`. The new `shortcuts.yaml` follows the same naming pattern in the same directory.
- `app/src/keyboard.rs:38` — `load_custom_keybindings(app: &mut AppContext)` startup hook. Sister entry point is the natural home for `load_custom_shortcuts(app)`.
- `app/src/keyboard.rs:97` — `keybinding_file_path()` → `config_local_dir().join(KEYBINDINGS_FILE_NAME)`. Mirror for `shortcuts_file_path()`.
- `app/src/lib.rs:2167` — startup call to `keyboard::load_custom_keybindings(ctx)`. The new `shortcuts::load(ctx)` call goes right next to it.
- `crates/warpui_core/src/keymap.rs:897` — `Keystroke::parse` — the exact chord parser we delegate to (PRODUCT §4). Accepts `cmdorctrl-shift-D` style. Errors are `anyhow::Error` with human-readable messages we can surface to users.
- `crates/warpui_core/src/keymap.rs:794` — `VALID_SPECIAL_KEYS` — canonical list of named keys (PRODUCT §9 cross-checked).
- `crates/warpui_core/src/keymap.rs:405` — `register_editable_bindings(...)`.
- `crates/warpui_core/src/keymap.rs:439-440` — *"the most recently registered editable binding will have the highest precedence"*. Means registering custom shortcuts **after** the built-ins is the entire mechanism for PRODUCT §15 (custom wins).
- `app/src/workspace/mod.rs:492-563` — `EditableBinding::new(...).with_context_predicate(id!("Workspace")).with_group(...).with_key_binding("cmdorctrl-1")` — exact registration shape for the new shortcut bindings.
- `app/src/workspace/action.rs:94` — `pub enum WorkspaceAction { ... }`. Two new variants live here: `RunCustomShortcut { id }` and `CancelRunningShortcut`.
- `app/src/workspace/view.rs:4093` — `active_tab_index(&self) -> usize`. Used by the executor to capture the initial target tab when a sequence starts.
- `app/src/workspace/view.rs:9538` — `Workspace::add_terminal_tab(hide_homepage: bool, ctx)`. The implementation of the `new_tab` action.
- `app/src/workspace/view.rs:5884` — `read_from_active_terminal_view(...)` — pattern for accessing the active terminal view of a given tab. Adapt to a `with_active_terminal_view_for_tab(tab_index, ...)` variant for the executor.
- `app/src/workspace/view.rs:8417` — `Workspace::write_to_pty(data, ctx)`. **Not directly callable from the executor** — it operates on `self` (the workspace's own pane). The executor instead targets a tab → pane group → active session view → terminal view → `write_to_pty` chain (model below).
- `app/src/workspace/view.rs:17424` — `AddDefaultTab => …` precedent for a `WorkspaceAction` arm that opens a new tab.
- `app/src/workspace/toast_stack.rs:36` — `ToastStack::add_persistent_toast(toast, window_id, ctx)`. Backbone of PRODUCT §18.
- `app/src/view_components/dismissible_toast.rs:346` — `DismissibleToast::default(main_text: String) -> Self`. Constructor.
- `app/src/terminal/view.rs:8417` — `TerminalView::write_to_pty<B: Into<Cow<'static, [u8]>>>(data, ctx)` — final write surface for both `type` and `press`.
- `app/src/terminal/view.rs:8444` — `clear_line_editor_and_write_to_pty_with_mac_workaround_hack` — a precedent for chunked PTY writes with a 10ms inter-chunk delay. Relevant if `type` strings ever grow past the 1 KB PTY chunk limit (PRODUCT does not require us to handle that, but the workaround is the safe default if we use the existing helper).
- `app/src/terminal/escape_sequences.rs` (referenced from `terminal/view.rs:8454`) — `C0` constants for control bytes. Source of truth for the `press` → bytes table.

## Proposed changes

### 1. Sub-phase split

**Recommendation: single PR.** The four primitives are tightly coupled — the parser without an executor ships dead code that can't be smoke-tested, and the executor without registration into the keymap can't be reached from a user. Splitting also doubles the spec-PR overhead. Reserve a 2-PR split as a fallback if the executor or the cancellation handling grows controversial during review; the natural cut line if forced is "config + parser (loaded but does nothing)" vs. "executor + bindings + cancel".

### 2. New module `app/src/shortcuts/`

```
app/src/shortcuts/
├── mod.rs          // public API: load(ctx), Registry, ShortcutId
├── config.rs       // parse_shortcuts_yaml(text) -> ParseResult
├── action.rs       // Action enum, KeyName enum
├── executor.rs     // Runner state machine
├── key_to_bytes.rs // KeyName → PTY byte sequence
└── shortcuts_tests.rs
```

Wired into `app/src/lib.rs` next to the existing `keyboard` module.

### 3. Types

```rust
// shortcuts/action.rs
pub enum Action {
    NewTab,
    Type(String),
    Press(KeyName),
    Wait(Duration),
}

pub enum KeyName { Enter, Tab, Escape, Backspace, Space, Up, Down, Left, Right,
                   Home, End, PageUp, PageDown, Delete, Insert, NumpadEnter,
                   F(u8) /* 1..=12 */ }

// shortcuts/mod.rs
pub struct Shortcut {
    pub keys: Keystroke,        // parsed via warpui_core::keymap::Keystroke::parse
    pub actions: Vec<Action>,
    pub source_line: usize,     // for error / log messages
    pub binding_name: String,   // "shortcuts:user_<index>"
}

pub type ShortcutId = u32;

pub struct Registry {
    by_id: Vec<Shortcut>,
}
```

`Registry` is owned by a singleton model (`ShortcutsModel`), accessed via `ShortcutsModel::handle(ctx).read(...)`.

### 4. Config parsing (`shortcuts/config.rs`)

```rust
pub struct ParseResult {
    pub shortcuts: Vec<Shortcut>,
    pub errors: Vec<String>,    // each prefixed with "shortcuts.yaml: "
}

pub fn parse_shortcuts_yaml(text: &str) -> ParseResult;
```

Implementation notes:

- Parse into `serde_yaml::Value` first (not a `#[derive(Deserialize)]` struct) so we can produce custom messages per PRODUCT §19. `serde_yaml::Value` preserves line/column info via `serde_yaml::with::singleton_map` features; if that proves fiddly, fall back to a hand-walked YAML tree and identify entries by 1-based list index (PRODUCT §19's messages use index, not line).
- A single parse failure at the root → return `errors: vec![format!("shortcuts.yaml: failed to parse: {e}")]` (PRODUCT §20) and `shortcuts: vec![]`.
- For each entry that validates, build a `Shortcut` and append. For each entry that fails, append a message to `errors` and continue (PRODUCT §17).
- Chord parsing delegates to `warpui_core::keymap::Keystroke::parse(keys_str)`. On `Err`, emit the "invalid key chord" message (PRODUCT §19); the inner `anyhow` chain is dropped to keep the message tight.
- Casing fix-up for the `shift-<letter>` debug-panic: pre-normalize lowercase letters in the chord string to uppercase when `shift` is in the modifier set (e.g. `cmd-shift-d` → `cmd-shift-D`) before handing to `Keystroke::parse`. Documented in PRODUCT §4.
- Duplicate detection: after parsing, dedup by `Keystroke::normalized()` (PRODUCT §19 last row). Earlier entries are dropped; the surviving entry is the last in source order.
- Empty `actions:` list → error (PRODUCT §19 row 4). Empty file or `shortcuts: []` → empty `ParseResult` with no errors (PRODUCT §21).
- The `type` action's value is checked for newlines (PRODUCT §19, "Newline in type" row). Tabs are allowed.
- `wait` durations parse via a small parser accepting `\d+(ms|s|m)`. Out-of-range (`< 1ms` or `> 60s`) → error. `humantime` is overkill; a 10-line parser is fine.
- `press` values normalize to `KeyName`; anything outside `VALID_SPECIAL_KEYS` (filtered to v1's allowed subset — PRODUCT §9) → error. Modifier presence (`-` in the value) → "Modifier in press" error.

### 5. Action model

PRODUCT distinguishes four primitives, but they share a runtime: `WorkspaceAction` should not carry the heavy payload (the whole action list) of every shortcut binding. Use the **executor model**:

```rust
// app/src/workspace/action.rs (additions near line 210)
RunCustomShortcut { id: ShortcutId },
CancelRunningShortcut,
```

`RunCustomShortcut { id }` is dispatched by the keymap when the user presses a custom chord. The handler fetches the action list from `Registry` and starts a runner. `CancelRunningShortcut` is dispatched by the conditional Escape binding (§7).

Why not one variant per action: the actions never appear in user-bindable form on their own — there is no `WorkspaceAction::CustomType { text }` worth exposing in the command palette. Keeping them out of the enum keeps the enum small and prevents accidental coupling.

### 6. Registration into the keymap

At startup, after `keyboard::load_custom_keybindings(ctx)`, call `shortcuts::load(ctx)`:

```rust
pub fn load(app: &mut AppContext) {
    let text = match std::fs::read_to_string(shortcuts_file_path()) {
        Ok(t) => t,
        Err(_) => return,  // missing/unreadable file → no shortcuts (PRODUCT §21)
    };
    let ParseResult { shortcuts, errors } = parse_shortcuts_yaml(&text);

    let mut bindings = Vec::with_capacity(shortcuts.len());
    for (id, sc) in shortcuts.iter().enumerate() {
        bindings.push(
            EditableBinding::new(
                sc.binding_name.as_str(),                    // "shortcuts:user_0", ...
                format!("Custom shortcut: {}", sc.keys.normalized()),
                WorkspaceAction::RunCustomShortcut { id: id as ShortcutId },
            )
            .with_context_predicate(id!("Workspace"))
            .with_group("Custom shortcuts")
            .with_key_binding(sc.keys.normalized().as_str()),
        );
    }
    app.register_editable_bindings(bindings);

    ShortcutsModel::handle(app).update(app, |model, _| {
        model.registry = Registry { by_id: shortcuts };
        model.errors = errors;
    });
}
```

Built-in shortcut override (PRODUCT §15) is automatic: per `crates/warpui_core/src/keymap.rs:439-440`, registration order determines precedence (later wins). Built-ins are registered earlier in startup; custom shortcuts register after. No special override flag needed.

### 7. Escape cancellation (PRODUCT §13)

The Escape key is already routed by the keymap when no surface consumes it. We want bare `escape` to dispatch `WorkspaceAction::CancelRunningShortcut` **only when a runner is active**, and to be inert (fall through to the active pane) otherwise.

Approach: a single `EditableBinding` registered alongside the custom shortcuts:

```rust
EditableBinding::new(
    "shortcuts:cancel_running",
    "Cancel running custom shortcut",
    WorkspaceAction::CancelRunningShortcut,
)
.with_context_predicate(id!("Workspace") & id!(flags::SHORTCUT_RUNNING))
.with_key_binding("escape")
```

`flags::SHORTCUT_RUNNING` is a new context flag (alongside the existing `flags::SHOW_PROJECT_EXPLORER` at `workspace/mod.rs:488`). The `ShortcutsModel` toggles it on when a runner starts and off when the runner finishes/aborts.

Because the binding is gated on the flag, normal Escape behavior (closing a modal, the terminal pane consuming Escape, etc.) is unaffected when no sequence is in flight. When a sequence is in flight, the gated binding takes precedence (registered late → highest precedence) and consumes the Escape press (PRODUCT §13's "not also passed through").

If during impl `id!(flags::FOO)` flag predicates turn out to be set/queried only via a specific pathway (not arbitrary bool toggles), fall back to wrapping the executor in a `View` whose context predicate is active iff the runner is alive — same effect via a different surface.

### 8. Executor (`shortcuts/executor.rs`)

```rust
pub struct Runner {
    id: ShortcutId,
    action_idx: usize,
    target_tab: usize,
    target_window: WindowId,
    cancelled: bool,
}
```

A single runner lives in `ShortcutsModel`; only one in flight at a time (PRODUCT §12). Starting a new run while one is active → log + drop the trigger.

Execution loop (driven from the `WorkspaceAction::RunCustomShortcut` handler in `workspace/view.rs`):

1. Resolve the `Workspace` for the current window; capture `active_tab_index` as `target_tab` and `window_id` as `target_window`.
2. Set `flags::SHORTCUT_RUNNING` to true on this workspace's context.
3. Step through actions:
   - `Action::NewTab` → `self.add_terminal_tab(false, ctx)`; then `self.target_tab = self.active_tab_index` (newly created tab becomes active per `add_terminal_tab`'s contract).
   - `Action::Type(text)` → resolve the target tab's active terminal view (see §9 below); call `terminal_view.write_to_pty(text.as_bytes().to_vec(), ctx)`. If the target tab/pane no longer exists (PRODUCT §16), abort.
   - `Action::Press(key)` → look up the byte sequence in `key_to_bytes::bytes_for(key)`; same write path as `Type`.
   - `Action::Wait(dur)` → `ctx.spawn(Timer::after(dur), move |runner, _, ctx| runner.continue_after_wait(ctx))`. The `Timer::after` pattern is already used in `app/src/throttle.rs:41` and `app/src/debounce.rs:107`. Inside the continuation, check `cancelled`; if true, finalize without continuing.
4. On the last action's completion (or abort), set `flags::SHORTCUT_RUNNING` to false. Drop the `Runner`.

Cancel path (`CancelRunningShortcut` handler): set `runner.cancelled = true`. A `wait` in flight checks the flag on its continuation and bails. A `type`/`press` already in progress writes its byte chunk and then bails on the next step. PRODUCT §13's "type that has already started writing is not rolled back" matches this directly.

### 9. PTY write path for a target tab

`Workspace::write_to_pty` (line 8417) writes to *the workspace's pane group*, not a specific tab. The executor instead resolves the terminal view by tab index:

```rust
fn write_to_target_pty(
    &self,                         // &Workspace
    target_tab: usize,
    bytes: Vec<u8>,
    ctx: &mut ViewContext<Workspace>,
) -> Result<(), TargetLost> {
    let pane_group = self.get_pane_group_view(target_tab).ok_or(TargetLost)?;
    let session_view = pane_group.read(ctx, |pg, ctx| pg.active_session_view(ctx).cloned())
        .ok_or(TargetLost)?;
    session_view.update(ctx, |terminal_view, ctx| {
        terminal_view.write_to_pty(bytes, ctx);
    });
    Ok(())
}
```

Modelled after `read_from_active_terminal_view` (line 5884) but parameterized by tab index. Returns `TargetLost` when the tab or its active pane has vanished mid-sequence; the executor catches it and aborts (PRODUCT §16).

For very long `type` strings (>1 KB), reuse the chunked-write helper at `terminal/view.rs:8444`. The PRODUCT spec does not impose a max `type` length, but the existing mac PTY workaround already covers the realistic upper bound; route `type` through the chunked variant unconditionally to avoid surprise.

### 10. `press` → bytes (`shortcuts/key_to_bytes.rs`)

A small const table:

| KeyName     | Bytes                  |
|-------------|------------------------|
| Enter       | `\r` (0x0D)            |
| NumpadEnter | `\r` (0x0D)            |
| Tab         | `\t` (0x09)            |
| Escape      | `\x1b`                 |
| Backspace   | `\x7f`                 |
| Space       | `\x20`                 |
| Up          | `\x1b[A`               |
| Down        | `\x1b[B`               |
| Right       | `\x1b[C`               |
| Left        | `\x1b[D`               |
| Home        | `\x1b[H`               |
| End         | `\x1b[F`               |
| PageUp      | `\x1b[5~`              |
| PageDown    | `\x1b[6~`              |
| Insert      | `\x1b[2~`              |
| Delete      | `\x1b[3~`              |
| F1–F4       | `\x1bOP` / `OQ` / `OR` / `OS` |
| F5–F12      | `\x1b[15~`..`\x1b[24~` |

Source: VT100/ANSI escape sequences (the same sequences a terminal emulator emits when the corresponding physical key is pressed in default mode). Prefer pulling constants from `app/src/terminal/escape_sequences.rs` where they already exist (the `C0` module has `ESC`, `CR`, `HT`); for CSI sequences not already in that module, define them in `key_to_bytes.rs` rather than expanding the existing module — keeps the change self-contained.

This intentionally does not implement Application Cursor Keys mode (`DECCKM`) — terminals in cursor-key mode emit `\x1bOA` for Up. For v1 the assumption is the running program receives the default sequence; if real usage shows that a `press: up` does the wrong thing in a `vim`/`less` context, the executor can query the terminal's mode before encoding. Track as a follow-up.

### 11. Toast surfacing (PRODUCT §18)

After `shortcuts::load(ctx)` returns and the `ShortcutsModel` holds any errors:

```rust
if !errors.is_empty() {
    let msg = format!("shortcuts.yaml has {} error(s) — see logs", errors.len());
    for e in &errors { log::warn!("{e}"); }
    ToastStack::handle(ctx).update(ctx, |stack, ctx| {
        stack.add_persistent_toast(
            DismissibleToast::default(msg),
            current_window_id,
            ctx,
        );
    });
}
```

`current_window_id` is the workspace's window at startup — at the moment `load` runs, there is exactly one window. (If startup ever changes to create multiple windows up front, use `WindowId::default()` or surface the toast to each workspace as it spawns; not a v1 concern.)

### 12. Module ownership boundaries

- `shortcuts::config` is pure (no `ctx`, no I/O beyond the input string). All tests live in `shortcuts_tests.rs` and run without harness.
- `shortcuts::executor` owns the action loop and the cancel flag; takes `&mut Workspace` + `ctx` from the action handler.
- `ShortcutsModel` is the single source of truth for the registry + error list + currently running `Runner`. Singleton model, same shape as `ToastStack`.
- The `WorkspaceAction` handler in `workspace/view.rs` is the only place that touches both `Workspace` mutation and `ShortcutsModel` — keeps the executor's bridging code in one file.

## Testing and validation

| PRODUCT § | Verification |
|-----------|--------------|
| §1 (config file path) | Unit test: `shortcuts_file_path()` joins `config_local_dir()` with `"shortcuts.yaml"`. Smoke step 1 places the file there and step 2 reads it. |
| §2 (top-level shape) | Parser unit tests in `shortcuts_tests.rs`: top-level list → error message exactly per §19 row 2; top-level scalar → same; missing `shortcuts:` key → error row 1; empty file → 0 shortcuts, 0 errors. |
| §3 (entry shape) | Parser unit tests: missing `keys` → row 3; missing/empty `actions` → row 4; unknown field → row 5. |
| §4 (chord normalization) | Parser unit tests: `cmd-shift-d` normalizes to `cmd-shift-D`; `Cmd-Shift-D` accepted; `cmdorctrl-1` parses; `cmd-k cmd-s` rejected (single chord only). Delegates to `Keystroke::parse` so its own coverage backstops us. |
| §5 (action vocabulary) | Parser unit tests: each of `new_tab`/`type`/`press`/`wait` parses to the corresponding `Action`; unknown token → row 7; multi-key map → row 6. |
| §6 (new_tab) | Workspace unit test: after `WorkspaceAction::RunCustomShortcut` whose sequence is `[NewTab]`, `active_tab_index` advanced; `target_tab` in the runner equals new index. Smoke step 4. |
| §7 (sticky target) | Workspace unit test: start runner with `[NewTab, Wait(50ms), Type("x")]`; mid-wait change `active_tab_index` to a different tab; verify the `Type` lands in the originally captured tab (poll the tab's PTY buffer). Smoke step 5. |
| §8 (type semantics) | Executor unit test: `Type("hello")` writes `b"hello"` to target PTY; no shell expansion, no twarp keybinding dispatched. Smoke step 3. |
| §8 (newline in type) | Parser unit test: `type: "a\nb"` → row 11. |
| §9 (press keys) | `key_to_bytes_tests.rs`: every supported `KeyName` returns its byte sequence; `bytes_for(Enter)` == b"\r". Smoke step 3 indirectly. |
| §9 (unknown press / modifier in press) | Parser unit tests: `press: zzz` → row 10; `press: ctrl-c` → row 11. |
| §10 (wait parsing + bounds) | Parser unit tests: `500ms`, `2s`, `1m` parse; `0ms`, `61s`, `1h`, `2x` → row 13. |
| §10 (wait does not block input) | Smoke step 5 (user types into another tab during the wait); confirm no impact on the sequence. |
| §11 (trigger) | Smoke steps 3 and 4. Plus: keymap unit test that registering a `Shortcut` results in a binding lookup for the normalized chord returning `RunCustomShortcut { id: 0 }`. |
| §12 (single in-flight) | Smoke step 7. Plus: executor unit test: starting a runner while one is in flight is a no-op (registry's `current_runner` unchanged). |
| §13 (cancellation) | Smoke step 6. Plus: executor unit test: setting `cancelled` mid-wait short-circuits the post-wait continuation; the consumed Escape does not produce a `\x1b` write on the target PTY. |
| §14 (no abort on focus loss) | Manual: alt-tab away during a `wait`, return — verify sequence completed. Not a numbered smoke step since it's hard to script. |
| §15 (built-in conflict) | Smoke step 11. Plus: keymap test asserting that after registration order (built-ins, then custom), the lookup for `cmdorctrl-t` returns the custom action. |
| §16 (target lost) | Executor unit test: with `[NewTab, Wait(50ms), Type("x")]`, close the created tab during the wait; assert the runner aborts and `flags::SHORTCUT_RUNNING` is cleared. |
| §17 / §18 (skipped invalid, toast) | Smoke step 8 / 9. Plus: parser unit test that one bad entry yields one error message and the remaining valid entries load. |
| §19 (exact error messages) | Parser unit tests assert each row's message verbatim against a manufactured malformed YAML. |
| §20 (unparseable yaml) | Parser unit test: indented-incorrectly YAML → single error containing `"failed to parse"`. |
| §21 (empty/missing) | Smoke step 10. Plus: parser unit tests for empty string, `shortcuts:\n` (null), `shortcuts: []`. |
| §22 (no leak when unfocused) | Manual only — relies on OS keystroke routing. Inherits behavior from existing keybindings. |
| §23 (reload) | Manual: edit `shortcuts.yaml` with twarp running, verify no reload; quit and relaunch, verify edits take effect. Smoke steps 8 and 10 indirectly. |
| §24 (telemetry) | Manual: spot-check that firing a custom shortcut emits the existing user-keybinding event (no per-action breakdown, no `type` payload in the event body). |

New test files:

- `app/src/shortcuts/shortcuts_tests.rs` — parser table-tests, key-to-bytes table-tests, executor unit tests with a stubbed `Workspace`.
- `app/src/workspace/view_test.rs` — integration-shaped test for `RunCustomShortcut` dispatch + target-lost abort, alongside the existing tab-color tests.

No new integration test (in `crates/integration`) is required for v1 — the four primitives are covered by unit tests plus the manual smoke test. Add an integration test in a follow-up if regressions appear.

Run `./script/presubmit` until green before opening the impl PR.

## Risks and mitigations

- **Risk: `id!(flags::FOO)` flags can't be toggled imperatively, so the Escape-cancel gate doesn't work as drawn.** Mitigation: research the flag mechanism during impl; fall back to a view-scoped predicate (executor view's context predicate is active iff the runner exists) if the flag approach won't take.
- **Risk: custom shortcut registration after built-ins is not actually the precedence rule in practice (e.g. a separate per-name dedup wins).** Mitigation: small keymap-level test asserting `cmdorctrl-t` resolves to the custom action after both registrations. Run early in impl; if precedence is per-name and built-ins win, switch to overriding the built-in via `set_custom_trigger` (PRODUCT §15 still satisfied).
- **Risk: `press: up` (etc.) sends the wrong escape sequence in `vim`-like apps that expect Application Cursor Keys mode.** Mitigation: documented as a known v1 limitation; track DECCKM-aware encoding as a follow-up.
- **Risk: a runaway sequence with many `wait`s plus user inputs creates a confusing UX (the sequence keeps targeting a tab the user has clearly moved on from).** Mitigation: PRODUCT §13's Escape cancel is the user's escape hatch. Documented in §7 of PRODUCT.md.
- **Risk: large `type` strings hit the mac PTY 1 KB chunk bug.** Mitigation: route `type` through the existing chunked-write helper (`terminal/view.rs:8444`).
- **Risk: error messages drift between the spec and implementation as messages get edited.** Mitigation: tests assert message text verbatim (table-test pattern). PRODUCT §19 is the source of truth.

## Follow-ups

- **Hot reload** of `shortcuts.yaml` — read on file change instead of only at startup.
- **Modifier-in-press** support (`press: ctrl-c`, `press: alt-f`) — generates the corresponding control bytes / Meta sequences.
- **Application Cursor Keys** mode awareness for arrow keys in `press`.
- **GUI editor** in the settings page (`Custom shortcuts` group, surface the YAML editor + validation panel).
- **More actions:** `new_window`, `focus_tab: <index>`, `close_tab`, `split_pane`, `run: <command>` (high-level `type` + `enter`).
- **A dedicated settings-page surface** for `shortcuts.yaml` errors instead of just the toast + log line.
- **Sequence interleaving / queuing** if user demand emerges (PRODUCT §12 currently drops, doesn't queue).

## Parallelization

Skipped — single PR, single sequential implementation. The four primitives, parser, executor, and registration are all tightly coupled and live in the same module; splitting across agents creates merge-burden without wall-clock win. The smoke test plus presubmit are the validation cycle.
