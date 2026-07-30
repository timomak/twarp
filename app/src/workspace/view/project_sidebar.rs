use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use settings::Setting as _;
use twarp_core::ui::theme::Fill as ThemeFill;
use twarp_core::ui::tokens::{border, radius, spacing, type_ramp, TypeStyle};
use twarp_core::ui::Icon;
use twarpui::elements::{
    resizable_state_handle, Align, Border, ChildView, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, DragAxis, DragBarSide, Draggable, DropTarget, Element, Empty,
    Expanded, Fill as ElementFill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, ParentElement,
    Radius,
    Resizable, SavePosition, ScrollbarWidth, Shrinkable, Text,
};
use twarpui::fonts::{Properties, Weight};
use twarpui::platform::{Cursor, FullscreenState};
use twarpui::ui_components::button::Button;
use twarpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use twarpui::{AppContext, EntityId, SingletonEntity};

use crate::app_state::RightToolKind;
use crate::appearance::Appearance;
use crate::palette::PaletteMode;
use crate::projects::ProjectManagementModel;
use crate::server::telemetry::PaletteSource;
use crate::tab::tab_position_id;
use crate::terminal::resizable_data::{
    ModalType, ResizableData, DEFAULT_CODE_REVIEW_TOOL_WIDTH, DEFAULT_FILES_TOOL_WIDTH,
    DEFAULT_PROJECTS_SIDEBAR_WIDTH,
};
use crate::util::traffic_lights::{traffic_light_data, TrafficLightSide};
use crate::window_settings::WindowSettings;
use crate::workspace::tab_settings::TabSettings;
use crate::workspace::{
    ProjectSidebarPaneDropTarget, ProjectSidebarPaneDropTargetData, TabContextMenuAnchor,
    WorkspaceAction,
};
use crate::FeatureFlag;

use super::{ProjectDirectoryResolution, Workspace, TOTAL_TAB_BAR_HEIGHT};

pub(super) const PROJECTS_SIDEBAR_POSITION_ID: &str = "workspace_view:projects_sidebar";
const PROJECT_ROW_POSITION_ID_PREFIX: &str = "workspace_view:projects_sidebar:project_row";
const PROJECT_MENU_POSITION_ID_PREFIX: &str = "workspace_view:projects_sidebar:project_menu";
const PROJECT_NEW_CHAT_POSITION_ID_PREFIX: &str =
    "workspace_view:projects_sidebar:project_new_chat";
const RIGHT_ACTIVITY_STRIP_WIDTH: f32 = spacing::XXL + spacing::SM;
const MINIMUM_CENTER_WORKSPACE_WIDTH: f32 = 480.;
const MINIMUM_PROJECTS_SIDEBAR_WIDTH: f32 = 144.;
const MAXIMUM_PROJECTS_SIDEBAR_WIDTH_RATIO: f32 = 0.4;

#[derive(Clone, Default)]
struct ProjectRowMouseStates {
    row: twarpui::elements::MouseStateHandle,
    menu: twarpui::elements::MouseStateHandle,
    new_chat: twarpui::elements::MouseStateHandle,
}

#[derive(Default)]
pub(super) struct ProjectSidebarMouseStates {
    project_rows: RefCell<HashMap<Option<PathBuf>, ProjectRowMouseStates>>,
    new_session: twarpui::elements::MouseStateHandle,
    search_sessions: twarpui::elements::MouseStateHandle,
    empty_new_project: twarpui::elements::MouseStateHandle,
    scheduled_tasks: twarpui::elements::MouseStateHandle,
    plugins: twarpui::elements::MouseStateHandle,
    pull_requests: twarpui::elements::MouseStateHandle,
}

impl ProjectSidebarMouseStates {
    fn project_row(&self, project_root: Option<PathBuf>) -> ProjectRowMouseStates {
        self.project_rows
            .borrow_mut()
            .entry(project_root)
            .or_default()
            .clone()
    }

    fn retain_project_rows(&self, project_roots: &HashSet<Option<PathBuf>>) {
        self.project_rows
            .borrow_mut()
            .retain(|project_root, _| project_roots.contains(project_root));
    }
}

fn project_row_position_id(first_tab_index: usize) -> String {
    format!("{PROJECT_ROW_POSITION_ID_PREFIX}:{first_tab_index}")
}

fn project_menu_position_id(first_tab_index: usize) -> String {
    format!("{PROJECT_MENU_POSITION_ID_PREFIX}:{first_tab_index}")
}

