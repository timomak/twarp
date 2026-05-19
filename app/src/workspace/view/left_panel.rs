use std::collections::HashSet;
use std::path::{Path, PathBuf};

use warp_core::ui::theme::color::internal_colors;
use warp_core::{send_telemetry_from_ctx, ui::Icon};
use warp_util::path::LineAndColumnArg;
use warpui::{
    elements::{
        new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig},
        resizable_state_handle, Border, ChildView, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, DispatchEventResult, DragBarSide, Element, Empty, EventHandler, Flex,
        Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
        Resizable, ResizableStateHandle, ScrollbarWidth, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    platform::Cursor,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle,
};

// twarp: 2c-d — AgentConversationsModel/AIConversationId stubs no longer needed in this file.
#[cfg(feature = "local_fs")]
use crate::code::file_tree::FileTreeEvent;
use crate::coding_panel_enablement_state::CodingPanelEnablementState;
use crate::drive::panel::{DrivePanel, DrivePanelEvent};
use crate::pane_group::working_directories::WorkingDirectory;
use crate::pane_group::{PaneGroup, WorkingDirectoriesEvent, WorkingDirectoriesModel};
#[cfg(feature = "local_fs")]
use crate::server::telemetry::CodePanelsFileOpenEntrypoint;
use crate::server::telemetry::{FileTreeSource, WarpDriveSource};
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::resolve_file_target_with_editor_choice;
use crate::util::openable_file_type::FileTarget;
// twarp: 2c-d — conversation_list removed
use crate::workspace::view::global_search::view::{
    Event as GlobalSearchViewEvent, GlobalSearchEntryFocus, GlobalSearchView,
};
use crate::workspace::view::{
    LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME, LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME,
    LEFT_PANEL_WARP_DRIVE_BINDING_NAME, OPEN_GLOBAL_SEARCH_BINDING_NAME,
    TOGGLE_PROJECT_EXPLORER_BINDING_NAME, TOGGLE_WARP_DRIVE_BINDING_NAME,
};
use crate::{
    appearance::Appearance,
    code::file_tree::FileTreeView,
    drive::panel::{MAX_SIDEBAR_WIDTH_RATIO, MIN_SIDEBAR_WIDTH},
    pane_group::pane::view::header::{components::HEADER_EDGE_PADDING, PANE_HEADER_HEIGHT},
    pane_group::{self},
    terminal::resizable_data::{ModalType, ResizableData},
    ui_components::{
        buttons::{icon_button, icon_button_with_color},
        icons,
    },
    util::bindings::keybinding_name_to_display_string,
    view_components::{ClickableTextInput, ClickableTextInputAction, ClickableTextInputEvent},
    workspace::WorkspaceAction,
    TelemetryEvent,
};
use warpui::keymap::Keystroke;

#[derive(Default)]
struct MouseStateHandles {
    project_explorer_button: MouseStateHandle,
    global_search_button: MouseStateHandle,
    warp_drive_button: MouseStateHandle,
    shortcuts_button: MouseStateHandle,
    add_new_shortcut_button: MouseStateHandle,
    // twarp: 2c-d — conversation_list_view_button removed
}

#[derive(Clone, Debug)]
pub enum LeftPanelAction {
    ProjectExplorer,
    GlobalSearch {
        entry_focus: GlobalSearchEntryFocus,
    },
    WarpDrive,
    /// Custom command shortcuts panel (PRODUCT 04 §26).
    Shortcuts,
    /// Open the inline detail editor in "create new" mode
    /// (PRODUCT §29). Triggered by the "+ New shortcut" link. 4c
    /// originally appended a placeholder shortcut here; 4d replaced
    /// that with the editor flow now that the editor exists.
    ShortcutsAddNew,
    /// Open `shortcuts.yaml` in the OS default editor so the user can
    /// hand-edit a shortcut's chord, actions, or both. Used both as
    /// the row-click action (PRODUCT §30 simplified) and as the
    /// "Rename" context-menu item — inline keystroke capture is 4d.
    ShortcutsOpenInEditor,
    /// Toggle the inline Rename / Delete menu for a specific row
    /// (PRODUCT §§30, 31). Set by right-clicking a row; cleared by a
    /// subsequent click or by a menu item firing.
    ShortcutsToggleRowMenu(usize),
    /// Close any open per-row inline menu (e.g. clicking outside).
    ShortcutsCloseRowMenu,
    /// Remove the shortcut at the given index from the registry,
    /// persist via 4b's `save_to_disk`, and reload. (PRODUCT §31.)
    ShortcutsDelete(usize),
    /// Open the inline detail editor for the shortcut at this index
    /// (PRODUCT §30, smoke test step 9). Triggered by the inline
    /// right-click menu's [edit] item. `ShortcutsAddNew` opens the
    /// editor in create mode instead, replacing 4c's placeholder-
    /// append behavior.
    ShortcutsBeginEdit(usize),
    /// Discard the in-flight detail editor without saving (PRODUCT §29).
    ShortcutsEditCancel,
    /// Validate and persist the in-flight detail editor (PRODUCT §36).
    /// Triggers a hot reload on success; surfaces an inline error
    /// banner on failure.
    ShortcutsEditSave,
    /// Click the chord field; enters keystroke-capture mode (PRODUCT §32).
    /// The next non-modifier-only keystroke replaces the chord.
    ShortcutsEditChordFieldClick,
    /// A keystroke was captured while the chord field was in
    /// capture mode (PRODUCT §32). Stores it and returns to display mode.
    ShortcutsEditChordCaptured(Keystroke),
    /// Escape while in chord-capture mode: restore the previous chord
    /// (PRODUCT §32).
    ShortcutsEditChordCancel,
    /// Cycle the action kind on the row at the given index
    /// (PRODUCT §33). Order: NewTab → NewPane → Type → Press → Wait → NewTab.
    /// Cycle-buttons stand in for the PRODUCT-spec dropdown so the editor
    /// doesn't have to manage one `FilterableDropdown` View handle per row.
    ShortcutsEditActionCycleKind(usize),
    /// Cycle the action's enum-valued parameter (PRODUCT §33). Applies to
    /// `new_pane` (right/down/left/up) and `press` (one of the v1
    /// supported named keys). Other action kinds ignore.
    ShortcutsEditActionCycleParam(usize),
    /// Append a new action row defaulting to `new_tab` (PRODUCT §33).
    ShortcutsEditActionAdd,
    /// Remove the action row at the given index (PRODUCT §33). When the
    /// list is empty the [+ Add action] button remains.
    ShortcutsEditActionRemove(usize),
    /// Reorder: move the action row at the given index up by one slot
    /// (PRODUCT §33). No-op for index 0.
    ShortcutsEditActionMoveUp(usize),
    /// Reorder: move the action row at the given index down by one slot.
    /// No-op when already last.
    ShortcutsEditActionMoveDown(usize),
    /// 5d: collapse/expand the Timeline section at the bottom of the
    /// Project Explorer panel. Collapsed = header-only; expanded =
    /// header + entries with a drag-resize handle on top.
    /// PRODUCT §§18–23.
    TimelineToggleExpanded,
    /// 5d (PRODUCT §20): append the next page of Timeline entries for
    /// the currently focused file.
    TimelineLoadMore,
    /// 5d (PRODUCT §21): open a read-only inline-diff pane showing the
    /// chosen commit's changes for the focused file (`git show <sha>^:<p>`
    /// as base, `git show <sha>:<p>` as content).
    TimelineSelectCommit {
        repo_path: PathBuf,
        file_path: PathBuf,
        sha: String,
    },
    // twarp: 2c-d — kept for legacy call-sites; AI conversation list deleted.
    ConversationListView,
}

pub enum LeftPanelEvent {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    FileTree(pane_group::Event),
    WarpDrive(DrivePanelEvent),
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    /// 5d (PRODUCT §21): open a read-only commit diff for a Timeline
    /// entry. The workspace handler fetches `git show <sha>^:<path>` /
    /// `git show <sha>:<path>`, writes the post content to a
    /// session-scoped tempdir, and routes it through the existing
    /// 5e diff-pane path with `read_only = true`.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    OpenTimelineCommitDiff {
        repo_path: PathBuf,
        file_path: PathBuf,
        sha: String,
    },
    // twarp: 2c-d — kept for legacy call-sites; AI conversation list deleted.
    NewConversationInNewTab,
    ShowDeleteConfirmationDialog {
        conversation_id: crate::app_state::AIConversationId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPanelView {
    ProjectExplorer,
    GlobalSearch {
        entry_focus: GlobalSearchEntryFocus,
    },
    WarpDrive,
    /// Custom command shortcuts panel (PRODUCT 04 §26). The full GUI lives
    /// in a future sub-phase; 4c renders the tab plus a placeholder so the
    /// integration lights up.
    Shortcuts,
    // twarp: 2c-d — variant kept so legacy call-sites compile; AI conversation list deleted.
    ConversationListView,
}

/// Encapsulates the active view state to enforce that all mutations go through
/// `active_view_state::set`, which handles necessary side effects.
mod active_view_state {
    use super::ToolPanelView;
    use warpui::ViewContext;

    pub struct ActiveViewState(ToolPanelView);

    impl ActiveViewState {
        pub fn get(&self) -> ToolPanelView {
            self.0
        }
    }

    pub fn new(view: ToolPanelView) -> ActiveViewState {
        ActiveViewState(view)
    }

    pub fn set(
        left_panel: &mut super::LeftPanelView,
        new_view: ToolPanelView,
        ctx: &mut ViewContext<super::LeftPanelView>,
    ) {
        let _previous = left_panel.active_view.0;
        left_panel.active_view.0 = new_view;
        left_panel.update_button_active_states();
        ctx.notify();

        // twarp: 2c-d — conversation list visibility tracking removed

        left_panel.update_active_file_tree_subscription_state(ctx);
    }
}

pub struct ToolbeltButtonConfig {
    pub icon: warp_core::ui::Icon,
    /// Optional icon to use when the given toolbelt option is in an active state.
    pub active_icon: Option<warp_core::ui::Icon>,
    pub tooltip_text: String,
    pub action: LeftPanelAction,
    /// Whether the button should be rendered with an "active" state.
    pub render_with_active_state: bool,
    /// Ordered list of binding names used to populate the tooltip keybinding display.
    ///
    /// Earlier bindings in the list are preferred in the tooltip.
    pub tooltip_keybinding_names: Vec<&'static str>,
    /// Cached keybinding display string for the tooltip.
    ///
    /// This is updated in response to [`KeybindingChangedEvent`]s.
    pub tooltip_keybinding: Option<String>,
}

pub struct LeftPanelView {
    resizable_state_handle: ResizableStateHandle,
    mouse_state_handles: MouseStateHandles,
    close_button_mouse_state: MouseStateHandle,
    warp_drive_view: ViewHandle<DrivePanel>,
    // twarp: 2c-d — conversation_list_view removed
    active_view: active_view_state::ActiveViewState,
    toolbelt_buttons: Vec<ToolbeltButtonConfig>,
    active_pane_group: Option<WeakViewHandle<PaneGroup>>,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    working_directories_model: ModelHandle<WorkingDirectoriesModel>,
    panel_position: super::PanelPosition,
    /// Index of the row whose inline Delete button is currently
    /// shown. Set by right-clicking a row, cleared by any subsequent
    /// click or by Delete completing.
    shortcut_context_menu_target: Option<usize>,
    /// Mouse state for the inline Delete button. One state total since
    /// only one row's menu is open at a time.
    shortcut_delete_mouse_state: MouseStateHandle,
    /// Persistent mouse states for the shortcut list rows. Grown on
    /// demand in `render_shortcut_row` to match the registry length.
    /// `Hoverable::on_click` only fires when the mouse-down and
    /// mouse-up events see the *same* `MouseStateHandle` — a fresh
    /// inline state per render breaks click detection silently, so
    /// these need to be stable across renders. Behind a `RefCell` so
    /// the `&self` render path can extend the vec when a new shortcut
    /// is added.
    shortcut_row_mouse_states: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// State for the inline detail editor (PRODUCT §§29-30, 32-38).
    /// `None` when no editor is open and the panel shows the list view;
    /// `Some` while the user is creating or editing one shortcut, in
    /// which case the panel renders the editor in place of the list.
    editing_shortcut: Option<EditingShortcutState>,

    /// twarp 5d (PRODUCT §§18–23): per-file commit Timeline section at
    /// the bottom of the Project Explorer tab. State is per-repo,
    /// keyed on the currently-active editor file. Re-fetched whenever
    /// `ActiveFileModel::ActiveFileChanged` fires for the active pane
    /// group (so opening/switching tabs refocuses the Timeline).
    timeline_state: TimelineSectionState,
    /// Collapsed by default — header-only. Click chevron expands.
    timeline_expanded: bool,
    /// Resizable height for the Timeline section when expanded. Drag
    /// bar sits on top of the section header so dragging up enlarges
    /// the Timeline and shrinks the file tree above. Height is
    /// session-scoped — not persisted across restarts in this PR
    /// (follow-up via `PaneSettings` if the owner wants persistence).
    timeline_resizable_handle: ResizableStateHandle,
    timeline_header_mouse_state: MouseStateHandle,
    timeline_load_more_mouse_state: MouseStateHandle,
    timeline_scroll_state: warpui::elements::ClippedScrollStateHandle,
    /// Per-entry stable mouse states. Grown lazily inside `render_*`
    /// since `Hoverable::on_click` requires the same handle across
    /// renders to detect click cycles.
    timeline_entry_mouse_states: std::cell::RefCell<Vec<MouseStateHandle>>,
}

/// twarp 5d: ephemeral data for the Project Explorer Timeline section.
/// Lives on the `LeftPanelView` (not a separate model) — only the
/// Project Explorer renders this, and the state has the same lifetime.
#[derive(Default)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
struct TimelineSectionState {
    /// Resolved git repo root for the focused file.
    repo_path: Option<PathBuf>,
    /// Path the Timeline is currently tracking. Stored as the full
    /// (absolute) path so callers can resolve repo-relative inside the
    /// async fetch task; equality checks compare absolute paths.
    focused_path: Option<PathBuf>,
    /// Entries loaded so far, most-recent first.
    entries: Vec<crate::code_review::timeline::TimelineEntry>,
    /// Suppresses duplicate fetches and dims `[Load more]`.
    loading: bool,
    /// Drives whether `[Load more]` renders. Goes false once a page
    /// returns fewer than `TIMELINE_PAGE_SIZE` entries.
    has_more: bool,
    /// SHAs in `<upstream>..HEAD` for the focused path — drives the
    /// `↑` ahead-of-upstream marker (PRODUCT §23).
    local_only_shas: HashSet<String>,
}

/// Action kinds that round-trip with the shortcuts parser. Mirrors
/// `shortcuts::action::Action` shape but holds raw parameter text so
/// the editor can offer in-progress edits without requiring a valid
/// parse on every keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditingActionKind {
    NewTab,
    NewPane,
    Type,
    Press,
    Wait,
}

