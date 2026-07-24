use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::coding_panel_enablement_state::CodingPanelEnablementState;

use async_channel::Sender;
use pathfinder_geometry::vector::vec2f;
use string_offset::{ByteOffset, CharCounter};
use twarp_editor::editor::NavigationKey;
use twarp_ripgrep::search::{Match as RipgrepMatch, Submatch};

use crate::code::global_buffer_model::GlobalBufferModel;
use crate::code::icon_from_file_path;
use crate::debounce::debounce;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, InteractionState,
    PropagateAndNoOpNavigationKeys, PropagateHorizontalNavigationKeys, TextOptions,
};
use crate::search::ItemHighlightState as SearchHighlightState;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as UiIcon;
use crate::ui_components::item_highlight::{ImageOrIcon, ItemHighlightState};
use crate::ui_components::render_file_search_row::{render_file_search_row, FileSearchRowOptions};
use crate::view_components::action_button::{
    ActionButton, ButtonSize, NakedTheme, PrimaryTheme, SecondaryTheme,
};
use crate::workspace::view::global_search::model::GlobalSearch;
use crate::workspace::view::global_search::replace_model::{
    apply_preview_to_disk, generate_preview, make_fingerprint, ReplaceApplySummary,
    ReplaceFilePreview, ReplaceFingerprint, ReplacePreview,
};
use crate::workspace::view::global_search::SearchConfig;
use crate::TelemetryEvent;
use twarp_core::send_telemetry_from_ctx;
use twarp_core::ui::appearance::Appearance;
use twarp_core::ui::theme::color::internal_colors;
use twarp_core::ui::theme::{AnsiColorIdentifier, Fill as ThemeFill};
use twarp_core::ui::Icon;
use twarpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DispatchEventResult, Empty, EventHandler, Fill, Flex, FormattedTextElement,
    Highlight, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    Padding, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition,
    ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable, Stack, Text,
    UniformList, UniformListState,
};
use twarpui::fonts::{Properties, Weight};
use twarpui::keymap::FixedBinding;
use twarpui::platform::Cursor;
use twarpui::text_layout::{TextAlignment, TextStyle};
use twarpui::ui_components::components::{UiComponent as _, UiComponentStyles};
use twarpui::ui_components::text::Span;
use twarpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle,
};

const BORDER_RADIUS: f32 = 6.;
const BORDER_WIDTH: f32 = 1.;
const DO_NOT_TRUNCATE_CHAR_COUNT: usize = 40;
const DO_TRUNCATE_END_CHAR_COUNT: usize = 200;
const PRE_MATCH_CHARS: usize = 15;
const MAX_MATCH_COUNT: usize = 20000;

const QUERY_EDITOR_MAX_LINES: usize = 6;

const QUERY_DEBOUNCE_PERIOD: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalSearchEntryFocus {
    QueryEditor,
    Results,
}

enum FocusMode {
    QueryEditor,
    ResultsList,
}

#[derive(Debug, Clone)]
pub enum GlobalSearchAction {
    SelectRow {
        directory_path: PathBuf,
        file_path: PathBuf,
        match_index: Option<usize>,
    },
    ToggleFileCollapsed {
        directory_path: PathBuf,
        file_path: PathBuf,
    },
    ToggleDirectoryCollapsed {
        directory_path: PathBuf,
    },
    OpenMatch {
        path: PathBuf,
        line_number: u32,
        column_num: Option<usize>,
    },
    ResultsUp,
    ResultsDown,
    ResultsLeft,
    ResultsRight,
    ResultsEnter,
    FocusQueryEditor,
    FocusResultsList,
    FocusIncludeEditor,
    FocusExcludeEditor,
    FocusReplaceEditor,
    ToggleRegexSearch,
    ToggleCaseSensitivity,
    GenerateReplacePreview,
    ToggleReplacePreviewFile {
        path: PathBuf,
    },
    CancelReplacePreview,
    ConfirmReplacePreview,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum GlobalSearchEvent {
    Started {
        search_id: u32,
    },
    Progress {
        search_id: u32,
        result: RipgrepMatch,
    },
    ProgressBatch {
        search_id: u32,
        items: Vec<RipgrepMatch>,
    },
    Completed {
        search_id: u32,
        total_match_count: usize,
    },
    Failed {
        search_id: u32,
        error: String,
    },
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum Event {
    OpenMatch {
        path: PathBuf,
        line_number: u32,
        column_num: Option<usize>,
    },
}

enum SelectionDirection {
    Up,
    Down,
}

/// Absolute row index in the flattened view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GlobalIndex(usize);

/// Hierarchical index that identifies a specific row by its position in the
/// directory/file/match hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RowIndex {
    directory_index: usize,
    index_type: RowIndexType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowIndexType {
    DirectoryHeader,
    FileHeader {
        path_index: usize,
    },
    Match {
        path_index: usize,
        match_index: usize,
    },
}

/// A root directory containing matched files.
struct DirectoryEntry {
    path: PathBuf,
    is_collapsed: bool,
    mouse_state: MouseStateHandle,
    matched_paths: MatchedPaths,
}

impl DirectoryEntry {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            is_collapsed: false,
            mouse_state: MouseStateHandle::default(),
            matched_paths: MatchedPaths::new(),
        }
    }

    /// Returns the number of visible rows for this directory entry.
    /// If collapsed, only the directory header is visible (1 row).
    /// Otherwise, includes directory header + all visible file rows.
    fn visible_count(&self) -> usize {
        if self.is_collapsed {
            1
        } else {
            1 + self.matched_paths.visible_count()
        }
    }

    /// Returns the total number of matches across all files in this directory.
    fn total_match_count(&self) -> usize {
        self.matched_paths
            .paths
            .iter()
            .map(|p| p.matches.len())
            .sum()
    }
}

/// Collection of matched files within a directory.
struct MatchedPaths {
    paths: Vec<MatchedPath>,
    index_by_path: HashMap<PathBuf, usize>,
}

impl MatchedPaths {
    fn new() -> Self {
        Self {
            paths: Vec::new(),
            index_by_path: HashMap::new(),
        }
    }

    /// Returns the total number of visible rows across all files.
    fn visible_count(&self) -> usize {
        self.paths.iter().map(|p| p.visible_count()).sum()
    }

    /// Gets or creates a MatchedPath entry for the given file path.
    /// Returns a mutable reference to the entry and its index.
    fn get_or_create(&mut self, path: &Path) -> (&mut MatchedPath, usize) {
        if let Some(&index) = self.index_by_path.get(path) {
            (&mut self.paths[index], index)
        } else {
            let index = self.paths.len();
            self.paths.push(MatchedPath::new(path.to_path_buf()));
            self.index_by_path.insert(path.to_path_buf(), index);
            (&mut self.paths[index], index)
        }
    }

    /// Gets a mutable MatchedPath entry by file path.
    fn get_mut(&mut self, path: &Path) -> Option<&mut MatchedPath> {
        self.index_by_path
            .get(path)
            .copied()
            .map(|idx| &mut self.paths[idx])
    }
}

/// A file containing matches.
struct MatchedPath {
    path: PathBuf,
    is_collapsed: bool,
    mouse_state: MouseStateHandle,
    matches: Vec<Match>,
}

impl MatchedPath {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            is_collapsed: false,
            mouse_state: MouseStateHandle::default(),
            matches: Vec::new(),
        }
    }

    /// Returns the number of visible rows for this file.
    /// If collapsed, only the file header is visible (1 row).
    /// Otherwise, includes file header + all matches.
    fn visible_count(&self) -> usize {
        if self.is_collapsed {
            1
        } else {
            1 + self.matches.len()
        }
    }
}

/// A single match within a file.
struct Match {
    line_text: String,
    line_number: u32,
    submatches: Vec<Submatch>,
    mouse_state: MouseStateHandle,
}

impl Match {
    fn new(line_text: String, line_number: u32, submatches: Vec<Submatch>) -> Self {
        Self {
            line_text,
            line_number,
            submatches,
            mouse_state: MouseStateHandle::default(),
        }
    }
}

pub struct GlobalSearchView {
    find_model: ModelHandle<GlobalSearch>,
    query_editor: ViewHandle<EditorView>,
    replace_editor: ViewHandle<EditorView>,
    /// VSCode-style "files to include" glob input (comma-separated).
    include_editor: ViewHandle<EditorView>,
    /// VSCode-style "files to exclude" glob input (comma-separated).
    exclude_editor: ViewHandle<EditorView>,
    query_change_tx: Sender<()>,
    /// All terminal working directories for display grouping (preserved as-is)
    root_directories: Vec<PathBuf>,
    /// Deduplicated roots for ripgrep search (excludes nested subdirectories)
    search_roots: Vec<PathBuf>,
    last_searched_pattern: Option<String>,
    /// The include/exclude globs used for the last dispatched search, so a
    /// filter-only change (same pattern) still re-runs.
    last_searched_filters: (Vec<String>, Vec<String>),
    directory_entries: Vec<DirectoryEntry>,
    directory_path_to_directory_index_entry: HashMap<PathBuf, usize>,
    selected_row: Option<RowIndex>,
    total_match_count: usize,
    is_search_in_progress: bool,
    capped_matches: bool,
    last_error: Option<String>,
    scroll_state: ScrollStateHandle,
    uniform_list_state: UniformListState,
    handle: WeakViewHandle<GlobalSearchView>,
    focus_mode: FocusMode,
    enablement: CodingPanelEnablementState,
    current_search_id: Option<u32>,
    regex_search_enabled: bool,
    regex_button: ViewHandle<ActionButton>,
    case_sensitivity_enabled: bool,
    case_sensitivity_button: ViewHandle<ActionButton>,
    replace_all_button: ViewHandle<ActionButton>,
    confirm_replace_button: ViewHandle<ActionButton>,
    cancel_replace_button: ViewHandle<ActionButton>,
    replace_preview: Option<ReplacePreview>,
    replace_is_generating_preview: bool,
    replace_is_applying: bool,
    replace_error: Option<String>,
    replace_summary: Option<ReplaceApplySummary>,
    search_results_stale: bool,
    /// The Files sidebar already supplies the surrounding context and has
    /// limited vertical space, so its embedded search omits the large
    /// marketing-style pre-search empty state.
    show_pre_search_zero_state: bool,
}

impl Entity for GlobalSearchView {
    type Event = Event;
}

