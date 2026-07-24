use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use twarp_core::features::FeatureFlag;
use twarp_core::ui::theme::color::internal_colors;
use twarp_core::ui::tokens::{border, radius, spacing, type_ramp};
use twarp_core::{send_telemetry_from_ctx, ui::Icon};
use twarp_util::path::LineAndColumnArg;
use twarpui::{
    elements::{
        new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig},
        resizable_state_handle, Border, ChildView, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, DragBarSide, Element, Empty, Flex, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, ParentElement, Radius, Resizable, ResizableStateHandle,
        ScrollbarWidth, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    platform::{Cursor, FullscreenState},
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle, WindowId,
};

// twarp: 2c-d — AgentConversationsModel/AIConversationId stubs no longer needed in this file.
// twarp 07 (7b): the Claude Code chat moved to a main-content pane (re-spec #70);
// this left panel no longer hosts it.
use crate::app_state::RightToolKind;
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
use crate::util::traffic_lights::{traffic_light_data, TrafficLightSide};
use crate::window_settings::WindowSettings;
// twarp: 2c-d — conversation_list removed
use crate::workspace::view::global_search::view::{
    Event as GlobalSearchViewEvent, GlobalSearchEntryFocus, GlobalSearchView,
};
use crate::workspace::view::{
    LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME, LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME,
    LEFT_PANEL_TWARP_DRIVE_BINDING_NAME, OPEN_GLOBAL_SEARCH_BINDING_NAME,
    TOGGLE_PROJECT_EXPLORER_BINDING_NAME, TOGGLE_TWARP_DRIVE_BINDING_NAME,
};
use crate::workspace::WorkspaceAction;
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
    TelemetryEvent,
};

// twarp 08e: single-line search field for the Claude sessions list.
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};

// twarp: theme-following sidebar palette. The sidebar surface now matches the
// active tab fill (`surface_overlay_2` == fg_overlay_2), so the sidebar follows
// the active twarp theme instead of the old pinned macOS-light look. Every
// child color derives from the same theme so the panel reads correctly in both
// light and dark themes (the old §25 theme-leakage trap is gone now that the
// surface itself is theme-driven rather than a fixed light fill).

/// Primary sidebar text (session titles, active pill icon).
fn sidebar_text(appearance: &Appearance) -> twarpui::color::ColorU {
    let theme = appearance.theme();
    theme.main_text_color(theme.background()).into_solid()
}

/// Muted secondary text (section headers, timestamps, inactive pill icons).
fn sidebar_subtext(appearance: &Appearance) -> twarpui::color::ColorU {
    let theme = appearance.theme();
    theme.sub_text_color(theme.background()).into_solid()
}

/// Soft row hover/selection highlight, layered over the sidebar surface.
fn sidebar_row_highlight(appearance: &Appearance) -> twarpui::color::ColorU {
    appearance.theme().surface_overlay_2().into_solid()
}

/// Recessed track behind the segmented tool switcher (and the search border).
fn sidebar_pill_track(appearance: &Appearance) -> twarpui::color::ColorU {
    appearance.theme().surface_1().into_solid()
}

/// twarp 08f polish: shared horizontal content inset for the sidebar panels
/// (file tree, Warp Drive). The macOS sidebar gives rows a little breathing
/// room from the edge instead of the old flush 2px; the sessions/shortcuts
/// panels keep their own 10px text inset.
const SIDEBAR_CONTENT_INSET: f32 = 8.;
/// twarp 08f polish: bottom inset so the last row / Timeline section doesn't
/// sit flush against the window edge.
const SIDEBAR_BOTTOM_INSET: f32 = 8.;

#[derive(Default)]
struct MouseStateHandles {
    /// Opens the dedicated Search tool from the Files header.
    global_search_button: MouseStateHandle,
    // twarp: 2c-d — conversation_list_view_button removed
    // twarp: shortcuts moved to Settings; its toolbelt mouse states removed.
    // twarp sidebar rework: project_explorer/claude_sessions toolbelt
    // buttons removed with the segmented switcher.
}

#[derive(Clone, Debug)]
pub enum LeftPanelAction {
    ProjectExplorer,
    GlobalSearch {
        entry_focus: GlobalSearchEntryFocus,
    },
    TwarpDrive,
    // twarp: the Shortcuts panel + its inline editor actions moved to
    // Settings > Shortcuts (see settings_view::shortcuts_page).
    /// twarp 07 (7h, PRODUCT §35): switch the panel to the Claude Code
    /// session list.
    ClaudeSessions,
    /// twarp 07 (7h, PRODUCT §36): resume the stored session at this index
    /// of the current list in a main-content Claude Code pane.
    ClaudeSessionResume(usize),
    /// twarp: clear the session-list search field (the inline X button that
    /// appears once the query is non-empty).
    ClaudeSessionsClearSearch,
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
    TwarpDrive(DrivePanelEvent),
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
    /// twarp 07 (7h, PRODUCT §36): open a Claude Code pane resuming this
    /// stored session. The workspace handler routes it through the same
    /// pane-opening path as the `claude` terminal trigger.
    ResumeClaudeSession {
        session_id: String,
        jsonl_path: PathBuf,
        cwd: PathBuf,
        provider: claude_code::driver::AgentProvider,
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
    TwarpDrive,
    /// Custom command shortcuts panel (PRODUCT 04 §26). The full GUI lives
    /// in a future sub-phase; 4c renders the tab plus a placeholder so the
    /// integration lights up.
    Shortcuts,
    /// twarp 07 (7h, PRODUCT §35): read-only list of the cwd's stored Claude
    /// Code sessions (from `claude`'s own `~/.claude/projects` store). Hosts
    /// NO chat — selecting an entry opens a main-content pane via
    /// `claude --resume`. The toolbelt button only shows when sessions exist.
    ClaudeSessions,
    // twarp: 2c-d — variant kept so legacy call-sites compile; AI conversation list deleted.
    ConversationListView,
}

/// Encapsulates the active view state to enforce that all mutations go through
/// `active_view_state::set`, which handles necessary side effects.
mod active_view_state {
    use super::ToolPanelView;
    use twarpui::ViewContext;

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
    pub icon: twarp_core::ui::Icon,
    /// Optional icon to use when the given toolbelt option is in an active state.
    pub active_icon: Option<twarp_core::ui::Icon>,
    /// Short label shown next to the icon in the toolbelt segment (e.g.
    /// "Files", "Search", "Code").
    pub label: String,
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
    window_id: WindowId,
    mouse_state_handles: MouseStateHandles,
    twarp_drive_view: ViewHandle<DrivePanel>,
    // twarp: 2c-d — conversation_list_view removed
    active_view: active_view_state::ActiveViewState,
    toolbelt_buttons: Vec<ToolbeltButtonConfig>,
    active_pane_group: Option<WeakViewHandle<PaneGroup>>,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    working_directories_model: ModelHandle<WorkingDirectoriesModel>,
    panel_position: super::PanelPosition,
    // twarp: shortcuts list/editor state moved to Settings > Shortcuts
    // (settings_view::shortcuts_page).
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
    timeline_scroll_state: twarpui::elements::ClippedScrollStateHandle,
    /// Per-entry stable mouse states. Grown lazily inside `render_*`
    /// since `Hoverable::on_click` requires the same handle across
    /// renders to detect click cycles.
    timeline_entry_mouse_states: std::cell::RefCell<Vec<MouseStateHandle>>,