impl EditingActionKind {
    fn label(self) -> &'static str {
        match self {
            EditingActionKind::NewTab => "new_tab",
            EditingActionKind::NewPane => "new_pane",
            EditingActionKind::Type => "type",
            EditingActionKind::Press => "press",
            EditingActionKind::Wait => "wait",
        }
    }

    /// Cycle to the next kind (PRODUCT §33 dropdown stand-in).
    fn next(self) -> Self {
        match self {
            EditingActionKind::NewTab => EditingActionKind::NewPane,
            EditingActionKind::NewPane => EditingActionKind::Type,
            EditingActionKind::Type => EditingActionKind::Press,
            EditingActionKind::Press => EditingActionKind::Wait,
            EditingActionKind::Wait => EditingActionKind::NewTab,
        }
    }
}

/// The directions accepted by `new_pane`, in cycle order. Matches
/// PRODUCT §7's expanded v1 set (right/down/left/up).
const NEW_PANE_DIRECTIONS: [&str; 4] = ["right", "down", "left", "up"];

/// The named keys offered by the action editor's `press` selector.
/// A subset of PRODUCT §10's full v1 list — the editor cycles through
/// the common cases; users wanting `f1`–`f12` or arrow keys can hand-
/// edit `shortcuts.yaml` (which the parser still accepts).
const PRESS_KEY_CYCLE: [&str; 7] = ["enter", "tab", "escape", "backspace", "space", "up", "down"];

/// Whether this kind takes a free-text parameter or a cycled enum.
fn kind_param_is_freetext(kind: EditingActionKind) -> bool {
    matches!(kind, EditingActionKind::Type | EditingActionKind::Wait)
}

/// Whether this kind takes no parameter at all.
fn kind_has_no_param(kind: EditingActionKind) -> bool {
    matches!(kind, EditingActionKind::NewTab)
}

/// One row in the action editor. Holds the kind and the current parameter
/// representation: a cycle-state index for enum-valued params, a string
/// for free-text params, or nothing for `new_tab`.
struct EditingAction {
    kind: EditingActionKind,
    /// Free-text parameter (used when `kind` is `Type` / `Wait`).
    /// Backed by a `ClickableTextInput` child view so it round-trips
    /// through warpui's editor without us managing focus by hand.
    param_text: String,
    param_text_input: Option<ViewHandle<ClickableTextInput>>,
    /// Cycle index into the enum's accepted values
    /// (`NEW_PANE_DIRECTIONS` for `NewPane`, `PRESS_KEY_CYCLE` for
    /// `Press`). Ignored when `kind` is `NewTab` / `Type` / `Wait`.
    param_cycle_idx: usize,
    /// Mouse state for the cycle-kind button.
    kind_button_mouse: MouseStateHandle,
    /// Mouse state for the cycle-param button (when applicable).
    param_button_mouse: MouseStateHandle,
    /// Mouse state for the [×] remove affordance.
    remove_button_mouse: MouseStateHandle,
    /// Mouse states for the up/down reorder buttons.
    move_up_mouse: MouseStateHandle,
    move_down_mouse: MouseStateHandle,
}

impl EditingAction {
    /// Default action used for new rows added via [+ Add action]. PRODUCT
    /// §33 specifies `new_tab` as the default; matches our cycle order.
    fn new_default() -> Self {
        Self {
            kind: EditingActionKind::NewTab,
            param_text: String::new(),
            param_text_input: None,
            param_cycle_idx: 0,
            kind_button_mouse: MouseStateHandle::default(),
            param_button_mouse: MouseStateHandle::default(),
            remove_button_mouse: MouseStateHandle::default(),
            move_up_mouse: MouseStateHandle::default(),
            move_down_mouse: MouseStateHandle::default(),
        }
    }

    /// Hydrate an editor row from a parsed `Action` on disk. The
    /// cycle indices are recovered from the action's payload so the
    /// row renders the value the user originally wrote.
    fn from_runtime(action: &crate::shortcuts::action::Action) -> Self {
        use crate::pane_group::Direction;
        use crate::shortcuts::action::Action as A;
        let mut row = Self::new_default();
        match action {
            A::NewTab => {
                row.kind = EditingActionKind::NewTab;
            }
            A::NewPane(dir) => {
                row.kind = EditingActionKind::NewPane;
                let dir_str = match dir {
                    Direction::Right => "right",
                    Direction::Down => "down",
                    Direction::Left => "left",
                    Direction::Up => "up",
                };
                row.param_cycle_idx = NEW_PANE_DIRECTIONS
                    .iter()
                    .position(|c| *c == dir_str)
                    .unwrap_or(0);
            }
            A::Type(text) => {
                row.kind = EditingActionKind::Type;
                row.param_text = text.clone();
            }
            A::Press(key) => {
                row.kind = EditingActionKind::Press;
                let key_str = key_name_to_label(*key);
                row.param_cycle_idx = PRESS_KEY_CYCLE
                    .iter()
                    .position(|c| *c == key_str)
                    .unwrap_or(0);
            }
            A::Wait(dur) => {
                row.kind = EditingActionKind::Wait;
                // Express the duration in whichever unit yields a clean integer.
                let ms = dur.as_millis() as u64;
                row.param_text = if ms >= 60_000 && ms.is_multiple_of(60_000) {
                    format!("{}m", ms / 60_000)
                } else if ms >= 1_000 && ms.is_multiple_of(1_000) {
                    format!("{}s", ms / 1_000)
                } else {
                    format!("{ms}ms")
                };
            }
        }
        // `param_text_input` is filled in by `ensure_param_input_for_row` once we
        // have a `ViewContext` — `from_runtime` is `ViewContext`-free.
        row
    }

    /// Build the runtime `Action` for this row, returning an error
    /// message suitable for `validation_error` if the parameter is
    /// missing or malformed. Mirrors PRODUCT §20 vocabulary where it can.
    fn to_runtime_action(
        &self,
        one_based_index: usize,
    ) -> Result<crate::shortcuts::action::Action, String> {
        use crate::pane_group::Direction;
        use crate::shortcuts::action::Action as A;
        match self.kind {
            EditingActionKind::NewTab => Ok(A::NewTab),
            EditingActionKind::NewPane => {
                let dir_str = NEW_PANE_DIRECTIONS
                    .get(self.param_cycle_idx)
                    .copied()
                    .unwrap_or("right");
                let direction = match dir_str {
                    "right" => Direction::Right,
                    "down" => Direction::Down,
                    "left" => Direction::Left,
                    "up" => Direction::Up,
                    other => {
                        return Err(format!(
                            "action #{one_based_index}: invalid 'new_pane' direction '{other}'"
                        ));
                    }
                };
                Ok(A::NewPane(direction))
            }
            EditingActionKind::Type => {
                if self.param_text.contains('\n') {
                    return Err(format!(
                        "action #{one_based_index}: 'type' value contains a newline; use 'press: enter' to submit input"
                    ));
                }
                Ok(A::Type(self.param_text.clone()))
            }
            EditingActionKind::Press => {
                let key_str = PRESS_KEY_CYCLE
                    .get(self.param_cycle_idx)
                    .copied()
                    .unwrap_or("enter");
                let key = label_to_key_name(key_str).ok_or_else(|| {
                    format!("action #{one_based_index}: unknown key '{key_str}' in 'press'")
                })?;
                Ok(A::Press(key))
            }
            EditingActionKind::Wait => {
                let dur = parse_wait_value(&self.param_text)
                    .map_err(|msg| format!("action #{one_based_index}: {msg}"))?;
                Ok(A::Wait(dur))
            }
        }
    }
}

/// Convert a `KeyName` to the string we use in the cycle list and on disk.
fn key_name_to_label(key: crate::shortcuts::action::KeyName) -> &'static str {
    use crate::shortcuts::action::KeyName;
    match key {
        KeyName::Enter => "enter",
        KeyName::Tab => "tab",
        KeyName::Escape => "escape",
        KeyName::Backspace => "backspace",
        KeyName::Space => "space",
        KeyName::Up => "up",
        KeyName::Down => "down",
        KeyName::Left => "left",
        KeyName::Right => "right",
        KeyName::Home => "home",
        KeyName::End => "end",
        KeyName::PageUp => "pageup",
        KeyName::PageDown => "pagedown",
        KeyName::Delete => "delete",
        KeyName::Insert => "insert",
        KeyName::NumpadEnter => "numpadenter",
        KeyName::F(n) => match n {
            1 => "f1",
            2 => "f2",
            3 => "f3",
            4 => "f4",
            5 => "f5",
            6 => "f6",
            7 => "f7",
            8 => "f8",
            9 => "f9",
            10 => "f10",
            11 => "f11",
            12 => "f12",
            _ => "f1",
        },
    }
}