impl TypedActionView for GlobalSearchView {
    type Action = GlobalSearchAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GlobalSearchAction::SelectRow {
                directory_path,
                file_path,
                match_index,
            } => {
                if let Some(row_index) =
                    self.path_to_row_index(directory_path, file_path, *match_index)
                {
                    self.set_selected_row(row_index, ctx);
                    self.enter_results_mode(ctx);
                }
            }
            GlobalSearchAction::ToggleFileCollapsed {
                directory_path,
                file_path,
            } => {
                self.toggle_file_collapsed(directory_path, file_path, ctx);
            }
            GlobalSearchAction::ToggleDirectoryCollapsed { directory_path } => {
                self.toggle_directory_collapsed(directory_path, ctx);
            }
            GlobalSearchAction::OpenMatch {
                path,
                line_number,
                column_num,
            } => {
                ctx.emit(Event::OpenMatch {
                    path: path.clone(),
                    line_number: *line_number,
                    column_num: *column_num,
                });
                self.enter_query_mode(ctx);
            }
            GlobalSearchAction::ResultsDown => {
                if !matches!(self.focus_mode, FocusMode::ResultsList) {
                    return;
                }
                if self.directory_entries.is_empty() {
                    return;
                }
                self.move_selection(SelectionDirection::Down, ctx);
            }
            GlobalSearchAction::ResultsUp => {
                if !matches!(self.focus_mode, FocusMode::ResultsList) {
                    return;
                }
                if self.directory_entries.is_empty() {
                    return;
                }
                if self.is_first_row_selected() {
                    self.enter_query_mode(ctx);
                } else {
                    self.move_selection(SelectionDirection::Up, ctx);
                }
            }
            GlobalSearchAction::ResultsLeft => {
                if !matches!(self.focus_mode, FocusMode::ResultsList) {
                    return;
                }
                let Some(selected) = self.selected_row.clone() else {
                    return;
                };

                match &selected.index_type {
                    RowIndexType::DirectoryHeader => {
                        let is_collapsed = self.is_directory_collapsed(&selected);
                        if !is_collapsed {
                            if let Some(dir_path) =
                                self.directory_path_for_row_index(&selected).cloned()
                            {
                                self.toggle_directory_collapsed(&dir_path, ctx);
                            }
                        }
                    }
                    RowIndexType::FileHeader { .. } => {
                        let is_collapsed = self.is_file_collapsed(&selected);
                        if !is_collapsed {
                            if let (Some(dir_path), Some(file_path)) = (
                                self.directory_path_for_row_index(&selected).cloned(),
                                self.file_path_for_row_index(&selected).cloned(),
                            ) {
                                self.toggle_file_collapsed(&dir_path, &file_path, ctx);
                            }
                        }
                    }
                    RowIndexType::Match { .. } => {}
                }
            }
            GlobalSearchAction::ResultsRight => {
                if !matches!(self.focus_mode, FocusMode::ResultsList) {
                    return;
                }
                let Some(selected) = self.selected_row.clone() else {
                    return;
                };

                match &selected.index_type {
                    RowIndexType::DirectoryHeader => {
                        let is_collapsed = self.is_directory_collapsed(&selected);
                        if is_collapsed {
                            if let Some(dir_path) =
                                self.directory_path_for_row_index(&selected).cloned()
                            {
                                self.toggle_directory_collapsed(&dir_path, ctx);
                            }
                        }
                    }
                    RowIndexType::FileHeader { .. } => {
                        let is_collapsed = self.is_file_collapsed(&selected);
                        if is_collapsed {
                            if let (Some(dir_path), Some(file_path)) = (
                                self.directory_path_for_row_index(&selected).cloned(),
                                self.file_path_for_row_index(&selected).cloned(),
                            ) {
                                self.toggle_file_collapsed(&dir_path, &file_path, ctx);
                            }
                        }
                    }
                    RowIndexType::Match { .. } => {}
                }
            }
            GlobalSearchAction::ResultsEnter => {
                if !matches!(self.focus_mode, FocusMode::ResultsList) {
                    return;
                }
                self.ensure_selection(ctx);
                if self.selected_row.is_some() {
                    self.activate_selected_row(ctx);
                }
            }
            GlobalSearchAction::FocusQueryEditor => {
                self.enter_query_mode(ctx);
            }
            GlobalSearchAction::FocusResultsList => {
                self.enter_results_mode(ctx);
            }
            GlobalSearchAction::FocusIncludeEditor => {
                // Enter query mode so a filter-only edit still passes the
                // re-run guard in `rerun_search_from_query`.
                self.set_query_mode_state(ctx);
                ctx.focus(&self.include_editor);
            }
            GlobalSearchAction::FocusExcludeEditor => {
                self.set_query_mode_state(ctx);
                ctx.focus(&self.exclude_editor);
            }
            GlobalSearchAction::FocusReplaceEditor => {
                self.set_query_mode_state(ctx);
                ctx.focus(&self.replace_editor);
            }
            GlobalSearchAction::ToggleRegexSearch => {
                self.dismiss_replace_preview(ctx);
                self.regex_search_enabled = !self.regex_search_enabled;
                self.regex_button.update(ctx, |button, ctx| {
                    button.set_active(self.regex_search_enabled, ctx);
                });
                self.rerun_search_from_query(ctx, true);
            }
            GlobalSearchAction::ToggleCaseSensitivity => {
                self.dismiss_replace_preview(ctx);
                self.case_sensitivity_enabled = !self.case_sensitivity_enabled;
                self.case_sensitivity_button.update(ctx, |button, ctx| {
                    button.set_active(self.case_sensitivity_enabled, ctx);
                });
                self.rerun_search_from_query(ctx, true);
            }
            GlobalSearchAction::GenerateReplacePreview => {
                self.generate_replace_preview(ctx);
            }
            GlobalSearchAction::ToggleReplacePreviewFile { path } => {
                self.toggle_replace_preview_file(path, ctx);
            }
            GlobalSearchAction::CancelReplacePreview => {
                self.dismiss_replace_preview(ctx);
            }
            GlobalSearchAction::ConfirmReplacePreview => {
                self.confirm_replace_preview(ctx);
            }
        }
    }
}

impl GlobalSearchView {
    /// Calculate the 1-indexed column number from the first submatch.
    /// Returns None if there are no submatches or the line is empty.
    fn column_from_submatches(line_text: &str, submatches: &[Submatch]) -> Option<usize> {
        if line_text.is_empty() || submatches.is_empty() {
            return None;
        }

        let first_submatch = &submatches[0];
        let max_byte = ByteOffset::from(line_text.len());
        let start_b = first_submatch.byte_start.min(max_byte);

        let mut char_counter = CharCounter::new(line_text);
        let start_char = char_counter.char_offset(start_b)?;

        // Return 1-indexed column number
        Some(start_char.as_usize() + 1)
    }

    /// Convert submatch byte ranges into character indices for highlighting.
    fn highlight_indices_from_submatches(line_text: &str, submatches: &[Submatch]) -> Vec<usize> {
        if line_text.is_empty() || submatches.is_empty() {
            return Vec::new();
        }

        let max_byte = ByteOffset::from(line_text.len());
        let total_chars = line_text.chars().count();
        let mut indices = Vec::new();

        let mut char_counter = CharCounter::new(line_text);
        for submatch in submatches {
            let start_b = submatch.byte_start.min(max_byte);
            let end_b = submatch.byte_end.min(max_byte);
            if start_b >= end_b {
                continue;
            }

            // NOTE: Ripgrep submatch offsets are byte-based (and `end_b` is exclusive). If a match
            // ends at the end of the string, `end_b == line_text.len()` and there is no character
            // starting at that byte offset; `CharCounter::char_offset(end_b)` would return `None`.
            // Handle that case explicitly.
            let Some(start_char) = char_counter.char_offset(start_b) else {
                continue;
            };

            let end_char = if end_b == max_byte {
                total_chars
            } else {
                let Some(end_char) = char_counter.char_offset(end_b) else {
                    continue;
                };
                end_char.as_usize()
            };

            let start_char = start_char.as_usize();
            if start_char >= end_char {
                continue;
            }

            indices.extend(start_char..end_char);
        }

        indices.sort_unstable();
        indices
    }

    fn elide_match_text(
        line_text: &str,
        mut highlight_indices: Vec<usize>,
    ) -> (String, Vec<usize>) {
        if line_text.is_empty() || highlight_indices.is_empty() {
            return (line_text.to_owned(), highlight_indices);
        }

        let total_chars = line_text.chars().count();
        if total_chars <= DO_NOT_TRUNCATE_CHAR_COUNT {
            return (line_text.to_owned(), highlight_indices);
        }

        highlight_indices.sort_unstable();
        let first_highlight = highlight_indices[0];

        let snippet_start_char = first_highlight.saturating_sub(PRE_MATCH_CHARS);
        let snippet_end_char = (snippet_start_char + DO_TRUNCATE_END_CHAR_COUNT).min(total_chars);

        let prefix_ellipsis = snippet_start_char > 0;
        let suffix_ellipsis = snippet_end_char < total_chars;

        let snippet_core: String = line_text
            .chars()
            .skip(snippet_start_char)
            .take(snippet_end_char.saturating_sub(snippet_start_char))
            .collect();

        let mut snippet = String::new();
        if prefix_ellipsis {
            snippet.push('…');
        }
        snippet.push_str(&snippet_core);
        if suffix_ellipsis {
            snippet.push('…');
        }

        let prefix_offset = if prefix_ellipsis { 1 } else { 0 };
        highlight_indices.retain(|&idx| idx >= snippet_start_char && idx < snippet_end_char);
        for idx in &mut highlight_indices {
            *idx = idx.saturating_sub(snippet_start_char) + prefix_offset;
        }

        (snippet, highlight_indices)
    }

    pub fn init(app: &mut AppContext) {
        use twarpui::keymap::macros::*;

        app.register_fixed_bindings([
            FixedBinding::new(
                "up",
                GlobalSearchAction::ResultsUp,
                id!(GlobalSearchView::ui_name()),
            ),
            FixedBinding::new(
                "down",
                GlobalSearchAction::ResultsDown,
                id!(GlobalSearchView::ui_name()),
            ),
            FixedBinding::new(
                "left",
                GlobalSearchAction::ResultsLeft,
                id!(GlobalSearchView::ui_name()),
            ),
            FixedBinding::new(
                "right",
                GlobalSearchAction::ResultsRight,
                id!(GlobalSearchView::ui_name()),
            ),
            FixedBinding::new(
                "enter",
                GlobalSearchAction::ResultsEnter,
                id!(GlobalSearchView::ui_name()),
            ),
        ]);
    }