    /// twarp 07 (7h, PRODUCT §35): the active cwd's stored Claude Code
    /// sessions, refreshed when the tab opens or the cwd changes. Read-only —
    /// `claude` owns the store.
    claude_sessions: Vec<claude_code::sessions::StoredSession>,
    /// Whether the active cwd has any stored sessions — gates the toolbelt
    /// button (§35: the entry appears only when sessions exist). Kept by an
    /// existence-only probe on directory changes; never lists in render.
    has_claude_sessions: bool,
    /// Per-row stable mouse states (same pattern as the Timeline's).
    claude_session_row_mouse_states: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// twarp: vertical scroll state for the sessions list so a long list scrolls
    /// instead of overflowing the panel (same pattern as the Timeline).
    claude_sessions_scroll_state: twarpui::elements::ClippedScrollStateHandle,
    /// twarp 08e (PRODUCT §17–§20): live search field over the Claude sessions
    /// list. A single-line `EditorView`; the panel reads its `buffer_text` each
    /// render and case-insensitive substring-filters `claude_sessions` by
    /// `session.title`. Transient view state — never touches persisted data.
    claude_sessions_search: ViewHandle<EditorView>,
    /// twarp: mouse state for the inline X button that clears the session-list
    /// search; the button only renders while the query is non-empty.
    claude_sessions_clear_mouse_state: MouseStateHandle,
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
    /// `(owner, repo)` parsed from `git remote get-url origin` for the
    /// focused file's repo. `None` for non-GitHub remotes (Gitlab,
    /// Bitbucket, self-hosted, …) — those skip the API and fall back
    /// to noreply-email parsing + Gravatar. Recomputed when the
    /// focused file's repo root changes.
    github_repo: Option<(String, String)>,
    /// Per-SHA cache of GitHub commits-API author info. `Some(Some)` =
    /// resolved with author data; `Some(None)` = resolved 404 (don't
    /// retry); not present = not yet fetched. Lives for the lifetime
    /// of the `LeftPanelView` so switching between files reuses
    /// previously-fetched authors.
    commit_author_cache: HashMap<String, Option<crate::code_review::github_author::GithubAuthor>>,
    /// `Some(Some)` once we've resolved a token (env var or
    /// `gh auth token`); `Some(None)` if we tried and failed; `None`
    /// = not yet tried. Cached for the lifetime of `LeftPanelView`
    /// to avoid spawning a `gh` subprocess per fetch.
    github_token: Option<Option<String>>,
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
        let width_kind = if super::project_sidebar_enabled() {
            ModalType::FilesToolWidth
        } else {
            ModalType::LeftPanelWidth
        };
        let resizable_state_handle = match resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), width_kind)
        {
            Some(handle) => handle,
            None => {
                log::error!("Couldn't retrieve left panel resizable state handle.");
                resizable_state_handle(600.0)
            }
        };
        let twarp_drive_view = ctx.add_typed_action_view(DrivePanel::new);

        ctx.subscribe_to_view(&twarp_drive_view, |_me, _, event, ctx| {
            ctx.emit(LeftPanelEvent::TwarpDrive(event.clone()));
        });

        // twarp: 2c-d — conversation_list_view subscription removed

        let active_view = views.first().copied().unwrap_or(ToolPanelView::TwarpDrive);
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

                // twarp 07 (7h): the session-list entry follows the active
                // cwd (PRODUCT §35) — existence probe for the toolbelt
                // button; full re-list only while the tab is open.
                me.has_claude_sessions = me
                    .active_claude_sessions_cwd(ctx)
                    .is_some_and(|cwd| claude_code::sessions::has_sessions(&cwd));
                if me.active_view.get() == ToolPanelView::ClaudeSessions {
                    me.refresh_claude_sessions(ctx);
                }
                ctx.notify();
            }
        });

        // twarp 08e: the sessions search field. Mirrors keybindings_page.rs:94.
        let claude_sessions_search = {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_font_size(appearance),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            ctx.add_typed_action_view(|ctx| EditorView::single_line(options, ctx))
        };
        // Re-render the panel live as the query changes (PRODUCT §17).
        ctx.subscribe_to_view(&claude_sessions_search, |_me, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                ctx.notify();
            }
        });
        claude_sessions_search.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text("Search sessions", ctx);
        });

        let mut view = Self {
            resizable_state_handle,
            window_id: ctx.window_id(),
            mouse_state_handles: Default::default(),
            twarp_drive_view,
            // twarp: 2c-d — conversation_list_view removed
            active_view: active_view_state::new(active_view),
            toolbelt_buttons,
            active_pane_group: None,
            working_directories_model,
            panel_position: super::PanelPosition::Left,
            // 5d: Project Explorer Timeline section. Collapsed by
            // default; user expands via the chevron. PRODUCT §§18–23.
            timeline_state: TimelineSectionState::default(),
            timeline_expanded: false,
            // Default height = 220px when expanded; bounded to
            // (60, 70% of window) in the Resizable callback.
            timeline_resizable_handle: twarpui::elements::resizable_state_handle(220.0),
            timeline_header_mouse_state: MouseStateHandle::default(),
            timeline_load_more_mouse_state: MouseStateHandle::default(),
            timeline_scroll_state: twarpui::elements::ClippedScrollStateHandle::default(),
            timeline_entry_mouse_states: std::cell::RefCell::new(Vec::new()),
            claude_sessions: Vec::new(),
            has_claude_sessions: false,
            claude_session_row_mouse_states: std::cell::RefCell::new(Vec::new()),
            claude_sessions_scroll_state: twarpui::elements::ClippedScrollStateHandle::default(),
            claude_sessions_search,
            claude_sessions_clear_mouse_state: MouseStateHandle::default(),
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
                    label: "Files".to_string(),
                    tooltip_text: "Project explorer".to_string(),
                    action: LeftPanelAction::ProjectExplorer,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            // twarp sidebar rework: Search is no longer a sidebar tab — it
            // is a collapsible section at the top of the Files view. The
            // variant stays only for persisted-tab back-compat (a legacy
            // selection falls back to Project Explorer in
            // `update_available_views`), so this arm is a never-rendered stub.
            ToolPanelView::GlobalSearch { .. } => ToolbeltButtonConfig {
                icon: Icon::Search,
                active_icon: None,
                label: String::new(),
                tooltip_text: String::new(),
                action: LeftPanelAction::ProjectExplorer,
                render_with_active_state: false,
                tooltip_keybinding: None,
                tooltip_keybinding_names: vec![],
            },
            ToolPanelView::TwarpDrive => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_TWARP_DRIVE_BINDING_NAME,
                    TOGGLE_TWARP_DRIVE_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::TwarpDrive,
                    active_icon: None,
                    label: "Twarp Drive".to_string(),
                    tooltip_text: "Twarp Drive".to_string(),
                    action: LeftPanelAction::TwarpDrive,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            // twarp: Shortcuts moved to Settings; this toolbelt entry is never
            // produced (compute_left_panel_views no longer pushes it). The arm
            // stays only because the ToolPanelView variant is kept for
            // persisted-tab back-compat. Fall back to the ProjectExplorer
            // action like the ConversationListView stub below.
            ToolPanelView::Shortcuts => ToolbeltButtonConfig {
                icon: Icon::Keyboard,
                active_icon: None,
                label: String::new(),
                tooltip_text: String::new(),
                action: LeftPanelAction::ProjectExplorer,
                render_with_active_state: false,
                tooltip_keybinding: None,
                tooltip_keybinding_names: vec![],
            },
            // twarp sidebar rework: the session-list tab is gone — the Claude
            // pane welcome screen replaces it. Variant kept for persisted-tab
            // back-compat only; never-rendered stub falling back to Files.
            ToolPanelView::ClaudeSessions => ToolbeltButtonConfig {
                icon: Icon::History,
                active_icon: None,
                label: String::new(),
                tooltip_text: String::new(),
                action: LeftPanelAction::ProjectExplorer,
                render_with_active_state: false,
                tooltip_keybinding: None,
                tooltip_keybinding_names: vec![],
            },
            // twarp: 2c-d — ConversationListView arm: AI deleted, use ProjectExplorer config as fallback.
            ToolPanelView::ConversationListView => ToolbeltButtonConfig {
                icon: Icon::FileCopy,
                active_icon: None,
                label: String::new(),
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
        pane_group_id: twarpui::EntityId,
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
        pane_group_id: twarpui::EntityId,
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

    /// Route focus into the active project's GlobalSearchView, creating it
    /// lazily when the Search tool is opened for the first time.
    fn focus_project_search(
        &mut self,
        entry_focus: GlobalSearchEntryFocus,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_group) = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(ctx))
        else {
            return;
        };
        let global_search_view =
            self.get_or_create_global_search_view_for_pane_group(pane_group.id(), ctx);
        global_search_view.update(ctx, |view, ctx| {
            view.on_left_panel_focused(entry_focus, ctx);
        });
        ctx.focus(&global_search_view);
    }

    pub(crate) fn open_project_search(
        &mut self,
        entry_focus: GlobalSearchEntryFocus,
        ctx: &mut ViewContext<Self>,
    ) {
        active_view_state::set(self, ToolPanelView::GlobalSearch { entry_focus }, ctx);
        self.focus_project_search(entry_focus, ctx);
        ctx.notify();
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

    pub fn is_twarp_drive_active(&self) -> bool {
        self.active_view.get() == ToolPanelView::TwarpDrive
    }

    pub fn is_file_tree_active(&self) -> bool {
        self.active_view.get() == ToolPanelView::ProjectExplorer
    }

    pub fn twarp_drive_view(&self) -> &ViewHandle<DrivePanel> {
        &self.twarp_drive_view
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
        // twarp sidebar rework: legacy persisted tabs (Search / Code /
        // Shortcuts / conversation list) no longer exist — fall back to the
        // Files view, mirroring `update_available_views`.
        let view = match view {
            ToolPanelView::GlobalSearch { .. }
            | ToolPanelView::ClaudeSessions
            | ToolPanelView::Shortcuts
            | ToolPanelView::ConversationListView => ToolPanelView::ProjectExplorer,
            other => other,
        };
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

        // twarp 07 (7h): the session list follows the active pane group's cwd
        // (PRODUCT §35) — re-probe on every group switch.
        self.has_claude_sessions = self
            .active_claude_sessions_cwd(ctx)
            .is_some_and(|cwd| claude_code::sessions::has_sessions(&cwd));
        if self.active_view.get() == ToolPanelView::ClaudeSessions {
            self.refresh_claude_sessions(ctx);
        }

        ctx.notify();
    }

    /// twarp 07 (7h, PRODUCT §35): the most-recent working directory of the
    /// active pane group — the cwd whose stored Claude Code sessions the
    /// session list shows. `None` when no pane group is active yet.
    fn active_claude_sessions_cwd(&self, ctx: &AppContext) -> Option<PathBuf> {
        let group = self.active_pane_group.as_ref()?.upgrade(ctx)?;
        self.working_directories_model
            .as_ref(ctx)
            .most_recent_directories_for_pane_group(group.id())?
            .next()
            .map(|dir| dir.path)
    }

    /// twarp 07 (7h): re-list the active cwd's stored sessions (PRODUCT §35).
    /// Read-only over `claude`'s own store; called when the tab opens or the
    /// cwd changes — never from render.
    fn refresh_claude_sessions(&mut self, ctx: &AppContext) {
        self.claude_sessions = match self.active_claude_sessions_cwd(ctx) {
            Some(cwd) => claude_code::sessions::list_sessions(&cwd),
            None => Vec::new(),
        };
        self.has_claude_sessions = !self.claude_sessions.is_empty();
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
            ToolPanelView::TwarpDrive => {
                ctx.focus(&self.twarp_drive_view);
                self.twarp_drive_view.update(ctx, |view, ctx| {
                    view.reset_focused_index_in_twarp_drive(true, ctx);
                });
            }
            // 4c stub: Shortcuts panel has no internal child view to focus
            // yet. Full GUI (list, detail editor) lands in a follow-up.
            ToolPanelView::Shortcuts => {}
            // twarp 07 (7h): the session list is a plain click-to-resume
            // list — no internal child view to focus.
            ToolPanelView::ClaudeSessions => {}
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

        // Ignore our own commit-diff temp files. Opening one via a
        // Timeline click steals editor focus and fires
        // `ActiveFileChanged(<temp_path>)`; without this guard
        // `refocus_timeline` would see the temp file (outside any
        // repo) and `clear_timeline` would wipe the section out from
        // under the user. Keep Timeline anchored on the real file.
        if is_timeline_commit_diff_temp_path(&path) {
            return;
        }

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

        let repo_changed = self.timeline_state.repo_path.as_deref() != Some(repo_root.as_path());
        self.timeline_state.repo_path = Some(repo_root.clone());
        self.timeline_state.focused_path = Some(path.clone());
        self.timeline_state.entries.clear();
        self.timeline_state.has_more = true;
        self.timeline_state.loading = true;
        self.timeline_state.local_only_shas.clear();
        self.timeline_entry_mouse_states.borrow_mut().clear();
        if repo_changed {
            // Per-repo state — wipe when switching repos. SHA cache
            // stays because SHAs are globally unique.
            self.timeline_state.github_repo = None;
            self.fetch_github_origin(repo_root.clone(), ctx);
        }
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
                let shas: Vec<String> = entries.iter().map(|e| e.sha.clone()).collect();
                me.timeline_state.entries = entries;
                ctx.notify();
                me.fetch_github_authors(shas, ctx);
            },
        );
    }

    /// Run `git remote get-url origin` and cache the resulting
    /// `(owner, repo)` if it's a GitHub URL. Skipped for non-GitHub
    /// remotes — those leave `github_repo` as `None` and we fall back
    /// to the noreply/Gravatar avatar path.
    #[cfg(feature = "local_fs")]
    fn fetch_github_origin(&mut self, repo_root: PathBuf, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move {
                let url =
                    crate::util::git::run_git_command(&repo_root, &["remote", "get-url", "origin"])
                        .await
                        .ok()?;
                crate::code_review::github_author::parse_github_origin(&url)
            },
            |me, parsed: Option<(String, String)>, ctx| {
                me.timeline_state.github_repo = parsed;
                ctx.notify();
                if me.timeline_state.github_repo.is_some() {
                    // Fan out for any entries already loaded.
                    let shas: Vec<String> = me
                        .timeline_state
                        .entries
                        .iter()
                        .map(|e| e.sha.clone())
                        .collect();
                    me.fetch_github_authors(shas, ctx);
                }
            },
        );
    }

    /// Kick off GitHub commits-API author lookups for `shas` not yet
    /// in the cache. Each commit gets its own async task — results
    /// drop into the cache and trigger a re-render as they arrive.
    /// No-op when `github_repo` isn't resolved yet (the caller fires
    /// us again from `fetch_github_origin` once it lands).
    #[cfg(feature = "local_fs")]
    fn fetch_github_authors(&mut self, shas: Vec<String>, ctx: &mut ViewContext<Self>) {
        let Some((owner, repo)) = self.timeline_state.github_repo.clone() else {
            return;
        };
        let Some(repo_path) = self.timeline_state.repo_path.clone() else {
            return;
        };
        let needed: Vec<String> = shas
            .into_iter()
            .filter(|s| !self.timeline_state.commit_author_cache.contains_key(s))
            .collect();
        if needed.is_empty() {
            return;
        }
        // Resolve the token first so the fan-out can use it. After the
        // token lookup completes, `dispatch_github_author_fetches` is
        // re-invoked with the same `needed` list. Subsequent calls
        // short-circuit since `github_token` is now `Some`.
        if self.timeline_state.github_token.is_none() {
            let repo_path_for_token = repo_path.clone();
            ctx.spawn(
                async move {
                    crate::code_review::github_author::resolve_github_token(&repo_path_for_token)
                        .await
                },
                move |me, token: Option<String>, ctx| {
                    me.timeline_state.github_token = Some(token);
                    me.dispatch_github_author_fetches(needed, owner, repo, ctx);
                },
            );
            return;
        }
        self.dispatch_github_author_fetches(needed, owner, repo, ctx);
    }

    /// Internal: fire one spawn per SHA. Assumes `github_token` is
    /// already resolved (either Some or None — both are valid for the
    /// unauthenticated path).
    #[cfg(feature = "local_fs")]
    fn dispatch_github_author_fetches(
        &mut self,
        shas: Vec<String>,
        owner: String,
        repo: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let token = self
            .timeline_state
            .github_token
            .as_ref()
            .and_then(|t| t.clone());
        for sha in shas {
            let owner = owner.clone();
            let repo = repo.clone();
            let token = token.clone();
            let sha_for_async = sha.clone();
            ctx.spawn(
                async move {
                    let client = http_client::Client::default();
                    let result = crate::code_review::github_author::fetch_commit_author(
                        &client,
                        &owner,
                        &repo,
                        &sha_for_async,
                        token.as_deref(),
                    )
                    .await;
                    (sha_for_async, result)
                },
                |me, (sha, result), ctx| {
                    match result {
                        Ok(author) => {
                            me.timeline_state.commit_author_cache.insert(sha, author);
                            ctx.notify();
                        }
                        Err(err) => {
                            // Don't cache failures — let a future
                            // refocus retry (e.g. after the user sets
                            // up `gh auth`). Log at debug to avoid
                            // spamming on rate-limit hits.
                            log::debug!("[twarp 5d] github author fetch failed for {sha}: {err}");
                        }
                    }
                },
            );
        }
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
                let new_shas: Vec<String> = entries.iter().map(|e| e.sha.clone()).collect();
                me.timeline_state.entries.extend(entries);
                ctx.notify();
                me.fetch_github_authors(new_shas, ctx);
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
        // Only the **body** goes into the Resizable. The header
        // stays outside so dragging the resize handle (which sits
        // at the body's top edge, between header and entries)
        // doesn't collide with the header's `Hoverable::on_click` —
        // a drag that ends with mouse-up on the header would
        // otherwise toggle the section closed.
        let body = self.render_timeline_body(appearance);
        let resizable_body = Resizable::new(self.timeline_resizable_handle.clone(), body)
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
            .finish();
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(resizable_body)
            .finish()
    }

    /// Section header bar: chevron + `TIMELINE` label + optional
    /// focused-file basename. Click anywhere on the bar toggles
    /// expanded.
    fn render_timeline_header(&self, appearance: &Appearance) -> Box<dyn Element> {
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
        // twarp: theme-following muted header color; the sidebar surface now
        // follows the theme so sub_text_color reads correctly against it.
        .with_color(sidebar_subtext(appearance))
        .with_style(Properties::default().weight(Weight::Semibold))
        .soft_wrap(false)
        .finish();
        let hover_bg = sidebar_row_highlight(appearance);
        // twarp 08 (review): no padding around the TIMELINE header — the row
        // hugs its text with no surrounding inset.
        Hoverable::new(self.timeline_header_mouse_state.clone(), move |state| {
            let mut container =
                Container::new(text).with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if state.is_hovered() {
                container = container.with_background(twarp_core::ui::theme::Fill::Solid(hover_bg));
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
            twarpui::elements::Fill::None,
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

        let author_display = self.author_display_label(entry);
        let avatar = self.render_timeline_avatar(entry, appearance);

        let ts_local = chrono::DateTime::<chrono::Utc>::from_timestamp(entry.timestamp, 0)
            .map(crate::util::time_format::format_approx_duration_from_now_utc)
            .unwrap_or_else(|| "unknown".to_string());

        let mut meta_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        meta_row.add_child(
            Text::new(
                author_display,
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
                container = container.with_background(twarp_core::ui::theme::Fill::Solid(hover_bg));
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
    /// Display label for a Timeline row's author. Tries (in order):
    /// 1. GitHub `login` from the commits-API cache — real handle as
    ///    GitHub knows it.
    /// 2. Username extracted from the noreply email
    ///    (`<id>+<username>@users.noreply.github.com`).
    /// 3. Git's `author.name` as the last-resort fallback.
    fn author_display_label(&self, entry: &crate::code_review::timeline::TimelineEntry) -> String {
        if let Some(Some(author)) = self.timeline_state.commit_author_cache.get(&entry.sha) {
            return author.login.clone();
        }
        if let Some(username) = noreply_username_from_email(&entry.author_email) {
            return username;
        }
        entry.author_name.clone()
    }

    /// Avatar URL for a Timeline row. Cached GitHub `avatar_url` wins;
    /// otherwise noreply username → `github.com/<u>.png`; otherwise
    /// Gravatar identicon fallback. Always returns a non-empty URL so
    /// the `Avatar` widget renders something while loading.
    fn avatar_url_for_entry(&self, entry: &crate::code_review::timeline::TimelineEntry) -> String {
        if let Some(Some(author)) = self.timeline_state.commit_author_cache.get(&entry.sha) {
            return author.avatar_url.clone();
        }
        avatar_url_for_email(&entry.author_email)
    }

    fn render_timeline_avatar(
        &self,
        entry: &crate::code_review::timeline::TimelineEntry,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        use crate::ui_components::avatar::{Avatar, AvatarContent};
        let url = self.avatar_url_for_entry(entry);
        let display_name = self.author_display_label(entry);
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
                container = container.with_background(twarp_core::ui::theme::Fill::Solid(neutral));
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

/// Build an avatar URL for a commit author. PRODUCT §19.
///
/// GitHub now enables a privacy-preserving noreply email by default
/// (`<id>+<username>@users.noreply.github.com`, or the older bare
/// `<username>@users.noreply.github.com`). For commits made under
/// those addresses we resolve the username and hit GitHub's avatar
/// endpoint directly — no API call, no auth, and the user gets their
/// real profile picture. For everything else we fall back to Gravatar
/// with `d=identicon` so emails without a Gravatar account still get
/// a deterministic identicon instead of a blank.
///
/// `s=44` requests 2x the 22px avatar slot so the image stays crisp on
/// retina displays.
#[cfg(feature = "local_fs")]
fn avatar_url_for_email(email: &str) -> String {
    const SIZE: u32 = 44;
    let trimmed = email.trim();
    if let Some(prefix) = trimmed.strip_suffix("@users.noreply.github.com") {
        // `<id>+<username>` (new) or just `<username>` (legacy). The
        // numeric ID prefix is purely an anti-spam token; the username
        // after the `+` is what GitHub serves the avatar at.
        let username = prefix.split_once('+').map_or(prefix, |(_, u)| u);
        if !username.is_empty()
            && username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return format!("https://github.com/{username}.png?size={SIZE}");
        }
    }
    use sha2::{Digest, Sha256};
    let normalized = trimmed.to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("https://www.gravatar.com/avatar/{hex}?d=identicon&s={SIZE}")
}

#[cfg(not(feature = "local_fs"))]
fn avatar_url_for_email(_email: &str) -> String {
    String::new()
}

/// Extract the GitHub username from a noreply commit email. Returns
/// `None` for non-noreply addresses and for the (rare) case where the
/// local part isn't a valid GitHub-handle character set. Used as the
/// fallback display label when the GitHub commits API hasn't yet
/// (or won't ever) populate the per-SHA author cache.
fn noreply_username_from_email(email: &str) -> Option<String> {
    let trimmed = email.trim();
    let prefix = trimmed.strip_suffix("@users.noreply.github.com")?;
    let username = prefix.split_once('+').map_or(prefix, |(_, u)| u);
    if username.is_empty()
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    Some(username.to_string())
}

/// True if `path` looks like a commit-diff temp file we wrote on a
/// Timeline click. `WorkspaceView::open_timeline_commit_diff` creates
/// `$TMPDIR/twarp-timeline-<random>/<sha>-<basename>`; we recognize
/// our own files by the `twarp-timeline-` directory prefix and short-
/// circuit `refocus_timeline` so opening one doesn't clear the
/// section. Walking the parent chain (not just `path.parent()`) keeps
/// this robust if future code nests files one level deeper.
fn is_timeline_commit_diff_temp_path(path: &Path) -> bool {
    let mut current = path.parent();
    while let Some(p) = current {
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("twarp-timeline-") {
                return true;
            }
        }
        current = p.parent();
    }
    false
}

#[cfg(test)]
mod timeline_temp_path_tests {
    use super::is_timeline_commit_diff_temp_path;
    use std::path::PathBuf;

    #[test]
    fn matches_direct_child_of_temp_subdir() {
        let p = PathBuf::from("/var/folders/_/twarp-timeline-abcdef/abc1234-main.rs");
        assert!(is_timeline_commit_diff_temp_path(&p));
    }

    #[test]
    fn matches_nested_under_temp_subdir() {
        let p = PathBuf::from("/tmp/twarp-timeline-xyz/sub/abc1234-main.rs");
        assert!(is_timeline_commit_diff_temp_path(&p));
    }

    #[test]
    fn rejects_unrelated_path() {
        let p = PathBuf::from("/Users/me/code/repo/src/main.rs");
        assert!(!is_timeline_commit_diff_temp_path(&p));
    }

    #[test]
    fn rejects_similar_but_different_prefix() {
        let p = PathBuf::from("/tmp/twarp-other-thing/foo.rs");
        assert!(!is_timeline_commit_diff_temp_path(&p));
    }
}

#[cfg(test)]
mod noreply_username_tests {
    use super::noreply_username_from_email;

    #[test]
    fn extracts_username_from_id_form() {
        assert_eq!(
            noreply_username_from_email("12345+octocat@users.noreply.github.com"),
            Some("octocat".to_string())
        );
    }

    #[test]
    fn extracts_username_from_bare_form() {
        assert_eq!(
            noreply_username_from_email("octocat@users.noreply.github.com"),
            Some("octocat".to_string())
        );
    }

    #[test]
    fn returns_none_for_regular_email() {
        assert_eq!(noreply_username_from_email("ada@example.com"), None);
    }

    #[test]
    fn returns_none_for_garbage_in_local_part() {
        assert_eq!(
            noreply_username_from_email("weird name@users.noreply.github.com"),
            None
        );
    }

    #[test]
    fn returns_none_for_empty_username() {
        assert_eq!(
            noreply_username_from_email("@users.noreply.github.com"),
            None
        );
    }
}

#[cfg(all(test, feature = "local_fs"))]
mod avatar_url_tests {
    use super::avatar_url_for_email;

    #[test]
    fn noreply_with_id_resolves_to_github_avatar() {
        let url = avatar_url_for_email("123456789+octocat@users.noreply.github.com");
        assert_eq!(url, "https://github.com/octocat.png?size=44");
    }

    #[test]
    fn noreply_without_id_resolves_to_github_avatar() {
        let url = avatar_url_for_email("octocat@users.noreply.github.com");
        assert_eq!(url, "https://github.com/octocat.png?size=44");
    }

    #[test]
    fn noreply_with_invalid_username_falls_through_to_gravatar() {
        // Spaces/symbols in the local part shouldn't reach the GitHub
        // URL — we'd 404 and render the initial fallback forever.
        let url = avatar_url_for_email("weird name@users.noreply.github.com");
        assert!(url.starts_with("https://www.gravatar.com/avatar/"));
    }

    #[test]
    fn real_email_uses_gravatar() {
        let url = avatar_url_for_email("octocat@example.com");
        assert!(url.starts_with("https://www.gravatar.com/avatar/"));
        assert!(url.contains("d=identicon"));
    }

    #[test]
    fn email_normalization_lowercases_and_trims() {
        let mixed = avatar_url_for_email("  Octocat@Example.com  ");
        let lower = avatar_url_for_email("octocat@example.com");
        assert_eq!(mixed, lower);
    }
}

impl Entity for LeftPanelView {
    type Event = LeftPanelEvent;
}

impl LeftPanelView {
    fn update_button_active_states(&mut self) {
        for button in &mut self.toolbelt_buttons {
            button.render_with_active_state = match &button.action {
                LeftPanelAction::ProjectExplorer => {
                    self.active_view.get() == ToolPanelView::ProjectExplorer
                }
                LeftPanelAction::GlobalSearch { .. } => {
                    matches!(self.active_view.get(), ToolPanelView::GlobalSearch { .. })
                }
                LeftPanelAction::TwarpDrive => self.active_view.get() == ToolPanelView::TwarpDrive,
                LeftPanelAction::ClaudeSessions => {
                    self.active_view.get() == ToolPanelView::ClaudeSessions
                }
                // twarp: 2c-d — ConversationListView arm kept for legacy call-sites; AI deleted.
                LeftPanelAction::ConversationListView => false,
                // 5d: Timeline actions stay in the panel scope; no
                // force-open semantics. PRODUCT §§18–23.
                LeftPanelAction::TimelineToggleExpanded
                | LeftPanelAction::TimelineLoadMore
                | LeftPanelAction::TimelineSelectCommit { .. } => false,
                // twarp 07 (7h): row clicks are not a toolbelt tab.
                LeftPanelAction::ClaudeSessionResume(_) => false,
                // twarp: the search-clear X is not a toolbelt tab.
                LeftPanelAction::ClaudeSessionsClearSearch => false,
            };
        }
    }

    /// twarp 07 (7h, PRODUCT §35): the Claude Code session list for the
    /// active cwd. Read-only — one row per stored session (title snippet +
    /// relative timestamp), click to resume in a main-content pane (§36).
    /// No chat lives here. Empty cwd → a muted hint, never an empty chat.
    fn render_claude_sessions_panel(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // twarp 08e: read the live search query and filter the list by title
        // (case-insensitive substring). Empty query → full list (PRODUCT §18).
        let query = self.claude_sessions_search.as_ref(app).buffer_text(app);
        let matched_indices = filter_session_indices(&self.claude_sessions, &query);
        let has_query = !query.trim().is_empty();

        // twarp 08f: muted macOS section header (smaller, lighter).
        let heading = appearance
            .ui_builder()
            .span("Sessions".to_owned())
            .with_style(UiComponentStyles {
                font_color: Some(sidebar_subtext(appearance)),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish();

        // twarp 08e: the search field, above the heading/rows (PRODUCT §17).
        // Uses the same bordered, fill-less input container as the Global
        // Search page (radius 6, 1px surface_3 border, uniform 6 padding) so
        // the two side-panel search bars match.
        // twarp: when the query is non-empty, an inline X button clears it.
        let editor_cell =
            Shrinkable::new(1.0, ChildView::new(&self.claude_sessions_search).finish()).finish();
        let mut search_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.0)
            .with_child(editor_cell);
        if has_query {
            let clear_fill = appearance
                .theme()
                .sub_text_color(appearance.theme().background());
            let clear_button = Hoverable::new(
                self.claude_sessions_clear_mouse_state.clone(),
                move |state| {
                    let fill = if state.is_hovered() {
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().background())
                    } else {
                        clear_fill
                    };
                    ConstrainedBox::new(Icon::X.to_warpui_icon(fill).finish())
                        .with_width(14.)
                        .with_height(14.)
                        .finish()
                },
            )
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(LeftPanelAction::ClaudeSessionsClearSearch);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
            search_row = search_row.with_child(clear_button);
        }
        let search_field = Container::new(search_row.finish())
            .with_border(Border::all(1.).with_border_fill(appearance.theme().surface_3()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_padding_left(6.)
            .with_padding_right(6.)
            .with_padding_top(6.)
            .with_padding_bottom(6.)
            .finish();

        let body: Box<dyn Element> = if self.claude_sessions.is_empty() {
            // Zero stored sessions (PRODUCT §35 first-run empty state).
            appearance
                .ui_builder()
                .span(
                    "No Claude Code sessions in this directory yet. Run `claude` in a terminal \
                     to start one; finished sessions show up here for reopening.",
                )
                .with_soft_wrap()
                .with_style(UiComponentStyles {
                    font_color: Some(sidebar_subtext(appearance)),
                    ..Default::default()
                })
                .build()
                .finish()
        } else if has_query && matched_indices.is_empty() {
            // twarp 08e (PRODUCT §18): a distinct no-match empty state.
            appearance
                .ui_builder()
                .span("No matching sessions".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(sidebar_subtext(appearance)),
                    ..Default::default()
                })
                .build()
                .finish()
        } else {
            // Render the matched rows, preserving the ORIGINAL index so the
            // resume handler (PRODUCT §20) targets the right session.
            let rows = matched_indices
                .into_iter()
                .map(|idx| self.render_claude_session_row(idx, &self.claude_sessions[idx], app));
            let rows_column = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(2.0)
                .with_children(rows)
                .finish();
            // twarp: scroll a long sessions list instead of overflowing the
            // panel (same NewScrollable pattern as the Timeline body).
            let theme = appearance.theme();
            NewScrollable::vertical(
                SingleAxisConfig::Clipped {
                    handle: self.claude_sessions_scroll_state.clone(),
                    child: rows_column,
                },
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                twarpui::elements::Fill::None,
            )
            .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            .finish()
        };

        let column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.0)
            .with_child(search_field)
            .with_child(heading)
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

    /// One session row: first-message snippet + relative timestamp, hover +
    /// click to resume (PRODUCT §35–§36). Mouse states grow lazily, same
    /// pattern as the Timeline / shortcut rows.
    fn render_claude_session_row(
        &self,
        idx: usize,
        session: &claude_code::sessions::StoredSession,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let timestamp = chrono::DateTime::<chrono::Utc>::from(session.timestamp);
        let when = crate::util::time_format::format_approx_duration_from_now_utc(timestamp);

        // twarp 08f (§24): macOS-app typography — primary title, muted
        // secondary timestamp; both now theme-following.
        let title = appearance
            .ui_builder()
            .span(session.title.clone())
            .with_soft_wrap()
            .with_style(UiComponentStyles {
                font_color: Some(sidebar_text(appearance)),
                ..Default::default()
            })
            .build()
            .finish();
        let subtitle = appearance
            .ui_builder()
            .span(when)
            .with_style(UiComponentStyles {
                font_color: Some(sidebar_subtext(appearance)),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish();

        let row_mouse_state = {
            let mut states = self.claude_session_row_mouse_states.borrow_mut();
            while states.len() <= idx {
                states.push(MouseStateHandle::default());
            }
            states[idx].clone()
        };

        let inner = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.0)
            .with_child(title)
            .with_child(subtitle)
            .finish();

        // twarp: soft theme-following hover highlight over the sidebar surface.
        let row_highlight = sidebar_row_highlight(appearance);
        Hoverable::new(row_mouse_state, move |state| {
            let mut body = Container::new(inner)
                .with_uniform_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if state.is_hovered() {
                body = body.with_background_color(row_highlight);
            }
            body.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(LeftPanelAction::ClaudeSessionResume(idx));
        })
        .with_cursor(Cursor::PointingHand)
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

    /// Compact Files header. Search is a separate right-rail destination, so
    /// this button switches tools instead of inserting content above the tree.
    fn render_files_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = Text::new_inline(
            "FILES",
            appearance.ui_font_family(),
            type_ramp::CAPTION.size,
        )
        .with_color(sidebar_subtext(appearance))
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let search_button = Hoverable::new(
            self.mouse_state_handles.global_search_button.clone(),
            move |state| {
                let theme = appearance.theme();
                let bg = theme.background();
                let icon_fill = theme.sub_text_color(bg);
                let icon_el = ConstrainedBox::new(Icon::Search.to_warpui_icon(icon_fill).finish())
                    .with_width(spacing::LG)
                    .with_height(spacing::LG)
                    .finish();
                let mut chip = Container::new(icon_el)
                    .with_uniform_padding(spacing::XS)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)));
                if state.is_hovered() {
                    chip = chip.with_background_color(sidebar_row_highlight(appearance));
                }
                chip.finish()
            },
        )
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleRightTool(RightToolKind::Search));
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish())
                .with_child(search_button)
                .finish(),
        )
        .with_padding_left(spacing::MD)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::SM)
        .with_padding_bottom(spacing::XS)
        .finish()
    }

    fn render_search_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Text::new_inline(
                "SEARCH",
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_color(sidebar_subtext(appearance))
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish(),
        )
        .with_padding_left(spacing::MD)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::SM)
        .with_padding_bottom(spacing::XS)
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
                active_view_state::set(
                    self,
                    ToolPanelView::GlobalSearch {
                        entry_focus: *entry_focus,
                    },
                    ctx,
                );
                self.focus_project_search(*entry_focus, ctx);
                send_telemetry_from_ctx!(TelemetryEvent::GlobalSearchOpened, ctx);
                ctx.notify();
            }
            LeftPanelAction::TwarpDrive => {
                active_view_state::set(self, ToolPanelView::TwarpDrive, ctx);
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
            // twarp 07 (7h, PRODUCT §35): open the session list, re-listing
            // the active cwd's stored sessions on entry.
            LeftPanelAction::ClaudeSessions => {
                self.refresh_claude_sessions(ctx);
                active_view_state::set(self, ToolPanelView::ClaudeSessions, ctx);
            }
            // twarp 07 (7h, PRODUCT §36): a row click resumes that stored
            // session in a main-content Claude Code pane. The workspace owns
            // pane creation; emit up.
            LeftPanelAction::ClaudeSessionResume(index) => {
                let cwd = self.active_claude_sessions_cwd(ctx);
                if let (Some(session), Some(cwd)) = (self.claude_sessions.get(*index), cwd) {
                    ctx.emit(LeftPanelEvent::ResumeClaudeSession {
                        session_id: session.id.clone(),
                        jsonl_path: session.jsonl_path.clone(),
                        cwd,
                        provider: session.provider,
                    });
                }
            }
            // twarp: clear the session-list search and re-render the full list.
            LeftPanelAction::ClaudeSessionsClearSearch => {
                self.claude_sessions_search.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                });
                ctx.notify();
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
        pane_group_id: twarpui::EntityId,
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
                ToolPanelView::TwarpDrive => ctx.focus(&self.twarp_drive_view),
                // 4c stub: no internal view to focus yet.
                ToolPanelView::Shortcuts => {}
                // twarp 07 (7h): plain list, no internal view to focus.
                ToolPanelView::ClaudeSessions => {}
                // twarp: 2c-d — ConversationListView arm: AI deleted, no-op.
                ToolPanelView::ConversationListView => {}
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let use_design_shell = cfg!(target_os = "macos") && FeatureFlag::DesignShellV1.is_enabled();
        let use_project_shell = super::project_sidebar_enabled();

        let tool_header = match self.active_view.get() {
            ToolPanelView::ProjectExplorer => Some(self.render_files_header(appearance)),
            ToolPanelView::GlobalSearch { .. } => Some(self.render_search_header(appearance)),
            _ => None,
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
                        // twarp 08 (review): drop the right inset so the file
                        // tree fills the panel width and its scrollbar sits
                        // flush against the panel's right edge.
                        Container::new(ChildView::new(&file_tree_view).finish())
                            .with_padding_left(SIDEBAR_CONTENT_INSET)
                            .finish()
                    } else {
                        Container::new(Empty::new().finish()).finish()
                    };
                let timeline_element = self.render_timeline_section(appearance, app);
                let mut stacked = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Max);
                stacked.add_child(Shrinkable::new(1.0, file_tree_element).finish());
                stacked.add_child(timeline_element);
                Shrinkable::new(1.0, Container::new(stacked.finish()).finish()).finish()
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
            ToolPanelView::TwarpDrive => Shrinkable::new(
                1.0,
                Container::new(ChildView::new(&self.twarp_drive_view).finish())
                    .with_padding_left(SIDEBAR_CONTENT_INSET)
                    .with_padding_right(SIDEBAR_CONTENT_INSET)
                    .finish(),
            )
            .finish(),
            // twarp: Shortcuts moved to Settings > Shortcuts. The variant is
            // retained for persisted-tab back-compat but is never an active
            // toolbelt view, so it renders nothing.
            ToolPanelView::Shortcuts => {
                Shrinkable::new(1.0, Container::new(Empty::new().finish()).finish()).finish()
            }
            // twarp 07 (7h, PRODUCT §35): read-only session list for the
            // active cwd; rows resume via a main-content pane (§36).
            ToolPanelView::ClaudeSessions => self.render_claude_sessions_panel(app),
            // twarp: 2c-d — ConversationListView arm: AI deleted, use empty content.
            ToolPanelView::ConversationListView => {
                Shrinkable::new(1.0, Container::new(Empty::new().finish()).finish()).finish()
            }
        };

        let panel_body = {
            // twarp 08f polish: the explicit close-panel "X" is removed — the
            // panel still toggles via its keybinding (workspace:toggle_left_panel),
            // matching the macOS-app sidebar which has no close glyph in-header.
            let mut column = Flex::column();

            if use_design_shell && !use_project_shell {
                let is_window_fullscreen = app
                    .windows()
                    .platform_window(self.window_id)
                    .map(|window| window.fullscreen_state() == FullscreenState::Fullscreen)
                    .unwrap_or(false);
                let zoom_factor = WindowSettings::as_ref(app).zoom_level.as_zoom_factor();
                let reserve_traffic_light_zone = !is_window_fullscreen
                    && traffic_light_data(app, self.window_id).is_some_and(|data| {
                        data.side == TrafficLightSide::Left && data.width(zoom_factor) > 0.
                    });
                if reserve_traffic_light_zone {
                    column.add_child(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_height(super::TOTAL_TAB_BAR_HEIGHT)
                            .finish(),
                    );
                }
            }

            if let Some(header) = tool_header {
                column.add_child(header);
            }
            column.add_child(Shrinkable::new(1.0, content_area).finish());
            column.with_main_axis_size(MainAxisSize::Max).finish()
        };

        let panel_content = if use_design_shell {
            let seam = if use_project_shell {
                Border::left(border::HAIRLINE_WIDTH).with_border_fill(appearance.theme().outline())
            } else {
                Border::right(border::HAIRLINE_WIDTH).with_border_fill(appearance.theme().outline())
            };
            Container::new(panel_body)
                .with_background(appearance.theme().surface_1())
                .with_border(seam)
                // twarp 08f polish: breathing room at the bottom edge.
                .with_padding_bottom(SIDEBAR_BOTTOM_INSET)
                .finish()
        } else {
            // twarp: floating-card treatment — rounded corners, a gray outline, and
            // a soft drop shadow so the sidebar reads as floating above the terminal
            // background. The edge gap (margin) is applied OUTSIDE the Resizable
            // below so the drag bar sits on the card's border, not out in the gap.
            //
            // twarp: the sidebar surface is the main terminal background (shared
            // with the code review panel), so the card blends into the background
            // and is set apart only by its outline/gap/shadow. Child colors derive
            // from the theme (see the `sidebar_*` palette helpers).
            Container::new(panel_body)
                .with_background(super::floating_panel_surface_fill(app))
                .with_corner_radius(super::floating_panel_corner_radius())
                .with_border(super::floating_panel_border(app))
                .with_drop_shadow(super::floating_panel_drop_shadow())
                // twarp 08f polish: breathing room at the bottom edge.
                .with_padding_bottom(SIDEBAR_BOTTOM_INSET)
                .finish()
        };

        if twarpui::platform::is_mobile_device() {
            return Container::new(panel_content)
                .with_uniform_margin(super::FLOATING_PANEL_MARGIN)
                .finish();
        }

        let drag_side = if use_design_shell {
            if use_project_shell {
                DragBarSide::Left
            } else {
                DragBarSide::Right
            }
        } else {
            match self.panel_position {
                super::PanelPosition::Left => DragBarSide::Right,
                super::PanelPosition::Right => DragBarSide::Left,
            }
        };
        let resizable = Resizable::new(self.resizable_state_handle.clone(), panel_content)
            .with_dragbar_side(drag_side)
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .with_bounds_callback(Box::new(|window_size| {
                let min_width = MIN_SIDEBAR_WIDTH;
                let max_width = window_size.x() * MAX_SIDEBAR_WIDTH_RATIO;
                (min_width, max_width.max(min_width))
            }))
            .finish();
        // The floating gap goes here, around the Resizable, so the drag bar
        // (the rightmost few px of the Resizable's child) lands on the card's
        // border rather than 8px out in the gap.
        //
        // twarp (#3): the panel hugs the center — no gap on the terminal-facing
        // side. The card keeps its float against the window edge / top / bottom,
        // but butts directly up to the center pane (gap removed on the inner side
        // so there's no dead strip between the card and the terminal).
        if use_design_shell {
            return Container::new(resizable).finish();
        }

        let margin = super::FLOATING_PANEL_MARGIN;
        match self.panel_position {
            super::PanelPosition::Left => Container::new(resizable)
                .with_margin_top(margin)
                .with_margin_bottom(margin)
                .with_margin_left(margin)
                .finish(),
            super::PanelPosition::Right => Container::new(resizable)
                .with_margin_top(margin)
                .with_margin_bottom(margin)
                .with_margin_right(margin)
                .finish(),
        }
    }
}

