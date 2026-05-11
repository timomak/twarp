---
name: 04 — Custom command shortcuts
status: draft
---

# Custom command shortcuts — TECH

Companion to [PRODUCT.md](PRODUCT.md). Section numbers below refer to PRODUCT.md.

## Context

This feature has two halves: a runtime that interprets sequences of five primitive actions bound to keyboard chords, and a side-panel GUI for CRUD-ing those shortcuts. The runtime reuses existing infrastructure for chord parsing, keymap registration, PTY writes, pane splits, and toast surfacing. The GUI plugs into the existing left-panel tool-view system. Almost nothing in either half is novel architecture; the work is wiring.

Relevant files on master:

- `app/src/keyboard.rs:34` — `KEYBINDINGS_FILE_NAME = "keybindings.yaml"`. The new `shortcuts.yaml` follows the same naming pattern in the same directory.
- `app/src/keyboard.rs:38` — `load_custom_keybindings(app: &mut AppContext)` startup hook. Sister entry point is the natural home for `shortcuts::load(app)`.
- `app/src/keyboard.rs:97` — `keybinding_file_path()` → `config_local_dir().join(KEYBINDINGS_FILE_NAME)`. Mirror for `shortcuts_file_path()`.
- `app/src/lib.rs:2167` — startup call to `keyboard::load_custom_keybindings(ctx)`. The new `shortcuts::load(ctx)` call goes right next to it.
- `crates/warpui_core/src/keymap.rs:897` — `Keystroke::parse` — the exact chord parser we delegate to (PRODUCT §4). Accepts `cmdorctrl-shift-D` style.
- `crates/warpui_core/src/keymap.rs:794` — `VALID_SPECIAL_KEYS` — canonical list of named keys cross-checked against PRODUCT §10.
- `crates/warpui_core/src/keymap.rs:405` — `register_editable_bindings(...)`.
- `crates/warpui_core/src/keymap.rs:439-440` — *"the most recently registered editable binding will have the highest precedence"*. Means registering custom shortcuts **after** the built-ins is the mechanism for PRODUCT §16 (custom wins).
- `app/src/workspace/mod.rs:492-563` — `EditableBinding::new(...).with_context_predicate(id!("Workspace")).with_group(...).with_key_binding("cmdorctrl-1")` — registration shape for the new shortcut bindings.
- `app/src/workspace/action.rs:94` — `pub enum WorkspaceAction { ... }`. Two new variants here: `RunCustomShortcut { id }` and `CancelRunningShortcut`.
- `app/src/workspace/view.rs:4093` — `active_tab_index(&self) -> usize`. Used by the executor to capture the initial target tab.
- `app/src/workspace/view.rs:9538` — `Workspace::add_terminal_tab(hide_homepage: bool, ctx)`. Implementation of `new_tab`.
- `app/src/pane_group/mod.rs:459-475` — `EditableBinding::new("pane_group:add_right", ..., PaneGroupAction::Add(Direction::Right))` and the analogous `add_down`. **Implementation of `new_pane`** dispatches `PaneGroupAction::Add(Direction::Right|Down)` on the target tab's pane group.
- `app/src/util/bindings.rs:299` — `CustomAction::SplitPaneRight => Keystroke::parse(cmd_or_ctrl_shift("d")).ok()`. Confirms `cmdorctrl-shift-D` is the built-in's default — PRODUCT §16 / Driving examples already calls this out.
- `app/src/workspace/view.rs:5884` — `read_from_active_terminal_view(...)` — pattern for accessing the active terminal view; adapt for tab-indexed access from the executor.
- `app/src/terminal/view.rs:8417` — `TerminalView::write_to_pty<B: Into<Cow<'static, [u8]>>>(data, ctx)` — final write surface for `type` and `press`.
- `app/src/workspace/toast_stack.rs:36` — `ToastStack::add_persistent_toast(toast, window_id, ctx)`. Backbone of PRODUCT §19.
- `app/src/view_components/dismissible_toast.rs:346` — `DismissibleToast::default(main_text: String) -> Self`.
- `app/src/app_state.rs:889` — `LeftPanelDisplayedTab` enum. A new `Shortcuts` variant lives here.
- `app/src/workspace/view/left_panel.rs` — `ToolPanelView` enum + view dispatch. New `ToolPanelView::Shortcuts` variant.
- `app/src/workspace/view.rs:17257` — `compute_left_panel_views(ctx)` builds the ordered list of `ToolPanelView`s shown in the panel switcher. Insert `Shortcuts` immediately after `GlobalSearch` (PRODUCT §26).
- `app/src/warp_managed_paths_watcher.rs:7` — `notify_debouncer_full::notify` is already a dependency; reuse for the `shortcuts.yaml` file watcher (PRODUCT §24).