    /// Builds a single-line editor for the include/exclude glob inputs.
    fn build_filter_editor(
        placeholder: &str,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<EditorView> {
        ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                text: TextOptions::ui_text(Some(13.), appearance),
                select_all_on_focus: true,
                clear_selections_on_blur: true,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Always,
                convert_newline_to_space: true,
                single_line: true,
                autogrow: false,
                soft_wrap: false,
                ..Default::default()
            };

            let mut editor = EditorView::new(options, ctx);
            editor.set_placeholder_text(placeholder, ctx);
            editor
        })
    }

    fn build_replace_editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                text: TextOptions::ui_text(Some(13.), appearance),
                select_all_on_focus: true,
                clear_selections_on_blur: true,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Always,
                convert_newline_to_space: false,
                single_line: false,
                autogrow: true,
                soft_wrap: false,
                ..Default::default()
            };

            let mut editor = EditorView::new(options, ctx);
            editor.set_placeholder_text("Replace", ctx);
            editor
        })
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let find_model = ctx.add_model(|_| GlobalSearch::new());

        let query_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                text: TextOptions::ui_text(Some(14.), appearance),
                select_all_on_focus: true,
                clear_selections_on_blur: true,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::AtBoundary,
                propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Always,
                // Preserve newlines when pasting so users can search for multi-line patterns.
                convert_newline_to_space: false,
                single_line: false,
                autogrow: true,
                // Prefer explicit newlines (from paste or shift-enter) over soft wrapping.
                soft_wrap: false,
                ..Default::default()
            };

            let mut editor = EditorView::new(options, ctx);
            editor.set_placeholder_text("Search in files", ctx);
            editor
        });

        let replace_editor = Self::build_replace_editor(ctx);
        let include_editor = Self::build_filter_editor("files to include (e.g. src, *.rs)", ctx);
        let exclude_editor =
            Self::build_filter_editor("files to exclude (e.g. target, *.lock)", ctx);

        let (query_change_tx, query_change_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(
            debounce(QUERY_DEBOUNCE_PERIOD, query_change_rx),
            Self::handle_debounced_query_change,
            |_, _| {},
        );

        let case_sensitivity_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new_with_boxed_theme(String::new(), Arc::new(NakedTheme))
                .with_icon(UiIcon::CaseSensitivity)
                .with_tooltip("Toggle Case Sensitivity")
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GlobalSearchAction::ToggleCaseSensitivity);
                })
        });

        let regex_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new_with_boxed_theme(String::new(), Arc::new(NakedTheme))
                .with_icon(UiIcon::Regex)
                .with_tooltip("Toggle Regex")
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GlobalSearchAction::ToggleRegexSearch);
                })
        });

        let replace_all_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Replace all", PrimaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GlobalSearchAction::GenerateReplacePreview);
                })
        });

        let confirm_replace_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Confirm replace", PrimaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GlobalSearchAction::ConfirmReplacePreview);
                })
        });

        let cancel_replace_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Cancel", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GlobalSearchAction::CancelReplacePreview);
                })
        });

        ctx.subscribe_to_view(&query_editor, |me, _handle, event, ctx| {
            me.handle_query_editor_event(event, ctx);
        });

        ctx.subscribe_to_view(&replace_editor, |me, _handle, event, ctx| {
            me.handle_replace_editor_event(event, ctx);
        });

        ctx.subscribe_to_view(&include_editor, |me, _handle, event, ctx| {
            me.handle_filter_editor_event(event, ctx);
        });
        ctx.subscribe_to_view(&exclude_editor, |me, _handle, event, ctx| {
            me.handle_filter_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&find_model, |me, _handle, event, ctx| {
            me.handle_find_model_event(event, ctx);
        });
        replace_all_button.update(ctx, |button, ctx| {
            button.set_disabled(true, ctx);
        });
        confirm_replace_button.update(ctx, |button, ctx| {
            button.set_disabled(true, ctx);
        });
        let handle = ctx.handle();

        GlobalSearchView {
            find_model,
            query_editor,
            replace_editor,
            include_editor,
            exclude_editor,
            query_change_tx,
            root_directories: Vec::new(),
            search_roots: Vec::new(),
            last_searched_pattern: None,
            last_searched_filters: (Vec::new(), Vec::new()),
            directory_entries: Vec::new(),
            directory_path_to_directory_index_entry: HashMap::new(),
            selected_row: None,
            total_match_count: 0,
            is_search_in_progress: false,
            capped_matches: false,
            last_error: None,
            scroll_state: ScrollStateHandle::default(),
            uniform_list_state: UniformListState::new(),
            handle,
            focus_mode: FocusMode::QueryEditor,
            enablement: CodingPanelEnablementState::Enabled,
            current_search_id: None,
            regex_search_enabled: false,
            regex_button,
            case_sensitivity_enabled: false,
            case_sensitivity_button,
            replace_all_button,
            confirm_replace_button,
            cancel_replace_button,
            replace_preview: None,
            replace_is_generating_preview: false,
            replace_is_applying: false,
            replace_error: None,
            replace_summary: None,
            search_results_stale: false,
            show_pre_search_zero_state: true,
        }
    }

    pub fn set_embedded_in_files_sidebar(&mut self, embedded: bool, ctx: &mut ViewContext<Self>) {
        self.show_pre_search_zero_state = !embedded;
        ctx.notify();
    }

    pub fn on_left_panel_focused(
        &mut self,
        entry_focus: GlobalSearchEntryFocus,
        ctx: &mut ViewContext<Self>,
    ) {
        match entry_focus {
            GlobalSearchEntryFocus::QueryEditor => {
                self.enter_query_mode(ctx);
            }
            GlobalSearchEntryFocus::Results => {
                if self.directory_entries.is_empty() {
                    self.enter_query_mode(ctx);
                } else {
                    self.enter_results_mode(ctx);
                }
            }
        }
    }

    fn set_query_mode_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_mode = FocusMode::QueryEditor;
        self.query_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Editable, ctx);
        });
        self.selected_row = None;
        ctx.notify();
    }

    /// Returns the most specific root where the given file path should appear.
    /// Nested roots are searched once and displayed once, avoiding duplicate
    /// result rows for the same file.
    fn find_matching_directory<'a>(&'a self, file_path: &'a Path) -> Option<&'a PathBuf> {
        self.root_directories
            .iter()
            .filter(move |root| file_path.starts_with(root))
            .max_by_key(|root| {
                (
                    root.components().count(),
                    root.as_os_str().to_string_lossy().len(),
                )
            })
    }

    fn apply_progress_item(&mut self, result: RipgrepMatch, ctx: &mut ViewContext<Self>) {
        if self.total_match_count >= MAX_MATCH_COUNT {
            return;
        }

        let file_path = result.file_path.clone();

        let Some(directory_path) = self.find_matching_directory(&file_path).cloned() else {
            // File doesn't match any root directory, skip it
            let file_path_name = file_path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_else(|| std::borrow::Cow::Borrowed("<unknown>"));
            log::warn!("[Global search] file {file_path_name} was not found in directories");
            return;
        };

        // Populate hierarchical data model (directory_entries)
        let (directory_entries, directory_path_to_directory_index_entry) = (
            &mut self.directory_entries,
            &mut self.directory_path_to_directory_index_entry,
        );

        let dir_index = *directory_path_to_directory_index_entry
            .entry(directory_path.clone())
            .or_insert_with(|| {
                let idx = directory_entries.len();
                directory_entries.push(DirectoryEntry::new(directory_path.clone()));
                idx
            });

        let dir_entry = &mut directory_entries[dir_index];
        let (matched_path, _path_index) = dir_entry.matched_paths.get_or_create(&file_path);

        matched_path.matches.push(Match::new(
            result.line_text.clone(),
            result.line_number,
            result.submatches.clone(),
        ));

        self.total_match_count += 1;

        if self.total_match_count == MAX_MATCH_COUNT {
            self.abort_search(ctx);
        }
    }
    fn abort_search(&mut self, ctx: &mut ViewContext<Self>) {
        self.capped_matches = true;
        self.is_search_in_progress = false;
        self.current_search_id = None;

        self.find_model.update(ctx, |model, _| {
            model.abort_search();
        });
    }

    fn cancel_search(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_search_id = None;
        self.is_search_in_progress = false;
        self.find_model.update(ctx, |model, _| {
            model.abort_search();
        });
    }
    fn handle_debounced_query_change(&mut self, _event: (), ctx: &mut ViewContext<Self>) {
        self.rerun_search_from_query(ctx, false);
    }

    fn handle_query_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Down) => {
                if !matches!(self.focus_mode, FocusMode::QueryEditor) {
                    return;
                }
                if self.directory_entries.is_empty() {
                    return;
                }
                self.enter_results_mode(ctx);
            }
            EditorEvent::Edited(_)
            | EditorEvent::BufferReplaced
            | EditorEvent::BufferReinitialized => {
                self.dismiss_replace_preview(ctx);
                let query_text = self.query_editor.as_ref(ctx).buffer_text(ctx);
                if query_text.is_empty() {
                    self.cancel_search(ctx);
                    self.reset_search_state(true);
                    ctx.notify();
                    return;
                }

                self.notify_query_changed();
            }
            EditorEvent::Enter => {
                self.rerun_search_from_query(ctx, true);
            }
            _ => {}
        }
    }

    /// Handles edits in the include/exclude glob editors. Filter changes are
    /// debounced through the same channel as the query, then re-run the search
    /// even when the pattern itself is unchanged (see `rerun_search_from_query`).
    fn handle_filter_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_)
            | EditorEvent::BufferReplaced
            | EditorEvent::BufferReinitialized => {
                self.dismiss_replace_preview(ctx);
                self.notify_query_changed();
            }
            EditorEvent::Enter => {
                self.rerun_search_from_query(ctx, true);
            }
            _ => {}
        }
    }

    fn handle_replace_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_)
            | EditorEvent::BufferReplaced
            | EditorEvent::BufferReinitialized => {
                self.dismiss_replace_preview(ctx);
            }
            EditorEvent::Enter => {
                if self.can_generate_replace_preview() {
                    self.generate_replace_preview(ctx);
                }
            }
            _ => {}
        }
    }

    /// Parses a comma-separated list of glob patterns from a filter editor,
    /// dropping empty entries and surrounding whitespace.
    fn parse_globs(raw: &str) -> Vec<String> {
        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn notify_query_changed(&self) {
        let _ = self.query_change_tx.try_send(());
    }

    fn can_generate_replace_preview(&self) -> bool {
        self.last_error.is_none()
            && !self.search_results_stale
            && !self.is_search_in_progress
            && !self.replace_is_generating_preview
            && !self.replace_is_applying
            && self.total_match_count > 0
            && !self.directory_entries.is_empty()
    }

    fn can_confirm_replace_preview(&self) -> bool {
        self.replace_preview
            .as_ref()
            .is_some_and(|preview| preview.included_match_count() > 0)
            && !self.replace_is_generating_preview
            && !self.replace_is_applying
    }

    fn current_search_config(&self, ctx: &mut ViewContext<Self>) -> SearchConfig {
        SearchConfig {
            use_regex: self.regex_search_enabled,
            use_case_sensitivity: self.case_sensitivity_enabled,
            includes: Self::parse_globs(&self.include_editor.as_ref(ctx).buffer_text(ctx)),
            excludes: Self::parse_globs(&self.exclude_editor.as_ref(ctx).buffer_text(ctx)),
        }
    }

    fn current_replace_fingerprint(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ReplaceFingerprint> {
        let query = self.query_editor.as_ref(ctx).buffer_text(ctx);
        if query.is_empty() {
            return None;
        }

        Some(make_fingerprint(
            query,
            self.current_search_config(ctx),
            self.search_roots.clone(),
        ))
    }

    fn replace_candidate_paths(&self) -> Vec<PathBuf> {
        self.directory_entries
            .iter()
            .flat_map(|directory| directory.matched_paths.paths.iter())
            .map(|matched_path| matched_path.path.clone())
            .collect()
    }

    fn sync_replace_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        let replace_disabled = !self.can_generate_replace_preview();
        self.replace_all_button.update(ctx, |button, ctx| {
            button.set_disabled(replace_disabled, ctx);
        });

        let confirm_disabled = !self.can_confirm_replace_preview();
        self.confirm_replace_button.update(ctx, |button, ctx| {
            button.set_disabled(confirm_disabled, ctx);
        });
    }

    fn dismiss_replace_preview(&mut self, ctx: &mut ViewContext<Self>) {
        if self.replace_preview.is_none()
            && self.replace_error.is_none()
            && self.replace_summary.is_none()
            && !self.replace_is_generating_preview
            && !self.replace_is_applying
        {
            return;
        }

        self.replace_preview = None;
        self.replace_is_generating_preview = false;
        self.replace_is_applying = false;
        self.replace_error = None;
        self.replace_summary = None;
        self.sync_replace_button_state(ctx);
        ctx.notify();
    }

    fn generate_replace_preview(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.can_generate_replace_preview() {
            return;
        }

        let Some(fingerprint) = self.current_replace_fingerprint(ctx) else {
            return;
        };
        let replacement = self.replace_editor.as_ref(ctx).buffer_text(ctx);
        let candidate_paths = self.replace_candidate_paths();

        self.replace_preview = None;
        self.replace_summary = None;
        self.replace_error = None;
        self.replace_is_generating_preview = true;
        self.sync_replace_button_state(ctx);
        ctx.notify();

        ctx.spawn(
            generate_preview(fingerprint, replacement, candidate_paths),
            |me, result, ctx| {
                me.replace_is_generating_preview = false;
                match result {
                    Ok(preview) => {
                        me.replace_preview = Some(preview);
                        me.replace_error = None;
                    }
                    Err(err) => {
                        me.replace_preview = None;
                        me.replace_error =
                            Some(format!("Could not generate replace preview: {err}"));
                    }
                }
                me.sync_replace_button_state(ctx);
                ctx.notify();
            },
        );
    }

    fn toggle_replace_preview_file(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        let Some(preview) = self.replace_preview.as_mut() else {
            return;
        };

        let Some(file) = preview.files.iter_mut().find(|file| file.path == path) else {
            return;
        };

        file.included = !file.included;
        self.sync_replace_button_state(ctx);
        ctx.notify();
    }

    fn confirm_replace_preview(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.can_confirm_replace_preview() {
            return;
        }

        let Some(preview) = self.replace_preview.clone() else {
            return;
        };
        let Some(current_fingerprint) = self.current_replace_fingerprint(ctx) else {
            return;
        };
        if preview.fingerprint != current_fingerprint {
            self.replace_error =
                Some("Preview is stale. Generate a new preview before applying.".to_string());
            self.replace_preview = None;
            self.sync_replace_button_state(ctx);
            ctx.notify();
            return;
        }

        let included_paths: Vec<PathBuf> = preview
            .files
            .iter()
            .filter(|file| file.included)
            .map(|file| file.path.clone())
            .collect();

        let open_paths = GlobalBufferModel::handle(ctx).update(ctx, |buffer_model, buffer_ctx| {
            included_paths
                .iter()
                .filter(|path| buffer_model.path_has_active_buffer(path, buffer_ctx))
                .cloned()
                .collect::<HashSet<_>>()
        });

        self.replace_is_applying = true;
        self.replace_preview = None;
        self.replace_error = None;
        self.replace_summary = None;
        self.sync_replace_button_state(ctx);
        ctx.notify();

        ctx.spawn(
            async move { apply_preview_to_disk(&preview, open_paths).await },
            |me, summary, ctx| {
                me.replace_is_applying = false;
                me.replace_preview = None;
                me.replace_summary = Some(summary);
                me.search_results_stale = true;
                me.sync_replace_button_state(ctx);
                ctx.notify();
            },
        );
    }

    fn reset_search_state(&mut self, clear_last_pattern: bool) {
        self.directory_entries.clear();
        self.directory_path_to_directory_index_entry.clear();
        self.selected_row = None;
        self.total_match_count = 0;
        self.capped_matches = false;
        self.last_error = None;
        self.search_results_stale = false;
        self.uniform_list_state = UniformListState::new();

        if clear_last_pattern {
            self.last_searched_pattern = None;
        }
    }

    fn rerun_search_from_query(&mut self, ctx: &mut ViewContext<Self>, force: bool) {
        if !matches!(self.focus_mode, FocusMode::QueryEditor) && !force {
            return;
        }

        let pattern = self.query_editor.as_ref(ctx).buffer_text(ctx);
        if pattern.is_empty() {
            self.cancel_search(ctx);
            self.reset_search_state(true);
            ctx.notify();
            return;
        }

        if self.search_roots.is_empty() {
            log::warn!("GlobalSearch: no search roots; skipping search");
            self.cancel_search(ctx);
            self.reset_search_state(false);
            self.last_error =
                Some("Global search requires a local workspace directory.".to_string());
            ctx.notify();
            return;
        }

        let includes = Self::parse_globs(&self.include_editor.as_ref(ctx).buffer_text(ctx));
        let excludes = Self::parse_globs(&self.exclude_editor.as_ref(ctx).buffer_text(ctx));
        let filters = (includes.clone(), excludes.clone());

        let should_run_search = if force {
            true
        } else {
            match self.last_searched_pattern.as_deref() {
                Some(last) => last != pattern || self.last_searched_filters != filters,
                None => true,
            }
        };

        if !should_run_search {
            return;
        }

        self.last_searched_pattern = Some(pattern.clone());
        self.last_searched_filters = filters;

        let roots = self.search_roots.clone();
        self.find_model.update(ctx, |model, model_ctx| {
            model.run_search(
                pattern.clone(),
                roots,
                SearchConfig {
                    use_regex: self.regex_search_enabled,
                    use_case_sensitivity: self.case_sensitivity_enabled,
                    includes,
                    excludes,
                },
                model_ctx,
            );
        });
    }

    fn handle_find_model_event(&mut self, event: &GlobalSearchEvent, ctx: &mut ViewContext<Self>) {
        match event {
            GlobalSearchEvent::Started { search_id } => {
                send_telemetry_from_ctx!(TelemetryEvent::GlobalSearchQueryStarted, ctx);

                self.current_search_id = Some(*search_id);

                self.is_search_in_progress = true;
                self.replace_preview = None;
                self.replace_error = None;
                self.replace_summary = None;
                self.reset_search_state(false);
                self.sync_replace_button_state(ctx);
                ctx.notify();
            }
            GlobalSearchEvent::Progress { search_id, result } => {
                if Some(*search_id) != self.current_search_id {
                    return;
                }

                self.apply_progress_item(result.clone(), ctx);
                self.sync_replace_button_state(ctx);
                ctx.notify();
            }
            GlobalSearchEvent::ProgressBatch { search_id, items } => {
                if Some(*search_id) != self.current_search_id {
                    return;
                }

                for item in items {
                    self.apply_progress_item(item.clone(), ctx);

                    if self.capped_matches {
                        break;
                    }
                }
                self.sync_replace_button_state(ctx);
                ctx.notify();
            }
            GlobalSearchEvent::Completed {
                search_id,
                total_match_count,
            } => {
                if Some(*search_id) != self.current_search_id {
                    return;
                }

                self.is_search_in_progress = false;
                self.total_match_count = *total_match_count;
                self.sync_replace_button_state(ctx);
                ctx.notify();
            }
            GlobalSearchEvent::Failed { search_id, error } => {
                if Some(*search_id) != self.current_search_id {
                    return;
                }

                self.is_search_in_progress = false;
                self.reset_search_state(false);
                self.last_error = Some(error.clone());
                self.sync_replace_button_state(ctx);
                ctx.notify();
            }
        }
    }

    pub fn set_root_directories(&mut self, roots: Vec<PathBuf>, _ctx: &mut ViewContext<Self>) {
        // Ancestor-dedup search roots so we don't search the same file twice
        // when terminal directories are nested (e.g. `~/code` + `~/code/a`).
        // Shared with `FileTreeView` for consistency.
        self.search_roots = twarp_util::path::group_roots_by_common_ancestor(&roots).roots;
        self.root_directories = roots;
    }

    /// Pre-populates the search query with the given text.
    /// Selects all text so the user can easily overwrite or keep it, then triggers a search.
    pub fn set_initial_query(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.query_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&text, ctx);
            editor.select_all(ctx);
        });

        // Trigger the debounced search
        self.notify_query_changed();
        ctx.notify();
    }

    pub(crate) fn set_enablement_state(
        &mut self,
        enablement: CodingPanelEnablementState,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.enablement == enablement {
            return;
        }
        self.enablement = enablement;
        ctx.notify();
    }

    fn render_row_at_index(&self, index: usize, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Convert global index to hierarchical RowIndex
        let Some(row_index) = self.to_row_index(GlobalIndex(index)) else {
            return Empty::new().finish();
        };

        // Get the directory entry
        let Some(dir_entry) = self.directory_entries.get(row_index.directory_index) else {
            debug_assert!(self.to_row_index(GlobalIndex(index)).is_some());
            return Empty::new().finish();
        };

        match &row_index.index_type {
            RowIndexType::DirectoryHeader => {
                self.render_directory_header_from_entry(index, dir_entry, appearance, theme)
            }
            RowIndexType::FileHeader { path_index } => {
                let Some(matched_path) = dir_entry.matched_paths.paths.get(*path_index) else {
                    return Empty::new().finish();
                };
                self.render_file_header(
                    index,
                    &dir_entry.path,
                    matched_path,
                    appearance,
                    theme,
                    app,
                )
            }
            RowIndexType::Match {
                path_index,
                match_index,
            } => {
                let Some(matched_path) = dir_entry.matched_paths.paths.get(*path_index) else {
                    return Empty::new().finish();
                };
                let Some(matched) = matched_path.matches.get(*match_index) else {
                    return Empty::new().finish();
                };
                self.render_match_row(
                    index,
                    &dir_entry.path,
                    matched_path,
                    matched,
                    *match_index,
                    appearance,
                    theme,
                )
            }
        }
    }

    fn render_file_header(
        &self,
        index: usize,
        directory_path: &Path,
        matched_path: &MatchedPath,
        appearance: &Appearance,
        theme: &twarp_core::ui::theme::WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_selected = self.is_row_at_index_selected(index);
        let is_collapsed = matched_path.is_collapsed;
        let file_mouse_state = matched_path.mouse_state.clone();
        let file_path = matched_path.path.clone();
        let match_count = matched_path.matches.len();

        let directory_path_for_select = directory_path.to_path_buf();
        let file_path_clone = file_path.clone();
        let directory_path_for_toggle = directory_path.to_path_buf();

        let display_path = file_path
            .strip_prefix(directory_path)
            .unwrap_or(file_path.as_path());
        let display_path = display_path.to_path_buf();

        Hoverable::new(file_mouse_state, move |mouse_state| {
            let item_highlight_state = ItemHighlightState::new(is_selected, mouse_state);
            let search_highlight_state = SearchHighlightState::new(is_selected, mouse_state);
            let list_highlight_state = ItemHighlightState::new(is_selected, mouse_state);

            let chevron_icon_enum = if is_collapsed {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            };
            let icon_size = 16.0;
            let chevron_color = item_highlight_state.text_and_icon_color(appearance);
            let chevron_icon = chevron_icon_enum
                .to_warpui_icon(ThemeFill::from(chevron_color))
                .finish();
            let chevron_icon = ConstrainedBox::new(chevron_icon)
                .with_width(icon_size)
                .with_height(icon_size)
                .finish();
            let chevron_container = Container::new(chevron_icon).with_margin_right(8.).finish();

            let tooltip_text = file_path.to_string_lossy().to_string();

            let header_text_fill = match list_highlight_state {
                ItemHighlightState::None => {
                    ThemeFill::Solid(blended_colors::text_sub(theme, theme.background()))
                }
                ItemHighlightState::Hovered => {
                    ThemeFill::Solid(blended_colors::text_main(theme, theme.background()))
                }
                ItemHighlightState::Selected => ThemeFill::Solid(theme.foreground().into()),
            };

            let left = render_file_search_row(
                &display_path,
                FileSearchRowOptions {
                    item_font_size: Some(14.),
                    path_font_size: Some(12.),
                    highlight_state: search_highlight_state,
                    text_color_override: Some(header_text_fill),
                    max_combined_length: None,
                    ..Default::default()
                },
                app,
            );

            let count_text =
                Text::new_inline(match_count.to_string(), appearance.ui_font_family(), 12.0)
                    .with_color(appearance.theme().background().into())
                    .finish();

            let count_badge = Container::new(count_text)
                .with_horizontal_padding(4.)
                .with_vertical_padding(1.)
                .with_background_color(theme.terminal_colors().normal.yellow.into())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish();

            let icon_from_file_path = icon_from_file_path(&file_path.to_string_lossy(), appearance)
                .map(ImageOrIcon::Image)
                .unwrap_or(ImageOrIcon::Icon(Icon::File));

            let icon_color = list_highlight_state.text_and_icon_color(appearance);
            let file_icon = match icon_from_file_path {
                ImageOrIcon::Icon(icon) => {
                    icon.to_warpui_icon(ThemeFill::from(icon_color)).finish()
                }
                ImageOrIcon::Image(image) => image,
            };

            let file_icon_element = Container::new(
                ConstrainedBox::new(file_icon)
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish(),
            )
            .with_padding_right(8.)
            .finish();

            let left_container = Container::new(left).with_margin_right(8.).finish();

            let left_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(chevron_container)
                .with_child(file_icon_element)
                .with_child(Shrinkable::new(1.0, left_container).finish())
                .finish();

            let header_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1.0, left_row).finish())
                .with_child(count_badge)
                .finish();

            let mut row_with_padding =
                Container::new(header_row).with_padding(Padding::uniform(4.).with_left(8.)); // 8px indent under directory

            if let Some(background_fill) = list_highlight_state.background_color(appearance) {
                let corner_radius = list_highlight_state
                    .corner_radius()
                    .unwrap_or_else(|| CornerRadius::with_all(Radius::Pixels(4.)));
                row_with_padding = row_with_padding
                    .with_background(background_fill)
                    .with_corner_radius(corner_radius);
            }

            let base_row = row_with_padding.finish();

            // Add a tooltip on hover that shows the full (untruncated) file path.
            if mouse_state.is_hovered() {
                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(tooltip_text)
                    .build()
                    .finish();

                let mut stack = Stack::new().with_child(base_row);
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopLeft,
                    ),
                );
                stack.finish()
            } else {
                base_row
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GlobalSearchAction::SelectRow {
                directory_path: directory_path_for_select.clone(),
                file_path: file_path_clone.clone(),
                match_index: None,
            });
            ctx.dispatch_typed_action(GlobalSearchAction::ToggleFileCollapsed {
                directory_path: directory_path_for_toggle.clone(),
                file_path: file_path_clone.clone(),
            });
        })
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_match_row(
        &self,
        index: usize,
        directory_path: &Path,
        matched_path: &MatchedPath,
        matched: &Match,
        match_index: usize,
        appearance: &Appearance,
        theme: &twarp_core::ui::theme::WarpTheme,
    ) -> Box<dyn Element> {
        let is_selected = self.is_row_at_index_selected(index);
        let line_number = matched.line_number;
        let line_text = matched.line_text.clone();
        let submatches = matched.submatches.clone();
        let mouse_state = matched.mouse_state.clone();

        let directory_path_for_select = directory_path.to_path_buf();
        let file_path_for_select = matched_path.path.clone();
        let path_for_click = matched_path.path.clone();
        let match_position_id = format!(
            "global_search_match:{}:{}:{}",
            matched_path.path.display(),
            line_number,
            match_index
        );

        // Clone for the on_click closure since line_text and submatches are moved into Hoverable
        let line_text_for_click = line_text.clone();
        let submatches_for_click = submatches.clone();

        let row = Hoverable::new(mouse_state, move |mouse_state| {
            let list_highlight_state = ItemHighlightState::new(is_selected, mouse_state);

            let text_color = match list_highlight_state {
                ItemHighlightState::None => blended_colors::text_sub(theme, theme.background()),
                ItemHighlightState::Hovered => blended_colors::text_main(theme, theme.background()),
                ItemHighlightState::Selected => theme.foreground().into(),
            };

            let highlight_indices =
                GlobalSearchView::highlight_indices_from_submatches(&line_text, &submatches);

            let (display_text, display_highlight_indices) =
                GlobalSearchView::elide_match_text(&line_text, highlight_indices);

            let mut match_text = Text::new_inline(
                display_text,
                appearance.ui_font_family(),
                appearance.ui_font_size() + 2.,
            )
            .with_color(text_color);

            if !display_highlight_indices.is_empty() {
                let yellow_overlay_2 = theme.ansi_overlay_2(
                    AnsiColorIdentifier::Yellow.to_ansi_color(&theme.terminal_colors().normal),
                );
                let highlight_style = TextStyle::new().with_background_color(yellow_overlay_2);
                let highlight = Highlight::new().with_text_style(highlight_style);
                match_text = match_text.with_single_highlight(highlight, display_highlight_indices);
            }

            let mut row = Container::new(match_text.finish())
                .with_vertical_padding(4.)
                .with_padding_left(32.) // 8px file indent + 24px (chevron + margin) to align with file icon
                .with_padding_right(12.);

            if let Some(background_fill) = list_highlight_state.background_color(appearance) {
                let corner_radius = list_highlight_state
                    .corner_radius()
                    .unwrap_or_else(|| CornerRadius::with_all(Radius::Pixels(4.)));
                row = row
                    .with_background(background_fill)
                    .with_corner_radius(corner_radius);
            }

            row.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GlobalSearchAction::SelectRow {
                directory_path: directory_path_for_select.clone(),
                file_path: file_path_for_select.clone(),
                match_index: Some(match_index),
            });
            let column_num = GlobalSearchView::column_from_submatches(
                &line_text_for_click,
                &submatches_for_click,
            );
            ctx.dispatch_typed_action(GlobalSearchAction::OpenMatch {
                path: path_for_click.clone(),
                line_number,
                column_num,
            });
        })
        .finish();

        SavePosition::new(row, match_position_id.as_str()).finish()
    }

    fn items_count(&self) -> usize {
        self.items_count_from_entries()
    }

    /// Converts a global (flat) index to a hierarchical RowIndex.
    /// Returns None if the index is out of bounds.
    fn to_row_index(&self, global: GlobalIndex) -> Option<RowIndex> {
        let mut running_count = 0;
        for (dir_idx, dir_entry) in self.directory_entries.iter().enumerate() {
            let dir_visible = dir_entry.visible_count();
            if running_count + dir_visible > global.0 {
                // The target row is within this directory
                let offset_within_dir = global.0 - running_count;
                return self.find_row_within_directory(dir_idx, offset_within_dir);
            }
            running_count += dir_visible;
        }
        None
    }

    /// Helper to find a specific row within a directory given an offset.
    /// offset=0 means the directory header itself.
    fn find_row_within_directory(&self, dir_idx: usize, offset: usize) -> Option<RowIndex> {
        let dir_entry = self.directory_entries.get(dir_idx)?;

        // offset 0 is the directory header
        if offset == 0 {
            return Some(RowIndex {
                directory_index: dir_idx,
                index_type: RowIndexType::DirectoryHeader,
            });
        }

        // If directory is collapsed, only the header is visible
        if dir_entry.is_collapsed {
            return None;
        }

        // Skip the directory header
        let mut remaining = offset - 1;

        for (path_idx, matched_path) in dir_entry.matched_paths.paths.iter().enumerate() {
            let path_visible = matched_path.visible_count();
            if remaining < path_visible {
                // The target row is within this file
                if remaining == 0 {
                    // It's the file header
                    return Some(RowIndex {
                        directory_index: dir_idx,
                        index_type: RowIndexType::FileHeader {
                            path_index: path_idx,
                        },
                    });
                }
                // It's a match row (remaining - 1 because we skip the file header)
                let match_index = remaining - 1;
                if match_index < matched_path.matches.len() {
                    return Some(RowIndex {
                        directory_index: dir_idx,
                        index_type: RowIndexType::Match {
                            path_index: path_idx,
                            match_index,
                        },
                    });
                }
                return None;
            }
            remaining -= path_visible;
        }

        None
    }

    /// Converts a hierarchical RowIndex to a global (flat) index.
    fn to_global_index(&self, row: &RowIndex) -> usize {
        let mut running_count = 0;

        // Sum up all visible rows from directories before the target
        for dir_entry in self.directory_entries.iter().take(row.directory_index) {
            running_count += dir_entry.visible_count();
        }

        // Add the offset within the target directory
        running_count + self.offset_within_directory(row)
    }

    /// Calculates the offset of a RowIndex within its directory.
    fn offset_within_directory(&self, row: &RowIndex) -> usize {
        match &row.index_type {
            RowIndexType::DirectoryHeader => 0,
            RowIndexType::FileHeader { path_index } => {
                let Some(dir_entry) = self.directory_entries.get(row.directory_index) else {
                    debug_assert!(
                        false,
                        "RowIndex.directory_index {} out of bounds",
                        row.directory_index
                    );
                    return 0;
                };
                // 1 for directory header + sum of visible counts before this file
                1 + dir_entry
                    .matched_paths
                    .paths
                    .iter()
                    .take(*path_index)
                    .map(|p| p.visible_count())
                    .sum::<usize>()
            }
            RowIndexType::Match {
                path_index,
                match_index,
            } => {
                let Some(dir_entry) = self.directory_entries.get(row.directory_index) else {
                    debug_assert!(
                        false,
                        "RowIndex.directory_index {} out of bounds",
                        row.directory_index
                    );
                    return 0;
                };
                // 1 for directory header + sum of visible counts before this file
                // + 1 for file header + match_index
                1 + dir_entry
                    .matched_paths
                    .paths
                    .iter()
                    .take(*path_index)
                    .map(|p| p.visible_count())
                    .sum::<usize>()
                    + 1
                    + match_index
            }
        }
    }

    /// Returns the total number of visible rows
    fn items_count_from_entries(&self) -> usize {
        self.directory_entries
            .iter()
            .map(|d| d.visible_count())
            .sum()
    }

    /// Returns the total number of unique matches across all directories.
    fn unique_match_count(&self) -> usize {
        self.directory_entries
            .iter()
            .map(|d| d.matched_paths.paths.len())
            .sum()
    }

    /// Gets or creates a DirectoryEntry for the given path.
    /// Returns a mutable reference to the entry and its index.
    #[allow(dead_code)] // Will be used in later PRs
    fn get_or_create_directory_entry(&mut self, path: &Path) -> (&mut DirectoryEntry, usize) {
        if let Some(&index) = self.directory_path_to_directory_index_entry.get(path) {
            (&mut self.directory_entries[index], index)
        } else {
            let index = self.directory_entries.len();
            self.directory_entries
                .push(DirectoryEntry::new(path.to_path_buf()));
            self.directory_path_to_directory_index_entry
                .insert(path.to_path_buf(), index);
            (&mut self.directory_entries[index], index)
        }
    }

    /// Converts directory_path + file_path + optional match_index to a RowIndex.
    /// Returns None if the paths are not found
    fn path_to_row_index(
        &self,
        directory_path: &Path,
        file_path: &Path,
        match_index: Option<usize>,
    ) -> Option<RowIndex> {
        let &directory_index = self
            .directory_path_to_directory_index_entry
            .get(directory_path)?;
        let dir_entry = self.directory_entries.get(directory_index)?;
        let &path_index = dir_entry.matched_paths.index_by_path.get(file_path)?;

        let index_type = match match_index {
            None => RowIndexType::FileHeader { path_index },
            Some(match_idx) => RowIndexType::Match {
                path_index,
                match_index: match_idx,
            },
        };

        Some(RowIndex {
            directory_index,
            index_type,
        })
    }

    /// Gets the directory path for a given RowIndex.
    fn directory_path_for_row_index(&self, row: &RowIndex) -> Option<&PathBuf> {
        self.directory_entries
            .get(row.directory_index)
            .map(|e| &e.path)
    }

    /// Gets the file path for a given RowIndex (if it refers to a file or match).
    fn file_path_for_row_index(&self, row: &RowIndex) -> Option<&PathBuf> {
        let dir_entry = self.directory_entries.get(row.directory_index)?;
        match &row.index_type {
            RowIndexType::DirectoryHeader => None,
            RowIndexType::FileHeader { path_index } | RowIndexType::Match { path_index, .. } => {
                dir_entry
                    .matched_paths
                    .paths
                    .get(*path_index)
                    .map(|p| &p.path)
            }
        }
    }

    fn is_file_collapsed(&self, row: &RowIndex) -> bool {
        let Some(dir_entry) = self.directory_entries.get(row.directory_index) else {
            return false;
        };
        match &row.index_type {
            RowIndexType::DirectoryHeader => false,
            RowIndexType::FileHeader { path_index } | RowIndexType::Match { path_index, .. } => {
                dir_entry
                    .matched_paths
                    .paths
                    .get(*path_index)
                    .map(|p| p.is_collapsed)
                    .unwrap_or(false)
            }
        }
    }

    /// Checks if a directory is collapsed.
    fn is_directory_collapsed(&self, row: &RowIndex) -> bool {
        self.directory_entries
            .get(row.directory_index)
            .map(|d| d.is_collapsed)
            .unwrap_or(false)
    }

    fn render_results(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if self.directory_entries.is_empty() {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let handle = self.handle.clone();

        let item_count = self.items_count();

        let build_items = move |range: Range<usize>, app: &AppContext| {
            {
                let view_handle = handle
                    .upgrade(app)
                    .expect("GlobalSearchView handle should be upgradeable");
                let view = view_handle.as_ref(app);

                (range.start..range.end).map(|idx| view.render_row_at_index(idx, app))
            }
            .collect::<Vec<_>>()
            .into_iter()
        };

        let list = UniformList::new(self.uniform_list_state.clone(), item_count, build_items);

        let scrollable = Scrollable::vertical(
            self.scroll_state.clone(),
            list.finish_scrollable(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        Some(scrollable)
    }

    /// Converts a global index to a RowIndex
    fn row_index_for_global_index(&self, index: usize) -> Option<RowIndex> {
        self.to_row_index(GlobalIndex(index))
    }

    /// Returns the global index for the currently selected row.
    /// Uses to_global_index for O(directories) lookup.
    fn selected_row_index(&self) -> Option<usize> {
        let selected_row = self.selected_row.as_ref()?;
        // Validate the selection still exists in the data model
        let dir_entry = self.directory_entries.get(selected_row.directory_index)?;
        match &selected_row.index_type {
            RowIndexType::DirectoryHeader => {}
            RowIndexType::FileHeader { path_index } | RowIndexType::Match { path_index, .. } => {
                // Check path exists
                dir_entry.matched_paths.paths.get(*path_index)?;
            }
        }
        Some(self.to_global_index(selected_row))
    }

    fn set_selected_row(&mut self, selected_row: RowIndex, ctx: &mut ViewContext<Self>) {
        let global_index = self.to_global_index(&selected_row);
        self.selected_row = Some(selected_row);
        self.uniform_list_state.scroll_to(global_index);
        ctx.notify();
    }

    fn set_selected_row_by_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(row_index) = self.row_index_for_global_index(index) else {
            return;
        };
        self.selected_row = Some(row_index);
        self.uniform_list_state.scroll_to(index);
        ctx.notify();
    }

    /// Selects the first row (directory header at index 0).
    fn select_first_row(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.directory_entries.is_empty() {
            let row_index = RowIndex {
                directory_index: 0,
                index_type: RowIndexType::DirectoryHeader,
            };
            self.set_selected_row(row_index, ctx);
        }
    }

    fn ensure_selection(&mut self, ctx: &mut ViewContext<Self>) {
        // Validate current selection still exists
        if self.selected_row.is_some() && self.selected_row_index().is_none() {
            self.selected_row = None;
        }

        if self.selected_row.is_none() && !self.directory_entries.is_empty() {
            self.select_first_row(ctx);
        }
    }

    fn move_selection(&mut self, direction: SelectionDirection, ctx: &mut ViewContext<Self>) {
        let total_items = self.items_count_from_entries();
        if total_items == 0 {
            return;
        }

        self.ensure_selection(ctx);

        let current_index = self
            .selected_row_index()
            .unwrap_or_else(|| total_items.saturating_sub(1));

        let next_index = match direction {
            SelectionDirection::Up => current_index.saturating_sub(1),
            SelectionDirection::Down => current_index + 1,
        };

        let max_index = total_items.saturating_sub(1);
        let clamped_next = next_index.min(max_index);

        self.set_selected_row_by_index(clamped_next, ctx);
    }

    fn activate_selected_row(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(selected_row) = self.selected_row.clone() else {
            return;
        };

        match &selected_row.index_type {
            RowIndexType::FileHeader { .. } => {
                if let (Some(dir_path), Some(file_path)) = (
                    self.directory_path_for_row_index(&selected_row).cloned(),
                    self.file_path_for_row_index(&selected_row).cloned(),
                ) {
                    self.toggle_file_collapsed(&dir_path, &file_path, ctx);
                }
            }
            RowIndexType::Match {
                path_index,
                match_index,
            } => {
                let Some(dir_entry) = self.directory_entries.get(selected_row.directory_index)
                else {
                    return;
                };
                let Some(matched_path) = dir_entry.matched_paths.paths.get(*path_index) else {
                    return;
                };
                let Some(matched) = matched_path.matches.get(*match_index) else {
                    return;
                };
                let column_num =
                    Self::column_from_submatches(&matched.line_text, &matched.submatches);
                ctx.emit(Event::OpenMatch {
                    path: matched_path.path.clone(),
                    line_number: matched.line_number,
                    column_num,
                });
            }
            RowIndexType::DirectoryHeader => {
                if let Some(dir_path) = self.directory_path_for_row_index(&selected_row).cloned() {
                    self.toggle_directory_collapsed(&dir_path, ctx);
                }
            }
        }
    }

    fn toggle_directory_collapsed(&mut self, directory_path: &Path, ctx: &mut ViewContext<Self>) {
        let Some(&dir_idx) = self
            .directory_path_to_directory_index_entry
            .get(directory_path)
        else {
            return;
        };
        let Some(dir_entry) = self.directory_entries.get_mut(dir_idx) else {
            return;
        };

        let was_collapsed = dir_entry.is_collapsed;
        dir_entry.is_collapsed = !was_collapsed;

        // If collapsing and selection was inside this directory, move to directory header
        if !was_collapsed {
            if let Some(selected_row) = self.selected_row.as_ref() {
                if selected_row.directory_index == dir_idx
                    && !matches!(selected_row.index_type, RowIndexType::DirectoryHeader)
                {
                    self.selected_row = Some(RowIndex {
                        directory_index: dir_idx,
                        index_type: RowIndexType::DirectoryHeader,
                    });
                }
            }
        }

        self.ensure_selection(ctx);
        ctx.notify();
    }

    fn toggle_file_collapsed(
        &mut self,
        directory_path: &Path,
        file_path: &PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        // Get directory index
        let Some(&dir_idx) = self
            .directory_path_to_directory_index_entry
            .get(directory_path)
        else {
            return;
        };
        let Some(dir_entry) = self.directory_entries.get_mut(dir_idx) else {
            return;
        };
        let Some(matched_path) = dir_entry.matched_paths.get_mut(file_path) else {
            return;
        };

        let was_collapsed = matched_path.is_collapsed;
        matched_path.is_collapsed = !was_collapsed;

        // If collapsing and selection was on a match in this file, move to file header
        if !was_collapsed {
            if let Some(selected_row) = self.selected_row.as_ref() {
                if let RowIndexType::Match { path_index, .. } = &selected_row.index_type {
                    // Check if the selection is in this file
                    if let Some(dir_path) = self.directory_path_for_row_index(selected_row) {
                        if let Some(sel_file_path) = self.file_path_for_row_index(selected_row) {
                            if dir_path == directory_path && sel_file_path == file_path {
                                self.selected_row = Some(RowIndex {
                                    directory_index: selected_row.directory_index,
                                    index_type: RowIndexType::FileHeader {
                                        path_index: *path_index,
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }

        self.ensure_selection(ctx);
        ctx.notify();
    }

    fn enter_query_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_query_mode_state(ctx);
        ctx.focus(&self.query_editor);
    }

    fn enter_results_mode(&mut self, ctx: &mut ViewContext<Self>) {
        if self.directory_entries.is_empty() {
            return;
        }
        self.focus_mode = FocusMode::ResultsList;
        self.query_editor.update(ctx, |editor, ctx| {
            editor.clear_selections(ctx);
            editor.set_interaction_state(InteractionState::Disabled, ctx);
        });
        self.ensure_selection(ctx);
        ctx.focus_self();
        ctx.notify();
    }

    fn is_first_row_selected(&self) -> bool {
        matches!(self.selected_row_index(), Some(0))
    }

    /// Checks if the row at the given global index is currently selected.
    fn is_row_at_index_selected(&self, global_index: usize) -> bool {
        self.selected_row_index() == Some(global_index)
    }

    fn render_directory_header_from_entry(
        &self,
        index: usize,
        dir_entry: &DirectoryEntry,
        appearance: &Appearance,
        theme: &twarp_core::ui::theme::WarpTheme,
    ) -> Box<dyn Element> {
        let is_selected = self.is_row_at_index_selected(index);
        let mouse_state = dir_entry.mouse_state.clone();
        let match_count = dir_entry.total_match_count();
        let is_collapsed = dir_entry.is_collapsed;
        let directory_path = &dir_entry.path;

        // Get the display name (last component of the path)
        let display_name = directory_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| directory_path.to_string_lossy().to_string());
        let directory_path = directory_path.clone();
        let directory_path_for_click = directory_path.clone();

        let tooltip_text = directory_path.to_string_lossy().to_string();

        Hoverable::new(mouse_state, move |mouse_state| {
            let list_highlight_state = ItemHighlightState::new(is_selected, mouse_state);

            // Chevron - dynamic based on collapse state
            let chevron_icon_enum = if is_collapsed {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            };
            let icon_size = 16.0;
            let chevron_color = list_highlight_state.text_and_icon_color(appearance);
            let chevron_icon = chevron_icon_enum
                .to_warpui_icon(ThemeFill::from(chevron_color))
                .finish();
            let chevron_icon = ConstrainedBox::new(chevron_icon)
                .with_width(icon_size)
                .with_height(icon_size)
                .finish();
            let chevron_container = Container::new(chevron_icon).with_margin_right(8.).finish();

            // Folder icon
            let icon_color = list_highlight_state.text_and_icon_color(appearance);
            let folder_icon = Icon::Folder
                .to_warpui_icon(ThemeFill::from(icon_color))
                .finish();
            let folder_icon_element = Container::new(
                ConstrainedBox::new(folder_icon)
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish(),
            )
            .with_padding_right(8.)
            .finish();

            // Directory name
            let text_color = list_highlight_state.text_and_icon_color(appearance);
            let directory_text =
                Text::new_inline(display_name.clone(), appearance.ui_font_family(), 14.)
                    .with_color(text_color)
                    .with_style(Properties::default())
                    .finish();

            // Match count badge
            let count_text =
                Text::new_inline(match_count.to_string(), appearance.ui_font_family(), 12.0)
                    .with_color(appearance.theme().background().into())
                    .finish();

            let count_badge = Container::new(count_text)
                .with_horizontal_padding(4.)
                .with_vertical_padding(1.)
                .with_background_color(theme.terminal_colors().normal.yellow.into())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish();

            let left_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(chevron_container)
                .with_child(folder_icon_element)
                .with_child(Shrinkable::new(1.0, directory_text).finish())
                .finish();

            let header_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1.0, left_row).finish())
                .with_child(count_badge)
                .finish();

            let mut row_with_padding =
                Container::new(header_row).with_padding(Padding::uniform(4.).with_left(0.)); // Align chevron with header text

            if let Some(background_fill) = list_highlight_state.background_color(appearance) {
                let corner_radius = list_highlight_state
                    .corner_radius()
                    .unwrap_or_else(|| CornerRadius::with_all(Radius::Pixels(4.)));
                row_with_padding = row_with_padding
                    .with_background(background_fill)
                    .with_corner_radius(corner_radius);
            }

            let base_row = row_with_padding.finish();

            // Add tooltip on hover
            if mouse_state.is_hovered() {
                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(tooltip_text.clone())
                    .build()
                    .finish();

                let mut stack = Stack::new().with_child(base_row);
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopLeft,
                    ),
                );
                stack.finish()
            } else {
                base_row
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(GlobalSearchAction::ToggleDirectoryCollapsed {
                directory_path: directory_path_for_click.clone(),
            });
        })
        .finish()
    }
}

impl View for GlobalSearchView {
    fn ui_name() -> &'static str {
        "GlobalSearchView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        match self.enablement {
            CodingPanelEnablementState::PendingRemoteSession
            | CodingPanelEnablementState::RemoteSession { .. } => {
                return self.render_remote_state(app);
            }
            CodingPanelEnablementState::UnsupportedSession => {
                return self.render_unsupported_session_state(app);
            }
            CodingPanelEnablementState::Disabled => {
                return self.render_unavailable_state(app);
            }
            CodingPanelEnablementState::Enabled => {}
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // twarp: the "Search" header label above the input was removed — the
        // toolbelt tab already labels this panel.
        let editor_line_height = self
            .query_editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance);

        let max_editor_height = QUERY_EDITOR_MAX_LINES as f32 * editor_line_height;

        let case_sensitivity_button =
            Container::new(ChildView::new(&self.case_sensitivity_button).finish()).finish();

        let regex_button = Container::new(ChildView::new(&self.regex_button).finish()).finish();

        let mut editor_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    ConstrainedBox::new(
                        Clipped::new(ChildView::new(&self.query_editor).finish()).finish(),
                    )
                    .with_max_height(max_editor_height)
                    .finish(),
                )
                .finish(),
            );
        editor_row.add_child(case_sensitivity_button);
        editor_row.add_child(regex_button);
        let editor_row = editor_row.finish();

        let query_row_container = Container::new(editor_row)
            .with_padding(Padding::uniform(6.))
            .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.surface_3()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS)))
            .with_margin_top(4.)
            .with_margin_bottom(4.)
            .finish();

        let query_row = EventHandler::new(query_row_container)
            .on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(GlobalSearchAction::FocusQueryEditor);
                DispatchEventResult::StopPropagation
            })
            .finish();

        // VSCode-style "files to include / exclude" glob inputs, styled to match
        // the query row.
        let build_filter_row = |editor: &ViewHandle<EditorView>,
                                focus_action: GlobalSearchAction|
         -> Box<dyn Element> {
            let input = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(1.0, Clipped::new(ChildView::new(editor).finish()).finish())
                        .finish(),
                )
                .finish();

            let container = Container::new(input)
                .with_padding(Padding::uniform(6.))
                .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.surface_3()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS)))
                .with_margin_bottom(4.)
                .finish();

            EventHandler::new(container)
                .on_left_mouse_down(move |ctx, _, _| {
                    ctx.dispatch_typed_action(focus_action.clone());
                    DispatchEventResult::StopPropagation
                })
                .finish()
        };

        let include_row =
            build_filter_row(&self.include_editor, GlobalSearchAction::FocusIncludeEditor);
        let exclude_row =
            build_filter_row(&self.exclude_editor, GlobalSearchAction::FocusExcludeEditor);
        let replace_input = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    ConstrainedBox::new(
                        Clipped::new(ChildView::new(&self.replace_editor).finish()).finish(),
                    )
                    .with_max_height(max_editor_height)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(ChildView::new(&self.replace_all_button).finish())
                    .with_margin_left(6.)
                    .finish(),
            )
            .finish();

        let replace_row_container = Container::new(replace_input)
            .with_padding(Padding::uniform(6.))
            .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.surface_3()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS)))
            .with_margin_bottom(4.)
            .finish();

        let replace_row = EventHandler::new(replace_row_container)
            .on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(GlobalSearchAction::FocusReplaceEditor);
                DispatchEventResult::StopPropagation
            })
            .finish();

        let mut header_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(query_row)
            .with_child(include_row)
            .with_child(exclude_row)
            .with_child(replace_row);

        let files = self.unique_match_count();
        let file_word = if files == 1 { "file" } else { "files" };

        let message = if self.is_search_in_progress && self.total_match_count == 0 {
            "Searching...".to_string()
        } else if !self.is_search_in_progress && self.total_match_count == 0 {
            "No results found. Try adjusting your include/exclude filters.".to_string()
        } else {
            match self.total_match_count {
                1 => format!("1 result in {files} {file_word}"),
                n => format!("{n} results in {files} {file_word}"),
            }
        };

        let match_text_styles = UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(12.),
            font_color: Some(blended_colors::text_disabled(theme, theme.background())),
            ..Default::default()
        };

        let match_text = Span::new(message, match_text_styles)
            .with_soft_wrap()
            .build()
            .finish();

        let capped_text_styles = UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(12.),
            font_color: Some(blended_colors::text_sub(theme, theme.background())),
            ..Default::default()
        };
        let capped_message = "The result set only contains a subset of all matches. Be more specific in your search to narrow down results.".to_string();
        let capped_text = Span::new(capped_message, capped_text_styles)
            .with_soft_wrap()
            .build()
            .finish();

        if self.last_searched_pattern.is_some() && self.last_error.is_none() {
            header_column = header_column.with_child(match_text);
        }

        if self.capped_matches {
            let alert_icon = ConstrainedBox::new(
                Icon::AlertTriangle
                    .to_warpui_icon(ThemeFill::Solid(
                        theme.terminal_colors().normal.yellow.into(),
                    ))
                    .finish(),
            )
            .with_width(appearance.ui_font_size())
            .with_height(appearance.ui_font_size())
            .finish();

            let alert_icon = Container::new(alert_icon).with_margin_right(4.).finish();

            let capped_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(alert_icon)
                .with_child(Shrinkable::new(1.0, capped_text).finish())
                .finish();

            header_column =
                header_column.with_child(Container::new(capped_row).with_margin_top(4.0).finish());
        }

        if self.search_results_stale {
            let stale_styles = UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(12.),
                font_color: Some(blended_colors::text_sub(theme, theme.background())),
                ..Default::default()
            };
            let stale_text = Span::new(
                "Search results are stale after replace. Run the search again to refresh.",
                stale_styles,
            )
            .with_soft_wrap()
            .build()
            .finish();
            header_column =
                header_column.with_child(Container::new(stale_text).with_margin_top(4.0).finish());
        }

        if let Some(error) = &self.replace_error {
            let error_styles = UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(12.),
                font_color: Some(theme.terminal_colors().normal.red.into()),
                ..Default::default()
            };
            let error_text = Span::new(error.clone(), error_styles)
                .with_soft_wrap()
                .build()
                .finish();
            header_column =
                header_column.with_child(Container::new(error_text).with_margin_top(4.0).finish());
        }

        let header_section = Container::new(header_column.finish())
            .with_padding(Padding::uniform(12.).with_top(4.).with_bottom(8.))
            .finish();

        let mut body = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header_section);

        let should_render_pre_search_zero_state = self.show_pre_search_zero_state
            && self.last_searched_pattern.is_none()
            && self.query_editor.as_ref(app).is_empty(app);

        if let Some(preview) = &self.replace_preview {
            body = body.with_child(
                Shrinkable::new(1.0, self.render_replace_preview(preview, app)).finish(),
            );
        } else if let Some(summary) = &self.replace_summary {
            body = body.with_child(
                Shrinkable::new(1.0, self.render_replace_summary(summary, app)).finish(),
            );
        } else if self.replace_is_generating_preview {
            body = body
                .with_child(Shrinkable::new(1.0, self.render_replace_loading_state(app)).finish());
        } else if self.replace_is_applying {
            body = body
                .with_child(Shrinkable::new(1.0, self.render_replace_applying_state(app)).finish());
        } else if should_render_pre_search_zero_state {
            body =
                body.with_child(Shrinkable::new(1.0, self.render_pre_search_state(app)).finish());
        } else if let Some(error) = &self.last_error {
            body = body.with_child(
                Shrinkable::new(1.0, self.render_search_error_state(error, app)).finish(),
            );
        } else if let Some(results) = self.render_results(app) {
            let results_section = Container::new(results)
                .with_horizontal_padding(12.)
                .finish();
            body = body.with_child(Shrinkable::new(1.0, results_section).finish());
        } else if self.is_search_in_progress {
            body = body.with_child(Shrinkable::new(1.0, self.render_loading_state(app)).finish());
        } else if self.last_searched_pattern.is_some() {
            body =
                body.with_child(Shrinkable::new(1.0, self.render_no_results_state(app)).finish());
        }

        let has_results = !self.directory_entries.is_empty();

        EventHandler::new(body.finish())
            .on_left_mouse_down(move |ctx, _, _| {
                if has_results {
                    ctx.dispatch_typed_action(GlobalSearchAction::FocusResultsList);
                } else {
                    ctx.dispatch_typed_action(GlobalSearchAction::FocusQueryEditor);
                }
                DispatchEventResult::StopPropagation
            })
            .finish()
    }
}

