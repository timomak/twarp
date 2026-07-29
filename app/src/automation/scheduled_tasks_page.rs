//! twarp 20d: the real Scheduled Tasks automation page — a management UI over
//! the local task scheduler ([`super::scheduler`]).
//!
//! Layout mirrors [`super::plugins_page`]: a centered chrome-class column with
//! the task list as rows (name, human schedule + next run, provider chain,
//! enabled switch, status, Run now / Edit / Delete-with-confirm), an inline
//! expanding editor (no modals) with schedule preset chips, and a run-history
//! strip under the editor with "View session" links into stored sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{Local, TimeZone};
use claude_code::driver::{AgentProvider, PermissionMode};
use twarp_core::ui::tokens::{radius, spacing, type_ramp};
use twarpui::{
    elements::{
        new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig},
        Align, Border, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable, MainAxisSize, MouseStateHandle,
        ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
    },
    ui_components::switch::SwitchStateHandle,
    AppContext, SingletonEntity, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::automation::scheduler::{
    provider_from_name, provider_name, schedule_summary, validate_schedule, RunOutcome,
    ScheduledTaskEntry, SchedulerModel, TaskRunEntry,
};
use crate::editor::{EditorView, Event as EditorEvent};
use crate::search_bar::SearchBar;
use crate::view_components::{
    action_button::{ActionButton, DangerSecondaryTheme, NakedTheme, PrimaryTheme, SecondaryTheme},
    dropdown::DropdownItem,
    Dropdown,
};
use crate::workspace::WorkspaceAction;

use super::plugins_page::{
    new_multiline_editor, new_single_line_editor, render_form_field, render_form_label,
    render_labeled_switch,
};
use super::view::{AutomationView, AutomationViewAction};

/// Content column width; matches the conversation measure used app-wide.
const CONTENT_MAX_WIDTH: f32 = 720.;

/// Actions dispatched by the Scheduled Tasks page's controls, wrapped in
/// [`AutomationViewAction::ScheduledTasks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledTasksPageAction {
    OpenAdd,
    OpenEdit(String),
    /// First Delete click arms the confirm stage.
    RequestDelete(String),
    ConfirmDelete(String),
    ToggleEnabled(String),
    RunNow(String),
    /// Open the stored session backing a run (session id + task cwd).
    ViewSession(String, String),
    /// List filter chip ("all" | "active" | "paused").
    SetFilter(String),
    /// Empty-state suggestion row: open the Add form prefilled from the
    /// template with this id.
    UseSuggestion(String),
    /// Editor: schedule preset chip ("hourly" | "daily" | "weekly" | "custom").
    SetPreset(String),
    /// Editor: weekly day-of-week ("0".."6").
    SetWeeklyDay(String),
    /// Editor: provider / fallback / model / effort / permission mode
    /// dropdown selections (serialized values; empty = default/none).
    SetProvider(String),
    SetFallback(String),
    SetModel(String),
    SetEffort(String),
    SetPermissionMode(String),
    ToggleCatchUp,
    Save,
    Cancel,
}

fn action(inner: ScheduledTasksPageAction) -> AutomationViewAction {
    AutomationViewAction::ScheduledTasks(inner)
}

/// Persistent per-row UI handles so hover/switch state survives re-renders.
struct RowUi {
    enabled_switch: SwitchStateHandle,
    run_button: ViewHandle<ActionButton>,
    edit_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    confirm_delete_button: ViewHandle<ActionButton>,
}

/// Which subset of tasks the list shows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TaskFilter {
    All,
    Active,
    Paused,
}

impl TaskFilter {
    const ALL: [TaskFilter; 3] = [TaskFilter::All, TaskFilter::Active, TaskFilter::Paused];

    fn as_str(self) -> &'static str {
        match self {
            TaskFilter::All => "all",
            TaskFilter::Active => "active",
            TaskFilter::Paused => "paused",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == value)
    }

    fn label(self) -> &'static str {
        match self {
            TaskFilter::All => "All",
            TaskFilter::Active => "Active",
            TaskFilter::Paused => "Paused",
        }
    }

    fn matches(self, task: &ScheduledTaskEntry) -> bool {
        match self {
            TaskFilter::All => true,
            TaskFilter::Active => task.enabled,
            TaskFilter::Paused => !task.enabled,
        }
    }
}

/// A canned task template surfaced under the empty state (the reference
/// layout's Suggestions section) — clicking one opens the Add form prefilled.
struct TaskSuggestion {
    id: &'static str,
    name: &'static str,
    schedule_label: &'static str,
    cron: &'static str,
    description: &'static str,
    prompt: &'static str,
    icon: twarp_core::ui::Icon,
}

const SUGGESTIONS: [TaskSuggestion; 3] = [
    TaskSuggestion {
        id: "morning-brief",
        name: "Morning repo brief",
        schedule_label: "Weekdays at 9:00 AM",
        cron: "0 9 * * 1-5",
        description: "Start each weekday with a summary of new commits, open PRs, and failing CI",
        prompt: "Summarize activity in this repository since yesterday: new commits, \
                 open pull requests, failing CI, and anything that needs my attention. \
                 Keep it short and actionable.",
        icon: twarp_core::ui::Icon::Bell,
    },
    TaskSuggestion {
        id: "nightly-triage",
        name: "Nightly triage",
        schedule_label: "Daily at 10:00 PM",
        cron: "0 22 * * *",
        description: "Sweep the repo for TODOs, flaky tests, and stale branches every night",
        prompt: "Triage this repository: list new TODO/FIXME comments, tests that look \
                 flaky or are skipped, and branches with no activity for two weeks. \
                 Propose one cleanup for tomorrow.",
        icon: twarp_core::ui::Icon::Bug,
    },
    TaskSuggestion {
        id: "weekly-review",
        name: "Weekly review",
        schedule_label: "Fridays at 4:00 PM",
        cron: "0 16 * * 5",
        description: "Turn your recent work into a concise status update every Friday",
        prompt: "Look at the commits and merged pull requests from the last week and \
                 write a concise status update: what shipped, what's in progress, and \
                 what's next.",
        icon: twarp_core::ui::Icon::CalendarCheck,
    },
];

/// The schedule preset selected in the editor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SchedulePreset {
    Hourly,
    Daily,
    Weekly,
    Custom,
}

impl SchedulePreset {
    const ALL: [SchedulePreset; 4] = [
        SchedulePreset::Hourly,
        SchedulePreset::Daily,
        SchedulePreset::Weekly,
        SchedulePreset::Custom,
    ];