/// Inverse of `key_name_to_label`, scoped to the labels the editor's
/// cycle list can produce — `PRESS_KEY_CYCLE` plus the future-proofing
/// extras the parser can already round-trip.
fn label_to_key_name(label: &str) -> Option<crate::shortcuts::action::KeyName> {
    use crate::shortcuts::action::KeyName;
    Some(match label {
        "enter" => KeyName::Enter,
        "tab" => KeyName::Tab,
        "escape" => KeyName::Escape,
        "backspace" => KeyName::Backspace,
        "space" => KeyName::Space,
        "up" => KeyName::Up,
        "down" => KeyName::Down,
        "left" => KeyName::Left,
        "right" => KeyName::Right,
        "home" => KeyName::Home,
        "end" => KeyName::End,
        "pageup" => KeyName::PageUp,
        "pagedown" => KeyName::PageDown,
        "delete" => KeyName::Delete,
        "insert" => KeyName::Insert,
        "numpadenter" => KeyName::NumpadEnter,
        other if other.starts_with('f') => {
            let n: u8 = other[1..].parse().ok()?;
            if (1..=12).contains(&n) {
                KeyName::F(n)
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

/// PRODUCT §11: parse a `wait` value like `500ms` / `2s` / `1m`,
/// clamped to `[1ms, 60s]`. Returns a human-readable error message
/// (matching PRODUCT §20 wording) for failures.
fn parse_wait_value(raw: &str) -> Result<std::time::Duration, String> {
    let trimmed = raw.trim();
    let (num, unit) = if let Some(n) = trimmed.strip_suffix("ms") {
        (n, "ms")
    } else if let Some(n) = trimmed.strip_suffix('s') {
        (n, "s")
    } else if let Some(n) = trimmed.strip_suffix('m') {
        (n, "m")
    } else {
        return Err(format!(
            "invalid 'wait' value '{trimmed}'; expected a duration like '500ms', '2s', '1m' (1ms\u{2013}60s)"
        ));
    };
    let value: u64 = num.parse().map_err(|_| {
        format!(
            "invalid 'wait' value '{trimmed}'; expected a duration like '500ms', '2s', '1m' (1ms\u{2013}60s)"
        )
    })?;
    let ms = match unit {
        "ms" => value,
        "s" => value.saturating_mul(1_000),
        "m" => value.saturating_mul(60_000),
        _ => unreachable!(),
    };
    if !(1..=60_000).contains(&ms) {
        return Err(format!(
            "invalid 'wait' value '{trimmed}'; expected a duration like '500ms', '2s', '1m' (1ms\u{2013}60s)"
        ));
    }
    Ok(std::time::Duration::from_millis(ms))
}

/// What the in-flight editor is editing — a brand-new entry or a row by index.
#[derive(Clone, Copy, Debug)]
enum EditTarget {
    Create,
    Index(usize),
}

/// The detail editor's full mutable state.
///
/// Lives inside `LeftPanelView::editing_shortcut` while the editor is
/// open and is `take()`n on Save/Cancel. Owns the child `ClickableTextInput`
/// view handles for the name field and each action's param input.
struct EditingShortcutState {
    target: EditTarget,
    /// Display copy of the name; mirrors what `name_text_input` emits
    /// so renders don't have to read the child view every frame.
    name_text: String,
    name_text_input: ViewHandle<ClickableTextInput>,
    /// Captured chord. `None` while the user has not yet captured one.
    chord: Option<Keystroke>,
    /// Whether the chord field is currently intercepting the next keystroke.
    /// PRODUCT §32 says capture is one-shot: click to enter, next key (or
    /// Escape) exits.
    capturing_chord: bool,
    /// Chord value at the moment capture started; restored on Escape.
    chord_before_capture: Option<Keystroke>,
    chord_field_mouse: MouseStateHandle,
    actions: Vec<EditingAction>,
    add_action_button_mouse: MouseStateHandle,
    save_button_mouse: MouseStateHandle,
    cancel_button_mouse: MouseStateHandle,
    /// Most recent validation error from a Save attempt (PRODUCT §34/§36).
    /// Cleared on the next successful Save or any state mutation.
    validation_error: Option<String>,
}

/// Maximum display length for a shortcut name in the side-panel list
/// row. Names longer than this are truncated with `…`. Character-based
/// rather than pixel-based so it works without a layout pass.
const ROW_NAME_MAX_CHARS: usize = 28;

/// Truncate `name` to at most `max_chars` characters, appending `…` if
/// truncated. Char-count-based so multi-byte UTF-8 sequences are handled
/// correctly.
fn truncate_display_name(name: &str, max_chars: usize) -> String {
    let count = name.chars().count();
    if count <= max_chars {
        return name.to_owned();
    }
    let mut s: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    s.push('…');
    s
}

fn toolbelt_tooltip_keybinding(binding_names: &[&'static str], app: &AppContext) -> Option<String> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();

    // Preserve caller-provided ordering so we can prioritize specific bindings.
    for binding_name in binding_names {
        if let Some(displayed) = keybinding_name_to_display_string(binding_name, app) {
            if seen.insert(displayed.clone()) {
                parts.push(displayed);
            }
        }
    }

    (!parts.is_empty()).then(|| parts.join(", "))
}

impl LeftPanelView {
    pub fn new(
        working_directories_model: ModelHandle<WorkingDirectoriesModel>,
        views: Vec<ToolPanelView>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let resizable_data_handle = ResizableData::handle(ctx);
        let resizable_state_handle = match resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), ModalType::LeftPanelWidth)
        {
            Some(handle) => handle,
            None => {
                log::error!("Couldn't retrieve left panel resizable state handle.");
                resizable_state_handle(600.0)
            }
        };
        let warp_drive_view = ctx.add_typed_action_view(DrivePanel::new);

        ctx.subscribe_to_view(&warp_drive_view, |_me, _, event, ctx| {
            ctx.emit(LeftPanelEvent::WarpDrive(event.clone()));
        });

        // twarp: 2c-d — conversation_list_view subscription removed

        let active_view = views.first().copied().unwrap_or(ToolPanelView::WarpDrive);
        let toolbelt_buttons = views
            .iter()
            .map(|view| Self::create_toolbelt_button_config(view, ctx))
            .collect();

        ctx.subscribe_to_model(
            &KeybindingChangedNotifier::handle(ctx),
            |me, _, event, ctx| match event {
                KeybindingChangedEvent::BindingChanged { .. } => {
                    for button in &mut me.toolbelt_buttons {
                        button.tooltip_keybinding =
                            toolbelt_tooltip_keybinding(&button.tooltip_keybinding_names, ctx);
                    }

                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(&working_directories_model, |me, _, event, ctx| {
            if let WorkingDirectoriesEvent::DirectoriesChanged {
                pane_group_id,
                directories,
            } = event
            {
                let Some(active_pane_group) = &me.active_pane_group else {
                    return;
                };
                let Some(active_pane_group) = active_pane_group.upgrade(ctx) else {
                    return;
                };
                if active_pane_group.id() != *pane_group_id {
                    return;
                }
                let has_terminal_session = directories.iter().any(|dir| dir.terminal_id.is_some());

                // Update GlobalSearchView root directories based on all working directories
                let roots: Vec<PathBuf> = directories.iter().map(|d| d.path.clone()).collect();

                let global_search_view =
                    me.get_or_create_global_search_view_for_pane_group(active_pane_group.id(), ctx);
                global_search_view.update(ctx, |view, view_ctx| {
                    view.set_root_directories(roots, view_ctx);
                });

                let directories: Vec<PathBuf> =
                    directories.iter().map(|dir| dir.path.clone()).collect();

                // Directories are already in display order (most recent first) from the model
                let directories = deduplicate_by_directory_name(directories);
                let file_tree_view =
                    me.get_or_create_file_tree_view_for_pane_group(active_pane_group.id(), ctx);

                let is_visible =
                    active_pane_group.as_ref(ctx).left_panel_open && me.is_file_tree_active();
                file_tree_view.update(ctx, |view, ctx| {
                    view.set_root_directories(directories, ctx);
                    view.set_has_terminal_session(has_terminal_session, ctx);
                    view.set_is_active(is_visible, ctx);

                    if is_visible {
                        view.auto_expand_to_most_recent_directory(ctx);
                    }
                });
                ctx.notify();
            }
        });

        let mut view = Self {
            resizable_state_handle,
            mouse_state_handles: Default::default(),
            close_button_mouse_state: Default::default(),
            warp_drive_view,
            // twarp: 2c-d — conversation_list_view removed
            active_view: active_view_state::new(active_view),
            toolbelt_buttons,
            active_pane_group: None,
            working_directories_model,
            panel_position: super::PanelPosition::Left,
            shortcut_context_menu_target: None,
            shortcut_delete_mouse_state: MouseStateHandle::default(),
            shortcut_row_mouse_states: std::cell::RefCell::new(Vec::new()),
            editing_shortcut: None,
            // 5d: Project Explorer Timeline section. Collapsed by
            // default; user expands via the chevron. PRODUCT §§18–23.
            timeline_state: TimelineSectionState::default(),
            timeline_expanded: false,
            // Default height = 220px when expanded; bounded to
            // (60, 70% of window) in the Resizable callback.
            timeline_resizable_handle: warpui::elements::resizable_state_handle(220.0),
            timeline_header_mouse_state: MouseStateHandle::default(),
            timeline_load_more_mouse_state: MouseStateHandle::default(),
            timeline_scroll_state: warpui::elements::ClippedScrollStateHandle::default(),
            timeline_entry_mouse_states: std::cell::RefCell::new(Vec::new()),
        };
        view.update_button_active_states();

        view
    }

    pub fn set_panel_position(
        &mut self,
        position: super::PanelPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        self.panel_position = position;
        ctx.notify();
    }

    /// Updates the available tool panel views.
    /// If the currently active view is no longer available, switches to the first available view.
    pub fn update_available_views(
        &mut self,
        views: Vec<ToolPanelView>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Check if the current active view is still available
        let current_view = self.active_view.get();
        let is_current_view_available = views.iter().any(|v| {
            // Use discriminant comparison for GlobalSearch since it has inner data
            match (v, &current_view) {
                (ToolPanelView::GlobalSearch { .. }, ToolPanelView::GlobalSearch { .. }) => true,
                _ => std::mem::discriminant(v) == std::mem::discriminant(&current_view),
            }
        });

        // Rebuild toolbelt buttons
        self.toolbelt_buttons = views
            .iter()
            .map(|view| Self::create_toolbelt_button_config(view, ctx))
            .collect();

        // If current view is no longer available, switch to the first available view
        if !is_current_view_available {
            if let Some(first_view) = views.first().copied() {
                active_view_state::set(self, first_view, ctx);
            }
        } else {
            self.update_button_active_states();
        }

        ctx.notify();
    }

    fn create_toolbelt_button_config(
        view: &ToolPanelView,
        ctx: &ViewContext<Self>,
    ) -> ToolbeltButtonConfig {
        match view {
            ToolPanelView::ProjectExplorer => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME,
                    TOGGLE_PROJECT_EXPLORER_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::FileCopy,
                    active_icon: None,
                    tooltip_text: "Project explorer".to_string(),
                    action: LeftPanelAction::ProjectExplorer,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            ToolPanelView::GlobalSearch { .. } => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME,
                    OPEN_GLOBAL_SEARCH_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::Search,
                    active_icon: None,
                    tooltip_text: "Global search".to_string(),
                    action: LeftPanelAction::GlobalSearch {
                        entry_focus: GlobalSearchEntryFocus::QueryEditor,
                    },
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            ToolPanelView::WarpDrive => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_WARP_DRIVE_BINDING_NAME,
                    TOGGLE_WARP_DRIVE_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::WarpDrive,
                    active_icon: None,
                    tooltip_text: "Warp Drive".to_string(),
                    action: LeftPanelAction::WarpDrive,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            ToolPanelView::Shortcuts => ToolbeltButtonConfig {
                icon: Icon::Keyboard,
                active_icon: None,
                tooltip_text: "Custom shortcuts".to_owned(),
                action: LeftPanelAction::Shortcuts,
                render_with_active_state: false,
                tooltip_keybinding: None,
                tooltip_keybinding_names: vec![],
            },
            // twarp: 2c-d — ConversationListView arm: AI deleted, use ProjectExplorer config as fallback.
            ToolPanelView::ConversationListView => ToolbeltButtonConfig {
                icon: Icon::FileCopy,
                active_icon: None,
                tooltip_text: String::new(),
                action: LeftPanelAction::ProjectExplorer,
                render_with_active_state: false,
                tooltip_keybinding: None,
                tooltip_keybinding_names: vec![],
            },
        }
    }

    fn get_or_create_global_search_view_for_pane_group(
        &mut self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<GlobalSearchView> {
        if let Some(view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_global_search_view(pane_group_id)
        {
            return view;
        }

        let global_search_view = ctx.add_typed_action_view(GlobalSearchView::new);

        ctx.subscribe_to_view(&global_search_view, |me, _, event, ctx| {
            me.handle_global_search_event(event, ctx);
        });

        self.working_directories_model.update(ctx, |model, _ctx| {
            model.store_global_search_view(pane_group_id, global_search_view.clone());
        });

        global_search_view
    }

    fn get_or_create_file_tree_view_for_pane_group(
        &mut self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<FileTreeView> {
        if let Some(view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_file_tree_view(pane_group_id)
        {
            return view;
        }

        let file_tree_view = ctx.add_typed_action_view(FileTreeView::new);

        #[cfg(feature = "local_fs")]
        ctx.subscribe_to_view(&file_tree_view, |me, _, event, ctx| {
            me.handle_file_tree_event(event, ctx);
        });

        self.working_directories_model.update(ctx, |model, _ctx| {
            model.store_file_tree_view(pane_group_id, file_tree_view.clone());
        });

        file_tree_view
    }

    pub fn active_global_search_view(
        &self,
        app: &AppContext,
    ) -> Option<ViewHandle<GlobalSearchView>> {
        let pane_group_id = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(app))
            .map(|pane_group| pane_group.id())?;
        self.working_directories_model
            .as_ref(app)
            .get_global_search_view(pane_group_id)
    }

    fn active_file_tree_view(&self, app: &AppContext) -> Option<ViewHandle<FileTreeView>> {
        let pane_group_id = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(app))
            .map(|pane_group| pane_group.id())?;
        self.working_directories_model
            .as_ref(app)
            .get_file_tree_view(pane_group_id)
    }

    pub fn active_view(&self) -> ToolPanelView {
        self.active_view.get()
    }

    pub fn is_warp_drive_active(&self) -> bool {
        self.active_view.get() == ToolPanelView::WarpDrive
    }

    pub fn is_file_tree_active(&self) -> bool {
        self.active_view.get() == ToolPanelView::ProjectExplorer
    }

    pub fn warp_drive_view(&self) -> &ViewHandle<DrivePanel> {
        &self.warp_drive_view
    }

    pub(crate) fn auto_expand_active_file_tree_to_most_recent_directory(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(file_tree_view) = self.active_file_tree_view(ctx) {
            file_tree_view.update(ctx, |view, ctx| {
                view.auto_expand_to_most_recent_directory(ctx);
            });
        }
    }

    pub fn restore_active_view_from_snapshot(
        &mut self,
        view: ToolPanelView,
        ctx: &mut ViewContext<Self>,
    ) {
        active_view_state::set(self, view, ctx);
    }

    /// Updates the active pane group ID so we filter events correctly.
    pub fn set_active_pane_group(
        &mut self,
        pane_group: ViewHandle<PaneGroup>,
        working_directories_model: &ModelHandle<WorkingDirectoriesModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let pane_group_id = pane_group.id();

        let previous_pane_group_id = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(ctx))
            .map(|pane_group| pane_group.id());

        self.active_pane_group = Some(pane_group.downgrade());

        if let Some(previous_pane_group_id) = previous_pane_group_id {
            if previous_pane_group_id != pane_group_id {
                self.deactivate_file_tree_view_for_pane_group(previous_pane_group_id, ctx);
            }
        }

        // Query the current state from the model
        let active_directories: Vec<WorkingDirectory> =
            working_directories_model.read(ctx, |model, _| {
                model
                    .most_recent_directories_for_pane_group(pane_group_id)
                    .map(|dirs| dirs.collect())
                    .unwrap_or_default()
            });
        let has_terminal_session = active_directories
            .iter()
            .any(|dir| dir.terminal_id.is_some());

        // Update GlobalSearchView root directories based on all working directories
        let roots: Vec<PathBuf> = active_directories.iter().map(|d| d.path.clone()).collect();
        let global_search_view =
            self.get_or_create_global_search_view_for_pane_group(pane_group_id, ctx);
        global_search_view.update(ctx, |view, view_ctx| {
            view.set_root_directories(roots, view_ctx);
        });

        let directories: Vec<PathBuf> = active_directories
            .iter()
            .map(|dir| dir.path.clone())
            .collect();
        let directories = deduplicate_by_directory_name(directories);
        let active_file_model = pane_group.as_ref(ctx).active_file_model().clone();

        // 5d: subscribe to the new pane group's ActiveFileModel so the
        // Timeline section refocuses on each tab switch (PRODUCT §18).
        // The file tree's own subscription is independent — separate
        // concerns.
        #[cfg(feature = "local_fs")]
        {
            ctx.subscribe_to_model(&active_file_model, |me, _, event, ctx| {
                let crate::code::active_file::ActiveFileEvent::ActiveFileChanged { file_info } =
                    event;
                me.refocus_timeline(file_info.clone(), ctx);
            });
            // Refocus immediately for the file that's already active in
            // the new pane group (subscription only catches future
            // events).
            if let Some(current) = active_file_model.as_ref(ctx).active_file().cloned() {
                self.refocus_timeline(current, ctx);
            } else {
                self.clear_timeline(ctx);
            }
        }

        let file_tree_view = self.get_or_create_file_tree_view_for_pane_group(pane_group_id, ctx);
        let left_panel_open = pane_group.as_ref(ctx).left_panel_open;
        let is_visible = left_panel_open && self.is_file_tree_active();
        file_tree_view.update(ctx, |view, ctx| {
            view.set_root_directories(directories, ctx);
            view.set_has_terminal_session(has_terminal_session, ctx);
            view.set_active_file_model(active_file_model, ctx);
            view.set_is_active(is_visible, ctx);

            if is_visible {
                view.auto_expand_to_most_recent_directory(ctx);
            }
        });

        self.on_left_panel_visibility_changed(left_panel_open, ctx);

        ctx.notify();
    }

    pub fn update_coding_panel_enablement(
        &mut self,
        enablement: CodingPanelEnablementState,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(feature = "local_fs")]
        {
            if let Some(file_tree_view) = self.active_file_tree_view(ctx) {
                file_tree_view.update(ctx, |view, ctx| {
                    view.set_enablement_state(enablement, ctx);
                });
            }
        }

        if let Some(global_search_view) = self.active_global_search_view(ctx) {
            global_search_view.update(ctx, |view, view_ctx| {
                view.set_enablement_state(enablement, view_ctx);
            });
        }
    }

    pub fn focus_active_view_on_entry(&mut self, ctx: &mut ViewContext<Self>) {
        match self.active_view.get() {
            ToolPanelView::ProjectExplorer => {
                if let Some(file_tree_view) = self.active_file_tree_view(ctx) {
                    file_tree_view.update(ctx, |view, ctx| {
                        view.on_left_panel_focused(ctx);
                    });
                    ctx.focus(&file_tree_view);
                }
            }
            ToolPanelView::GlobalSearch { entry_focus } => {
                if let Some(global_search_view) = self.active_global_search_view(ctx) {
                    global_search_view.update(ctx, |view, ctx| {
                        view.on_left_panel_focused(entry_focus, ctx);
                    });
                }

                active_view_state::set(
                    self,
                    ToolPanelView::GlobalSearch {
                        entry_focus: GlobalSearchEntryFocus::Results,
                    },
                    ctx,
                );
            }
            ToolPanelView::WarpDrive => {
                ctx.focus(&self.warp_drive_view);
                self.warp_drive_view.update(ctx, |view, ctx| {
                    view.reset_focused_index_in_warp_drive(true, ctx);
                });
            }
            // 4c stub: Shortcuts panel has no internal child view to focus
            // yet. Full GUI (list, detail editor) lands in a follow-up.
            ToolPanelView::Shortcuts => {}
            // twarp: 2c-d — ConversationListView arm: AI deleted, no-op.
            ToolPanelView::ConversationListView => {}
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_global_search_event(
        &mut self,
        _event: &GlobalSearchViewEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(feature = "local_fs")]
    fn handle_global_search_event(
        &mut self,
        event: &GlobalSearchViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            GlobalSearchViewEvent::OpenMatch {
                path,
                line_number,
                column_num,
            } => {
                let line_col = LineAndColumnArg {
                    line_num: *line_number as usize,
                    column_num: *column_num,
                };

                let settings = EditorSettings::as_ref(ctx);
                let target = resolve_file_target_with_editor_choice(
                    path,
                    *settings.open_code_panels_file_editor,
                    *settings.prefer_markdown_viewer,
                    *settings.open_file_layout,
                    None,
                );

                send_telemetry_from_ctx!(
                    TelemetryEvent::CodePanelsFileOpened {
                        entrypoint: CodePanelsFileOpenEntrypoint::GlobalSearch,
                        target: target.clone(),
                    },
                    ctx
                );

                ctx.emit(LeftPanelEvent::OpenFileWithTarget {
                    path: path.clone(),
                    target,
                    line_col: Some(line_col),
                });
            }
        }
    }

    #[cfg(feature = "local_fs")]
    fn handle_file_tree_event(&mut self, event: &FileTreeEvent, ctx: &mut ViewContext<Self>) {
        match event {
            FileTreeEvent::FileRenamed { old_path, new_path } => {
                ctx.emit(LeftPanelEvent::FileTree(pane_group::Event::FileRenamed {
                    old_path: old_path.clone(),
                    new_path: new_path.clone(),
                }));
            }
            FileTreeEvent::FileDeleted { path } => {
                ctx.emit(LeftPanelEvent::FileTree(pane_group::Event::FileDeleted {
                    path: path.clone(),
                }));
            }
            FileTreeEvent::AttachAsContext { path } => {
                ctx.emit(LeftPanelEvent::FileTree(
                    pane_group::Event::AttachPathAsContext { path: path.clone() },
                ));
            }
            FileTreeEvent::OpenFile {
                path,
                target,
                line_col,
            } => {
                ctx.emit(LeftPanelEvent::OpenFileWithTarget {
                    path: path.clone(),
                    target: target.clone(),
                    line_col: *line_col,
                });
            }
            FileTreeEvent::CDToDirectory { path } => {
                ctx.emit(LeftPanelEvent::FileTree(pane_group::Event::CDToDirectory {
                    path: path.clone(),
                }));
            }
            FileTreeEvent::OpenDirectoryInNewTab { path } => {
                ctx.emit(LeftPanelEvent::FileTree(
                    pane_group::Event::OpenDirectoryInNewTab { path: path.clone() },
                ));
            }
        }
    }

    // ----- 5d Timeline helpers (PRODUCT §§18–23) -----------------------

    /// Switch the Timeline to a different focused file and fire a fresh
    /// log fetch. Called on each `ActiveFileEvent::ActiveFileChanged`
    /// from the active pane group's `ActiveFileModel`, and once
    /// directly on pane-group switch.
    ///
    /// `path` is the **absolute** path of the newly-active file. The
    /// fetch resolves the containing git repo via
    /// `DetectedRepositories`; files outside any repo clear the
    /// Timeline.
    #[cfg(feature = "local_fs")]
    fn refocus_timeline(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        use repo_metadata::repositories::DetectedRepositories;

        if self.timeline_state.focused_path.as_deref() == Some(path.as_path()) {
            return;
        }
        let repo_root = DetectedRepositories::as_ref(ctx).get_root_for_path(&path);
        let Some(repo_root) = repo_root else {
            self.clear_timeline(ctx);
            return;
        };
        let Ok(repo_relative) = path.strip_prefix(&repo_root) else {
            self.clear_timeline(ctx);
            return;
        };
        let repo_relative = repo_relative.to_path_buf();

        self.timeline_state.repo_path = Some(repo_root.clone());
        self.timeline_state.focused_path = Some(path.clone());
        self.timeline_state.entries.clear();
        self.timeline_state.has_more = true;
        self.timeline_state.loading = true;
        self.timeline_state.local_only_shas.clear();
        self.timeline_entry_mouse_states.borrow_mut().clear();
        ctx.notify();

        let path_for_log = repo_relative.clone();
        let path_for_upstream = repo_relative.clone();
        let path_token = path.clone();
        let repo_for_log = repo_root.clone();
        let repo_for_upstream = repo_root;
        ctx.spawn(
            async move {
                // `@{u}` is git shorthand for the current branch's
                // upstream tracking ref. Returns an error (no panic)
                // when no upstream is configured, in which case
                // `fetch_local_only_shas` is a no-op.
                let upstream_ref = crate::util::git::run_git_command(
                    &repo_for_upstream,
                    &["rev-parse", "--abbrev-ref", "@{u}"],
                )
                .await
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
                let entries = crate::code_review::timeline::fetch_log_page(
                    &repo_for_log,
                    &path_for_log,
                    0,
                    crate::code_review::timeline::TIMELINE_PAGE_SIZE,
                )
                .await
                .unwrap_or_else(|err| {
                    log::warn!(
                        "[twarp 5d] timeline log fetch failed for {}: {err}",
                        path_for_log.display()
                    );
                    Vec::new()
                });
                let local_only = crate::code_review::timeline::fetch_local_only_shas(
                    &repo_for_upstream,
                    &path_for_upstream,
                    upstream_ref.as_deref(),
                )
                .await
                .unwrap_or_else(|err| {
                    log::warn!(
                        "[twarp 5d] timeline ahead-of-upstream fetch failed for {}: {err}",
                        path_for_upstream.display()
                    );
                    HashSet::new()
                });
                (path_token, entries, local_only)
            },
            |me, (path, mut entries, local_only), ctx| {
                if me.timeline_state.focused_path.as_deref() != Some(path.as_path()) {
                    // User switched files mid-fetch — discard.
                    return;
                }
                me.timeline_state.loading = false;
                me.timeline_state.local_only_shas = local_only;
                crate::code_review::timeline::mark_local_only(
                    &mut entries,
                    &me.timeline_state.local_only_shas,
                );
                me.timeline_state.has_more =
                    entries.len() == crate::code_review::timeline::TIMELINE_PAGE_SIZE;
                me.timeline_state.entries = entries;
                ctx.notify();
            },
        );
    }

    /// Fetch the next page of Timeline entries (PRODUCT §20). No-op
    /// while a fetch is in flight or no more pages exist.
    #[cfg(feature = "local_fs")]
    fn load_more_timeline(&mut self, ctx: &mut ViewContext<Self>) {
        if self.timeline_state.loading || !self.timeline_state.has_more {
            return;
        }
        let (Some(repo), Some(focused_abs)) = (
            self.timeline_state.repo_path.clone(),
            self.timeline_state.focused_path.clone(),
        ) else {
            return;
        };
        let Ok(repo_relative) = focused_abs.strip_prefix(&repo).map(Path::to_path_buf) else {
            return;
        };
        let offset = self.timeline_state.entries.len();
        self.timeline_state.loading = true;
        ctx.notify();

        let path_token = focused_abs.clone();
        ctx.spawn(
            async move {
                let entries = crate::code_review::timeline::fetch_log_page(
                    &repo,
                    &repo_relative,
                    offset,
                    crate::code_review::timeline::TIMELINE_PAGE_SIZE,
                )
                .await
                .unwrap_or_else(|err| {
                    log::warn!(
                        "[twarp 5d] timeline load-more failed for {} at offset {offset}: {err}",
                        repo_relative.display()
                    );
                    Vec::new()
                });
                (path_token, entries)
            },
            |me, (path, mut entries), ctx| {
                if me.timeline_state.focused_path.as_deref() != Some(path.as_path()) {
                    return;
                }
                me.timeline_state.loading = false;
                me.timeline_state.has_more =
                    entries.len() == crate::code_review::timeline::TIMELINE_PAGE_SIZE;
                crate::code_review::timeline::mark_local_only(
                    &mut entries,
                    &me.timeline_state.local_only_shas,
                );
                me.timeline_state.entries.extend(entries);
                ctx.notify();
            },
        );
    }

    /// Reset Timeline state when there's no focused file or no git repo
    /// (e.g. focus moved to a file outside any repo). Keeps the section
    /// header rendered with its empty-state hint.
    #[cfg(feature = "local_fs")]
    fn clear_timeline(&mut self, ctx: &mut ViewContext<Self>) {
        self.timeline_state = TimelineSectionState::default();
        self.timeline_entry_mouse_states.borrow_mut().clear();
        ctx.notify();
    }

    #[cfg(not(feature = "local_fs"))]
    fn clear_timeline(&mut self, _ctx: &mut ViewContext<Self>) {}

    /// Render the Project Explorer Timeline section (PRODUCT §§18–23).
    /// Collapsed → header bar only. Expanded → header + scrollable
    /// entry list wrapped in a `Resizable` so the section can be drag-
    /// resized from the bar between file tree and Timeline header.
    fn render_timeline_section(
        &self,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let header = self.render_timeline_header(appearance);
        if !self.timeline_expanded {
            return header;
        }
        let body = self.render_timeline_body(appearance);
        let combined = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Shrinkable::new(1.0, body).finish())
            .finish();
        Resizable::new(self.timeline_resizable_handle.clone(), combined)
            .with_dragbar_side(DragBarSide::Top)
            .with_bounds_callback(Box::new(|window_size| {
                (
                    80.0_f32.min(window_size.y()),
                    (window_size.y() * 0.7).max(80.0),
                )
            }))
            .on_resize(|ctx, _| {
                ctx.notify();
            })
            .finish()
    }

    /// Section header bar: chevron + `TIMELINE` label + optional
    /// focused-file basename. Click anywhere on the bar toggles
    /// expanded.
    fn render_timeline_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let chevron = if self.timeline_expanded { '▾' } else { '▸' };
        let label = match self.timeline_state.focused_path.as_ref() {
            Some(path) => format!(
                "{chevron} TIMELINE  ·  {}",
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            ),
            None => format!("{chevron} TIMELINE"),
        };
        let text = Text::new(
            label,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.sub_text_color(theme.surface_2()).into())
        .with_style(Properties::default().weight(Weight::Semibold))
        .soft_wrap(false)
        .finish();
        let hover_bg = internal_colors::neutral_3(theme);
        Hoverable::new(self.timeline_header_mouse_state.clone(), move |state| {
            let mut container = Container::new(text)
                .with_horizontal_padding(8.)
                .with_vertical_padding(6.)
                .with_margin_top(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if state.is_hovered() {
                container = container.with_background(warp_core::ui::theme::Fill::Solid(hover_bg));
            }
            container.finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(LeftPanelAction::TimelineToggleExpanded);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    /// Scrollable body for the expanded Timeline section. Entries
    /// render one row per commit, with `[Load more]` at the bottom
    /// when more pages exist.
    fn render_timeline_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_alignment(MainAxisAlignment::Start);

        if self.timeline_state.focused_path.is_none() {
            column.add_child(
                self.render_timeline_hint("Open a file to see its commit history.", appearance),
            );
        } else if self.timeline_state.entries.is_empty() {
            let msg = if self.timeline_state.loading {
                "Loading commits…"
            } else {
                "No commits found for this file."
            };
            column.add_child(self.render_timeline_hint(msg, appearance));
        } else {
            self.ensure_timeline_entry_mouse_states();
            for (idx, entry) in self.timeline_state.entries.iter().enumerate() {
                column.add_child(self.render_timeline_entry(idx, entry, appearance));
            }
            if self.timeline_state.has_more {
                column.add_child(
                    self.render_timeline_load_more(self.timeline_state.loading, appearance),
                );
            }
        }

        NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.timeline_scroll_state.clone(),
                child: column.finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .finish()
    }

    fn render_timeline_hint(&self, msg: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text = Text::new(
            msg.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.sub_text_color(theme.surface_2()).into())
        .soft_wrap(true)
        .finish();
        Container::new(text)
            .with_horizontal_padding(10.)
            .with_vertical_padding(10.)
            .finish()
    }

    /// Single Timeline row: avatar (Gravatar by email), author + relative
    /// time + markers on the first line, commit subject on the second.
    /// Whole row clickable; dispatches `TimelineSelectCommit` to open
    /// the read-only commit diff (PRODUCT §21).
    fn render_timeline_entry(
        &self,
        idx: usize,
        entry: &crate::code_review::timeline::TimelineEntry,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.surface_2());
        let sub_color = theme.sub_text_color(theme.surface_2());

        let avatar = self.render_timeline_avatar(entry, appearance);

        let ts_local = chrono::DateTime::<chrono::Utc>::from_timestamp(entry.timestamp, 0)
            .map(crate::util::time_format::format_approx_duration_from_now_utc)
            .unwrap_or_else(|| "unknown".to_string());

        let mut meta_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        meta_row.add_child(
            Text::new(
                entry.author_name.clone(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(main_color.into())
            .with_style(Properties::default().weight(Weight::Bold))
            .soft_wrap(false)
            .finish(),
        );
        meta_row.add_child(
            Container::new(
                Text::new(
                    format!("· {ts_local}"),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(sub_color.into())
                .soft_wrap(false)
                .finish(),
            )
            .with_margin_left(6.)
            .finish(),
        );
        if entry.is_local_only {
            meta_row.add_child(
                Container::new(
                    Text::new(
                        "↑".to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(sub_color.into())
                    .soft_wrap(false)
                    .finish(),
                )
                .with_margin_left(6.)
                .finish(),
            );
        }
        if entry.is_rename_commit {
            let badge_text = Text::new(
                "R".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size() * 0.85,
            )
            .with_color(main_color.into())
            .with_style(Properties::default().weight(Weight::Bold))
            .soft_wrap(false)
            .finish();
            meta_row.add_child(
                Container::new(
                    Container::new(badge_text)
                        .with_horizontal_padding(4.)
                        .with_vertical_padding(1.)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                        .with_border(Border::all(1.).with_border_fill(theme.outline()))
                        .finish(),
                )
                .with_margin_left(6.)
                .finish(),
            );
        }

        let subject = Text::new(
            entry.subject.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(sub_color.into())
        .soft_wrap(true)
        .finish();

        let mut text_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(meta_row.finish())
            .with_child(Container::new(subject).with_margin_top(2.).finish());
        if let Some(orig) = entry.original_path.as_ref() {
            let orig_text = Text::new(
                format!("was: {}", orig.display()),
                appearance.ui_font_family(),
                appearance.ui_font_size() * 0.85,
            )
            .with_color(sub_color.into())
            .soft_wrap(true)
            .finish();
            text_column =
                text_column.with_child(Container::new(orig_text).with_margin_top(1.).finish());
        }

        let mouse_state = self
            .timeline_entry_mouse_states
            .borrow()
            .get(idx)
            .cloned()
            .unwrap_or_default();
        let hover_bg = internal_colors::neutral_3(theme);
        let sha = entry.sha.clone();
        let repo_path = self.timeline_state.repo_path.clone();
        let file_path = self.timeline_state.focused_path.clone();
        Hoverable::new(mouse_state, move |state| {
            let inner = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(avatar)
                .with_child(Shrinkable::new(1., text_column.finish()).finish())
                .finish();
            let mut container = Container::new(inner)
                .with_vertical_padding(6.)
                .with_horizontal_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if state.is_hovered() {
                container = container.with_background(warp_core::ui::theme::Fill::Solid(hover_bg));
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            let (Some(repo), Some(file)) = (repo_path.clone(), file_path.clone()) else {
                return;
            };
            ctx.dispatch_typed_action(LeftPanelAction::TimelineSelectCommit {
                repo_path: repo,
                file_path: file,
                sha: sha.clone(),
            });
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    /// Avatar circle for a Timeline entry. Uses the `Avatar` widget
    /// with a Gravatar URL (SHA-256 of lowercased email +
    /// `?d=identicon` fallback so users without an account still get a
    /// deterministic visual). PRODUCT §19.
    fn render_timeline_avatar(
        &self,
        entry: &crate::code_review::timeline::TimelineEntry,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        use crate::ui_components::avatar::{Avatar, AvatarContent};
        let url = gravatar_url_for_email(&entry.author_email);
        let display_name = entry.author_name.clone();
        let theme = appearance.theme();
        let avatar = Avatar::new(
            AvatarContent::Image { url, display_name },
            UiComponentStyles {
                width: Some(22.),
                height: Some(22.),
                border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                font_family_id: Some(appearance.ui_font_family()),
                font_weight: Some(Weight::Bold),
                background: Some(theme.accent().into()),
                font_size: Some(11.),
                font_color: Some(pathfinder_color::ColorU::black()),
                ..Default::default()
            },
        );
        Container::new(avatar.build().finish())
            .with_margin_right(8.)
            .finish()
    }

    fn render_timeline_load_more(
        &self,
        loading: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let label = if loading { "Loading…" } else { "Load more" };
        let label_owned = label.to_string();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let color = if loading {
            theme.sub_text_color(theme.surface_2())
        } else {
            theme.accent()
        };
        if loading {
            let text = Text::new(label_owned, font_family, font_size)
                .with_color(color.into())
                .soft_wrap(false)
                .finish();
            return Container::new(text)
                .with_vertical_padding(6.)
                .with_horizontal_padding(8.)
                .finish();
        }
        let neutral = internal_colors::neutral_3(theme);
        let mouse_state = self.timeline_load_more_mouse_state.clone();
        Hoverable::new(mouse_state, move |state| {
            let text = Text::new(label_owned.clone(), font_family, font_size)
                .with_color(color.into())
                .soft_wrap(false)
                .finish();
            let mut container = Container::new(text)
                .with_vertical_padding(6.)
                .with_horizontal_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if state.is_hovered() {
                container = container.with_background(warp_core::ui::theme::Fill::Solid(neutral));
            }
            container.finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(LeftPanelAction::TimelineLoadMore);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    /// Grow the per-entry mouse-state vec on demand so each row's
    /// `Hoverable::on_click` sees the same handle across renders.
    fn ensure_timeline_entry_mouse_states(&self) {
        let needed = self.timeline_state.entries.len();
        let mut states = self.timeline_entry_mouse_states.borrow_mut();
        while states.len() < needed {
            states.push(MouseStateHandle::default());
        }
    }
}

/// Build a Gravatar avatar URL from an email address. Lowercased +
/// trimmed per Gravatar's documented hashing convention. SHA-256 is
/// the recommended modern hash; we add `d=identicon` so emails without
/// a Gravatar account still get a deterministic identicon, and `s=44`
/// for 2x the 22px slot. PRODUCT §19.
#[cfg(feature = "local_fs")]
fn gravatar_url_for_email(email: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = email.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("https://www.gravatar.com/avatar/{hex}?d=identicon&s=44")
}

#[cfg(not(feature = "local_fs"))]
fn gravatar_url_for_email(_email: &str) -> String {
    String::new()
}

impl Entity for LeftPanelView {
    type Event = LeftPanelEvent;
}

impl LeftPanelView {
    fn close_button(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder().clone();
        let tooltip_keybinding =
            keybinding_name_to_display_string("workspace:toggle_left_panel", app);

        let tooltip = if let Some(keybinding) = tooltip_keybinding {
            ui_builder
                .tool_tip_with_sublabel("Close panel".to_string(), keybinding)
                .build()
                .finish()
        } else {
            ui_builder
                .tool_tip("Close panel".to_string())
                .build()
                .finish()
        };

        let icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        icon_button_with_color(
            appearance,
            icons::Icon::X,
            false,
            self.close_button_mouse_state.clone(),
            icon_color,
        )
        .with_tooltip(move || tooltip)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleLeftPanel);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn update_button_active_states(&mut self) {
        for button in &mut self.toolbelt_buttons {
            button.render_with_active_state = match &button.action {
                LeftPanelAction::ProjectExplorer => {
                    self.active_view.get() == ToolPanelView::ProjectExplorer
                }
                LeftPanelAction::GlobalSearch { .. } => {
                    matches!(self.active_view.get(), ToolPanelView::GlobalSearch { .. })
                }
                LeftPanelAction::WarpDrive => self.active_view.get() == ToolPanelView::WarpDrive,
                LeftPanelAction::Shortcuts => self.active_view.get() == ToolPanelView::Shortcuts,
                LeftPanelAction::ShortcutsAddNew
                | LeftPanelAction::ShortcutsOpenInEditor
                | LeftPanelAction::ShortcutsToggleRowMenu(_)
                | LeftPanelAction::ShortcutsCloseRowMenu
                | LeftPanelAction::ShortcutsDelete(_)
                | LeftPanelAction::ShortcutsBeginEdit(_)
                | LeftPanelAction::ShortcutsEditCancel
                | LeftPanelAction::ShortcutsEditSave
                | LeftPanelAction::ShortcutsEditChordFieldClick
                | LeftPanelAction::ShortcutsEditChordCaptured(_)
                | LeftPanelAction::ShortcutsEditChordCancel
                | LeftPanelAction::ShortcutsEditActionCycleKind(_)
                | LeftPanelAction::ShortcutsEditActionCycleParam(_)
                | LeftPanelAction::ShortcutsEditActionAdd
                | LeftPanelAction::ShortcutsEditActionRemove(_)
                | LeftPanelAction::ShortcutsEditActionMoveUp(_)
                | LeftPanelAction::ShortcutsEditActionMoveDown(_) => false,
                // twarp: 2c-d — ConversationListView arm kept for legacy call-sites; AI deleted.
                LeftPanelAction::ConversationListView => false,
                // 5d: Timeline actions stay in the panel scope; no
                // force-open semantics. PRODUCT §§18–23.
                LeftPanelAction::TimelineToggleExpanded
                | LeftPanelAction::TimelineLoadMore
                | LeftPanelAction::TimelineSelectCommit { .. } => false,
            };
        }
    }

    /// One row in the Custom shortcuts list. Hoverable + clickable.
    /// PRODUCT 04 §§27, 30, 31.
    ///
    /// Layout: name on the left (truncated with `…` if too long for the
    /// panel width), styled chord pill on the right (separate boxed
    /// glyphs per modifier + key via `appearance.ui_builder().keyboard_shortcut(...)`).
    /// When `shortcut.name` is `None`, falls back to an arrow-form summary
    /// of the action sequence.
    ///
    /// - Left click on the row body → dispatch
    ///   `LeftPanelAction::ShortcutsOpenInEditor`, which opens
    ///   `shortcuts.yaml` in a new twarp tab (not the OS app) so users
    ///   stay inside the terminal.
    /// - Right click → toggle the inline Delete menu. Rename moved to
    ///   "left-click row" — the inline keystroke-capture editor is 4d.
    fn render_shortcut_row(
        &self,
        idx: usize,
        shortcut: &crate::shortcuts::config::Shortcut,
        menu_target: Option<usize>,
        delete_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let label_text = shortcut
            .name
            .clone()
            .unwrap_or_else(|| crate::shortcuts::summary::summarize_actions(&shortcut.actions));
        let truncated = truncate_display_name(&label_text, ROW_NAME_MAX_CHARS);
        let name_span = appearance.ui_builder().span(truncated).build().finish();
        let chord_pill = appearance
            .ui_builder()
            .keyboard_shortcut(&shortcut.keys)
            .with_space_between_keys(2.)
            .build()
            .finish();

        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1.0, name_span).finish())
            .with_child(chord_pill)
            .finish();

        let mut row_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.0)
            .with_child(header_row);

        if menu_target == Some(idx) {
            // PRODUCT §30 ([edit]) + §31 ([delete]). Edit opens the
            // inline detail editor for this row; Delete drops it from
            // the registry. The mouse state for [edit] is shared with
            // [delete] since only one menu is open at a time and reuse
            // simplifies the LeftPanelView state surface.
            let edit_link = appearance
                .ui_builder()
                .link(
                    "Edit".to_owned(),
                    None,
                    Some(Box::new(move |ctx| {
                        ctx.dispatch_typed_action(LeftPanelAction::ShortcutsBeginEdit(idx));
                    })),
                    delete_state.clone(),
                )
                .build()
                .finish();
            let delete_link = appearance
                .ui_builder()
                .link(
                    "Delete".to_owned(),
                    None,
                    Some(Box::new(move |ctx| {
                        ctx.dispatch_typed_action(LeftPanelAction::ShortcutsDelete(idx));
                    })),
                    delete_state,
                )
                .build()
                .finish();
            row_column = row_column.with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(12.0)
                    .with_child(edit_link)
                    .with_child(delete_link)
                    .finish(),
            );
        }

        let body: Box<dyn Element> = Container::new(row_column.finish())
            .with_padding_top(6.0)
            .with_padding_bottom(6.0)
            .finish();

        let row_mouse_state = {
            let mut states = self.shortcut_row_mouse_states.borrow_mut();
            while states.len() <= idx {
                states.push(MouseStateHandle::default());
            }
            states[idx].clone()
        };

        Hoverable::new(row_mouse_state, move |_state| body)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(LeftPanelAction::ShortcutsOpenInEditor);
            })
            .on_right_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(LeftPanelAction::ShortcutsToggleRowMenu(idx));
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    /// Renders the Custom shortcuts panel content: header with
    /// "+ New shortcut" link + a row per registered shortcut showing
    /// chord + arrow-form summary, OR an empty-state hint if the
    /// registry is empty. PRODUCT 04 §§27-29.
    fn render_shortcuts_panel(&self, app: &AppContext) -> Box<dyn Element> {
        // PRODUCT §§29-30, 32-38: when the detail editor is open the
        // panel becomes the editor; the list returns once Save/Cancel
        // takes us back to `editing_shortcut = None`.
        if self.editing_shortcut.is_some() {
            return self.render_shortcuts_editor(app);
        }
        let appearance = Appearance::as_ref(app);
        let (registry, errors) = {
            let model = crate::shortcuts::ShortcutsModel::handle(app);
            let m = model.as_ref(app);
            (m.registry.clone(), m.errors.clone())
        };

        let add_new_link = appearance
            .ui_builder()
            .link(
                "+ New shortcut".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsAddNew);
                })),
                self.mouse_state_handles.add_new_shortcut_button.clone(),
            )
            .build()
            .finish();

        // Errors banner: PRODUCT 04 §35. Shown whenever parsing produced
        // any errors so users see that some entries in `shortcuts.yaml`
        // were dropped, with the offending message inline. Without this
        // the panel just silently misses entries (which is what the user
        // saw with `new_pane: left`).
        let errors_banner: Option<Box<dyn Element>> = if errors.is_empty() {
            None
        } else {
            let mut col = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(4.0);
            col = col.with_child(
                appearance
                    .ui_builder()
                    .span(format!("shortcuts.yaml has {} error(s):", errors.len()))
                    .build()
                    .finish(),
            );
            for err in &errors {
                col = col.with_child(
                    appearance
                        .ui_builder()
                        .span(err.clone())
                        .with_soft_wrap()
                        .build()
                        .finish(),
                );
            }
            Some(
                Container::new(col.finish())
                    .with_padding_top(6.0)
                    .with_padding_bottom(6.0)
                    .finish(),
            )
        };

        let body: Box<dyn Element> = if registry.is_empty() {
            appearance
                .ui_builder()
                .span(
                    "Custom shortcuts run a sequence of terminal actions when you press a chord. \
                     Click \"+ New shortcut\" to add one, then edit `shortcuts.yaml` to customize.",
                )
                .with_soft_wrap()
                .build()
                .finish()
        } else {
            let menu_target = self.shortcut_context_menu_target;
            let delete_state = self.shortcut_delete_mouse_state.clone();
            let rows = registry.iter().enumerate().map(|(idx, sc)| {
                self.render_shortcut_row(idx, sc, menu_target, delete_state.clone(), appearance)
            });
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(6.0)
                .with_children(rows)
                .finish()
        };

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(10.0)
            .with_child(add_new_link);
        if let Some(banner) = errors_banner {
            column = column.with_child(banner);
        }
        let column = column
            .with_child(Shrinkable::new(1.0, body).finish())
            .finish();

        Shrinkable::new(
            1.0,
            Container::new(column)
                .with_padding_left(10.)
                .with_padding_right(10.)
                .with_padding_top(8.)
                .finish(),
        )
        .finish()
    }

    /// PRODUCT §§29-30, 32-38: the inline detail editor. Renders in
    /// place of the list view while `editing_shortcut.is_some()`.
    ///
    /// Layout (top to bottom): header row with Cancel + Save, name
    /// field, chord field, validation banner (if any), action list
    /// (one row per action), [+ Add action] button.
    fn render_shortcuts_editor(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let state = self
            .editing_shortcut
            .as_ref()
            .expect("render_shortcuts_editor called without editor state");

        let header_text = match state.target {
            EditTarget::Create => "New shortcut",
            EditTarget::Index(_) => "Edit shortcut",
        };
        let header_label = appearance
            .ui_builder()
            .span(header_text.to_owned())
            .build()
            .finish();
        let cancel_link = appearance
            .ui_builder()
            .link(
                "Cancel".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditCancel);
                })),
                state.cancel_button_mouse.clone(),
            )
            .build()
            .finish();
        let save_link = appearance
            .ui_builder()
            .link(
                "Save".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditSave);
                })),
                state.save_button_mouse.clone(),
            )
            .build()
            .finish();
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(header_label)
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(12.0)
                    .with_child(cancel_link)
                    .with_child(save_link)
                    .finish(),
            )
            .finish();

        // Name field — a `ClickableTextInput` child view sits in a
        // labelled row.
        let name_label = appearance
            .ui_builder()
            .span("Name".to_owned())
            .build()
            .finish();
        let name_input_view = Container::new(ChildView::new(&state.name_text_input).finish())
            .with_padding_top(2.0)
            .with_padding_bottom(2.0)
            .finish();
        let name_row = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.0)
            .with_child(name_label)
            .with_child(name_input_view)
            .finish();

        // Chord field — a clickable line that toggles capture mode.
        let chord_text = if state.capturing_chord {
            "Press a chord (Esc to cancel)…".to_owned()
        } else if let Some(chord) = &state.chord {
            chord.normalized()
        } else {
            "(no chord set — click to capture)".to_owned()
        };
        let chord_pill = appearance.ui_builder().span(chord_text).build().finish();
        let chord_field_clickable = Hoverable::new(state.chord_field_mouse.clone(), |_| {
            Container::new(chord_pill)
                .with_padding_top(4.0)
                .with_padding_bottom(4.0)
                .with_padding_left(8.0)
                .with_padding_right(8.0)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditChordFieldClick);
        })
        .finish();
        // While in capture mode, wrap the chord field in an EventHandler
        // that intercepts the next keydown event and turns it into a
        // typed action (PRODUCT §32). This is the same hook the
        // keybindings settings page uses for chord rebinding.
        let chord_field: Box<dyn Element> = if state.capturing_chord {
            EventHandler::new(chord_field_clickable)
                .on_keydown(|ctx, _, keystroke| {
                    // Bare Escape exits capture without committing
                    // (PRODUCT §32 last sentence). Anything else is
                    // committed as the new chord.
                    if keystroke.key == "escape"
                        && !keystroke.cmd
                        && !keystroke.ctrl
                        && !keystroke.alt
                        && !keystroke.shift
                        && !keystroke.meta
                    {
                        ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditChordCancel);
                    } else {
                        ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditChordCaptured(
                            keystroke.clone(),
                        ));
                    }
                    DispatchEventResult::StopPropagation
                })
                .finish()
        } else {
            chord_field_clickable
        };
        let chord_label = appearance
            .ui_builder()
            .span("Chord".to_owned())
            .build()
            .finish();
        let chord_row = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.0)
            .with_child(chord_label)
            .with_child(chord_field)
            .finish();

        // Validation error banner (PRODUCT §34/§36).
        let error_banner: Option<Box<dyn Element>> = state.validation_error.as_ref().map(|msg| {
            appearance
                .ui_builder()
                .span(msg.clone())
                .with_soft_wrap()
                .build()
                .finish()
        });

        // Action editor: one row per action, plus [+ Add action].
        let actions_label = appearance
            .ui_builder()
            .span("Actions".to_owned())
            .build()
            .finish();
        let mut action_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(6.0)
            .with_child(actions_label);
        for (idx, row) in state.actions.iter().enumerate() {
            action_column = action_column.with_child(self.render_action_row(idx, row, appearance));
        }
        let add_action_link = appearance
            .ui_builder()
            .link(
                "+ Add action".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditActionAdd);
                })),
                state.add_action_button_mouse.clone(),
            )
            .build()
            .finish();
        action_column = action_column.with_child(add_action_link);
        let action_section = action_column.finish();

        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(10.0)
            .with_child(header)
            .with_child(name_row)
            .with_child(chord_row);
        if let Some(banner) = error_banner {
            column = column.with_child(banner);
        }
        column = column.with_child(action_section);
        let column = column.finish();

        Shrinkable::new(
            1.0,
            Container::new(column)
                .with_padding_left(10.)
                .with_padding_right(10.)
                .with_padding_top(8.)
                .finish(),
        )
        .finish()
    }

    /// Render one row of the action editor (PRODUCT §33). Layout:
    /// `[kind cycle button] [param widget] [↑] [↓] [×]`. The param
    /// widget shape depends on the kind: free-text rows embed the
    /// row's `ClickableTextInput`; enum rows show a cycle button; the
    /// `new_tab` row shows a dash.
    fn render_action_row(
        &self,
        idx: usize,
        row: &EditingAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let kind_label = format!("type: {}", row.kind.label());
        let kind_button = appearance
            .ui_builder()
            .link(
                kind_label,
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditActionCycleKind(idx));
                })),
                row.kind_button_mouse.clone(),
            )
            .build()
            .finish();

        let param_element: Box<dyn Element> = match row.kind {
            EditingActionKind::NewTab => appearance
                .ui_builder()
                .span("(no parameter)".to_owned())
                .build()
                .finish(),
            EditingActionKind::NewPane => {
                let dir = NEW_PANE_DIRECTIONS
                    .get(row.param_cycle_idx)
                    .copied()
                    .unwrap_or("right");
                appearance
                    .ui_builder()
                    .link(
                        format!("direction: {dir}"),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(
                                LeftPanelAction::ShortcutsEditActionCycleParam(idx),
                            );
                        })),
                        row.param_button_mouse.clone(),
                    )
                    .build()
                    .finish()
            }
            EditingActionKind::Press => {
                let key = PRESS_KEY_CYCLE
                    .get(row.param_cycle_idx)
                    .copied()
                    .unwrap_or("enter");
                appearance
                    .ui_builder()
                    .link(
                        format!("key: {key}"),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(
                                LeftPanelAction::ShortcutsEditActionCycleParam(idx),
                            );
                        })),
                        row.param_button_mouse.clone(),
                    )
                    .build()
                    .finish()
            }
            EditingActionKind::Type | EditingActionKind::Wait => {
                if let Some(input) = &row.param_text_input {
                    Container::new(ChildView::new(input).finish())
                        .with_padding_top(2.0)
                        .with_padding_bottom(2.0)
                        .finish()
                } else {
                    appearance
                        .ui_builder()
                        .span("(click to edit)".to_owned())
                        .build()
                        .finish()
                }
            }
        };

        let move_up_link = appearance
            .ui_builder()
            .link(
                "↑".to_owned(),
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditActionMoveUp(idx));
                })),
                row.move_up_mouse.clone(),
            )
            .build()
            .finish();
        let move_down_link = appearance
            .ui_builder()
            .link(
                "↓".to_owned(),
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditActionMoveDown(idx));
                })),
                row.move_down_mouse.clone(),
            )
            .build()
            .finish();
        let remove_link = appearance
            .ui_builder()
            .link(
                "×".to_owned(),
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(LeftPanelAction::ShortcutsEditActionRemove(idx));
                })),
                row.remove_button_mouse.clone(),
            )
            .build()
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(kind_button)
            .with_child(Shrinkable::new(1.0, param_element).finish())
            .with_child(move_up_link)
            .with_child(move_down_link)
            .with_child(remove_link)
            .finish()
    }

    fn render_button(
        button_config: &ToolbeltButtonConfig,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let action = button_config.action.clone();
        let ui_builder = appearance.ui_builder().clone();
        let tooltip_keybinding = button_config.tooltip_keybinding.clone();

        let icon_color = if button_config.render_with_active_state {
            appearance.theme().foreground().into_solid()
        } else {
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into_solid()
        };

        let tooltip = if let Some(keybinding) = tooltip_keybinding {
            ui_builder
                .tool_tip_with_sublabel(button_config.tooltip_text.clone(), keybinding)
                .build()
                .finish()
        } else {
            ui_builder
                .tool_tip(button_config.tooltip_text.clone())
                .build()
                .finish()
        };

        let icon = if button_config.render_with_active_state {
            button_config.active_icon.unwrap_or(button_config.icon)
        } else {
            button_config.icon
        };

        icon_button(
            appearance,
            icon,
            button_config.render_with_active_state,
            mouse_state.clone(),
        )
        .with_tooltip(move || tooltip)
        .with_style(UiComponentStyles {
            font_color: Some(icon_color),
            height: Some(24.),
            width: Some(24.),
            padding: Some(Coords::uniform(4.)),
            ..Default::default()
        })
        .with_active_styles(UiComponentStyles {
            font_color: Some(icon_color),
            height: Some(24.),
            width: Some(24.),
            padding: Some(Coords::uniform(4.)),
            background: Some(internal_colors::fg_overlay_3(appearance.theme()).into()),
            ..Default::default()
        })
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

impl LeftPanelView {
    pub fn handle_action_with_force_open(
        &mut self,
        action: &LeftPanelAction,
        force_open: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            LeftPanelAction::ProjectExplorer => {
                active_view_state::set(self, ToolPanelView::ProjectExplorer, ctx);
                if force_open {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::FileTreeToggled {
                            source: FileTreeSource::ForceOpened,
                            is_code_mode_v2: true,
                            cli_agent: None,
                        },
                        ctx
                    );
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::FileTreeToggled {
                            source: FileTreeSource::LeftPanelToolbelt,
                            is_code_mode_v2: true,
                            cli_agent: None,
                        },
                        ctx
                    );
                }
            }
            LeftPanelAction::GlobalSearch { entry_focus } => {
                let was_active = self.active_view.get()
                    == ToolPanelView::GlobalSearch {
                        entry_focus: *entry_focus,
                    };
                active_view_state::set(
                    self,
                    ToolPanelView::GlobalSearch {
                        entry_focus: *entry_focus,
                    },
                    ctx,
                );
                if !was_active {
                    send_telemetry_from_ctx!(TelemetryEvent::GlobalSearchOpened, ctx);
                }
            }
            LeftPanelAction::WarpDrive => {
                active_view_state::set(self, ToolPanelView::WarpDrive, ctx);
                if force_open {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::WarpDriveOpened {
                            source: WarpDriveSource::ForceOpened,
                            is_code_mode_v2: true
                        },
                        ctx
                    );
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::WarpDriveOpened {
                            source: WarpDriveSource::LeftPanelToolbelt,
                            is_code_mode_v2: true
                        },
                        ctx
                    );
                }
            }
            LeftPanelAction::Shortcuts => {
                active_view_state::set(self, ToolPanelView::Shortcuts, ctx);
            }
            LeftPanelAction::ShortcutsAddNew => {
                // PRODUCT §29: opens the empty detail editor.
                self.shortcut_context_menu_target = None;
                self.editing_shortcut = Some(Self::new_editing_state(EditTarget::Create, ctx));
                ctx.notify();
            }
            LeftPanelAction::ShortcutsBeginEdit(idx) => {
                self.shortcut_context_menu_target = None;
                if let Some(state) = Self::editing_state_for_existing(*idx, ctx) {
                    self.editing_shortcut = Some(state);
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsEditCancel => {
                if self.editing_shortcut.is_some() {
                    self.editing_shortcut = None;
                    // Capture mode is one-shot; make sure global dispatch
                    // is enabled in case we cancel mid-capture.
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsEditSave => {
                self.handle_edit_save(ctx);
            }
            LeftPanelAction::ShortcutsEditChordFieldClick => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if state.capturing_chord {
                        // Click again to bail out of capture mode without
                        // changing anything (PRODUCT §32 doesn't strictly
                        // require this — but it avoids trap states if a
                        // user changes their mind mid-capture).
                        state.capturing_chord = false;
                        ctx.enable_key_bindings_dispatching();
                    } else {
                        state.chord_before_capture = state.chord.clone();
                        state.capturing_chord = true;
                        ctx.disable_key_bindings_dispatching();
                    }
                    state.validation_error = None;
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsEditChordCaptured(keystroke) => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    state.chord = Some(keystroke.clone());
                    state.capturing_chord = false;
                    state.validation_error = None;
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsEditChordCancel => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    state.chord = state.chord_before_capture.clone();
                    state.capturing_chord = false;
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsEditActionCycleKind(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if let Some(row) = state.actions.get_mut(idx) {
                        row.kind = row.kind.next();
                        // Reset parameter representation to the new kind's defaults.
                        row.param_text.clear();
                        row.param_cycle_idx = 0;
                        row.param_text_input = None;
                        state.validation_error = None;
                        Self::ensure_param_input_for_row(row, ctx);
                        ctx.notify();
                    }
                }
            }
            LeftPanelAction::ShortcutsEditActionCycleParam(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if let Some(row) = state.actions.get_mut(idx) {
                        let modulus = match row.kind {
                            EditingActionKind::NewPane => NEW_PANE_DIRECTIONS.len(),
                            EditingActionKind::Press => PRESS_KEY_CYCLE.len(),
                            _ => 0,
                        };
                        if modulus > 0 {
                            row.param_cycle_idx = (row.param_cycle_idx + 1) % modulus;
                            state.validation_error = None;
                            ctx.notify();
                        }
                    }
                }
            }
            LeftPanelAction::ShortcutsEditActionAdd => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    let mut row = EditingAction::new_default();
                    Self::ensure_param_input_for_row(&mut row, ctx);
                    state.actions.push(row);
                    state.validation_error = None;
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsEditActionRemove(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if idx < state.actions.len() {
                        state.actions.remove(idx);
                        state.validation_error = None;
                        ctx.notify();
                    }
                }
            }
            LeftPanelAction::ShortcutsEditActionMoveUp(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if idx > 0 && idx < state.actions.len() {
                        state.actions.swap(idx, idx - 1);
                        state.validation_error = None;
                        ctx.notify();
                    }
                }
            }
            LeftPanelAction::ShortcutsEditActionMoveDown(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if idx + 1 < state.actions.len() {
                        state.actions.swap(idx, idx + 1);
                        state.validation_error = None;
                        ctx.notify();
                    }
                }
            }
            LeftPanelAction::ShortcutsOpenInEditor => {
                self.shortcut_context_menu_target = None;
                let full_path = crate::shortcuts::shortcuts_file_path();
                // If `shortcuts.yaml` already has an open code pane in
                // some tab, activate that tab instead of creating a
                // duplicate (user request). Falls back to
                // `OpenFileInNewTab` when no existing pane is found.
                let existing = crate::code::editor_management::CodeManager::handle(ctx)
                    .read(ctx, |manager, _ctx| {
                        manager.get_locator_for_path_anywhere(&full_path)
                    });
                if let Some(locator) = existing {
                    log::info!(
                        "shortcuts: shortcuts.yaml already open at locator {locator:?}; activating tab"
                    );
                    ctx.dispatch_typed_action_deferred(WorkspaceAction::FocusPane(locator));
                } else {
                    log::info!("shortcuts: opening {full_path:?} in a new twarp tab");
                    // Defer the workspace-action dispatch — we're currently
                    // inside our own handle_action, and dispatching another
                    // typed action synchronously re-enters the view update
                    // machinery and panics ("Circular view update"). The
                    // deferred path queues the action onto `pending_effects`
                    // so it fires after this handler returns.
                    ctx.dispatch_typed_action_deferred(WorkspaceAction::OpenFileInNewTab {
                        full_path,
                        line_and_column: None,
                    });
                }
                ctx.notify();
            }
            LeftPanelAction::ShortcutsToggleRowMenu(index) => {
                self.shortcut_context_menu_target =
                    if self.shortcut_context_menu_target == Some(*index) {
                        None
                    } else {
                        Some(*index)
                    };
                ctx.notify();
            }
            LeftPanelAction::ShortcutsCloseRowMenu => {
                if self.shortcut_context_menu_target.is_some() {
                    self.shortcut_context_menu_target = None;
                    ctx.notify();
                }
            }
            LeftPanelAction::ShortcutsDelete(index) => {
                let index = *index;
                self.shortcut_context_menu_target = None;
                Self::delete_shortcut_at_index(index, ctx);
            }
            // twarp: 2c-d — ConversationListView is a stub kept for legacy call-sites.
            LeftPanelAction::ConversationListView => {}
            LeftPanelAction::TimelineToggleExpanded => {
                self.timeline_expanded = !self.timeline_expanded;
                ctx.notify();
            }
            LeftPanelAction::TimelineLoadMore => {
                #[cfg(feature = "local_fs")]
                self.load_more_timeline(ctx);
            }
            LeftPanelAction::TimelineSelectCommit {
                repo_path,
                file_path,
                sha,
            } => {
                ctx.emit(LeftPanelEvent::OpenTimelineCommitDiff {
                    repo_path: repo_path.clone(),
                    file_path: file_path.clone(),
                    sha: sha.clone(),
                });
            }
        }
    }

    /// Construct the initial state for a brand-new shortcut. Spawns
    /// the name-field `ClickableTextInput` child view; per-action-row
    /// inputs are spawned lazily as actions are added.
    fn new_editing_state(target: EditTarget, ctx: &mut ViewContext<Self>) -> EditingShortcutState {
        let name_input = Self::make_text_input(String::new(), ctx);
        EditingShortcutState {
            target,
            name_text: String::new(),
            name_text_input: name_input,
            chord: None,
            capturing_chord: false,
            chord_before_capture: None,
            chord_field_mouse: MouseStateHandle::default(),
            actions: Vec::new(),
            add_action_button_mouse: MouseStateHandle::default(),
            save_button_mouse: MouseStateHandle::default(),
            cancel_button_mouse: MouseStateHandle::default(),
            validation_error: None,
        }
    }

    /// Construct editor state pre-populated from the row at `index` in
    /// the current registry. Returns `None` when the index is out of
    /// range (race with reload).
    fn editing_state_for_existing(
        index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Option<EditingShortcutState> {
        let shortcut = {
            let model = crate::shortcuts::ShortcutsModel::handle(ctx);
            let m = model.as_ref(ctx);
            m.registry.get(index).cloned()
        }?;

        let name_text = shortcut.name.clone().unwrap_or_default();
        let name_input = Self::make_text_input(name_text.clone(), ctx);

        let actions = shortcut
            .actions
            .iter()
            .map(|a| {
                let mut row = EditingAction::from_runtime(a);
                Self::ensure_param_input_for_row(&mut row, ctx);
                row
            })
            .collect();

        Some(EditingShortcutState {
            target: EditTarget::Index(index),
            name_text,
            name_text_input: name_input,
            chord: Some(shortcut.keys.clone()),
            capturing_chord: false,
            chord_before_capture: None,
            chord_field_mouse: MouseStateHandle::default(),
            actions,
            add_action_button_mouse: MouseStateHandle::default(),
            save_button_mouse: MouseStateHandle::default(),
            cancel_button_mouse: MouseStateHandle::default(),
            validation_error: None,
        })
    }

    /// Spawn a `ClickableTextInput` child view pre-populated with the
    /// initial text. Subscribes to its Submit event so edits route
    /// back to the editor's typed-action surface — when the input
    /// emits a Submit, we look up which slot it belonged to in the
    /// current editor state and update `param_text` / `name_text`.
    fn make_text_input(
        initial: String,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<ClickableTextInput> {
        let handle = ctx
            .add_typed_action_view(|child_ctx| ClickableTextInput::new(initial.clone(), child_ctx));
        let weak = handle.downgrade();
        ctx.subscribe_to_view(&handle, move |me, _, event, ctx| {
            let ClickableTextInputEvent::Submit(text) = event;
            // Resolve which slot this submit came from by comparing
            // ViewHandle identity inside the current editor state, then
            // push the new text back into the input's `text` field so
            // its display-mode label shows the saved value. Without
            // this UpdateText round-trip the input would emit Submit,
            // flip back to display mode, and render the empty `text`
            // it was created with — values appear to vanish even
            // though they did make it into our state and onto disk.
            if let Some(state) = me.editing_shortcut.as_mut() {
                if let Some(other) = weak.upgrade(ctx) {
                    if state.name_text_input == other {
                        state.name_text = text.clone();
                        state.validation_error = None;
                        let new_text = text.clone();
                        state.name_text_input.update(ctx, |input, ctx| {
                            input.handle_action(
                                &ClickableTextInputAction::UpdateText(new_text),
                                ctx,
                            );
                        });
                        ctx.notify();
                        return;
                    }
                    let updated_param: Option<(ViewHandle<ClickableTextInput>, String)> =
                        state.actions.iter_mut().find_map(|row| {
                            row.param_text_input.as_ref().and_then(|input| {
                                if *input == other {
                                    row.param_text = text.clone();
                                    Some((input.clone(), text.clone()))
                                } else {
                                    None
                                }
                            })
                        });
                    if let Some((input, new_text)) = updated_param {
                        state.validation_error = None;
                        input.update(ctx, |input_view, ctx| {
                            input_view.handle_action(
                                &ClickableTextInputAction::UpdateText(new_text),
                                ctx,
                            );
                        });
                        ctx.notify();
                    }
                }
            }
        });
        handle
    }

    /// Ensure that a row whose kind takes free-text input has a backing
    /// `ClickableTextInput`, and that rows whose kind doesn't take
    /// free-text input have `None`. Called whenever the row's kind
    /// changes or a row is added/loaded.
    fn ensure_param_input_for_row(row: &mut EditingAction, ctx: &mut ViewContext<Self>) {
        if kind_param_is_freetext(row.kind) {
            if row.param_text_input.is_none() {
                let input = Self::make_text_input(row.param_text.clone(), ctx);
                row.param_text_input = Some(input);
            }
        } else {
            row.param_text_input = None;
        }
    }

    /// PRODUCT §36 save semantics: validate the editor against the same
    /// parser the file path uses, build a `Shortcut`, splice it into
    /// the current registry snapshot, write to disk, and reload. On
    /// failure surface the error inline so the user can fix it.
    fn handle_edit_save(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(state) = self.editing_shortcut.as_mut() else {
            return;
        };
        if state.capturing_chord {
            // Refuse to save mid-capture; we don't have a chord yet.
            state.validation_error = Some("Press a chord (or Escape) before saving.".to_owned());
            ctx.notify();
            return;
        }
        let Some(chord) = state.chord.clone() else {
            state.validation_error = Some("Pick a chord before saving.".to_owned());
            ctx.notify();
            return;
        };
        if state.actions.is_empty() {
            state.validation_error = Some("Add at least one action before saving.".to_owned());
            ctx.notify();
            return;
        }
        let runtime_actions: Result<Vec<crate::shortcuts::action::Action>, String> = state
            .actions
            .iter()
            .enumerate()
            .map(|(idx, row)| row.to_runtime_action(idx + 1))
            .collect();
        let runtime_actions = match runtime_actions {
            Ok(actions) => actions,
            Err(msg) => {
                state.validation_error = Some(msg);
                ctx.notify();
                return;
            }
        };
        let trimmed_name = state.name_text.trim();
        let name = if trimmed_name.is_empty() {
            None
        } else {
            Some(trimmed_name.to_owned())
        };
        let target = state.target;

        let mut snapshot: Vec<crate::shortcuts::config::Shortcut> =
            crate::shortcuts::ShortcutsModel::handle(ctx)
                .as_ref(ctx)
                .registry
                .clone();
        let new_shortcut = crate::shortcuts::config::Shortcut {
            keys: chord,
            actions: runtime_actions,
            name,
            binding_name: String::new(), // assigned on reload
        };
        match target {
            EditTarget::Create => snapshot.push(new_shortcut),
            EditTarget::Index(idx) => {
                if idx < snapshot.len() {
                    snapshot[idx] = new_shortcut;
                } else {
                    // The row we were editing was deleted out from under us
                    // by a concurrent reload; degrade to an append rather
                    // than dropping the edit.
                    snapshot.push(new_shortcut);
                }
            }
        }
        match crate::shortcuts::save::save_to_disk(&snapshot) {
            Ok(path) => {
                log::info!(
                    "shortcuts: detail editor saved to {path:?} ({entries} entr{plural})",
                    entries = snapshot.len(),
                    plural = if snapshot.len() == 1 { "y" } else { "ies" }
                );
                self.editing_shortcut = None;
                ctx.enable_key_bindings_dispatching();
                crate::shortcuts::reload(ctx);
                ctx.notify();
            }
            Err(err) => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    state.validation_error = Some(format!("Failed to write shortcuts.yaml: {err}"));
                }
                ctx.notify();
            }
        }
    }

    /// PRODUCT 04 §31 (delete). Removes the shortcut at `index` from the
    /// in-memory registry snapshot, writes the result via 4b's
    /// `save_to_disk`, and triggers a reload so the keymap drops the
    /// binding immediately. Out-of-range indices are a no-op.
    fn delete_shortcut_at_index(index: usize, ctx: &mut ViewContext<Self>) {
        let mut snapshot: Vec<crate::shortcuts::config::Shortcut> =
            crate::shortcuts::ShortcutsModel::handle(ctx)
                .as_ref(ctx)
                .registry
                .clone();
        if index >= snapshot.len() {
            log::warn!(
                "shortcuts: delete requested for out-of-range index {index} (registry has {len})",
                len = snapshot.len()
            );
            return;
        }
        let removed = snapshot.remove(index);
        match crate::shortcuts::save::save_to_disk(&snapshot) {
            Ok(path) => {
                log::info!(
                    "shortcuts: deleted shortcut #{index} ({chord}) and saved to {:?}",
                    path,
                    chord = removed.keys.normalized()
                );
                crate::shortcuts::reload(ctx);
            }
            Err(err) => {
                log::warn!("shortcuts: failed to save after delete: {err}");
            }
        }
    }

    pub fn on_left_panel_visibility_changed(
        &self,
        _is_now_open: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // twarp: 2c-d — conversation list visibility tracking removed

        self.update_active_file_tree_subscription_state(ctx);
    }

    fn deactivate_file_tree_view_for_pane_group(
        &self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_file_tree_view(pane_group_id)
        {
            view.update(ctx, |view, ctx| {
                view.set_is_active(false, ctx);
            });
        }
    }

    fn update_active_file_tree_subscription_state(&self, ctx: &mut ViewContext<Self>) {
        let Some(active_pane_group) = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(ctx))
        else {
            return;
        };

        let is_visible = active_pane_group.as_ref(ctx).left_panel_open
            && self.active_view.get() == ToolPanelView::ProjectExplorer;

        if let Some(file_tree_view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_file_tree_view(active_pane_group.id())
        {
            file_tree_view.update(ctx, |view, ctx| {
                view.set_is_active(is_visible, ctx);
            });
        }
    }

    // twarp: 2c-d — on_conversation_list_view_visibility_changed removed
}