impl GlobalSearchView {
    fn render_replace_preview(
        &self,
        preview: &ReplacePreview,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let file_word = if preview.total_file_count() == 1 {
            "file"
        } else {
            "files"
        };
        let match_word = if preview.total_match_count() == 1 {
            "match"
        } else {
            "matches"
        };
        let summary_text = format!(
            "Preview: {} {} and {} {}",
            preview.total_file_count(),
            file_word,
            preview.total_match_count(),
            match_word
        );

        let mut preview_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(summary_text, appearance.ui_font_family(), 13.)
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(theme.foreground().into())
                    .finish(),
            );

        let actions = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&self.cancel_replace_button).finish())
            .with_child(
                Container::new(ChildView::new(&self.confirm_replace_button).finish())
                    .with_margin_left(8.)
                    .finish(),
            )
            .finish();

        preview_column = preview_column.with_child(
            Container::new(actions)
                .with_margin_top(8.)
                .with_margin_bottom(8.)
                .finish(),
        );

        let header = Container::new(preview_column.finish())
            .with_horizontal_padding(12.)
            .finish();

        let mut body = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header);

        if preview.files.is_empty() {
            body = body.with_child(
                Container::new(
                    Text::new_inline(
                        "No current matches found. Files may have changed since the search completed.",
                        appearance.ui_font_family(),
                        13.,
                    )
                    .with_color(blended_colors::text_sub(theme, theme.background()).into())
                    .finish(),
                )
                .with_horizontal_padding(12.)
                .finish(),
            );
        } else {
            let files = preview.files.clone();
            let item_count = files.len();
            let build_items = move |range: Range<usize>, app: &AppContext| {
                (range.start..range.end)
                    .map(|idx| Self::render_replace_file_preview(&files[idx], app))
                    .collect::<Vec<_>>()
                    .into_iter()
            };
            let list = UniformList::new(UniformListState::new(), item_count, build_items);
            let scrollable = Scrollable::vertical(
                self.scroll_state.clone(),
                list.finish_scrollable(),
                ScrollbarWidth::Auto,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish();

            body = body.with_child(
                Shrinkable::new(
                    1.0,
                    Container::new(scrollable)
                        .with_horizontal_padding(12.)
                        .with_padding_bottom(12.)
                        .finish(),
                )
                .finish(),
            );
        }

        body.finish()
    }

    fn render_replace_file_preview(
        file: &ReplaceFilePreview,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let path = file.path.clone();
        let file_name = file
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| file.path.to_string_lossy().to_string());
        let action_label = if file.included { "Exclude" } else { "Include" };
        let included_label = if file.included {
            "Included"
        } else {
            "Excluded"
        };

        let action_text = Text::new_inline(action_label, appearance.ui_font_family(), 12.)
            .with_color(theme.accent().into())
            .finish();
        let action = EventHandler::new(
            Container::new(action_text)
                .with_horizontal_padding(6.)
                .with_vertical_padding(2.)
                .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.surface_3()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
        )
        .on_left_mouse_down(move |ctx, _, _| {
            ctx.dispatch_typed_action(GlobalSearchAction::ToggleReplacePreviewFile {
                path: path.clone(),
            });
            DispatchEventResult::StopPropagation
        })
        .finish();

        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Text::new_inline(file_name, appearance.ui_font_family(), 13.)
                        .with_style(Properties::default().weight(Weight::Semibold))
                        .with_color(theme.foreground().into())
                        .finish(),
                )
                .finish(),
            )
            .with_child(action)
            .finish();

        let sub_text = format!("{}: {} matches", included_label, file.matches.len());

        let mut file_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header)
            .with_child(
                Text::new_inline(sub_text, appearance.ui_font_family(), 12.)
                    .with_color(blended_colors::text_sub(theme, theme.background()).into())
                    .finish(),
            );

        for matched in &file.matches {
            let mut match_column = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Text::new_inline(
                        format!(
                            "Line {}, Col {}",
                            matched.line_number, matched.column_number
                        ),
                        appearance.ui_font_family(),
                        12.,
                    )
                    .with_color(blended_colors::text_disabled(theme, theme.background()).into())
                    .finish(),
                );

            for context in &matched.context_before {
                match_column =
                    match_column.with_child(Self::render_replace_context_line(context, app));
            }
            match_column =
                match_column.with_child(Self::render_replace_context_line(&matched.line_text, app));
            for context in &matched.context_after {
                match_column =
                    match_column.with_child(Self::render_replace_context_line(context, app));
            }
            match_column = match_column
                .with_child(Self::render_replace_delta_line(
                    "-",
                    &matched.old_text,
                    false,
                    app,
                ))
                .with_child(Self::render_replace_delta_line(
                    "+",
                    &matched.replacement_text,
                    true,
                    app,
                ));

            file_column = file_column.with_child(
                Container::new(match_column.finish())
                    .with_margin_top(8.)
                    .with_padding(Padding::uniform(6.))
                    .with_background(theme.surface_1())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            );
        }

        Container::new(file_column.finish())
            .with_margin_top(8.)
            .with_padding(Padding::uniform(8.))
            .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.surface_3()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS)))
            .finish()
    }

    fn render_replace_context_line(line: &str, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Text::new_inline(line.to_string(), appearance.ui_font_family(), 12.)
            .with_color(blended_colors::text_sub(theme, theme.background()).into())
            .finish()
    }

    fn render_replace_delta_line(
        prefix: &str,
        text: &str,
        is_replacement: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let color = if is_replacement {
            theme.terminal_colors().normal.green
        } else {
            theme.terminal_colors().normal.red
        };
        Text::new_inline(format!("{prefix} {text}"), appearance.ui_font_family(), 12.)
            .with_color(color.into())
            .finish()
    }

    fn render_replace_summary(
        &self,
        summary: &ReplaceApplySummary,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    format!(
                        "Replace complete: {} files changed",
                        summary.changed_file_count()
                    ),
                    appearance.ui_font_family(),
                    14.,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.foreground().into())
                .finish(),
            );

        if !summary.skipped.is_empty() {
            column = column.with_child(
                Text::new_inline(
                    format!("{} files skipped", summary.skipped.len()),
                    appearance.ui_font_family(),
                    13.,
                )
                .with_color(theme.terminal_colors().normal.yellow.into())
                .finish(),
            );
            for skipped in &summary.skipped {
                column = column.with_child(self.render_replace_result_line(
                    &skipped.path,
                    &skipped.reason,
                    app,
                ));
            }
        }

        if !summary.failed.is_empty() {
            column = column.with_child(
                Text::new_inline(
                    format!("{} files failed", summary.failed.len()),
                    appearance.ui_font_family(),
                    13.,
                )
                .with_color(theme.terminal_colors().normal.red.into())
                .finish(),
            );
            for failed in &summary.failed {
                column = column.with_child(self.render_replace_result_line(
                    &failed.path,
                    &failed.error,
                    app,
                ));
            }
        }

        Container::new(column.finish())
            .with_horizontal_padding(12.)
            .with_padding_bottom(12.)
            .finish()
    }

    fn render_replace_result_line(
        &self,
        path: &Path,
        detail: &str,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Text::new_inline(
            format!("{}: {}", path.display(), detail),
            appearance.ui_font_family(),
            12.,
        )
        .with_color(blended_colors::text_sub(theme, theme.background()).into())
        .finish()
    }

    fn render_replace_loading_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::Search,
            "Generating preview",
            "No files are changed until the preview is confirmed.",
            app,
        )
    }

    fn render_replace_applying_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::Save,
            "Applying replace",
            "Writing included files and verifying previewed spans.",
            app,
        )
    }

    fn render_zero_state(
        &self,
        icon: Icon,
        title: impl Into<Cow<'static, str>>,
        body: impl Into<Cow<'static, str>>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let main_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        icon.to_warpui_icon(ThemeFill::Solid(internal_colors::neutral_6(theme)))
                            .finish(),
                    )
                    .with_width(24.)
                    .with_height(24.)
                    .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
            )
            .with_child(
                Text::new(
                    title,
                    appearance.ui_font_family(),
                    appearance.ui_font_size() + 2.,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                ConstrainedBox::new(
                    Shrinkable::new(
                        1.,
                        Container::new(
                            Shrinkable::new(
                                1.,
                                FormattedTextElement::from_str(
                                    body,
                                    appearance.ui_font_family(),
                                    appearance.ui_font_size() + 2.,
                                )
                                .with_alignment(TextAlignment::Center)
                                .with_color(theme.disabled_text_color(theme.background()).into())
                                .finish(),
                            )
                            .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .with_max_width(425.)
                .finish(),
            )
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., main_column).finish())
                .finish(),
        )
        .with_horizontal_margin(16.)
        .finish()
    }

    fn render_pre_search_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::Search,
            "Global search",
            "Search in files across your current directories.",
            app,
        )
    }

    fn render_loading_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::Search,
            "Searching",
            "Results will appear here as matches are found.",
            app,
        )
    }

    fn render_no_results_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::Search,
            "No results found",
            "Try adjusting your query or include/exclude filters.",
            app,
        )
    }

    fn render_search_error_state(&self, error: &str, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(Icon::AlertTriangle, "Search failed", error.to_string(), app)
    }

    fn render_unavailable_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::AlertTriangle,
            "Global search unavailable",
            "Global search requires access to your local workspace. Open a new session or navigate to an active session to view.",
            app,
        )
    }

    fn render_remote_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::AlertTriangle,
            "Global search unavailable",
            "Global search requires access to your local workspace, which isn't supported in remote sessions",
            app,
        )
    }

    fn render_unsupported_session_state(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_zero_state(
            Icon::AlertTriangle,
            "Global search unavailable",
            "Global search doesn't currently work in Git Bash or WSL.",
            app,
        )
    }
}