    fn as_str(self) -> &'static str {
        match self {
            SchedulePreset::Hourly => "hourly",
            SchedulePreset::Daily => "daily",
            SchedulePreset::Weekly => "weekly",
            SchedulePreset::Custom => "custom",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }

    fn label(self) -> &'static str {
        match self {
            SchedulePreset::Hourly => "Hourly",
            SchedulePreset::Daily => "Daily",
            SchedulePreset::Weekly => "Weekly",
            SchedulePreset::Custom => "Custom cron",
        }
    }
}

/// Break a stored 5-field cron down into the editor's preset + fields, when it
/// matches one of the simple shapes.
fn preset_from_cron(expr: &str) -> (SchedulePreset, String, String, u32) {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if let [min, hour, "*", "*", dow] = fields.as_slice() {
        if let Ok(minute) = min.parse::<u32>() {
            if *hour == "*" && *dow == "*" {
                return (SchedulePreset::Hourly, min.to_string(), String::new(), 1);
            }
            if let Ok(h) = hour.parse::<u32>() {
                let time = format!("{h:02}:{minute:02}");
                if *dow == "*" {
                    return (SchedulePreset::Daily, min.to_string(), time, 1);
                }
                if let Ok(day) = dow.parse::<u32>() {
                    if day <= 6 {
                        return (SchedulePreset::Weekly, min.to_string(), time, day);
                    }
                }
            }
        }
    }
    (SchedulePreset::Custom, String::new(), String::new(), 1)
}

const WEEKDAYS: [(&str, &str); 7] = [
    ("1", "Monday"),
    ("2", "Tuesday"),
    ("3", "Wednesday"),
    ("4", "Thursday"),
    ("5", "Friday"),
    ("6", "Saturday"),
    ("0", "Sunday"),
];

/// The inline Add / Edit form.
struct TaskEditor {
    /// `Some` when editing an existing task, `None` when adding.
    existing_id: Option<String>,
    name_editor: ViewHandle<EditorView>,
    prompt_editor: ViewHandle<EditorView>,
    cwd_editor: ViewHandle<EditorView>,
    /// Minute field for the Hourly preset ("0".."59").
    minute_editor: ViewHandle<EditorView>,
    /// "HH:MM" field for the Daily / Weekly presets.
    time_editor: ViewHandle<EditorView>,
    /// Raw 5-field cron for the Custom preset.
    cron_editor: ViewHandle<EditorView>,
    preset: SchedulePreset,
    preset_chips: Vec<ViewHandle<ActionButton>>,
    weekly_day: u32,
    weekly_day_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    provider: AgentProvider,
    provider_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    fallback: Option<AgentProvider>,
    fallback_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    model: Option<String>,
    model_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    effort: Option<String>,
    effort_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    permission_mode: Option<PermissionMode>,
    permission_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    catch_up: bool,
    catch_up_switch: SwitchStateHandle,
    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    /// Validation error surfaced above the Save row.
    error: Option<String>,
}

/// State backing the Scheduled Tasks page, owned by [`AutomationView`] when
/// its page is [`super::AutomationPage::ScheduledTasks`].
pub struct ScheduledTasksPageState {
    scroll_state: ClippedScrollStateHandle,
    add_button: ViewHandle<ActionButton>,
    /// Dedicated CTA for the empty state (20e) — a view handle can't be
    /// mounted both in the header and in the empty state at once.
    empty_add_button: ViewHandle<ActionButton>,
    search_editor: ViewHandle<EditorView>,
    search_bar: ViewHandle<SearchBar>,
    filter: TaskFilter,
    filter_chips: Vec<ViewHandle<ActionButton>>,
    /// One persistent hover state per suggestion row, keyed by template id.
    suggestion_hover: HashMap<&'static str, MouseStateHandle>,
    rows: HashMap<String, RowUi>,
    /// Task id whose Delete button is currently in its Confirm stage.
    pending_delete: Option<String>,
    editor: Option<TaskEditor>,
    /// Per-run "View session" buttons, keyed by run id.
    run_buttons: HashMap<String, ViewHandle<ActionButton>>,
}