impl TypedActionView for LeftPanelView {
    type Action = LeftPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        self.handle_action_with_force_open(action, false, ctx);
    }
}

impl View for LeftPanelView {
    fn ui_name() -> &'static str {
        "LeftPanelView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // Focus the active tool panel view on-left-panel-focus.
        if focus_ctx.is_self_focused() {
            match self.active_view.get() {
                ToolPanelView::ProjectExplorer => {
                    if let Some(view) = self.active_file_tree_view(ctx) {
                        ctx.focus(&view);
                    }
                }
                ToolPanelView::GlobalSearch { .. } => {
                    if let Some(view) = self.active_global_search_view(ctx) {
                        ctx.focus(&view);
                    }
                }
                ToolPanelView::WarpDrive => ctx.focus(&self.warp_drive_view),
                // 4c stub: no internal view to focus yet.
                ToolPanelView::Shortcuts => {}
                // twarp: 2c-d — ConversationListView arm: AI deleted, no-op.
                ToolPanelView::ConversationListView => {}
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mouse_state_handles = vec![
            self.mouse_state_handles.project_explorer_button.clone(),
            self.mouse_state_handles.global_search_button.clone(),
            self.mouse_state_handles.warp_drive_button.clone(),
            self.mouse_state_handles.shortcuts_button.clone(),
            // twarp: 2c-d — conversation_list_view_button removed
        ];

        // If there is only one button in the toolbelt row,
        // there is no need to show it as it's a bit redundant.
        let toolbelt_button_row = if self.toolbelt_buttons.len() > 1 {
            Some(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.0)
                    .with_children(self.toolbelt_buttons.iter().zip(&mouse_state_handles).map(
                        |(button_config, mouse_state)| {
                            Self::render_button(button_config, mouse_state.clone(), appearance)
                        },
                    ))
                    .with_main_axis_size(MainAxisSize::Min)
                    .finish(),
            )
        } else {
            None
        };

        let content_area: Box<dyn Element> = match self.active_view.get() {
            ToolPanelView::ProjectExplorer => {
                // 5d (PRODUCT §§18–23): Project Explorer is now a
                // vertical stack — file tree on top, collapsible
                // Timeline section pinned to the bottom. The two
                // sections scroll independently (each is its own
                // scroll surface). When the Timeline is expanded, the
                // drag bar between them lets the user resize.
                let file_tree_element: Box<dyn Element> =
                    if let Some(file_tree_view) = self.active_file_tree_view(app) {
                        Container::new(ChildView::new(&file_tree_view).finish())
                            .with_padding_left(2.)
                            .with_padding_right(2.)
                            .finish()
                    } else {
                        Container::new(Empty::new().finish()).finish()
                    };
                let timeline_element = self.render_timeline_section(appearance, app);
                let stacked = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(Shrinkable::new(1.0, file_tree_element).finish())
                    .with_child(timeline_element)
                    .finish();
                Shrinkable::new(1.0, Container::new(stacked).finish()).finish()
            }
            ToolPanelView::GlobalSearch { .. } => {
                if let Some(global_search_view) = self.active_global_search_view(app) {
                    Shrinkable::new(
                        1.0,
                        Container::new(ChildView::new(&global_search_view).finish()).finish(),
                    )
                    .finish()
                } else {
                    Shrinkable::new(1.0, Container::new(Empty::new().finish()).finish()).finish()
                }
            }
            ToolPanelView::WarpDrive => Shrinkable::new(
                1.0,
                Container::new(ChildView::new(&self.warp_drive_view).finish())
                    .with_padding_left(2.)
                    .with_padding_right(2.)
                    .finish(),
            )
            .finish(),
            // PRODUCT 04 §§27-29: read-only list of shortcuts + "+ New
            // shortcut" link. Inline editing (keystroke capture, action
            // editor, validation) is deferred to 4d; users hand-edit
            // `shortcuts.yaml` for now, and 4b's hot reload keeps that
            // loop tight.
            ToolPanelView::Shortcuts => self.render_shortcuts_panel(app),
            // twarp: 2c-d — ConversationListView arm: AI deleted, use empty content.
            ToolPanelView::ConversationListView => {
                Shrinkable::new(1.0, Container::new(Empty::new().finish()).finish()).finish()
            }
        };

        let panel_content = Container::new({
            let column = Flex::column();

            let header_left = if let Some(row) = toolbelt_button_row {
                row
            } else {
                Flex::row().finish()
            };

            let header_row = Container::new(
                ConstrainedBox::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Shrinkable::new(1.0, header_left).finish())
                        .with_child(self.close_button(appearance, app))
                        .finish(),
                )
                .with_height(PANE_HEADER_HEIGHT)
                .finish(),
            )
            .with_padding_left(10.)
            .with_padding_right(HEADER_EDGE_PADDING)
            .finish();

            column
                .with_child(header_row)
                .with_child(Shrinkable::new(1.0, content_area).finish())
                .with_main_axis_size(MainAxisSize::Max)
                .finish()
        })
        .finish();

        if warpui::platform::is_mobile_device() {
            return panel_content;
        }

        let drag_side = match self.panel_position {
            super::PanelPosition::Left => DragBarSide::Right,
            super::PanelPosition::Right => DragBarSide::Left,
        };
        Resizable::new(self.resizable_state_handle.clone(), panel_content)
            .with_dragbar_side(drag_side)
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .with_bounds_callback(Box::new(|window_size| {
                let min_width = MIN_SIDEBAR_WIDTH;
                let max_width = window_size.x() * MAX_SIDEBAR_WIDTH_RATIO;
                (min_width, max_width.max(min_width))
            }))
            .finish()
    }
}