/// twarp 08e (PRODUCT §17): case-insensitive substring filter of stored
/// sessions by `title`. Returns the indices (into the original slice) of the
/// sessions that match `query`, preserving order. An empty/whitespace-only
/// query is not a filter — every index is returned (PRODUCT §18). Free function
/// so it's unit-testable without a view harness; the filter is transient view
/// state and never mutates session data (PRODUCT §19, §20).
fn filter_session_indices(
    sessions: &[claude_code::sessions::StoredSession],
    query: &str,
) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return (0..sessions.len()).collect();
    }
    sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| s.title.to_lowercase().contains(&needle))
        .map(|(idx, _)| idx)
        .collect()
}

fn deduplicate_by_directory_name(directories: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    directories
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .collect()
}

#[cfg(test)]
mod sessions_search_tests {
    use super::*;
    use std::time::SystemTime;

    fn session(title: &str) -> claude_code::sessions::StoredSession {
        claude_code::sessions::StoredSession {
            id: title.to_owned(),
            title: title.to_owned(),
            timestamp: SystemTime::UNIX_EPOCH,
            jsonl_path: PathBuf::from("/dev/null"),
            provider: claude_code::driver::AgentProvider::Claude,
        }
    }

    #[test]
    fn empty_query_matches_everything() {
        let sessions = vec![session("Alpha"), session("Beta")];
        assert_eq!(filter_session_indices(&sessions, ""), vec![0, 1]);
        assert_eq!(filter_session_indices(&sessions, "   "), vec![0, 1]);
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let sessions = vec![
            session("Fix the parser bug"),
            session("Refactor CONFIG loader"),
            session("Write docs"),
        ];
        // Case-insensitive on both sides.
        assert_eq!(filter_session_indices(&sessions, "config"), vec![1]);
        assert_eq!(filter_session_indices(&sessions, "PARSER"), vec![0]);
        // Substring, not prefix.
        assert_eq!(filter_session_indices(&sessions, "the"), vec![0]);
        // Query is trimmed before matching.
        assert_eq!(filter_session_indices(&sessions, "  docs  "), vec![2]);
    }

    #[test]
    fn no_match_returns_empty_and_preserves_order() {
        let sessions = vec![session("zebra"), session("apple"), session("mango")];
        assert!(filter_session_indices(&sessions, "qqq").is_empty());
        // Order of original indices is preserved across multiple matches.
        let sessions = vec![session("a one"), session("b one"), session("c two")];
        assert_eq!(filter_session_indices(&sessions, "one"), vec![0, 1]);
    }
}