impl ScheduledTasksPageState {
    pub fn new(ctx: &mut ViewContext<AutomationView>) -> Self {
        let add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add task", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(ScheduledTasksPageAction::OpenAdd));
            })
        });
        let empty_add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add task", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(ScheduledTasksPageAction::OpenAdd));
            })
        });
        let search_editor = new_single_line_editor("Search scheduled tasks", "", ctx);
        // The list is filtered from the buffer at render time; typing just
        // needs to trigger a re-render.
        ctx.subscribe_to_view(&search_editor, |_, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                ctx.notify();
            }
        });
        let search_bar = {
            let editor = search_editor.clone();
            ctx.add_typed_action_view(|_| SearchBar::new(editor))
        };
        let mut state = Self {
            scroll_state: Default::default(),
            add_button,
            empty_add_button,
            search_editor,
            search_bar,
            filter: TaskFilter::All,
            filter_chips: Self::new_filter_chips(TaskFilter::All, ctx),
            suggestion_hover: SUGGESTIONS
                .iter()
                .map(|s| (s.id, MouseStateHandle::default()))
                .collect(),
            rows: HashMap::new(),
            pending_delete: None,
            editor: None,
            run_buttons: HashMap::new(),
        };
        state.sync_rows(ctx);
        state
    }

    /// Keep one [`RowUi`] alive per task (created on demand, dropped when the
    /// task goes away).
    pub fn sync_rows(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let ids: Vec<String> = SchedulerModel::as_ref(ctx)
            .tasks()
            .iter()
            .map(|t| t.id.clone())
            .collect();
        self.rows.retain(|id, _| ids.iter().any(|i| i == id));
        if self
            .pending_delete
            .as_ref()
            .is_some_and(|id| !ids.iter().any(|i| i == id))
        {
            self.pending_delete = None;
        }
        for id in ids {
            if self.rows.contains_key(&id) {
                continue;
            }
            let run_id = id.clone();
            let run_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Run now", SecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::RunNow(
                        run_id.clone(),
                    )));
                })
            });
            let edit_id = id.clone();
            let edit_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Edit", SecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::OpenEdit(
                        edit_id.clone(),
                    )));
                })
            });
            let delete_id = id.clone();
            let delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::RequestDelete(
                        delete_id.clone(),
                    )));
                })
            });
            let confirm_id = id.clone();
            let confirm_delete_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Confirm delete", DangerSecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::ConfirmDelete(
                        confirm_id.clone(),
                    )));
                })
            });
            self.rows.insert(
                id,
                RowUi {
                    enabled_switch: Default::default(),
                    run_button,
                    edit_button,
                    delete_button,
                    confirm_delete_button,
                },
            );
        }
    }

    pub fn handle_action(
        &mut self,
        action: &ScheduledTasksPageAction,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        use ScheduledTasksPageAction::*;
        match action {
            OpenAdd => {
                self.pending_delete = None;
                self.editor = Some(self.new_editor(None, ctx));
            }
            OpenEdit(id) => {
                self.pending_delete = None;
                let entry = SchedulerModel::as_ref(ctx).get(id).cloned();
                if let Some(entry) = entry {
                    self.editor = Some(self.new_editor(Some(entry), ctx));
                }
            }
            RequestDelete(id) => {
                self.pending_delete = Some(id.clone());
            }
            ConfirmDelete(id) => {
                self.pending_delete = None;
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.existing_id.as_deref() == Some(id))
                {
                    self.editor = None;
                }
                SchedulerModel::handle(ctx).update(ctx, |m, mctx| m.delete(id, mctx));
                self.sync_rows(ctx);
            }
            ToggleEnabled(id) => {
                SchedulerModel::handle(ctx).update(ctx, |m, mctx| m.toggle_enabled(id, mctx));
            }
            RunNow(id) => {
                SchedulerModel::handle(ctx).update(ctx, |m, mctx| m.run_now(id, mctx));
            }
            ViewSession(session_id, cwd) => {
                self.open_session(session_id, cwd, ctx);
            }
            UseSuggestion(id) => {
                if let Some(template) = SUGGESTIONS.iter().find(|s| s.id == id) {
                    self.pending_delete = None;
                    let entry = ScheduledTaskEntry {
                        id: String::new(),
                        name: template.name.to_owned(),
                        prompt: template.prompt.to_owned(),
                        cwd: dirs::home_dir()
                            .map(|home| home.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        schedule: template.cron.to_owned(),
                        provider: AgentProvider::Claude,
                        fallback_provider: None,
                        model: None,
                        effort: None,
                        permission_mode: None,
                        enabled: true,
                        catch_up: false,
                        next_run_at: None,
                        created_at: 0,
                    };
                    self.editor = Some(self.new_editor(Some(entry), ctx));
                }
            }
            SetFilter(value) => {
                if let Some(filter) = TaskFilter::from_str(value) {
                    self.filter = filter;
                    self.filter_chips = Self::new_filter_chips(filter, ctx);
                }
            }
            SetPreset(value) => {
                if let (Some(editor), Some(preset)) =
                    (self.editor.as_mut(), SchedulePreset::from_str(value))
                {
                    editor.preset = preset;
                    let chips = Self::new_preset_chips(preset, ctx);
                    if let Some(editor) = self.editor.as_mut() {
                        editor.preset_chips = chips;
                    }
                }
            }
            SetWeeklyDay(value) => {
                if let (Some(editor), Ok(day)) = (self.editor.as_mut(), value.parse::<u32>()) {
                    if day <= 6 {
                        editor.weekly_day = day;
                    }
                }
            }
            SetProvider(value) => {
                if let (Some(editor), Some(provider)) =
                    (self.editor.as_mut(), provider_from_name(value))
                {
                    if editor.provider != provider {
                        editor.provider = provider;
                        // Model/effort/permission choices are per-provider;
                        // reset and rebuild their dropdowns.
                        editor.model = None;
                        editor.effort = None;
                        editor.permission_mode = None;
                        if editor.fallback == Some(provider) {
                            editor.fallback = None;
                        }
                        self.rebuild_provider_dependent_dropdowns(ctx);
                    }
                }
            }
            SetFallback(value) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.fallback = provider_from_name(value).filter(|p| *p != editor.provider);
                }
            }
            SetModel(value) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.model = (!value.is_empty()).then(|| value.clone());
                }
            }
            SetEffort(value) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.effort = (!value.is_empty()).then(|| value.clone());
                }
            }
            SetPermissionMode(value) => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.permission_mode = PermissionMode::from_cli_arg(value)
                        .filter(|m| *m != PermissionMode::BypassPermissions);
                }
            }
            ToggleCatchUp => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.catch_up = !editor.catch_up;
                }
            }
            Save => self.save(ctx),
            Cancel => self.editor = None,
        }
        ctx.notify();
    }

    /// "View session": resume the stored session in a Claude Code pane (the
    /// workspace focuses the pane instead if the session is still open).
    fn open_session(&self, session_id: &str, cwd: &str, ctx: &mut ViewContext<AutomationView>) {
        let cwd = PathBuf::from(cwd);
        let Some(jsonl_path) = claude_code::sessions::session_file(&cwd, session_id) else {
            return;
        };
        // Scheduled runs are Claude/Codex; the stored-session resume path only
        // covers the Claude jsonl store, so gate on the file existing.
        if !jsonl_path.exists() {
            return;
        }
        ctx.dispatch_typed_action(&WorkspaceAction::ResumeClaudeCodeSession {
            session_id: session_id.to_owned(),
            jsonl_path,
            cwd,
            provider: AgentProvider::Claude,
        });
    }

    fn new_filter_chips(
        selected: TaskFilter,
        ctx: &mut ViewContext<AutomationView>,
    ) -> Vec<ViewHandle<ActionButton>> {
        TaskFilter::ALL
            .into_iter()
            .map(|filter| {
                let value = filter.as_str().to_owned();
                ctx.add_typed_action_view(move |_| {
                    let button = if filter == selected {
                        ActionButton::new(filter.label(), SecondaryTheme)
                    } else {
                        ActionButton::new(filter.label(), NakedTheme)
                    };
                    button.on_click(move |ctx| {
                        ctx.dispatch_typed_action(action(ScheduledTasksPageAction::SetFilter(
                            value.clone(),
                        )));
                    })
                })
            })
            .collect()
    }

    fn new_preset_chips(
        selected: SchedulePreset,
        ctx: &mut ViewContext<AutomationView>,
    ) -> Vec<ViewHandle<ActionButton>> {
        SchedulePreset::ALL
            .into_iter()
            .map(|preset| {
                let value = preset.as_str().to_owned();
                ctx.add_typed_action_view(move |_| {
                    let button = if preset == selected {
                        ActionButton::new(preset.label(), PrimaryTheme)
                    } else {
                        ActionButton::new(preset.label(), SecondaryTheme)
                    };
                    button.on_click(move |ctx| {
                        ctx.dispatch_typed_action(action(ScheduledTasksPageAction::SetPreset(
                            value.clone(),
                        )));
                    })
                })
            })
            .collect()
    }

    fn rebuild_provider_dependent_dropdowns(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let provider = editor.provider;
        let fallback = editor.fallback;
        let model = editor.model.clone();
        let effort = editor.effort.clone();
        let permission_mode = editor.permission_mode;
        let fallback_dropdown = new_fallback_dropdown(provider, fallback, ctx);
        let model_dropdown = new_model_dropdown(provider, model.as_deref(), ctx);
        let effort_dropdown = new_effort_dropdown(provider, effort.as_deref(), ctx);
        let permission_dropdown = new_permission_dropdown(provider, permission_mode, ctx);
        if let Some(editor) = self.editor.as_mut() {
            editor.fallback_dropdown = fallback_dropdown;
            editor.model_dropdown = model_dropdown;
            editor.effort_dropdown = effort_dropdown;
            editor.permission_dropdown = permission_dropdown;
        }
    }

    fn new_editor(
        &self,
        entry: Option<ScheduledTaskEntry>,
        ctx: &mut ViewContext<AutomationView>,
    ) -> TaskEditor {
        let entry = entry.unwrap_or_else(|| ScheduledTaskEntry {
            id: String::new(),
            name: String::new(),
            prompt: String::new(),
            cwd: dirs::home_dir()
                .map(|home| home.to_string_lossy().into_owned())
                .unwrap_or_default(),
            schedule: "0 9 * * *".to_owned(),
            provider: AgentProvider::Claude,
            fallback_provider: None,
            model: None,
            effort: None,
            permission_mode: None,
            enabled: true,
            catch_up: false,
            next_run_at: None,
            created_at: 0,
        });
        let existing_id = (!entry.id.is_empty()).then(|| entry.id.clone());

        let (preset, minute, time, weekly_day) = preset_from_cron(&entry.schedule);
        let name_editor = new_single_line_editor("e.g. Nightly triage", &entry.name, ctx);
        let prompt_editor =
            new_multiline_editor("What should the agent do each run?", &entry.prompt, ctx);
        let cwd_editor = new_single_line_editor("Absolute directory path", &entry.cwd, ctx);
        let minute_editor = new_single_line_editor("Minute (0-59)", &minute, ctx);
        let time_editor = new_single_line_editor("HH:MM (24h)", &time, ctx);
        let cron_editor = new_single_line_editor(
            "min hour day month weekday, e.g. */30 9-17 * * 1-5",
            &entry.schedule,
            ctx,
        );

        let weekly_day_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                WEEKDAYS
                    .into_iter()
                    .map(|(value, label)| {
                        DropdownItem::new(
                            label,
                            action(ScheduledTasksPageAction::SetWeeklyDay(value.to_owned())),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_action(
                action(ScheduledTasksPageAction::SetWeeklyDay(
                    weekly_day.to_string(),
                )),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });

        let provider = entry.provider;
        let provider_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                [AgentProvider::Claude, AgentProvider::Codex]
                    .into_iter()
                    .map(|p| {
                        DropdownItem::new(
                            provider_label(p),
                            action(ScheduledTasksPageAction::SetProvider(
                                provider_name(p).to_owned(),
                            )),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_action(
                action(ScheduledTasksPageAction::SetProvider(
                    provider_name(provider).to_owned(),
                )),
                ctx,
            );
            dropdown.set_top_bar_max_width(220.);
            dropdown
        });

        let fallback_dropdown = new_fallback_dropdown(provider, entry.fallback_provider, ctx);
        let model_dropdown = new_model_dropdown(provider, entry.model.as_deref(), ctx);
        let effort_dropdown = new_effort_dropdown(provider, entry.effort.as_deref(), ctx);
        let permission_dropdown = new_permission_dropdown(provider, entry.permission_mode, ctx);

        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(ScheduledTasksPageAction::Save));
            })
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(ScheduledTasksPageAction::Cancel));
            })
        });

        TaskEditor {
            existing_id,
            name_editor,
            prompt_editor,
            cwd_editor,
            minute_editor,
            time_editor,
            cron_editor,
            preset,
            preset_chips: Self::new_preset_chips(preset, ctx),
            weekly_day,
            weekly_day_dropdown,
            provider,
            provider_dropdown,
            fallback: entry.fallback_provider,
            fallback_dropdown,
            model: entry.model.clone(),
            model_dropdown,
            effort: entry.effort.clone(),
            effort_dropdown,
            permission_mode: entry.permission_mode,
            permission_dropdown,
            catch_up: entry.catch_up,
            catch_up_switch: Default::default(),
            save_button,
            cancel_button,
            error: None,
        }
    }

    /// Build the canonical 5-field cron from the editor's preset + fields.
    fn canonical_cron(editor: &TaskEditor, ctx: &AppContext) -> Result<String, String> {
        let text =
            |handle: &ViewHandle<EditorView>| handle.as_ref(ctx).buffer_text(ctx).trim().to_owned();
        let parse_time = |value: &str| -> Result<(u32, u32), String> {
            let (h, m) = value
                .split_once(':')
                .ok_or_else(|| "Time must be HH:MM (24-hour).".to_owned())?;
            let hour: u32 = h.trim().parse().map_err(|_| "Invalid hour.".to_owned())?;
            let minute: u32 = m.trim().parse().map_err(|_| "Invalid minute.".to_owned())?;
            if hour > 23 || minute > 59 {
                return Err("Time must be HH:MM (24-hour).".to_owned());
            }
            Ok((hour, minute))
        };
        let cron = match editor.preset {
            SchedulePreset::Hourly => {
                let minute_text = text(&editor.minute_editor);
                let minute: u32 = if minute_text.is_empty() {
                    0
                } else {
                    minute_text
                        .parse()
                        .ok()
                        .filter(|m| *m <= 59)
                        .ok_or_else(|| "Minute must be 0-59.".to_owned())?
                };
                format!("{minute} * * * *")
            }
            SchedulePreset::Daily => {
                let (hour, minute) = parse_time(&text(&editor.time_editor))?;
                format!("{minute} {hour} * * *")
            }
            SchedulePreset::Weekly => {
                let (hour, minute) = parse_time(&text(&editor.time_editor))?;
                format!("{minute} {hour} * * {}", editor.weekly_day)
            }
            SchedulePreset::Custom => text(&editor.cron_editor),
        };
        validate_schedule(&cron)?;
        Ok(cron)
    }

    /// Validate the form and commit it to the scheduler.
    fn save(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let text =
            |handle: &ViewHandle<EditorView>| handle.as_ref(ctx).buffer_text(ctx).trim().to_owned();
        let name = text(&editor.name_editor);
        let prompt = editor
            .prompt_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();
        let cwd = text(&editor.cwd_editor);

        let cron = match Self::canonical_cron(editor, ctx) {
            Ok(cron) => cron,
            Err(error) => {
                editor.error = Some(error);
                return;
            }
        };
        let error = if name.is_empty() {
            Some("Name is required.".to_owned())
        } else if SchedulerModel::as_ref(ctx).name_taken(&name, editor.existing_id.as_deref()) {
            Some("A task with this name already exists.".to_owned())
        } else if prompt.is_empty() {
            Some("Prompt is required.".to_owned())
        } else if cwd.is_empty() {
            Some("Working directory is required.".to_owned())
        } else if !Path::new(&cwd).is_dir() {
            Some("Working directory does not exist.".to_owned())
        } else {
            None
        };
        if let Some(error) = error {
            editor.error = Some(error);
            return;
        }

        let existing = editor
            .existing_id
            .as_deref()
            .and_then(|id| SchedulerModel::as_ref(ctx).get(id).cloned());
        let entry = ScheduledTaskEntry {
            id: editor
                .existing_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            prompt,
            cwd,
            schedule: cron,
            provider: editor.provider,
            fallback_provider: editor.fallback.filter(|p| *p != editor.provider),
            model: editor.model.clone(),
            effort: editor.effort.clone(),
            permission_mode: editor.permission_mode,
            enabled: existing.as_ref().map(|t| t.enabled).unwrap_or(true),
            catch_up: editor.catch_up,
            next_run_at: None, // recomputed by upsert
            created_at: existing
                .map(|t| t.created_at)
                .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        };
        SchedulerModel::handle(ctx).update(ctx, |m, mctx| m.upsert(entry, mctx));
        self.editor = None;
        self.sync_rows(ctx);
    }

    /// Keep one "View session" button alive per visible run.
    fn sync_run_buttons(
        &mut self,
        task: &ScheduledTaskEntry,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        let runs: Vec<(String, String)> = SchedulerModel::as_ref(ctx)
            .runs_for(&task.id)
            .iter()
            .filter_map(|run| {
                run.session_id
                    .clone()
                    .map(|session| (run.id.clone(), session))
            })
            .collect();
        let cwd = task.cwd.clone();
        self.run_buttons
            .retain(|id, _| runs.iter().any(|(rid, _)| rid == id));
        for (run_id, session_id) in runs {
            if self.run_buttons.contains_key(&run_id) {
                continue;
            }
            let cwd = cwd.clone();
            let button = ctx.add_typed_action_view(|_| {
                ActionButton::new("View session", SecondaryTheme).on_click(move |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::ViewSession(
                        session_id.clone(),
                        cwd.clone(),
                    )));
                })
            });
            self.run_buttons.insert(run_id, button);
        }
    }

    /// Called by [`AutomationView`] before render when the scheduler model
    /// notified — keeps row/run button maps in sync with the task list.
    pub fn sync_from_model(&mut self, ctx: &mut ViewContext<AutomationView>) {
        self.sync_rows(ctx);
        let editing: Option<ScheduledTaskEntry> = self
            .editor
            .as_ref()
            .and_then(|e| e.existing_id.as_deref())
            .and_then(|id| SchedulerModel::as_ref(ctx).get(id).cloned());
        if let Some(task) = editing {
            self.sync_run_buttons(&task, ctx);
        }
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let scheduler = SchedulerModel::as_ref(app);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // 20e: while the empty state (with its own CTA) shows, the header's
        // Add button is hidden — one clear next action, not two.
        let show_empty_state = scheduler.tasks().is_empty() && self.editor.is_none();

        // Header: title left, Add button right.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new_inline(
                            "Scheduled Tasks",
                            appearance.ui_font_family(),
                            type_ramp::HEADING.size,
                        )
                        .with_line_height_ratio(type_ramp::HEADING.line_height)
                        .with_color(theme.main_text_color(theme.background()).into())
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );
        if !show_empty_state {
            header.add_child(ChildView::new(&self.add_button).finish());
        }
        column.add_child(header.finish());
        column.add_child(
            Container::new(
                Text::new_inline(
                    "Tasks run a headless agent session on a schedule while twarp is open. \
                     Each run opens as a background tab you can inspect.",
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::LG)
            .finish(),
        );

        if !show_empty_state {
            column.add_child(
                Container::new(ChildView::new(&self.search_bar).finish())
                    .with_margin_bottom(spacing::MD)
                    .finish(),
            );
            let mut chips = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::XS);
            for chip in &self.filter_chips {
                chips.add_child(ChildView::new(chip).finish());
            }
            column.add_child(
                Container::new(chips.finish())
                    .with_margin_bottom(spacing::MD)
                    .finish(),
            );
        }

        if let Some(editor) = &self.editor {
            column.add_child(self.render_editor(editor, app));
            if let Some(id) = editor.existing_id.as_deref() {
                if let Some(task) = scheduler.get(id) {
                    column.add_child(self.render_run_history(task, app));
                }
            }
        }

        let tasks = scheduler.tasks();
        if show_empty_state {
            column.add_child(super::render_empty_state(
                twarp_core::ui::Icon::Clock,
                "Automate twarp on a schedule",
                "Run an agent on a cron — daily briefs, nightly triage, repo monitors. \
                 Each run lands as a background tab you can inspect.",
                &self.empty_add_button,
                app,
            ));
            column.add_child(self.render_suggestions(app));
        }
        let query = self
            .search_editor
            .as_ref(app)
            .buffer_text(app)
            .trim()
            .to_lowercase();
        let visible: Vec<&ScheduledTaskEntry> = tasks
            .iter()
            .filter(|t| self.filter.matches(t))
            .filter(|t| query.is_empty() || t.name.to_lowercase().contains(&query))
            .collect();
        if !show_empty_state && visible.is_empty() {
            column.add_child(
                Text::new_inline(
                    "No tasks match.",
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            );
        }
        for task in visible {
            column.add_child(self.render_row(task, app));
        }

        let content = Container::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(CONTENT_MAX_WIDTH)
                .finish(),
        )
        .with_uniform_padding(spacing::XL)
        .finish();

        let centered = Align::new(content).top_center().finish();

        let scrollable = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: centered,
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            twarpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        // The page nests editors (form fields); propagate deltas this vertical
        // scrollable can't act on so they still reach the inner editors.
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        Container::new(scrollable)
            .with_background(theme.background())
            .finish()
    }

    /// One task row: name + schedule/next-run/provider-chain/status left;
    /// enabled switch, Run now, Edit, Delete right.
    fn render_row(&self, task: &ScheduledTaskEntry, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let scheduler = SchedulerModel::as_ref(app);
        let Some(row_ui) = self.rows.get(&task.id) else {
            // A task created outside this view's actions; drawn without
            // controls until the next sync. (No Empty stubs — they lay out at
            // constraint.max.)
            return Flex::row().with_main_axis_size(MainAxisSize::Min).finish();
        };

        let chain = match task.fallback_provider {
            Some(fallback) => format!(
                "{} → {}",
                provider_label(task.provider),
                provider_label(fallback)
            ),
            None => provider_label(task.provider).to_owned(),
        };
        let mut sub = format!("{} · {}", schedule_summary(&task.schedule), chain);
        if !task.enabled {
            sub.push_str(" · Paused");
        } else if let Some(next) = task.next_run_at {
            sub.push_str(&format!(" · Next run {}", format_relative(next)));
        }

        let running = scheduler.is_running(&task.id);
        // Only surface a third line while running or after a failure — healthy
        // rows stay at two lines like the reference layout.
        let status = if running {
            Some(("Running…".to_owned(), theme.active_ui_detail()))
        } else {
            scheduler
                .last_run(&task.id)
                .filter(|run| matches!(run.outcome, RunOutcome::Error | RunOutcome::BothFailed))
                .map(|run| (run_one_liner(run), theme.ui_error_color().into()))
        };

        // Paused rows render dimmed.
        let name_color = if task.enabled {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };
        let mut labels = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    task.name.clone(),
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(name_color.into())
                .finish(),
            )
            .with_child(
                Text::new_inline(sub, appearance.ui_font_family(), type_ramp::CAPTION.size)
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
            );
        if let Some((label, color)) = status {
            labels.add_child(
                Text::new_inline(label, appearance.ui_font_family(), type_ramp::CAPTION.size)
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(color.into())
                    .finish(),
            );
        }

        let toggle_id = task.id.clone();
        let delete_button = if self.pending_delete.as_deref() == Some(task.id.as_str()) {
            &row_ui.confirm_delete_button
        } else {
            &row_ui.delete_button
        };
        let controls = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(render_labeled_switch(
                "Enabled",
                task.enabled,
                row_ui.enabled_switch.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::ToggleEnabled(
                        toggle_id.clone(),
                    )));
                },
                appearance,
            ))
            .with_child(ChildView::new(&row_ui.run_button).finish())
            .with_child(ChildView::new(&row_ui.edit_button).finish())
            .with_child(ChildView::new(delete_button).finish())
            .finish();

        // Leading status glyph: running clock, paused play affordance,
        // otherwise a quiet circle — the reference list's left rail.
        let (glyph, glyph_color) = if running {
            (twarp_core::ui::Icon::ClockLoader, theme.active_ui_detail())
        } else if task.enabled {
            (
                twarp_core::ui::Icon::Circle,
                theme.sub_text_color(theme.background()),
            )
        } else {
            (
                twarp_core::ui::Icon::Play,
                theme.sub_text_color(theme.background()),
            )
        };
        let glyph = ConstrainedBox::new(glyph.to_warpui_icon(glyph_color).finish())
            .with_width(spacing::LG)
            .with_height(spacing::LG)
            .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::MD)
                .with_child(glyph)
                .with_child(Shrinkable::new(1., labels.finish()).finish())
                .with_child(controls)
                .finish(),
        )
        .with_padding_top(spacing::SM)
        .with_padding_bottom(spacing::SM)
        .finish()
    }

    /// The Suggestions section under the empty state: canned task templates
    /// as hoverable rows — tinted icon chip, name + schedule, description.
    /// Clicking one opens the Add form prefilled.
    fn render_suggestions(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(
            Container::new(
                Text::new_inline(
                    "SUGGESTIONS",
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(sub.into())
                .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        for (index, suggestion) in SUGGESTIONS.iter().enumerate() {
            let Some(state) = self.suggestion_hover.get(suggestion.id) else {
                continue;
            };
            // Rotate through the tinted chip colors so the list reads as a
            // set of distinct, lively entry points rather than a gray column.
            let (icon_color, tint) = match index % 3 {
                0 => (theme.accent(), theme.accent_overlay()),
                1 => (theme.ui_green_color().into(), theme.green_overlay_1()),
                _ => (theme.ui_yellow_color().into(), theme.yellow_overlay_1()),
            };
            let icon = suggestion.icon;
            let name = suggestion.name;
            let schedule_label = suggestion.schedule_label;
            let description = suggestion.description;
            let family = appearance.ui_font_family();
            let main = theme.main_text_color(theme.background());
            let row = Hoverable::new(state.clone(), move |hover| {
                let title_row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::SM)
                    .with_child(
                        Text::new_inline(name, family, type_ramp::UI.size)
                            .with_line_height_ratio(type_ramp::UI.line_height)
                            .with_color(main.into())
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline(schedule_label, family, type_ramp::CAPTION.size)
                            .with_line_height_ratio(type_ramp::CAPTION.line_height)
                            .with_color(sub.into())
                            .finish(),
                    )
                    .finish();
                let labels = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(spacing::XXS)
                    .with_child(title_row)
                    .with_child(
                        Text::new(description, family, type_ramp::CAPTION.size)
                            .with_line_height_ratio(type_ramp::CAPTION.line_height)
                            .with_color(sub.into())
                            .finish(),
                    )
                    .finish();
                let mut container = Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(spacing::MD)
                        .with_child(super::render_icon_chip(
                            icon,
                            icon_color,
                            tint,
                            spacing::LG,
                            spacing::SM,
                        ))
                        .with_child(Shrinkable::new(1., labels).finish())
                        .finish(),
                )
                .with_uniform_padding(spacing::MD)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
                if hover.is_hovered() {
                    container = container.with_background(theme.surface_overlay_1());
                }
                container.finish()
            });
            let id = suggestion.id;
            column.add_child(
                row.on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::UseSuggestion(
                        id.to_owned(),
                    )));
                })
                .finish(),
            );
        }

        Container::new(column.finish())
            .with_margin_top(spacing::LG)
            .finish()
    }

    /// The inline Add / Edit form.
    fn render_editor(&self, editor: &TaskEditor, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut form = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        form.add_child(render_form_label(
            if editor.existing_id.is_some() {
                "Edit task"
            } else {
                "New task"
            },
            type_ramp::UI.size,
            appearance,
        ));

        form.add_child(render_form_field("Name", &editor.name_editor, appearance));
        form.add_child(render_form_field(
            "Prompt",
            &editor.prompt_editor,
            appearance,
        ));
        form.add_child(render_form_field(
            "Working directory",
            &editor.cwd_editor,
            appearance,
        ));

        // Schedule: preset chips, then the preset's fields.
        form.add_child(render_form_label(
            "Schedule",
            type_ramp::LABEL.size,
            appearance,
        ));
        let mut chips = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::XS);
        for chip in &editor.preset_chips {
            chips.add_child(ChildView::new(chip).finish());
        }
        form.add_child(
            Container::new(chips.finish())
                .with_margin_bottom(spacing::SM)
                .finish(),
        );
        match editor.preset {
            SchedulePreset::Hourly => {
                form.add_child(render_form_field(
                    "At minute",
                    &editor.minute_editor,
                    appearance,
                ));
            }
            SchedulePreset::Daily => {
                form.add_child(render_form_field(
                    "At time",
                    &editor.time_editor,
                    appearance,
                ));
            }
            SchedulePreset::Weekly => {
                form.add_child(
                    Container::new(
                        Flex::column()
                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                            .with_child(render_form_label(
                                "On day",
                                type_ramp::LABEL.size,
                                appearance,
                            ))
                            .with_child(ChildView::new(&editor.weekly_day_dropdown).finish())
                            .finish(),
                    )
                    .with_margin_bottom(spacing::SM)
                    .finish(),
                );
                form.add_child(render_form_field(
                    "At time",
                    &editor.time_editor,
                    appearance,
                ));
            }
            SchedulePreset::Custom => {
                form.add_child(render_form_field(
                    "Cron expression",
                    &editor.cron_editor,
                    appearance,
                ));
            }
        }

        // Provider row: provider + fallback side by side.
        form.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(spacing::LG)
                .with_child(render_dropdown_field(
                    "Provider",
                    &editor.provider_dropdown,
                    appearance,
                ))
                .with_child(render_dropdown_field(
                    "Fallback",
                    &editor.fallback_dropdown,
                    appearance,
                ))
                .finish(),
        );
        form.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(spacing::LG)
                .with_child(render_dropdown_field(
                    "Model",
                    &editor.model_dropdown,
                    appearance,
                ))
                .with_child(render_dropdown_field(
                    "Effort",
                    &editor.effort_dropdown,
                    appearance,
                ))
                .with_child(render_dropdown_field(
                    "Permissions",
                    &editor.permission_dropdown,
                    appearance,
                ))
                .finish(),
        );

        form.add_child(
            Container::new(render_labeled_switch(
                "Catch up on missed runs at launch",
                editor.catch_up,
                editor.catch_up_switch.clone(),
                |ctx| {
                    ctx.dispatch_typed_action(action(ScheduledTasksPageAction::ToggleCatchUp));
                },
                appearance,
            ))
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::SM)
            .finish(),
        );

        if let Some(error) = &editor.error {
            form.add_child(
                Container::new(
                    Text::new_inline(
                        error.clone(),
                        appearance.ui_font_family(),
                        type_ramp::LABEL.size,
                    )
                    .with_line_height_ratio(type_ramp::LABEL.line_height)
                    .with_color(theme.ui_error_color().into())
                    .finish(),
                )
                .with_margin_bottom(spacing::SM)
                .finish(),
            );
        }

        form.add_child(
            Flex::row()
                .with_spacing(spacing::SM)
                .with_child(ChildView::new(&editor.save_button).finish())
                .with_child(ChildView::new(&editor.cancel_button).finish())
                .finish(),
        );

        Container::new(form.finish())
            .with_uniform_padding(spacing::LG)
            .with_margin_bottom(spacing::LG)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }

    /// The run-history strip under the editor: last runs with time, provider,
    /// outcome, and a "View session" action when a session exists.
    fn render_run_history(&self, task: &ScheduledTaskEntry, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let runs = SchedulerModel::as_ref(app).runs_for(&task.id);

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(render_form_label(
            "RECENT RUNS",
            type_ramp::CAPTION.size,
            appearance,
        ));
        if runs.is_empty() {
            column.add_child(
                Text::new_inline(
                    "This task has not run yet.",
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            );
        }
        for run in runs {
            let color = match run.outcome {
                RunOutcome::Success => theme.sub_text_color(theme.background()),
                RunOutcome::Running => theme.active_ui_detail(),
                RunOutcome::Error | RunOutcome::BothFailed => theme.ui_error_color().into(),
            };
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(
                    Shrinkable::new(
                        1.,
                        Text::new_inline(
                            run_one_liner(run),
                            appearance.ui_font_family(),
                            type_ramp::CAPTION.size,
                        )
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(color.into())
                        .finish(),
                    )
                    .finish(),
                );
            if run.session_id.is_some() {
                if let Some(button) = self.run_buttons.get(&run.id) {
                    row.add_child(ChildView::new(button).finish());
                }
            }
            column.add_child(
                Container::new(row.finish())
                    .with_padding_top(spacing::XXS)
                    .with_padding_bottom(spacing::XXS)
                    .finish(),
            );
        }

        Container::new(column.finish())
            .with_uniform_padding(spacing::LG)
            .with_margin_bottom(spacing::LG)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }
}