## Sub-phase split

**Recommendation: split into 4a and 4b.** Total scope is large enough — runtime plus GUI — that a single PR would be reviewer-hostile, and each half delivers user value on its own.

- **4a — Runtime.** Parser + executor + bindings + cancel + toast. Hand-edit `shortcuts.yaml`, restart twarp to apply. Covers PRODUCT §§1–23, §25.
- **4b — Side-panel GUI + hot reload.** Adds the `ToolPanelView::Shortcuts` view (list + detail editor + keystroke capture + validation surfacing + conflict warnings), plus file-watch reload that the GUI relies on for its save → live-update flow. Covers PRODUCT §24 (hot reload) and §§26–38 (GUI).

This split is durable: 4a's public API (`ShortcutsModel`, `Registry`, `parse_shortcuts_yaml`, error message vocabulary) is exactly what 4b consumes. No interface churn at the boundary; 4b is layering, not retrofitting.

The fallback if 4b grows: split it further into "hot reload only" and "GUI only" — but they share so much (the GUI assumes hot reload so its save reflects live) that a single 4b PR is the default plan.

## Proposed changes — 4a (Runtime)

### 1. New module `app/src/shortcuts/`

```
app/src/shortcuts/
├── mod.rs           // public API: load(ctx), ShortcutsModel, ShortcutId
├── config.rs        // parse_shortcuts_yaml(text) -> ParseResult, serialize_shortcuts(...)
├── action.rs        // Action, Direction, KeyName enums
├── executor.rs      // Runner state machine
├── key_to_bytes.rs  // KeyName → PTY byte sequence
└── shortcuts_tests.rs
```

`serialize_shortcuts` lives in `config.rs` from 4a even though only 4b uses it — having parse and serialize in one module keeps round-trip tests trivially local.

### 2. Types

```rust
// action.rs
pub enum Action {
    NewTab,
    NewPane(Direction),
    Type(String),
    Press(KeyName),
    Wait(Duration),
}

pub enum Direction { Right, Down }

pub enum KeyName { Enter, Tab, Escape, Backspace, Space, Up, Down, Left, Right,
                   Home, End, PageUp, PageDown, Delete, Insert, NumpadEnter,
                   F(u8) /* 1..=12 */ }

// mod.rs
pub struct Shortcut {
    pub keys: Keystroke,        // parsed via warpui_core::keymap::Keystroke::parse
    pub actions: Vec<Action>,
    pub binding_name: String,   // "shortcuts:user_<index>"
}

pub type ShortcutId = u32;

pub struct ShortcutsModel {
    pub registry: Vec<Shortcut>,
    pub errors: Vec<String>,
    pub current_runner: Option<Runner>,
}
```

`ShortcutsModel` is a singleton model, same shape as `ToastStack` (`workspace/toast_stack.rs`).

### 3. Config parsing (`shortcuts/config.rs`)

```rust
pub struct ParseResult {
    pub shortcuts: Vec<Shortcut>,
    pub errors: Vec<String>,    // each prefixed with "shortcuts.yaml: "
}

pub fn parse_shortcuts_yaml(text: &str) -> ParseResult;
pub fn serialize_shortcuts(shortcuts: &[Shortcut]) -> String;  // used by 4b
```

Implementation notes:

- Parse into `serde_yaml::Value` first (not a `#[derive(Deserialize)]` struct) so we can produce PRODUCT §20's exact messages. Identify entries by 1-based list index.
- A single parse failure at the root → return `errors: vec![format!("shortcuts.yaml: failed to parse: {e}")]` (PRODUCT §21) and `shortcuts: vec![]`.
- For each entry that validates, build a `Shortcut`. For each that fails, append to `errors` and continue (PRODUCT §18).
- Chord parsing: pre-normalize `shift-<lowercase>` to `shift-<UPPERCASE>` (PRODUCT §4), then delegate to `Keystroke::parse`. On error → §20 row "Malformed key chord".
- Duplicate detection: dedup by `Keystroke::normalized()` (§20 row "Duplicate keys"). Last in source order survives.
- `wait` durations parse via a ~10-line parser accepting `\d+(ms|s|m)` clamped to `[1ms, 60s]`. `humantime` is overkill.
- `new_pane` direction parses to `Direction::Right` / `Direction::Down`. Missing / invalid → §20 rows.
- `type` value: must be a string; reject newlines (§9, §20).
- `press` value: must be in the v1 subset (PRODUCT §10) and must not contain modifiers.

### 4. Action model

```rust
// app/src/workspace/action.rs (additions near line 210)
RunCustomShortcut { id: ShortcutId },
CancelRunningShortcut,
```

Why not one variant per action: `WorkspaceAction` is the action-palette / keybinding surface; the five primitives are private to the executor and have no user-bindable shape on their own. Keep them out of the enum.

### 5. Registration

At startup, after `keyboard::load_custom_keybindings(ctx)`, call `shortcuts::load(ctx)`:

```rust
pub fn load(app: &mut AppContext) {
    let path = shortcuts_file_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        // PRODUCT §1: bootstrap a default file on first launch so the
        // driving examples work out of the box. `create_dir_all` first
        // because the parent may not exist for a fresh install.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(write_err) = std::fs::write(&path, DEFAULT_SHORTCUTS_YAML) {
                log::warn!("shortcuts: failed to write default shortcuts.yaml: {write_err}");
                return;
            }
            DEFAULT_SHORTCUTS_YAML.to_owned()
        }
        Err(_) => return,
    };
    let ParseResult { shortcuts, errors } = parse_shortcuts_yaml(&text);

    register_shortcut_bindings(app, &shortcuts);
    register_escape_cancel_binding(app);  // §7

    ShortcutsModel::handle(app).update(app, |model, _| {
        model.registry = shortcuts;
        model.errors = errors;
    });

    surface_errors(app);  // §11
}
```

`DEFAULT_SHORTCUTS_YAML` is a `const &str` in `shortcuts/mod.rs` holding the literal YAML for the two driving examples (PRODUCT §Driving examples). A unit test parses it round-trip to keep the default in sync with the parser's accepted shape.

Built-in conflict precedence (PRODUCT §16) for **EditableBinding-routed** chords is automatic: per `keymap.rs:439-440`, registration order determines precedence (later wins). Built-ins register earlier; custom shortcuts register after. No special override flag.

**Menu-routed chords on macOS need explicit suppression.** Several built-in `CustomAction`s (e.g. `SplitPaneRight`, `SplitPaneDown`, `NewTab`) are exposed as `NSMenuItem`s with key equivalents in `app/src/app_menus.rs`. macOS NSMenu intercepts those chords before the app's keymap matcher runs. To make custom shortcuts override them, `custom_shortcut(action, ctx)` in `app_menus.rs` consults `ShortcutsModel::handle(ctx)` at menu-build time and returns `None` (no key equivalent on the menu item) when the chord is claimed by a custom shortcut. The menu entry remains and is clickable; only its keyboard shortcut is suppressed. Order requirement: `shortcuts::load` (in `launch()`'s init_fn) runs before `menu_bar_builder` (in `warp_app_will_finish_launching`) on macOS, so the registry is populated when `menu_bar` runs.

### 6. Action implementations (`executor.rs`)

Each primitive maps to one existing API:

- **`NewTab`** → `Workspace::add_terminal_tab(false, ctx)`. The newly added tab becomes active per its existing contract.
- **`NewPane(direction)`** → on the target tab's pane group, dispatch `PaneGroupAction::Add(direction)`. Reuses `app/src/pane_group/mod.rs:459-475`. The pane group's existing handler creates the new pane and focuses it; the executor relies on that focus shift to make subsequent `type`/`press` actions land in the new pane (PRODUCT §8 last sentence).
- **`Type(text)`** → resolve the target tab's active terminal view (see §9), call `terminal_view.write_to_pty(text.as_bytes().to_vec(), ctx)`. Long strings route through `terminal/view.rs:8444`'s chunked-write helper to avoid the macOS PTY 1 KB bug.
- **`Press(key)`** → look up bytes via `key_to_bytes::bytes_for(key)`; same write path as `Type`.
- **`Wait(dur)`** → `ctx.spawn(Timer::after(dur), move |runner_handle, _, ctx| runner_handle.continue_after_wait(ctx))`. Pattern matches `app/src/throttle.rs:41` and `app/src/debounce.rs:107`.

`Runner` state:

```rust
pub struct Runner {
    id: ShortcutId,
    action_idx: usize,
    target_tab: usize,
    target_window: WindowId,
    cancelled: bool,
}
```

Only one runner in flight (PRODUCT §13). A new trigger while alive → log + drop.

### 7. Escape cancellation (PRODUCT §14)

Register a flag-gated binding alongside the custom shortcuts:

```rust
EditableBinding::new(
    "shortcuts:cancel_running",
    "Cancel running custom shortcut",
    WorkspaceAction::CancelRunningShortcut,
)
.with_context_predicate(id!("Workspace") & id!(flags::SHORTCUT_RUNNING))
.with_key_binding("escape")
```