fn project_new_chat_position_id(first_tab_index: usize) -> String {
    format!("{PROJECT_NEW_CHAT_POSITION_ID_PREFIX}:{first_tab_index}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProjectListTarget {
    LiveProject(Vec<usize>),
    Library(PathBuf),
}

fn merged_project_targets(
    local_roots: impl IntoIterator<Item = (usize, Option<PathBuf>)>,
    library_paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<ProjectListTarget> {
    let local_roots: Vec<_> = local_roots.into_iter().collect();
    let mut grouped_live_projects: Vec<(Option<PathBuf>, Vec<usize>)> = Vec::new();
    for (tab_index, root) in &local_roots {
        if let Some((_, tab_indices)) = root.as_ref().and_then(|root| {
            grouped_live_projects
                .iter_mut()
                .find(|(candidate, _)| candidate.as_ref() == Some(root))
        }) {
            tab_indices.push(*tab_index);
        } else {
            grouped_live_projects.push((root.clone(), vec![*tab_index]));
        }
    }
    let mut targets: Vec<_> = grouped_live_projects
        .into_iter()
        .map(|(_, tab_indices)| ProjectListTarget::LiveProject(tab_indices))
        .collect();
    let local_roots: HashSet<_> = local_roots
        .iter()
        .filter_map(|(_, root)| root.clone())
        .collect();
    let mut seen_library_paths = HashSet::new();
    targets.extend(library_paths.into_iter().filter_map(|path| {
        (!local_roots.contains(&path) && seen_library_paths.insert(path.clone()))
            .then_some(ProjectListTarget::Library(path))
    }));
    targets
}

fn should_responsively_collapse_right_tool(
    window_width: f32,
    projects_width: f32,
    tool_width: f32,
) -> bool {
    window_width - projects_width - tool_width - RIGHT_ACTIVITY_STRIP_WIDTH
        < MINIMUM_CENTER_WORKSPACE_WIDTH
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RightToolTransition {
    pub tool: RightToolKind,
    pub open: bool,
    pub clear_code_review_maximize: bool,
}

pub(super) fn toggle_right_tool_state(
    current_tool: RightToolKind,
    currently_open: bool,
    clicked_tool: RightToolKind,
    code_review_maximized: bool,
) -> RightToolTransition {
    let closes_current = currently_open && current_tool == clicked_tool;
    RightToolTransition {
        tool: clicked_tool,
        open: !closes_current,
        clear_code_review_maximize: code_review_maximized
            && (clicked_tool != RightToolKind::CodeReview || closes_current),
    }
}

pub(super) fn code_review_tool_is_open(tool: RightToolKind, tool_host_open: bool) -> bool {
    tool_host_open && tool == RightToolKind::CodeReview
}

pub(super) fn project_root_for_new_tab(
    project_shell_enabled: bool,
    source_has_settings: bool,
    source_project_root: Option<&Path>,
) -> Option<PathBuf> {
    if project_shell_enabled && !source_has_settings {
        source_project_root.map(Path::to_path_buf)
    } else {
        None
    }
}

fn project_title(
    custom_title: Option<&str>,
    project_root: Option<&Path>,
    derived_roots: &[PathBuf],
    display_title: &str,
) -> String {
    if let Some(custom_title) = custom_title.filter(|title| !title.trim().is_empty()) {
        return custom_title.to_owned();
    }
    let root = project_root.or_else(|| (derived_roots.len() == 1).then(|| &*derived_roots[0]));
    if let Some(folder) = root.and_then(Path::file_name) {
        let folder = folder.to_string_lossy();
        if !folder.is_empty() {
            return folder.into_owned();
        }
    }
    if !display_title.trim().is_empty() {
        return display_title.to_owned();
    }
    "Untitled project".to_owned()
}

fn project_tab_title(custom_title: Option<&str>, display_title: &str) -> String {
    custom_title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(display_title)
        .trim()
        .to_owned()
}

/// twarp: sidebar legibility. The sidebar is read at a glance from across the
/// window, so its rows use the reading style (`PROSE`) rather than the dense
/// 13pt `UI` style, and its meta text uses `LABEL` rather than `CAPTION`.
const SIDEBAR_ROW_TEXT: TypeStyle = type_ramp::PROSE;
const SIDEBAR_META_TEXT: TypeStyle = type_ramp::LABEL;

fn sidebar_icon(icon: Icon, color: ThemeFill) -> Box<dyn Element> {
    ConstrainedBox::new(icon.to_warpui_icon(color).finish())
        .with_width(spacing::LG)
        .with_height(spacing::LG)
        .finish()
}

fn sidebar_chrome_icon(icon: Icon, color: ThemeFill) -> Box<dyn Element> {
    ConstrainedBox::new(icon.to_warpui_icon(color).finish())
        .with_width(spacing::LG)
        .with_height(spacing::LG)
        .finish()
}

fn project_matches_search(query: &str, fields: impl IntoIterator<Item = String>) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || fields
            .into_iter()
            .any(|field| field.to_lowercase().contains(&query))
}

fn project_disambiguation(root: Option<&Path>, ordinal: usize) -> String {
    let folder = root
        .and_then(Path::file_name)
        .map(|part| part.to_string_lossy());
    let parent = root
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|part| part.to_string_lossy());
    match (parent, folder) {
        (Some(parent), Some(folder)) if !parent.is_empty() && !folder.is_empty() => {
            format!("{parent}/{folder}")
        }
        (_, Some(folder)) if !folder.is_empty() => folder.into_owned(),
        _ => format!("Project {}", ordinal + 1),
    }
}

pub(super) fn resolve_project_directory(
    assigned_root: Option<PathBuf>,
    candidate_roots: impl IntoIterator<Item = PathBuf>,
) -> ProjectDirectoryResolution {
    if let Some(root) = assigned_root {
        return if root.is_dir() && std::fs::read_dir(&root).is_ok() {
            ProjectDirectoryResolution::Resolved(root)
        } else {
            ProjectDirectoryResolution::Unavailable
        };
    }
    let mut roots = Vec::new();
    for root in candidate_roots {
        if root.is_dir() && std::fs::read_dir(&root).is_ok() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    match roots.len() {
        0 => ProjectDirectoryResolution::Unavailable,
        1 => ProjectDirectoryResolution::Resolved(roots.remove(0)),
        _ => ProjectDirectoryResolution::ChooseFrom(roots),
    }
}

impl Workspace {
    pub(super) fn right_tool_collapsed_for_window(&self, app: &AppContext) -> bool {
        let Some(bounds) = app.window_bounds(&self.window_id) else {
            return false;
        };
        let modal_sizes = ResizableData::handle(app)
            .as_ref(app)
            .get_all_handles(self.window_id);
        let projects_width = if self.projects_sidebar_open {
            modal_sizes
                .and_then(|sizes| {
                    sizes
                        .projects_sidebar_width
                        .lock()
                        .ok()
                        .map(|state| state.size())
                })
                .unwrap_or(DEFAULT_PROJECTS_SIDEBAR_WIDTH)
        } else {
            0.
        };
        let tool_width = match self.right_tool {
            RightToolKind::Files | RightToolKind::Search => modal_sizes
                .and_then(|sizes| sizes.files_tool_width.lock().ok().map(|state| state.size()))
                .unwrap_or(DEFAULT_FILES_TOOL_WIDTH),
            RightToolKind::CodeReview => modal_sizes
                .and_then(|sizes| {
                    sizes
                        .code_review_tool_width
                        .lock()
                        .ok()
                        .map(|state| state.size())
                })
                .unwrap_or(DEFAULT_CODE_REVIEW_TOOL_WIDTH),
        };
        should_responsively_collapse_right_tool(bounds.width(), projects_width, tool_width)
    }

    fn project_roots(&self, project_id: EntityId, app: &AppContext) -> Vec<PathBuf> {
        self.working_directories_model
            .as_ref(app)
            .most_recent_directories_for_pane_group(project_id)
            .map(|directories| directories.map(|directory| directory.path).collect())
            .unwrap_or_default()
    }

    pub(super) fn project_list_targets(&self, app: &AppContext) -> Vec<ProjectListTarget> {
        let local_roots = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| {
                let pane_group = tab.pane_group.as_ref(app);
                !pane_group.has_settings_panes() && !pane_group.has_automation_panes()
            })
            .map(|(index, tab)| (index, tab.project_root.clone()));
        let library_paths = ProjectManagementModel::as_ref(app).project_paths_by_recency();
        merged_project_targets(local_roots, library_paths)
    }

    fn project_target_title(&self, target: &ProjectListTarget, app: &AppContext) -> String {
        match target {
            ProjectListTarget::LiveProject(tab_indices) => {
                let Some(tab) = tab_indices.first().and_then(|index| self.tabs.get(*index)) else {
                    return "Untitled project".to_owned();
                };
                let pane_group = tab.pane_group.as_ref(app);
                let roots = self.project_roots(tab.pane_group.id(), app);
                let custom_name = tab
                    .project_root
                    .as_ref()
                    .and_then(|root| ProjectManagementModel::as_ref(app).project_name(root));
                project_title(
                    custom_name.as_deref(),
                    tab.project_root.as_deref(),
                    &roots,
                    &pane_group.display_title(app),
                )
            }
            ProjectListTarget::Library(path) => project_title(
                ProjectManagementModel::as_ref(app)
                    .project_name(path)
                    .as_deref(),
                Some(path),
                &[],
                "",
            ),
        }
    }

    fn project_title_occurrences(&self, title: &str, app: &AppContext) -> usize {
        self.project_list_targets(app)
            .iter()
            .filter(|target| {
                self.project_target_title(target, app)
                    .eq_ignore_ascii_case(title)
            })
            .count()
    }

    pub(super) fn project_target_matches_query(
        &self,
        target: &ProjectListTarget,
        app: &AppContext,
    ) -> bool {
        let mut fields = vec![self.project_target_title(target, app)];
        match target {
            ProjectListTarget::LiveProject(tab_indices) => {
                for index in tab_indices {
                    let Some(tab) = self.tabs.get(*index) else {
                        continue;
                    };
                    let pane_group = tab.pane_group.as_ref(app);
                    fields.push(pane_group.display_title(app));
                    if let Some(custom_title) = pane_group.custom_title(app) {
                        fields.push(custom_title);
                    }
                    fields.extend(
                        self.project_roots(tab.pane_group.id(), app)
                            .iter()
                            .map(|root| root.display().to_string()),
                    );
                    if let Some(root) = tab.project_root.as_ref() {
                        fields.push(root.display().to_string());
                    }
                }
            }
            ProjectListTarget::Library(path) => fields.push(path.display().to_string()),
        }
        project_matches_search(&self.vertical_tabs_panel.search_query, fields)
    }

    pub(super) fn matching_project_targets(&self, app: &AppContext) -> Vec<ProjectListTarget> {
        self.project_list_targets(app)
            .into_iter()
            .filter(|target| self.project_target_matches_query(target, app))
            .collect()
    }

    fn render_project_header(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        if self.projects_search_open {
            return Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Shrinkable::new(
                            1.,
                            ChildView::new(&self.vertical_tabs_search_input).finish(),
                        )
                        .finish(),
                    )
                    .with_child(
                        Hoverable::new(self.projects_search_mouse_state.clone(), move |state| {
                            let color = if state.is_hovered() {
                                theme.main_text_color(theme.background())
                            } else {
                                theme.sub_text_color(theme.background())
                            };
                            Container::new(sidebar_icon(Icon::X, color))
                                .with_uniform_padding(spacing::XS)
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    radius::CHIP,
                                )))
                                .finish()
                        })
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(WorkspaceAction::ToggleProjectsSearch)
                        })
                        .with_cursor(Cursor::PointingHand)
                        .finish(),
                    )
                    .finish(),
            )
            .with_padding_left(spacing::MD)
            .with_padding_right(spacing::SM)
            .with_padding_top(spacing::SM)
            .with_padding_bottom(spacing::XS)
            .finish();
        }

        let label = Text::new_inline(
            "PROJECTS",
            appearance.ui_font_family(),
            SIDEBAR_META_TEXT.size,
        )
        .with_line_height_ratio(SIDEBAR_META_TEXT.line_height)
        .with_color(theme.sub_text_color(theme.background()).into())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let create_button = |mouse_state: twarpui::elements::MouseStateHandle| {
            Hoverable::new(mouse_state, move |state| {
                let color = if state.is_hovered() {
                    theme.main_text_color(theme.background())
                } else {
                    theme.sub_text_color(theme.background())
                };
                let mut container = Container::new(sidebar_icon(Icon::Plus, color))
                    .with_uniform_padding(spacing::XS)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)));
                if state.is_hovered() {
                    container = container.with_background(theme.surface_overlay_2());
                }
                container.finish()
            })
            .on_click(|ctx, _, position| {
                ctx.dispatch_typed_action(WorkspaceAction::ToggleProjectCreateMenu { position })
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
        };

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(create_button(self.projects_create_mouse_state.clone()))
                .finish(),
        )
        .with_padding_left(spacing::MD)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::SM)
        .with_padding_bottom(spacing::XS)
        .finish()
    }

    fn render_sidebar_action(
        &self,
        mouse_state: twarpui::elements::MouseStateHandle,
        icon: Icon,
        label: &'static str,
        action: WorkspaceAction,
        app: &AppContext,
    ) -> Box<dyn Element> {
        self.render_sidebar_action_with_count(mouse_state, icon, label, None, false, action, app)
    }

    /// Whether the active tab is showing the given automation page — used to
    /// highlight that page's sidebar row while it is open and active.
    fn active_tab_shows_automation_page(
        &self,
        page: crate::automation::AutomationPage,
        app: &AppContext,
    ) -> bool {
        self.tabs.get(self.active_tab_index()).is_some_and(|tab| {
            tab.pane_group
                .as_ref(app)
                .automation_pages(app)
                .contains(&page)
        })
    }

    /// twarp 20e: like [`Self::render_sidebar_action`] but with an optional
    /// right-aligned count (used by the automation rows; `Some(0)` renders no
    /// badge) and an `active` highlight for the row whose page is showing in
    /// the active tab.
    fn render_sidebar_action_with_count(
        &self,
        mouse_state: twarpui::elements::MouseStateHandle,
        icon: Icon,
        label: &'static str,
        count: Option<usize>,
        active: bool,
        action: WorkspaceAction,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Hoverable::new(mouse_state, move |state| {
            let mut contents = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(sidebar_icon(icon, theme.sub_text_color(theme.background())))
                .with_child(
                    Shrinkable::new(
                        1.,
                        Align::new(
                            Text::new_inline(
                                label,
                                appearance.ui_font_family(),
                                SIDEBAR_ROW_TEXT.size,
                            )
                            .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
                            .with_color(theme.main_text_color(theme.background()).into())
                            .finish(),
                        )
                        .left()
                        .finish(),
                    )
                    .finish(),
                );
            if let Some(count) = count.filter(|count| *count > 0) {
                contents.add_child(
                    Text::new_inline(
                        count.to_string(),
                        appearance.ui_font_family(),
                        SIDEBAR_META_TEXT.size,
                    )
                    .with_line_height_ratio(SIDEBAR_META_TEXT.line_height)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                );
            }
            let contents = contents.finish();
            let mut row = Container::new(contents)
                .with_uniform_padding(spacing::SM)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if active {
                row = row.with_background(theme.surface_3());
            } else if state.is_hovered() {
                row = row.with_background(theme.surface_overlay_2());
            }
            row.finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_sidebar_actions(&self, app: &AppContext) -> Box<dyn Element> {
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(spacing::XXS)
                .with_child(self.render_sidebar_action(
                    self.projects_sidebar_mouse_states.new_session.clone(),
                    Icon::NewConversation,
                    "New session",
                    WorkspaceAction::OpenClaudeCodeInNewTab,
                    app,
                ))
                // twarp 20a/20e: automation entry points, owner-placed
                // directly under "New session". Each swaps a full-page
                // automation pane into the current tab.
                .with_child(
                    self.render_sidebar_action_with_count(
                        self.projects_sidebar_mouse_states.scheduled_tasks.clone(),
                        Icon::Clock,
                        "Scheduled Tasks",
                        // Enabled tasks only — the number that will actually run.
                        Some(
                            crate::automation::scheduler::SchedulerModel::as_ref(app)
                                .tasks()
                                .iter()
                                .filter(|task| task.enabled)
                                .count(),
                        ),
                        self.active_tab_shows_automation_page(
                            crate::automation::AutomationPage::ScheduledTasks,
                            app,
                        ),
                        WorkspaceAction::ShowScheduledTasks,
                        app,
                    ),
                )
                // twarp 23b: one Plugins row replaces the Skills and MCPs
                // rows; count = number of plugins.
                .with_child(
                    self.render_sidebar_action_with_count(
                        self.projects_sidebar_mouse_states.plugins.clone(),
                        Icon::Dataflow,
                        "Plugins",
                        Some(
                            crate::plugin_registry::PluginRegistryModel::as_ref(app)
                                .plugins()
                                .len(),
                        ),
                        self.active_tab_shows_automation_page(
                            crate::automation::AutomationPage::Plugins,
                            app,
                        ),
                        WorkspaceAction::ShowPlugins,
                        app,
                    ),
                )
                // twarp 21a: Pull Requests page entry point, directly below
                // the automation rows.
                .with_child(self.render_sidebar_action_with_count(
                    self.projects_sidebar_mouse_states.pull_requests.clone(),
                    Icon::GitPullRequest,
                    "Pull Requests",
                    None,
                    self.active_tab_shows_automation_page(
                        crate::automation::AutomationPage::PullRequests,
                        app,
                    ),
                    WorkspaceAction::ShowPullRequests,
                    app,
                ))
                .with_child(self.render_sidebar_action(
                    self.projects_sidebar_mouse_states.search_sessions.clone(),
                    Icon::Search,
                    "Search sessions…",
                    WorkspaceAction::OpenPalette {
                        mode: PaletteMode::Command,
                        source: PaletteSource::TitleBarSearchBar,
                        query: Some("claude: ".to_owned()),
                    },
                    app,
                ))
                .finish(),
        )
        .with_padding_left(spacing::SM)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::SM)
        .with_padding_bottom(spacing::MD)
        .finish()
    }

    fn project_pane_drag_active(&self, app: &AppContext) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.pane_group.as_ref(app).any_pane_being_dragged(app))
    }

    fn render_live_project_group(
        &self,
        tab_indices: &[usize],
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let first_index = *tab_indices.first()?;
        let selected_index = if tab_indices.contains(&self.active_tab_index()) {
            self.active_tab_index()
        } else {
            first_index
        };
        let selected_tab = self.tabs.get(selected_index)?;
        let project_id = selected_tab.pane_group.id();
        let target = ProjectListTarget::LiveProject(tab_indices.to_vec());
        if !self.project_target_matches_query(&target, app) {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let title = self.project_target_title(&target, app);
        let keyboard_focused = self.projects_search_open
            && self
                .matching_project_targets(app)
                .get(self.projects_search_selection)
                .is_some_and(|selected| selected == &target);
        let pane_drop_target = ProjectSidebarPaneDropTarget {
            project_root: selected_tab.project_root.clone(),
            tab_insert_index: tab_indices
                .iter()
                .copied()
                .max()
                .map_or(self.tabs.len(), |index| index + 1),
        };
        let pane_drop_active = self
            .hovered_project_pane_drop_target
            .as_ref()
            .is_some_and(|target| target == &pane_drop_target);
        let pane_drag_active = self.project_pane_drag_active(app);
        let ProjectRowMouseStates {
            row: row_mouse_state,
            menu: menu_mouse_state,
            new_chat: new_chat_mouse_state,
        } = self
            .projects_sidebar_mouse_states
            .project_row(selected_tab.project_root.clone());
        let project_tab_indices = tab_indices.to_vec();
        let row_position_id = project_row_position_id(first_index);
        let menu_position_id = project_menu_position_id(first_index);
        let new_chat_position_id = project_new_chat_position_id(first_index);
        let color: ThemeFill = selected_tab
            .color()
            .map(|color| {
                ThemeFill::Solid(color.to_tab_color(&theme.terminal_colors().normal).into())
            })
            .unwrap_or_else(|| theme.hint_text_color(theme.background()));

        let rename_root = selected_tab.project_root.clone();
        let is_renaming_project =
            rename_root.is_some() && self.project_being_renamed == rename_root;
        let title_element: Box<dyn Element> = if is_renaming_project {
            ChildView::new(&self.project_rename_editor).finish()
        } else {
            Text::new_inline(
                title.clone(),
                appearance.ui_font_family(),
                SIDEBAR_ROW_TEXT.size,
            )
            .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
            .with_clip(twarpui::text_layout::ClipConfig::end())
            .with_color(theme.main_text_color(theme.background()).into())
            .finish()
        };
        let project_row = Hoverable::new(row_mouse_state, move |state| {
            let identity = Container::new(sidebar_icon(Icon::CircleFilled, color))
                .with_padding_left(spacing::XS)
                .finish();
            let project_identity = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(identity)
                .with_child(title_element)
                .finish();
            let mut contents = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., project_identity).finish());
            if state.is_hovered() && !pane_drag_active {
                let menu = Hoverable::new(menu_mouse_state.clone(), move |button_state| {
                    let color = if button_state.is_hovered() {
                        theme.main_text_color(theme.background())
                    } else {
                        theme.sub_text_color(theme.background())
                    };
                    Container::new(sidebar_icon(Icon::DotsHorizontal, color))
                        .with_padding_left(spacing::XS)
                        .with_padding_right(spacing::XS)
                        .finish()
                })
                .on_click(move |ctx, _, position| {
                    ctx.dispatch_typed_action(WorkspaceAction::ToggleProjectRightClickMenu {
                        tab_index: selected_index,
                        tab_indices: project_tab_indices.clone(),
                        anchor: TabContextMenuAnchor::Pointer(position),
                    })
                })
                .with_cursor(Cursor::PointingHand)
                .finish();
                let menu = SavePosition::new(menu, &menu_position_id).finish();
                let new_chat = Hoverable::new(new_chat_mouse_state.clone(), move |button_state| {
                    let color = if button_state.is_hovered() {
                        theme.main_text_color(theme.background())
                    } else {
                        theme.sub_text_color(theme.background())
                    };
                    Container::new(sidebar_icon(Icon::Plus, color))
                        .with_padding_left(spacing::XS)
                        .with_padding_right(spacing::XS)
                        .finish()
                })
                .on_click(move |ctx, _, position| {
                    ctx.dispatch_typed_action(WorkspaceAction::NewProjectChat {
                        project_id,
                        position,
                    })
                })
                .with_cursor(Cursor::PointingHand)
                .finish();
                let new_chat = SavePosition::new(new_chat, &new_chat_position_id).finish();
                contents.add_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(spacing::XXS)
                        .with_child(menu)
                        .with_child(new_chat)
                        .finish(),
                );
            }
            let mut container = Container::new(contents.finish())
                .with_padding_left(spacing::SM)
                .with_padding_right(spacing::SM)
                .with_padding_top(spacing::XS)
                .with_padding_bottom(spacing::XS)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if pane_drop_active || keyboard_focused {
                container = container.with_background(theme.surface_3());
            } else if state.is_hovered() {
                container = container.with_background(theme.surface_overlay_2());
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ActivateTab(selected_index))
        })
        .on_double_click(move |ctx, _, _| {
            if let Some(project_root) = rename_root.clone() {
                ctx.dispatch_typed_action(WorkspaceAction::RenameProject { project_root })
            }
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .finish();
        let project_row = SavePosition::new(
            DropTarget::new(
                project_row,
                ProjectSidebarPaneDropTargetData {
                    target: pane_drop_target,
                },
            )
            .finish(),
            &row_position_id,
        )
        .finish();

        let mut group = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        group.add_child(project_row);
        for index in tab_indices {
            if let Some(row) = self.render_project_tab_row(*index, app) {
                group.add_child(Container::new(row).with_padding_left(spacing::LG).finish());
            }
        }
        Some(group.finish())
    }

    fn render_project_tab_row(&self, index: usize, app: &AppContext) -> Option<Box<dyn Element>> {
        let tab = self.tabs.get(index)?;
        let pane_group = tab.pane_group.as_ref(app);
        let custom_title = pane_group.custom_title(app);
        let display_title = pane_group.display_title(app);
        let title = project_tab_title(custom_title.as_deref(), &display_title);
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let active = index == self.active_tab_index();
        let is_renaming = self.current_workspace_state.tab_being_renamed() == Some(index);
        let attention = pane_group.claude_code_tab_status(app);
        // An unreviewed-completion ✓ paints in the tab's theme color so the
        // signal reads as belonging to its project; other statuses keep their
        // semantic colors (blocked yellow, error red, …).
        let attention_icon: Option<Box<dyn Element>> = attention.as_ref().map(|status| {
            let (icon, color) = status.status_icon_and_color(theme);
            let fill = if matches!(status, crate::app_state::ConversationStatus::SuccessUnseen) {
                tab.color()
                    .map(|tab_color| {
                        ThemeFill::Solid(
                            tab_color
                                .to_tab_color(&theme.terminal_colors().normal)
                                .into(),
                        )
                    })
                    .unwrap_or_else(|| ThemeFill::from(color))
            } else {
                ThemeFill::from(color)
            };
            icon.to_warpui_icon(fill).finish()
        });
        let row_mouse_state = tab.tab_mouse_state.clone();
        let menu_mouse_state = tab.tooltip_mouse_state.clone();
        let pane_drag_active = self.project_pane_drag_active(app);
        let title_element: Box<dyn Element> = if is_renaming {
            ChildView::new(&self.tab_rename_editor).finish()
        } else {
            let mut labels = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
            labels.add_child(
                Text::new_inline(
                    title.clone(),
                    appearance.ui_font_family(),
                    SIDEBAR_ROW_TEXT.size,
                )
                .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            );
            labels.finish()
        };

        let row = Hoverable::new(row_mouse_state, move |state| {
            // Trailing cluster (status icon + hover menu) pinned to the row's
            // right edge; the title takes the remaining width on the left.
            let mut trailing = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM);

            if let Some(status_icon) = attention_icon {
                // `Icon::layout` greedily takes `constraint.max`, so an
                // unconstrained icon swallows the whole row and shrinks the
                // title to nothing — box it like `sidebar_icon` does.
                trailing.add_child(
                    ConstrainedBox::new(status_icon)
                        .with_width(spacing::MD)
                        .with_height(spacing::MD)
                        .finish(),
                );
            }
            if state.is_hovered() && !pane_drag_active && !is_renaming {
                // Archive closes the session tab; it stays resumable from
                // past sessions. The full menu remains on right-click.
                let archive = Hoverable::new(menu_mouse_state.clone(), move |button_state| {
                    let color = if button_state.is_hovered() {
                        theme.main_text_color(theme.background())
                    } else {
                        theme.sub_text_color(theme.background())
                    };
                    Container::new(sidebar_icon(Icon::Archive, color))
                        .with_padding_left(spacing::XS)
                        .with_padding_right(spacing::XS)
                        .finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::CloseTab(index))
                })
                .with_cursor(Cursor::PointingHand)
                .finish();
                trailing.add_child(archive);
            }
            let contents = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(Shrinkable::new(1., title_element).finish())
                .with_child(trailing.finish());
            let mut container = Container::new(contents.finish())
                .with_padding_left(spacing::SM)
                .with_padding_right(spacing::SM)
                .with_padding_top(spacing::XS)
                .with_padding_bottom(spacing::XS)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if active {
                container = container.with_background(theme.surface_3());
            } else if state.is_hovered() {
                container = container.with_background(theme.surface_overlay_2());
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::ActivateTab(index)))
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::RenameTab(index))
        })
        .on_right_click(move |ctx, _, position| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleProjectChatRightClickMenu {
                tab_index: index,
                anchor: TabContextMenuAnchor::Pointer(position),
            })
        })
        .on_middle_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::CloseTab(index))
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .finish();

        let draggable = Draggable::new(tab.draggable_state.clone(), row)
            .on_drag_start(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::StartTabDrag))
            .on_drag(move |ctx, _, rect, _| {
                ctx.dispatch_typed_action(WorkspaceAction::DragTab {
                    tab_index: index,
                    tab_position: rect,
                })
            })
            .on_drop(|ctx, _, _, _| ctx.dispatch_typed_action(WorkspaceAction::DropTab));
        let row = SavePosition::new(draggable.finish(), &tab_position_id(index)).finish();
        Some(row)
    }

    fn render_library_project_row(
        &self,
        path: PathBuf,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let target = ProjectListTarget::Library(path.clone());
        if !self.project_target_matches_query(&target, app) {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let title = self.project_target_title(&target, app);
        let disambiguation = (self.project_title_occurrences(&title, app) > 1)
            .then(|| project_disambiguation(Some(&path), 0));
        let keyboard_focused = self.projects_search_open
            && self
                .matching_project_targets(app)
                .get(self.projects_search_selection)
                .is_some_and(|selected| selected == &target);
        let color: ThemeFill = FeatureFlag::DirectoryTabColors
            .is_enabled()
            .then(|| {
                TabSettings::as_ref(app)
                    .directory_tab_colors
                    .value()
                    .color_for_directory(&path)
                    .and_then(|color| color.ansi_color())
            })
            .flatten()
            .map(|color| {
                ThemeFill::Solid(color.to_tab_color(&theme.terminal_colors().normal).into())
            })
            .unwrap_or_else(|| theme.hint_text_color(theme.background()));

        let is_renaming_project = self.project_being_renamed.as_ref() == Some(&path);
        let mut labels = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
        labels.add_child(if is_renaming_project {
            ChildView::new(&self.project_rename_editor).finish()
        } else {
            Text::new_inline(title, appearance.ui_font_family(), SIDEBAR_ROW_TEXT.size)
                .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
                .with_color(theme.main_text_color(theme.background()).into())
                .finish()
        });
        if let Some(disambiguation) = disambiguation {
            labels.add_child(
                Text::new_inline(
                    disambiguation,
                    appearance.ui_font_family(),
                    SIDEBAR_META_TEXT.size,
                )
                .with_line_height_ratio(SIDEBAR_META_TEXT.line_height)
                .with_color(theme.hint_text_color(theme.background()).into())
                .finish(),
            );
        }

        let pane_drop_target = ProjectSidebarPaneDropTarget {
            project_root: Some(path.clone()),
            tab_insert_index: self.tabs.len(),
        };
        let pane_drop_active = self
            .hovered_project_pane_drop_target
            .as_ref()
            .is_some_and(|target| target == &pane_drop_target);
        let row_mouse_state = self
            .projects_sidebar_mouse_states
            .project_row(Some(path.clone()))
            .row;
        let row = Hoverable::new(row_mouse_state, move |state| {
            let identity = Container::new(sidebar_icon(Icon::CircleFilled, color))
                .with_padding_left(spacing::XS)
                .finish();
            let contents = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(identity)
                .with_child(Shrinkable::new(1., labels.finish()).finish())
                .finish();
            let mut container = Container::new(contents)
                .with_padding_left(spacing::SM)
                .with_padding_right(spacing::SM)
                .with_padding_top(spacing::XS)
                .with_padding_bottom(spacing::XS)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if pane_drop_active || keyboard_focused {
                container = container.with_background(theme.surface_3());
            } else if state.is_hovered() {
                container = container.with_background(theme.surface_overlay_2());
            }
            container.finish()
        })
        .on_click({
            let path = path.clone();
            move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::OpenProjectLibraryEntry {
                    path: path.clone(),
                })
            }
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::RenameProject {
                project_root: path.clone(),
            })
        })
        .with_cursor(Cursor::PointingHand)
        .finish();
        Some(
            DropTarget::new(
                row,
                ProjectSidebarPaneDropTargetData {
                    target: pane_drop_target,
                },
            )
            .finish(),
        )
    }

    pub(super) fn render_projects_sidebar(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut list = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let mut visible_projects = 0;
        let targets = self.project_list_targets(app);
        let project_roots = targets
            .iter()
            .map(|target| match target {
                ProjectListTarget::LiveProject(tab_indices) => tab_indices
                    .first()
                    .and_then(|index| self.tabs.get(*index))
                    .and_then(|tab| tab.project_root.clone()),
                ProjectListTarget::Library(path) => Some(path.clone()),
            })
            .collect();
        self.projects_sidebar_mouse_states
            .retain_project_rows(&project_roots);
        for target in &targets {
            let row = match target {
                ProjectListTarget::LiveProject(tab_indices) => {
                    self.render_live_project_group(tab_indices, app)
                }
                ProjectListTarget::Library(path) => {
                    self.render_library_project_row(path.clone(), app)
                }
            };
            if let Some(row) = row {
                list.add_child(row);
                visible_projects += 1;
            }
        }
        if targets.is_empty()
            || (visible_projects == 0 && !self.vertical_tabs_panel.search_query.is_empty())
        {
            let label = if targets.is_empty() {
                "No projects yet"
            } else {
                "No matching projects"
            };
            list.add_child(
                Container::new(
                    Text::new_inline(label, appearance.ui_font_family(), SIDEBAR_ROW_TEXT.size)
                        .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                )
                .with_uniform_padding(spacing::MD)
                .finish(),
            );
            if targets.is_empty() {
                list.add_child(
                    Hoverable::new(
                        self.projects_sidebar_mouse_states.empty_new_project.clone(),
                        move |state| {
                            let mut button = Container::new(
                                Flex::row()
                                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                    .with_spacing(spacing::SM)
                                    .with_child(sidebar_icon(
                                        Icon::Plus,
                                        theme.main_text_color(theme.background()),
                                    ))
                                    .with_child(
                                        Text::new_inline(
                                            "New project",
                                            appearance.ui_font_family(),
                                            SIDEBAR_ROW_TEXT.size,
                                        )
                                        .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
                                        .with_color(
                                            theme.main_text_color(theme.background()).into(),
                                        )
                                        .finish(),
                                    )
                                    .finish(),
                            )
                            .with_uniform_padding(spacing::SM)
                            .with_corner_radius(
                                CornerRadius::with_all(Radius::Pixels(radius::CARD)),
                            );
                            if state.is_hovered() {
                                button = button.with_background(theme.surface_overlay_2());
                            }
                            button.finish()
                        },
                    )
                    .on_click(|ctx, _, position| {
                        ctx.dispatch_typed_action(WorkspaceAction::ToggleProjectCreateMenu {
                            position,
                        })
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish(),
                );
            }
        }

        let scrollable = ClippedScrollable::vertical(
            self.projects_scroll_state.clone(),
            list.finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let settings_active = self
            .tabs
            .get(self.active_tab_index())
            .is_some_and(|tab| tab.pane_group.as_ref(app).has_settings_panes());
        let settings = Hoverable::new(self.projects_settings_mouse_state.clone(), move |state| {
            let mut row = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::SM)
                    .with_child(sidebar_icon(
                        Icon::Settings,
                        theme.sub_text_color(theme.background()),
                    ))
                    .with_child(
                        Text::new_inline(
                            "Settings",
                            appearance.ui_font_family(),
                            SIDEBAR_ROW_TEXT.size,
                        )
                        .with_line_height_ratio(SIDEBAR_ROW_TEXT.line_height)
                        .with_color(theme.main_text_color(theme.background()).into())
                        .finish(),
                    )
                    .finish(),
            )
            .with_uniform_padding(spacing::SM)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if settings_active {
                row = row.with_background(theme.surface_3());
            } else if state.is_hovered() {
                row = row.with_background(theme.surface_overlay_2());
            }
            row.finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::ShowSettings))
        .with_cursor(Cursor::PointingHand)
        .finish();

        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let is_fullscreen = app
            .windows()
            .platform_window(self.window_id)
            .is_some_and(|window| window.fullscreen_state() == FullscreenState::Fullscreen);
        let zoom = WindowSettings::as_ref(app).zoom_level.as_zoom_factor();
        if !is_fullscreen
            && traffic_light_data(app, self.window_id)
                .is_some_and(|data| data.side == TrafficLightSide::Left && data.width(zoom) > 0.)
        {
            column.add_child(
                ConstrainedBox::new(Empty::new().finish())
                    .with_height(TOTAL_TAB_BAR_HEIGHT)
                    .finish(),
            );
        }
        column.add_child(self.render_sidebar_actions(app));
        column.add_child(self.render_project_header(app));
        column.add_child(Expanded::new(1., scrollable).finish());
        column.add_child(
            Container::new(settings)
                .with_uniform_padding(spacing::SM)
                .finish(),
        );

        let content = Container::new(column.finish())
            .with_background(theme.surface_1())
            .with_border(Border::right(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .finish();
        let handle = ResizableData::handle(app)
            .as_ref(app)
            .get_handle(self.window_id, ModalType::ProjectsSidebarWidth)
            .unwrap_or_else(|| resizable_state_handle(DEFAULT_PROJECTS_SIDEBAR_WIDTH));
        SavePosition::new(
            Resizable::new(handle, content)
                .with_dragbar_side(DragBarSide::Right)
                .on_resize(|ctx, _| ctx.notify())
                .with_bounds_callback(Box::new(|window_size| {
                    let max_width = window_size.x() * MAXIMUM_PROJECTS_SIDEBAR_WIDTH_RATIO;
                    (
                        MINIMUM_PROJECTS_SIDEBAR_WIDTH,
                        max_width.max(MINIMUM_PROJECTS_SIDEBAR_WIDTH),
                    )
                }))
                .finish(),
            PROJECTS_SIDEBAR_POSITION_ID,
        )
        .finish()
    }

    fn render_activity_button(
        &self,
        tool: RightToolKind,
        icon: Icon,
        mouse_state: twarpui::elements::MouseStateHandle,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let active = self.right_tool_open && self.right_tool == tool;
        let tooltip_label = match tool {
            RightToolKind::Files => "Files",
            RightToolKind::Search => "Search",
            RightToolKind::CodeReview => "Code Review",
        };
        let ui_builder = appearance.ui_builder().clone();
        let tooltip = ui_builder
            .tool_tip(tooltip_label.to_owned())
            .build()
            .finish();
        let color = if active {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };
        let contents = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(sidebar_icon(icon, color));
        let default_styles = UiComponentStyles {
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(radius::CHIP))),
            padding: Some(Coords::uniform(spacing::SM)),
            ..Default::default()
        };
        let hovered_styles = UiComponentStyles {
            background: Some(theme.surface_overlay_2().into()),
            ..default_styles
        };
        let active_styles = UiComponentStyles {
            background: Some(theme.surface_3().into()),
            ..default_styles
        };
        let mut button = Button::new(
            mouse_state,
            default_styles,
            Some(hovered_styles),
            None,
            None,
        )
        .with_custom_label(contents.finish())
        .with_tooltip(move || tooltip);
        if active {
            button = button.active().with_active_styles(active_styles);
        }
        button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::ToggleRightTool(tool))
            })
            .finish()
    }

    pub(super) fn render_right_activity_strip(&self, app: &AppContext) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::XS)
                    .with_child(self.render_activity_button(
                        RightToolKind::Files,
                        Icon::Folder,
                        self.files_tool_mouse_state.clone(),
                        app,
                    ))
                    .with_child(self.render_activity_button(
                        RightToolKind::Search,
                        Icon::Search,
                        self.search_tool_mouse_state.clone(),
                        app,
                    ))
                    .with_child(self.render_activity_button(
                        RightToolKind::CodeReview,
                        Icon::Code1,
                        self.code_review_tool_mouse_state.clone(),
                        app,
                    ))
                    .finish(),
            )
            .with_background(theme.surface_1())
            .with_border(Border::left(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .with_padding_top(spacing::SM)
            .finish(),
        )
        .with_width(RIGHT_ACTIVITY_STRIP_WIDTH)
        .finish()
    }

    pub(super) fn render_projects_reopen_button(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_projects_top_button(
            Icon::LayoutAlt01,
            self.projects_toggle_mouse_state.clone(),
            WorkspaceAction::ToggleProjectsSidebar,
            app,
        )
    }

    pub(super) fn render_projects_search_button(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_projects_top_button(
            Icon::Search,
            self.projects_search_mouse_state.clone(),
            WorkspaceAction::TogglePalette {
                mode: PaletteMode::Command,
                source: PaletteSource::TitleBarSearchBar,
            },
            app,
        )
    }

    fn render_projects_top_button(
        &self,
        icon: Icon,
        mouse_state: twarpui::elements::MouseStateHandle,
        action: WorkspaceAction,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        Hoverable::new(mouse_state, move |state| {
            let color = if state.is_hovered() {
                theme.main_text_color(theme.background())
            } else {
                theme.sub_text_color(theme.background())
            };
            let mut button = Container::new(sidebar_chrome_icon(icon, color))
                .with_uniform_padding(spacing::SM)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)));
            if state.is_hovered() {
                button = button.with_background(theme.surface_overlay_2());
            }
            button.finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

#[cfg(test)]
#[path = "project_sidebar_tests.rs"]
mod tests;