fn provider_label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Claude",
        AgentProvider::Codex => "Codex",
    }
}

/// "Jul 28 14:30" in local time — static absolute times, no ticking labels.
fn format_unix(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%b %-d %H:%M").to_string())
        .unwrap_or_default()
}

/// "in 2 minutes" / "in 15 hours" / "in 3 days" for a future unix timestamp.
/// Kept fresh by the page's 30 s repaint tick (see `AutomationView::new`).
fn format_relative(ts: i64) -> String {
    let delta = ts - chrono::Utc::now().timestamp();
    if delta <= 60 {
        return "in under a minute".to_owned();
    }
    let (value, unit) = if delta < 3600 {
        (delta / 60, "minute")
    } else if delta < 86_400 {
        (delta / 3600, "hour")
    } else {
        (delta / 86_400, "day")
    };
    let plural = if value == 1 { "" } else { "s" };
    format!("in {value} {unit}{plural}")
}

/// One-line run description for the list row and history strip.
fn run_one_liner(run: &TaskRunEntry) -> String {
    let when = format_unix(run.started_at);
    let provider = provider_label(run.provider_used);
    let outcome = match run.outcome {
        RunOutcome::Running => "running…",
        RunOutcome::Success => "succeeded",
        RunOutcome::Error => "failed",
        RunOutcome::BothFailed => "both providers failed",
    };
    match run.summary.as_deref().filter(|s| !s.is_empty()) {
        Some(summary) => {
            let mut summary = summary.replace('\n', " ");
            if summary.chars().count() > 80 {
                summary = summary.chars().take(80).collect::<String>() + "…";
            }
            format!("{when} · {provider} · {outcome} — {summary}")
        }
        None => format!("{when} · {provider} · {outcome}"),
    }
}