`flags::SHORTCUT_RUNNING` is a new context flag (sibling of `flags::SHOW_PROJECT_EXPLORER` in `workspace/mod.rs:488`), toggled on/off by `Runner` lifecycle. When no runner is alive the binding does not intercept Escape — terminal/modal Escape behavior is untouched. Registration order (custom-after-built-in) means this binding takes precedence when active and consumes the Escape press (PRODUCT §14's "not also passed through").

If `id!(flags::FOO)` flags turn out not to support imperative toggling, fall back to wrapping the runner in a View whose context predicate is alive iff the runner exists (same effect, different surface).

### 8. PTY write path for a target tab

`Workspace::write_to_pty` (line 8417) writes to the workspace's own pane, not a specific tab. The executor resolves the terminal view by tab index:

```rust
fn write_to_target_pty(
    &self,
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

Modelled after `read_from_active_terminal_view` (line 5884), parameterized by tab index. `TargetLost` triggers PRODUCT §17.

### 9. `press` → bytes (`key_to_bytes.rs`)

Const table:

| KeyName     | Bytes                  |
|-------------|------------------------|
| Enter       | `\r` (0x0D)            |
| NumpadEnter | `\r`                   |
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
| F1–F4       | `\x1bOP`/`OQ`/`OR`/`OS`|
| F5–F12      | `\x1b[15~`..`\x1b[24~` |

Source: VT100/ANSI. Pull `ESC`, `CR`, `HT` from `terminal/escape_sequences.rs::C0`; define CSI sequences inline rather than expanding that module.

DECCKM (Application Cursor Keys) mode is **not** v1-aware: arrow keys always emit the default `\x1b[?` form. Track as follow-up if `press: up` in `vim` proves wrong.

### 10. Toast surfacing (PRODUCT §19)

```rust
fn surface_errors(ctx: &mut AppContext) {
    let model = ShortcutsModel::handle(ctx);
    let errors = model.read(ctx, |m, _| m.errors.clone());
    if errors.is_empty() { return; }
    let msg = format!("shortcuts.yaml has {} error(s) — see logs or open Custom shortcuts", errors.len());
    for e in &errors { log::warn!("{e}"); }
    ToastStack::handle(ctx).update(ctx, |stack, ctx| {
        stack.add_persistent_toast(
            DismissibleToast::default(msg),
            current_window_id(ctx),
            ctx,
        );
    });
}
```

4b adds the "open Custom shortcuts" link to the toast (intent: clicking the toast opens the panel). 4a's toast is plain text.

## Proposed changes — 4b (Side panel GUI + hot reload)

### 11. File watcher → registry reload (PRODUCT §24)

Reuse `notify_debouncer_full::notify` (already a dependency, see `warp_managed_paths_watcher.rs:7`). A new `shortcuts/watcher.rs` registers a 200ms-debounced watch on `shortcuts_file_path()`. On any modify/create/remove event:

1. Re-read the file.
2. Call `parse_shortcuts_yaml` to produce a fresh `ParseResult`.
3. Update `ShortcutsModel.registry` and `ShortcutsModel.errors`.
4. Re-register editable bindings: remove the previous shortcut bindings, register new ones.
5. Re-surface toast if errors changed since last load.
6. In-flight runner (if any) is **not** interrupted (PRODUCT §24 last sentence) — it completes with its captured action list (`Runner` holds the action list it started with, not a registry index — see §6 above; verify during impl and adjust `Runner` shape if it currently dereferences the registry mid-flight).

The runtime registry refresh and the binding re-registration happen on the AppContext thread, gated by an internal mutex to serialize against in-flight saves from the GUI.

### 12. `ToolPanelView::Shortcuts`

Add the new variant in `app/src/workspace/view/left_panel.rs` and the matching `LeftPanelDisplayedTab::Shortcuts` in `app/src/app_state.rs:889`. Update `From<ToolPanelView> for LeftPanelDisplayedTab` (line 896) and the reverse mapping in `workspace/view.rs:3380-3385`.

In `compute_left_panel_views` (`workspace/view.rs:17257`), append `ToolPanelView::Shortcuts` immediately after `ToolPanelView::GlobalSearch` (PRODUCT §26).

The view itself lives at `app/src/shortcuts/view/`:

```
shortcuts/view/
├── mod.rs              // ShortcutsPanelView, the top-level View
├── list.rs             // ShortcutsList: row rendering + delete/edit dispatch
├── detail_editor.rs    // ShortcutsDetailEditor: form for one shortcut
├── keystroke_capture.rs // KeystrokeCaptureField: chord-capture widget
└── action_row.rs       // ActionRow: one row in the action editor
```

`ShortcutsPanelView` is a `View` with `ViewContext<Self>`; subscribes to `ShortcutsModel` changes to re-render on registry refresh.

### 13. Keystroke capture widget (PRODUCT §32)

`KeystrokeCaptureField` holds two states: `Display { chord: Option<Keystroke> }` and `Capturing { previous: Option<Keystroke> }`. In Capturing mode, the widget intercepts the next keystroke at the global key-event level (via the existing keystroke routing surface — investigate `app.subscribe_to_keystroke` or equivalent during impl; same hook the keymap uses to deliver `EditableBinding` dispatches). Escape during capture reverts to `Display { chord: previous }`.

Captured chord is platform-specific (cmd vs ctrl); PRODUCT §32 documents this and says portable `cmdorctrl-` requires hand-editing.

### 14. Action editor (PRODUCT §33)

`ShortcutsDetailEditor` holds an `EditingShortcut`:

```rust
struct EditingShortcut {
    keys: Option<Keystroke>,
    actions: Vec<EditingAction>,
    edit_target: EditTarget,  // CreateNew or Index(usize)
}

enum EditingAction {
    NewTab,
    NewPane { direction: Direction, parameter_error: Option<String> },
    Type { text: String, error: Option<String> },
    Press { key: Option<KeyName>, error: Option<String> },
    Wait { raw: String, parsed: Option<Duration>, error: Option<String> },
}
```

Inline validation reuses `config.rs`'s validators (PRODUCT §34). The Save button is disabled while any field has an error. On Save:

1. Build a `Shortcut` from `EditingShortcut`. Surface any final errors.
2. If `edit_target == CreateNew`, append; otherwise replace at the index.
3. Call `save_to_disk` (next section).

### 15. Save semantics (PRODUCT §36)

`shortcuts/save.rs`:

```rust
pub fn save_to_disk(shortcuts: &[Shortcut]) -> Result<(), SaveError> {
    let yaml = serialize_shortcuts(shortcuts);
    let path = shortcuts_file_path();
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, yaml)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
```

Atomic via temp-file + rename. After successful save, the file watcher (§11) picks up the change and refreshes the in-memory registry. The save path also calls the refresh directly so platforms where file-watching is unavailable still update the in-memory registry (PRODUCT §24).

Disk write failures surface inline in the editor (PRODUCT §36 last sentence) and roll back the in-memory state to match disk.

### 16. Conflict warnings (PRODUCT §38)

`shortcuts/conflict.rs::detect_conflicts(chord, current_registry, editing_index) -> Vec<Conflict>` returns up to two conflicts:

```rust
enum Conflict {
    BuiltIn { binding_name: String },
    Custom { entry_index: usize, keys: Keystroke },
}
```

Built-in detection: walk `app.editable_bindings()` looking for `EditableBinding`s with `binding.trigger() == chord` whose `name` does not start with `"shortcuts:"`. Custom detection: linear scan of `current_registry`, skipping `editing_index`. Both warnings render below the keystroke field; neither disables Save.

### 17. Errors banner (PRODUCT §35)

The panel's top-level view checks `ShortcutsModel.errors` on every render. If non-empty, renders a collapsible banner (similar shape to the existing left-panel banners — locate a precedent during impl). Click expands; each error message is rendered verbatim.

The invariant-by-invariant test plan (below) verifies that errors stick around until the underlying entries are fixed and that the banner clears on the next successful reload.

## Testing and validation

| PRODUCT § | Verification | Phase |
|-----------|--------------|-------|
| §1 (config file path) | Unit: `shortcuts_file_path()` joins `config_local_dir()` with `"shortcuts.yaml"`. Smoke step 1. | 4a |
| §2–§3 (top-level, entry shape) | Parser unit tests in `shortcuts_tests.rs`: top-level list / scalar / missing key → exact §20 message; missing `keys`/`actions`/empty actions → §20 rows. | 4a |
| §4 (chord normalization) | Parser unit: `cmd-shift-d` auto-normalizes to `cmd-shift-D`; `Cmd-Shift-D` accepted; `cmdorctrl-1` parses; `cmd-k cmd-s` rejected. Delegates to `Keystroke::parse`. | 4a |
| §5 (action vocabulary) | Parser unit: each of `new_tab`/`new_pane`/`type`/`press`/`wait` parses; unknown token → §20; multi-key map → §20. | 4a |
| §6 (new_tab) | Workspace unit: `RunCustomShortcut` with sequence `[NewTab]` advances `active_tab_index`; runner's `target_tab` equals new index. Smoke step 3 indirectly (the driving examples use `new_pane`, not `new_tab`; covered explicitly by smoke step 15 with the `cmdorctrl-t` override). | 4a |
| §7 (new_pane) | Pane-group unit: `RunCustomShortcut` with `[NewPane(Right)]` triggers `PaneGroupAction::Add(Direction::Right)`; new pane is active. Parser unit: missing direction → §20; invalid direction → §20. Smoke steps 3, 4. | 4a |
| §8 (sticky target — tab and pane) | Workspace unit: start runner with `[NewPane(Right), Wait(50ms), Type("x")]`; mid-wait click back to original pane; verify `Type` lands in the newly-created pane. Smoke step 5. | 4a |
| §9 (type semantics) | Executor unit: `Type("hello")` writes `b"hello"` to target PTY; no shell expansion, no twarp keybinding dispatched. Smoke step 3. | 4a |
| §9 (newline in type) | Parser unit: `type: "a\nb"` → §20. | 4a |
| §10 (press keys + bytes) | `key_to_bytes_tests.rs`: every supported `KeyName` returns the documented bytes; `bytes_for(Enter)` == `b"\r"`. | 4a |
| §10 (unknown press / modifier in press) | Parser unit: `press: zzz` → §20; `press: ctrl-c` → §20. | 4a |
| §11 (wait parsing + bounds) | Parser unit: `500ms`, `2s`, `1m` parse; `0ms`, `61s`, `1h`, `2x` → §20. | 4a |
| §11 (wait does not block input) | Manual: type into another pane during wait. | 4a |
| §12 (trigger) | Keymap unit: registering a Shortcut + chord lookup → `RunCustomShortcut { id }`. Smoke steps 3, 4. | 4a |
| §13 (single in-flight) | Smoke step 7. Executor unit: second start while in flight is a no-op. | 4a |
| §14 (cancellation) | Smoke step 6. Executor unit: `cancelled` mid-wait short-circuits; consumed Escape produces no `\x1b` on target PTY. | 4a |
| §15 (no abort on focus loss / non-Escape input) | Manual: alt-tab away during a wait, return — sequence completes. | 4a |
| §16 (built-in conflict) | Smoke step 15. Keymap unit: after built-in + custom registration, lookup for `cmdorctrl-t` returns the custom action. The driving-example `cmdorctrl-shift-D` shadowing of `SplitPaneRight` is exercised by smoke step 3. | 4a |
| §17 (target lost) | Executor unit: `[NewPane(Right), Wait(50ms), Type("x")]`, close the created pane during the wait; assert abort + `flags::SHORTCUT_RUNNING` cleared. | 4a |
| §18 / §19 (skipped invalid, toast) | Smoke step 12. Parser unit: one bad entry yields one error message; valid entries load. | 4a / 4b |
| §20 (exact error messages) | Parser unit asserts each row's message verbatim against manufactured malformed YAML. Includes `new_pane` direction errors. | 4a |
| §21 (unparseable yaml) | Parser unit: indented-incorrectly YAML → single error containing `"failed to parse"`. | 4a |
| §22 (empty/missing) | Smoke step 14. Parser units for empty string, `shortcuts:\n` (null), `shortcuts: []`. | 4a / 4b |
| §23 (no leak when unfocused) | Manual — inherits behavior from existing keybindings. | 4a |
| §24 (hot reload) | Smoke step 13. Watcher unit: file-modify event triggers parse + registry refresh; in-flight runner continues with its captured actions. | 4b |
| §25 (telemetry) | Manual spot-check: existing user-keybinding event fires; no per-action breakdown, no `type` payload. | 4a |
| §26 (panel location) | Smoke step 8. View test: `compute_left_panel_views` returns `[..., GlobalSearch, Shortcuts, ...]`. | 4b |
| §27 (list view) | Smoke step 8. View unit: registry with 2 entries renders 2 rows in source order with arrow-form summary. | 4b |
| §28 (empty state) | Smoke step 14. View unit: registry empty → empty-state widget rendered, no rows. | 4b |
| §29 (create) | Smoke step 10. View unit: "+ New shortcut" opens detail editor with empty fields; Save appends to registry; cancel discards. | 4b |
| §30 (edit) | Smoke step 9. View unit: row click pre-fills detail editor; Save replaces in place. | 4b |
| §31 (delete) | Smoke step 11. View unit: delete removes from registry and disk; no confirmation. | 4b |
| §32 (keystroke capture) | Smoke step 10 (capture flow). Widget unit: capture mode intercepts next keystroke; Escape reverts. | 4b |
| §33 (action editor) | Smoke step 10. View unit: each action type has the right parameter widget; up/down reordering swaps adjacent rows. | 4b |
| §34 (inline validation) | View unit: malformed `wait` value disables Save; valid value enables. | 4b |
| §35 (errors banner) | Smoke step 12. View unit: non-empty `ShortcutsModel.errors` renders banner with verbatim messages. | 4b |
| §36 (save semantics) | View unit: Save writes YAML to disk + triggers registry refresh; disk write failure shows inline error + rolls back. | 4b |
| §37 (yaml formatting) | Round-trip unit: parse → serialize → parse produces equivalent in-memory state; comments dropped. | 4b |
| §38 (conflict warnings) | Smoke step 15. View unit: chord matching a built-in shows the built-in warning; chord matching an existing custom shows the custom warning. | 4b |

New test files:

- 4a: `app/src/shortcuts/shortcuts_tests.rs` (parser table-tests, key-to-bytes, executor unit tests with stubbed `Workspace`), additions to `app/src/workspace/view_test.rs` for `RunCustomShortcut` dispatch + target-lost.
- 4b: `app/src/shortcuts/view/view_tests.rs` (panel rendering, detail editor flow, keystroke capture, conflict detection), watcher unit test in `shortcuts/watcher_tests.rs`.

No new integration test for v1 — manual smoke test (PRODUCT §Smoke test) is the canonical pre-merge check. Add an integration test in a follow-up if regressions accumulate.

`./script/presubmit` must be green before opening either impl PR.

## Risks and mitigations

- **Risk: `id!(flags::FOO)` flags can't be toggled imperatively; the Escape-cancel gate doesn't work.** Mitigation: research the flag mechanism in 4a; fall back to a view-scoped predicate if necessary.
- **Risk: custom registration after built-ins isn't actually the precedence rule in practice.** Mitigation: small keymap-level test asserting `cmdorctrl-t` resolves to the custom action after both registrations, run early in 4a. If precedence is per-name and built-ins win, switch to `set_custom_trigger`-based override.
- **Risk: `press: up` (etc.) sends the wrong escape sequence in `vim`-like apps that expect Application Cursor Keys mode.** Mitigation: documented v1 limitation; DECCKM-aware encoding is a follow-up.
- **Risk: `new_pane` on a context that suppresses the built-in split (e.g. flagged-off `ContextFlag::CreateNewSession`) becomes a silent no-op.** Mitigation: dispatch `PaneGroupAction::Add` directly to the target tab's pane group rather than through the keymap, so context flags don't gate it. PRODUCT §7 codifies this.
- **Risk: File-watch fires during a save and double-reloads.** Mitigation: 200ms debounce on `notify_debouncer_full`, and the GUI save path takes a lock that suppresses watcher-triggered reloads while a save is in flight.
- **Risk: GUI save rewrites a config file that the user is also hand-editing concurrently.** Mitigation: atomic temp-file + rename keeps the file consistent on disk; we accept that the user's in-flight edit is overwritten (the GUI Save is explicit user intent). Document this trade-off in the GUI as a follow-up.
- **Risk: Toast spam during rapid file edits.** Mitigation: debounce already covers it; surface the toast at most once per debounced reload event.
- **Risk: Error messages drift between spec and implementation.** Mitigation: parser tests assert message text verbatim. PRODUCT §20 is the source of truth, reused by both 4a (parser path) and 4b (GUI inline validation path).
- **Risk: `Runner` dereferences the live registry mid-flight and a 4b reload mutates it underfoot.** Mitigation: `Runner` captures its `Vec<Action>` at start (not just a `ShortcutId` into the registry). Verify during 4a impl; adjust the `Runner` shape if necessary.

## Follow-ups

- **DECCKM-aware** arrow-key encoding for `press`.
- **Modifier-in-press** (`press: ctrl-c`, `press: alt-f`).
- **More actions**: `new_window`, `focus_tab: <index>`, `close_pane`, `run: <command>` (high-level `type` + `enter`).
- **Drag-to-reorder** in the action editor.
- **Undo** for GUI delete.
- **Comment-preserving YAML round-trip** (would require a richer YAML round-tripper than `serde_yaml`).
- **Settings-page surface** for `shortcuts.yaml` errors in addition to the panel banner.
- **Sequence queuing** if user demand emerges (PRODUCT §13 currently drops, doesn't queue).
- **Portable chord normalization** in the keystroke-capture widget (detect cmd-on-mac / ctrl-on-others → normalize to `cmdorctrl-`).

## Parallelization

The two sub-phases ship as two sequential PRs (4a, then 4b) and are not parallelizable: 4b depends on 4a's parser and registry as its public API. Within each sub-phase, the work is sequential enough — small file count, tight coupling between executor and action types — that splitting across agents would just create merge churn.

If a follow-up adds many independent actions (e.g. `new_window`, `focus_tab`, `close_pane` all at once), parallelizing on a per-action basis becomes worthwhile; flag at that point.