fn deduplicate_by_directory_name(directories: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    directories
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .collect()
}

#[cfg(test)]
mod shortcuts_editor_tests {
    use super::*;
    use crate::shortcuts::action::{Action, KeyName};
    use std::time::Duration;

    #[test]
    fn parse_wait_value_accepts_supported_units_within_range() {
        assert_eq!(parse_wait_value("1ms"), Ok(Duration::from_millis(1)));
        assert_eq!(parse_wait_value("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse_wait_value("2s"), Ok(Duration::from_secs(2)));
        assert_eq!(parse_wait_value("1m"), Ok(Duration::from_secs(60)));
        // Trims surrounding whitespace because typed-input values often have it.
        assert_eq!(parse_wait_value("  150ms "), Ok(Duration::from_millis(150)));
    }

    #[test]
    fn parse_wait_value_rejects_zero_and_overflow() {
        assert!(parse_wait_value("0ms").is_err());
        // 1m1ms == 60_001 ms which is over the 60s cap.
        assert!(parse_wait_value("61s").is_err());
        assert!(parse_wait_value("2m").is_err());
    }

    #[test]
    fn parse_wait_value_rejects_garbage() {
        assert!(parse_wait_value("garbage").is_err());
        assert!(parse_wait_value("100").is_err());
        assert!(parse_wait_value("ms").is_err());
        assert!(parse_wait_value("3.5s").is_err());
    }

    #[test]
    fn key_name_label_round_trip() {
        let names = [
            KeyName::Enter,
            KeyName::Tab,
            KeyName::Escape,
            KeyName::Backspace,
            KeyName::Space,
            KeyName::Up,
            KeyName::Down,
            KeyName::Left,
            KeyName::Right,
            KeyName::Home,
            KeyName::End,
            KeyName::PageUp,
            KeyName::PageDown,
            KeyName::Delete,
            KeyName::Insert,
            KeyName::NumpadEnter,
            KeyName::F(1),
            KeyName::F(5),
            KeyName::F(12),
        ];
        for n in &names {
            let label = key_name_to_label(*n);
            let back = label_to_key_name(label)
                .unwrap_or_else(|| panic!("label_to_key_name lost {label} for {n:?}"));
            assert_eq!(back, *n, "round-trip failed for {label}");
        }
    }

    #[test]
    fn editing_action_round_trip_new_tab() {
        let row = EditingAction::from_runtime(&Action::NewTab);
        assert_eq!(row.kind, EditingActionKind::NewTab);
        let back = row.to_runtime_action(1).expect("new_tab serialises back");
        assert!(matches!(back, Action::NewTab));
    }

    #[test]
    fn editing_action_round_trip_new_pane_all_directions() {
        use crate::pane_group::Direction;
        // `Direction` doesn't derive `PartialEq` in pane_group (upstream
        // code we don't want to fork), so the round-trip asserts via the
        // expected label string instead of comparing enum values directly.
        let cases = [
            (Direction::Right, "right"),
            (Direction::Down, "down"),
            (Direction::Left, "left"),
            (Direction::Up, "up"),
        ];
        for (dir, label) in cases {
            let row = EditingAction::from_runtime(&Action::NewPane(dir));
            assert_eq!(row.kind, EditingActionKind::NewPane);
            assert_eq!(NEW_PANE_DIRECTIONS[row.param_cycle_idx], label);
            let back = row.to_runtime_action(1).expect("new_pane serialises back");
            match back {
                Action::NewPane(d) => {
                    let back_label = match d {
                        Direction::Right => "right",
                        Direction::Down => "down",
                        Direction::Left => "left",
                        Direction::Up => "up",
                    };
                    assert_eq!(back_label, label);
                }
                other => panic!("expected NewPane, got {other:?}"),
            }
        }
    }

    #[test]
    fn editing_action_round_trip_type_preserves_text() {
        let row = EditingAction::from_runtime(&Action::Type("hello world".to_owned()));
        assert_eq!(row.kind, EditingActionKind::Type);
        assert_eq!(row.param_text, "hello world");
        let back = row.to_runtime_action(1).expect("type serialises back");
        match back {
            Action::Type(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected Type, got {back:?}"),
        }
    }

    #[test]
    fn editing_action_type_rejects_newline() {
        let mut row = EditingAction::new_default();
        row.kind = EditingActionKind::Type;
        row.param_text = "a\nb".to_owned();
        assert!(row.to_runtime_action(1).is_err());
    }

    #[test]
    fn editing_action_round_trip_press_supported_keys() {
        for key_label in PRESS_KEY_CYCLE {
            let kn = label_to_key_name(key_label).expect("cycle list keys are recognised");
            let row = EditingAction::from_runtime(&Action::Press(kn));
            assert_eq!(row.kind, EditingActionKind::Press);
            assert_eq!(PRESS_KEY_CYCLE[row.param_cycle_idx], key_label);
            let back = row.to_runtime_action(1).expect("press serialises back");
            match back {
                Action::Press(k) => assert_eq!(k, kn),
                _ => panic!("expected Press, got {back:?}"),
            }
        }
    }

    #[test]
    fn editing_action_round_trip_wait_uses_clean_unit() {
        // ms-granularity (not a clean second multiple): rendered with `ms`.
        let row = EditingAction::from_runtime(&Action::Wait(Duration::from_millis(150)));
        assert_eq!(row.param_text, "150ms");
        let back = row.to_runtime_action(1).expect("wait serialises back");
        match back {
            Action::Wait(d) => assert_eq!(d, Duration::from_millis(150)),
            _ => panic!("expected Wait, got {back:?}"),
        }

        // Whole seconds: rendered with `s`.
        let row = EditingAction::from_runtime(&Action::Wait(Duration::from_secs(2)));
        assert_eq!(row.param_text, "2s");
        let back = row.to_runtime_action(1).expect("wait serialises back");
        assert!(matches!(back, Action::Wait(d) if d == Duration::from_secs(2)));

        // Whole minute: rendered with `m`.
        let row = EditingAction::from_runtime(&Action::Wait(Duration::from_secs(60)));
        assert_eq!(row.param_text, "1m");
    }

    #[test]
    fn editing_action_kind_cycle_is_total() {
        // Cycle through all five kinds once and confirm we land back on NewTab.
        let mut k = EditingActionKind::NewTab;
        for _ in 0..5 {
            k = k.next();
        }
        assert_eq!(k, EditingActionKind::NewTab);
    }
}