/// A labeled dropdown, mirroring `render_form_field`'s label styling.
fn render_dropdown_field(
    label: &str,
    dropdown: &ViewHandle<Dropdown<AutomationViewAction>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(render_form_label(label, type_ramp::LABEL.size, appearance))
            .with_child(ChildView::new(dropdown).finish())
            .finish(),
    )
    .with_margin_bottom(spacing::SM)
    .finish()
}

fn new_fallback_dropdown(
    provider: AgentProvider,
    selected: Option<AgentProvider>,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<Dropdown<AutomationViewAction>> {
    let selected = selected.filter(|p| *p != provider);
    ctx.add_typed_action_view(move |ctx| {
        let mut dropdown = Dropdown::new(ctx);
        let mut items = vec![DropdownItem::new(
            "None",
            action(ScheduledTasksPageAction::SetFallback(String::new())),
        )];
        for p in [AgentProvider::Claude, AgentProvider::Codex] {
            if p != provider {
                items.push(DropdownItem::new(
                    provider_label(p),
                    action(ScheduledTasksPageAction::SetFallback(
                        provider_name(p).to_owned(),
                    )),
                ));
            }
        }
        dropdown.set_items(items, ctx);
        dropdown.set_selected_by_action(
            action(ScheduledTasksPageAction::SetFallback(
                selected
                    .map(|p| provider_name(p).to_owned())
                    .unwrap_or_default(),
            )),
            ctx,
        );
        dropdown.set_top_bar_max_width(220.);
        dropdown
    })
}

/// Model / effort choices mirror the Agent settings page: the provider's
/// adapter enumerates them, with an empty value meaning "provider default".
fn new_model_dropdown(
    provider: AgentProvider,
    selected: Option<&str>,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<Dropdown<AutomationViewAction>> {
    let agent = cli_agent(provider);
    let selected = selected.map(str::to_owned);
    ctx.add_typed_action_view(move |ctx| {
        let mut dropdown = Dropdown::new(ctx);
        let mut items = vec![DropdownItem::new(
            "Provider default",
            action(ScheduledTasksPageAction::SetModel(String::new())),
        )];
        for option in agent.model_options() {
            items.push(DropdownItem::new(
                option.display_name.clone(),
                action(ScheduledTasksPageAction::SetModel(option.id.clone())),
            ));
        }
        dropdown.set_items(items, ctx);
        dropdown.set_selected_by_action(
            action(ScheduledTasksPageAction::SetModel(
                selected.clone().unwrap_or_default(),
            )),
            ctx,
        );
        dropdown.set_top_bar_max_width(220.);
        dropdown
    })
}

fn new_effort_dropdown(
    provider: AgentProvider,
    selected: Option<&str>,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<Dropdown<AutomationViewAction>> {
    let agent = cli_agent(provider);
    let selected = selected.map(str::to_owned);
    ctx.add_typed_action_view(move |ctx| {
        let mut dropdown = Dropdown::new(ctx);
        let mut items = vec![DropdownItem::new(
            "Provider default",
            action(ScheduledTasksPageAction::SetEffort(String::new())),
        )];
        for option in agent.effort_options() {
            items.push(DropdownItem::new(
                option.label,
                action(ScheduledTasksPageAction::SetEffort(option.value.to_owned())),
            ));
        }
        dropdown.set_items(items, ctx);
        dropdown.set_selected_by_action(
            action(ScheduledTasksPageAction::SetEffort(
                selected.clone().unwrap_or_default(),
            )),
            ctx,
        );
        dropdown.set_top_bar_max_width(220.);
        dropdown
    })
}

/// Permission modes for the provider, excluding BypassPermissions — a
/// scheduled, unattended run never bypasses permission prompts.
fn new_permission_dropdown(
    provider: AgentProvider,
    selected: Option<PermissionMode>,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<Dropdown<AutomationViewAction>> {
    let agent = cli_agent(provider);
    ctx.add_typed_action_view(move |ctx| {
        let mut dropdown = Dropdown::new(ctx);
        let mut items = vec![DropdownItem::new(
            "Provider default",
            action(ScheduledTasksPageAction::SetPermissionMode(String::new())),
        )];
        for mode in agent.permission_modes() {
            if *mode == PermissionMode::BypassPermissions || *mode == PermissionMode::Default {
                continue;
            }
            items.push(DropdownItem::new(
                permission_label(*mode),
                action(ScheduledTasksPageAction::SetPermissionMode(
                    mode.as_cli_arg().to_owned(),
                )),
            ));
        }
        dropdown.set_items(items, ctx);
        dropdown.set_selected_by_action(
            action(ScheduledTasksPageAction::SetPermissionMode(
                selected
                    .filter(|m| *m != PermissionMode::BypassPermissions)
                    .map(|m| m.as_cli_arg().to_owned())
                    .unwrap_or_default(),
            )),
            ctx,
        );
        dropdown.set_top_bar_max_width(220.);
        dropdown
    })
}

fn permission_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "Default (ask)",
        PermissionMode::AcceptEdits => "Accept edits",
        PermissionMode::Plan => "Plan mode",
        PermissionMode::BypassPermissions => "Bypass permissions",
    }
}

fn cli_agent(provider: AgentProvider) -> crate::app_state::CLIAgent {
    match provider {
        AgentProvider::Claude => crate::app_state::CLIAgent::Claude,
        AgentProvider::Codex => crate::app_state::CLIAgent::Codex,
    }
}
