//! twarp: Custom shortcuts settings page.
//!
//! This was previously the left-panel toolbelt tab `ToolPanelView::Shortcuts`.
//! It has been moved into Settings so the sidebar reads simply as
//! Files | Search | Code. The behavior here is a faithful port of the old
//! panel — a read-only list of shortcuts plus an inline detail editor
//! (keystroke capture, action rows, validation) — backed by the same
//! `crate::shortcuts` registry and `shortcuts.yaml` file.
//!
//! See `PRODUCT 04` for the original spec (§§26-38); the only change from the
//! left-panel version is the host view, not the interaction model.

use std::cell::RefCell;

use super::settings_page::{
    render_sub_header, MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle,
    SettingsWidget,
};
use super::SettingsSection;
use crate::appearance::Appearance;
use crate::view_components::{
    ClickableTextInput, ClickableTextInputAction, ClickableTextInputEvent,
};
use crate::workspace::WorkspaceAction;
use twarpui::{
    elements::{
        ChildView, Container, CrossAxisAlignment, DispatchEventResult, Element, EventHandler, Flex,
        Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Shrinkable,
    },
    keymap::Keystroke,
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

/// Maximum display length for a shortcut name in the list row. Names longer
/// than this are truncated with `…`. Character-based rather than pixel-based
/// so it works without a layout pass.
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

/// The directions accepted by `new_pane`, in cycle order. Matches PRODUCT §7's
/// expanded v1 set (right/down/left/up).
const NEW_PANE_DIRECTIONS: [&str; 4] = ["right", "down", "left", "up"];

/// The named keys offered by the action editor's `press` selector. A subset of
/// PRODUCT §10's full v1 list — the editor cycles through the common cases;
/// users wanting `f1`–`f12` or arrow keys can hand-edit `shortcuts.yaml`.
const PRESS_KEY_CYCLE: [&str; 7] = ["enter", "tab", "escape", "backspace", "space", "up", "down"];

/// Action kinds that round-trip with the shortcuts parser. Mirrors
/// `shortcuts::action::Action` shape but holds raw parameter text so the editor
/// can offer in-progress edits without requiring a valid parse on every
/// keystroke.
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

/// Whether this kind takes a free-text parameter or a cycled enum.
fn kind_param_is_freetext(kind: EditingActionKind) -> bool {
    matches!(kind, EditingActionKind::Type | EditingActionKind::Wait)
}

/// One row in the action editor. Holds the kind and the current parameter
/// representation: a cycle-state index for enum-valued params, a string for
/// free-text params, or nothing for `new_tab`.
struct EditingAction {
    kind: EditingActionKind,
    /// Free-text parameter (used when `kind` is `Type` / `Wait`). Backed by a
    /// `ClickableTextInput` child view so it round-trips through twarpui's editor
    /// without us managing focus by hand.
    param_text: String,
    param_text_input: Option<ViewHandle<ClickableTextInput>>,
    /// Cycle index into the enum's accepted values (`NEW_PANE_DIRECTIONS` for
    /// `NewPane`, `PRESS_KEY_CYCLE` for `Press`). Ignored otherwise.
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
    /// Default action used for new rows added via [+ Add action]. PRODUCT §33
    /// specifies `new_tab` as the default; matches our cycle order.
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

    /// Hydrate an editor row from a parsed `Action` on disk. The cycle indices
    /// are recovered from the action's payload so the row renders the value the
    /// user originally wrote.
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

    /// Build the runtime `Action` for this row, returning an error message
    /// suitable for `validation_error` if the parameter is missing or malformed.
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

/// Inverse of `key_name_to_label`, scoped to the labels the editor's cycle list
/// can produce — `PRESS_KEY_CYCLE` plus the future-proofing extras the parser
/// can already round-trip.
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

/// PRODUCT §11: parse a `wait` value like `500ms` / `2s` / `1m`, clamped to
/// `[1ms, 60s]`. Returns a human-readable error message for failures.
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

/// The detail editor's full mutable state. Lives inside
/// `ShortcutsSettingsPageView::editing_shortcut` while the editor is open and is
/// `take()`n on Save/Cancel. Owns the child `ClickableTextInput` view handles
/// for the name field and each action's param input.
struct EditingShortcutState {
    target: EditTarget,
    /// Display copy of the name; mirrors what `name_text_input` emits so renders
    /// don't have to read the child view every frame.
    name_text: String,
    name_text_input: ViewHandle<ClickableTextInput>,
    /// Captured chord. `None` while the user has not yet captured one.
    chord: Option<Keystroke>,
    /// Whether the chord field is currently intercepting the next keystroke.
    capturing_chord: bool,
    /// Chord value at the moment capture started; restored on Escape.
    chord_before_capture: Option<Keystroke>,
    chord_field_mouse: MouseStateHandle,
    actions: Vec<EditingAction>,
    add_action_button_mouse: MouseStateHandle,
    save_button_mouse: MouseStateHandle,
    cancel_button_mouse: MouseStateHandle,
    /// Most recent validation error from a Save attempt (PRODUCT §34/§36).
    validation_error: Option<String>,
}

/// The actions handled by the Shortcuts settings page (formerly the
/// `LeftPanelAction::Shortcuts*` family).
#[derive(Clone, Debug)]
pub enum ShortcutsSettingsPageAction {
    /// Open the inline detail editor in "create new" mode (PRODUCT §29).
    AddNew,
    /// Open `shortcuts.yaml` in a twarp tab so the user can hand-edit.
    OpenInEditor,
    /// Toggle the inline Edit / Delete menu for a specific row (§§30, 31).
    ToggleRowMenu(usize),
    /// Close any open per-row inline menu.
    CloseRowMenu,
    /// Remove the shortcut at the given index, persist, and reload (§31).
    Delete(usize),
    /// Open the inline detail editor for the shortcut at this index (§30).
    BeginEdit(usize),
    /// Discard the in-flight detail editor without saving (§29).
    EditCancel,
    /// Validate and persist the in-flight detail editor (§36).
    EditSave,
    /// Click the chord field; enters keystroke-capture mode (§32).
    EditChordFieldClick,
    /// A keystroke was captured while the chord field was in capture mode (§32).
    EditChordCaptured(Keystroke),
    /// Escape while in chord-capture mode: restore the previous chord (§32).
    EditChordCancel,
    /// Cycle the action kind on the row at the given index (§33).
    EditActionCycleKind(usize),
    /// Cycle the action's enum-valued parameter (§33).
    EditActionCycleParam(usize),
    /// Append a new action row defaulting to `new_tab` (§33).
    EditActionAdd,
    /// Remove the action row at the given index (§33).
    EditActionRemove(usize),
    /// Reorder: move the action row at the given index up by one slot (§33).
    EditActionMoveUp(usize),
    /// Reorder: move the action row at the given index down by one slot.
    EditActionMoveDown(usize),
}

pub struct ShortcutsSettingsPageView {
    page: PageType<Self>,
    /// Index of the row whose inline Edit/Delete menu is currently shown.
    shortcut_context_menu_target: Option<usize>,
    /// Mouse state for the inline Edit/Delete buttons. One state total since
    /// only one row's menu is open at a time.
    shortcut_delete_mouse_state: MouseStateHandle,
    /// Persistent mouse states for the shortcut list rows. Grown on demand to
    /// match the registry length; behind a `RefCell` so the `&self` render path
    /// can extend the vec.
    shortcut_row_mouse_states: RefCell<Vec<MouseStateHandle>>,
    /// Mouse state for the "+ New shortcut" link.
    add_new_shortcut_button: MouseStateHandle,
    /// State for the inline detail editor (PRODUCT §§29-30, 32-38). `None` when
    /// no editor is open and the page shows the list view.
    editing_shortcut: Option<EditingShortcutState>,
}

impl ShortcutsSettingsPageView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            page: PageType::new_monolith(ShortcutsWidget::default(), None, true),
            shortcut_context_menu_target: None,
            shortcut_delete_mouse_state: MouseStateHandle::default(),
            shortcut_row_mouse_states: RefCell::new(Vec::new()),
            add_new_shortcut_button: MouseStateHandle::default(),
            editing_shortcut: None,
        }
    }

    /// Render the page body: a "Custom shortcuts" sub-header plus either the
    /// read-only list (with "+ New shortcut") or the inline detail editor.
    fn render_content(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let subheader = render_sub_header(appearance, "Custom shortcuts", None);
        let body = if self.editing_shortcut.is_some() {
            self.render_editor(app)
        } else {
            self.render_list(app)
        };
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(subheader)
            .with_child(body)
            .finish()
    }

    /// One row in the Custom shortcuts list. Hoverable + clickable.
    /// PRODUCT 04 §§27, 30, 31.
    ///
    /// - Left click on the row body → dispatch `OpenInEditor`, which opens
    ///   `shortcuts.yaml` in a twarp tab.
    /// - Right click → toggle the inline Edit/Delete menu.
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
            // PRODUCT §30 ([edit]) + §31 ([delete]). The mouse state for [edit]
            // is shared with [delete] since only one menu is open at a time.
            let edit_link = appearance
                .ui_builder()
                .link(
                    "Edit".to_owned(),
                    None,
                    Some(Box::new(move |ctx| {
                        ctx.dispatch_typed_action(ShortcutsSettingsPageAction::BeginEdit(idx));
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
                        ctx.dispatch_typed_action(ShortcutsSettingsPageAction::Delete(idx));
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
                ctx.dispatch_typed_action(ShortcutsSettingsPageAction::OpenInEditor);
            })
            .on_right_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ShortcutsSettingsPageAction::ToggleRowMenu(idx));
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    /// Renders the read-only list: header with "+ New shortcut" link, an
    /// optional `shortcuts.yaml` errors banner, then a row per registered
    /// shortcut (or an empty-state hint). PRODUCT 04 §§27-29, 35.
    fn render_list(&self, app: &AppContext) -> Box<dyn Element> {
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::AddNew);
                })),
                self.add_new_shortcut_button.clone(),
            )
            .build()
            .finish();

        // Errors banner (PRODUCT 04 §35): shown whenever parsing dropped any
        // entries, with the offending message inline.
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
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(10.0)
            .with_child(add_new_link);
        if let Some(banner) = errors_banner {
            column = column.with_child(banner);
        }
        column.with_child(body).finish()
    }

    /// The inline detail editor (PRODUCT §§29-30, 32-38).
    fn render_editor(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let state = self
            .editing_shortcut
            .as_ref()
            .expect("render_editor called without editor state");

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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditCancel);
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditSave);
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

        // Name field — a `ClickableTextInput` child view in a labelled row.
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
            ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditChordFieldClick);
        })
        .finish();
        // While in capture mode, intercept the next keydown and turn it into a
        // typed action (PRODUCT §32).
        let chord_field: Box<dyn Element> = if state.capturing_chord {
            EventHandler::new(chord_field_clickable)
                .on_keydown(|ctx, _, keystroke| {
                    if keystroke.key == "escape"
                        && !keystroke.cmd
                        && !keystroke.ctrl
                        && !keystroke.alt
                        && !keystroke.shift
                        && !keystroke.meta
                    {
                        ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditChordCancel);
                    } else {
                        ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditChordCaptured(
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditActionAdd);
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
        column.finish()
    }

    /// Render one row of the action editor (PRODUCT §33). Layout:
    /// `[kind cycle button] [param widget] [↑] [↓] [×]`.
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditActionCycleKind(
                        idx,
                    ));
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
                                ShortcutsSettingsPageAction::EditActionCycleParam(idx),
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
                                ShortcutsSettingsPageAction::EditActionCycleParam(idx),
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditActionMoveUp(idx));
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditActionMoveDown(idx));
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
                    ctx.dispatch_typed_action(ShortcutsSettingsPageAction::EditActionRemove(idx));
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

    /// Construct the initial state for a brand-new shortcut.
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

    /// Construct editor state pre-populated from the row at `index` in the
    /// current registry. Returns `None` when the index is out of range.
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

    /// Spawn a `ClickableTextInput` child view pre-populated with the initial
    /// text, subscribing to its Submit event so edits route back into our state.
    fn make_text_input(
        initial: String,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<ClickableTextInput> {
        let handle = ctx
            .add_typed_action_view(|child_ctx| ClickableTextInput::new(initial.clone(), child_ctx));
        let weak = handle.downgrade();
        ctx.subscribe_to_view(&handle, move |me, _, event, ctx| {
            let ClickableTextInputEvent::Submit(text) = event;
            // Resolve which slot this submit came from by comparing ViewHandle
            // identity, then push the new text back into the input's `text`
            // field so its display-mode label shows the saved value.
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
    /// `ClickableTextInput`, and that other rows have `None`.
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

    /// PRODUCT §36 save semantics: validate the editor, build a `Shortcut`,
    /// splice it into the registry snapshot, write to disk, and reload. On
    /// failure surface the error inline.
    fn handle_edit_save(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(state) = self.editing_shortcut.as_mut() else {
            return;
        };
        if state.capturing_chord {
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
                    // The row was deleted out from under us by a concurrent
                    // reload; degrade to an append rather than dropping the edit.
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

    /// PRODUCT 04 §31 (delete). Removes the shortcut at `index`, writes via
    /// `save_to_disk`, and reloads. Out-of-range indices are a no-op.
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
}

impl Entity for ShortcutsSettingsPageView {
    type Event = ();
}

impl View for ShortcutsSettingsPageView {
    fn ui_name() -> &'static str {
        "ShortcutsSettingsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for ShortcutsSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Shortcuts
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<ShortcutsSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<ShortcutsSettingsPageView>) -> Self {
        SettingsPageViewHandle::Shortcuts(view_handle)
    }
}

impl TypedActionView for ShortcutsSettingsPageView {
    type Action = ShortcutsSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use ShortcutsSettingsPageAction as A;
        match action {
            A::AddNew => {
                self.shortcut_context_menu_target = None;
                self.editing_shortcut = Some(Self::new_editing_state(EditTarget::Create, ctx));
                ctx.notify();
            }
            A::BeginEdit(idx) => {
                self.shortcut_context_menu_target = None;
                if let Some(state) = Self::editing_state_for_existing(*idx, ctx) {
                    self.editing_shortcut = Some(state);
                    ctx.notify();
                }
            }
            A::EditCancel => {
                if self.editing_shortcut.is_some() {
                    self.editing_shortcut = None;
                    // Capture mode is one-shot; make sure global dispatch is
                    // enabled in case we cancel mid-capture.
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
            }
            A::EditSave => {
                self.handle_edit_save(ctx);
            }
            A::EditChordFieldClick => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if state.capturing_chord {
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
            A::EditChordCaptured(keystroke) => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    state.chord = Some(keystroke.clone());
                    state.capturing_chord = false;
                    state.validation_error = None;
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
            }
            A::EditChordCancel => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    state.chord = state.chord_before_capture.clone();
                    state.capturing_chord = false;
                    ctx.enable_key_bindings_dispatching();
                    ctx.notify();
                }
            }
            A::EditActionCycleKind(idx) => {
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
            A::EditActionCycleParam(idx) => {
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
            A::EditActionAdd => {
                if let Some(state) = self.editing_shortcut.as_mut() {
                    let mut row = EditingAction::new_default();
                    Self::ensure_param_input_for_row(&mut row, ctx);
                    state.actions.push(row);
                    state.validation_error = None;
                    ctx.notify();
                }
            }
            A::EditActionRemove(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if idx < state.actions.len() {
                        state.actions.remove(idx);
                        state.validation_error = None;
                        ctx.notify();
                    }
                }
            }
            A::EditActionMoveUp(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if idx > 0 && idx < state.actions.len() {
                        state.actions.swap(idx, idx - 1);
                        state.validation_error = None;
                        ctx.notify();
                    }
                }
            }
            A::EditActionMoveDown(idx) => {
                let idx = *idx;
                if let Some(state) = self.editing_shortcut.as_mut() {
                    if idx + 1 < state.actions.len() {
                        state.actions.swap(idx, idx + 1);
                        state.validation_error = None;
                        ctx.notify();
                    }
                }
            }
            A::OpenInEditor => {
                self.shortcut_context_menu_target = None;
                let full_path = crate::shortcuts::shortcuts_file_path();
                // If `shortcuts.yaml` already has an open code pane in some tab,
                // activate that tab instead of creating a duplicate.
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
                    // Defer — dispatching another typed action synchronously
                    // inside handle_action would re-enter the view update
                    // machinery and panic ("Circular view update").
                    ctx.dispatch_typed_action_deferred(WorkspaceAction::OpenFileInNewTab {
                        full_path,
                        line_and_column: None,
                    });
                }
                ctx.notify();
            }
            A::ToggleRowMenu(index) => {
                self.shortcut_context_menu_target =
                    if self.shortcut_context_menu_target == Some(*index) {
                        None
                    } else {
                        Some(*index)
                    };
                ctx.notify();
            }
            A::CloseRowMenu => {
                if self.shortcut_context_menu_target.is_some() {
                    self.shortcut_context_menu_target = None;
                    ctx.notify();
                }
            }
            A::Delete(index) => {
                let index = *index;
                self.shortcut_context_menu_target = None;
                Self::delete_shortcut_at_index(index, ctx);
            }
        }
    }
}

/// Monolith widget wrapper so the page participates in settings search/scroll.
#[derive(Default)]
struct ShortcutsWidget;

impl SettingsWidget for ShortcutsWidget {
    type View = ShortcutsSettingsPageView;

    fn search_terms(&self) -> &str {
        "shortcuts custom shortcuts chord keystroke macro"
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        view.render_content(app)
    }
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
