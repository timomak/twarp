//! twarp 20a/20b: the full-page main-pane view backing the automation surfaces
//! (Scheduled Tasks / Skills / MCPs). The MCPs page is a real management UI
//! over the MCP-server registry ([`super::mcps_page`]); the other pages remain
//! placeholders until their phases land.

use twarp_core::ui::tokens::{spacing, type_ramp};
use twarpui::elements::{
    Container, CrossAxisAlignment, Element, Flex, MainAxisAlignment, MainAxisSize, ParentElement,
    Text,
};
use twarpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent},
    BackingView, PaneConfiguration, PaneEvent,
};

use crate::pull_requests::list_page::{PullRequestsPageAction, PullRequestsPageState};

use super::mcps_page::{McpsPageAction, McpsPageState};
use super::scheduled_tasks_page::{ScheduledTasksPageAction, ScheduledTasksPageState};
use super::skills_page::{SkillsPageAction, SkillsPageState};
use super::AutomationPage;

/// A pane view showing one automation page as a centered full-page column.
pub struct AutomationView {
    page: AutomationPage,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// Present iff `page == AutomationPage::Mcps` (twarp 20b).
    mcps_state: Option<McpsPageState>,
    /// Present iff `page == AutomationPage::Skills` (twarp 20c).
    skills_state: Option<SkillsPageState>,
    /// Present iff `page == AutomationPage::ScheduledTasks` (twarp 20d).
    scheduled_state: Option<ScheduledTasksPageState>,
    /// Present iff `page == AutomationPage::PullRequests` (twarp 21a).
    pull_requests_state: Option<PullRequestsPageState>,
}

impl AutomationView {
    pub fn new(page: AutomationPage, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(page.title()));
        let mcps_state = (page == AutomationPage::Mcps).then(|| McpsPageState::new(ctx));
        let skills_state = (page == AutomationPage::Skills).then(|| {
            // Skills arrive asynchronously (background scans); keep row UI in
            // sync and re-render when the store model notifies.
            let store = crate::skills_store::SkillsStoreModel::handle(ctx);
            ctx.observe(&store, |view: &mut Self, _, ctx| {
                if let Some(state) = view.skills_state.as_mut() {
                    state.sync_rows(ctx);
                }
                ctx.notify();
            });
            SkillsPageState::new(ctx)
        });
        let scheduled_state = (page == AutomationPage::ScheduledTasks).then(|| {
            // Task/run state changes asynchronously (tick loop, run
            // completions); keep row UI in sync and re-render when the
            // scheduler notifies.
            let scheduler = crate::automation::scheduler::SchedulerModel::handle(ctx);
            ctx.observe(&scheduler, |view: &mut Self, _, ctx| {
                if let Some(mut state) = view.scheduled_state.take() {
                    state.sync_from_model(ctx);
                    view.scheduled_state = Some(state);
                }
                ctx.notify();
            });
            ScheduledTasksPageState::new(ctx)
        });
        let pull_requests_state = (page == AutomationPage::PullRequests).then(|| {
            // PR lists arrive asynchronously (background `gh` fetches);
            // resync the header controls and re-render when the store
            // notifies.
            let store = crate::pull_requests::PullRequestsStoreModel::handle(ctx);
            ctx.observe(&store, |view: &mut Self, _, ctx| {
                if let Some(mut state) = view.pull_requests_state.take() {
                    state.sync(ctx);
                    view.pull_requests_state = Some(state);
                }
                ctx.notify();
            });
            PullRequestsPageState::new(ctx)
        });
        Self {
            page,
            pane_configuration,
            focus_handle: None,
            mcps_state,
            skills_state,
            scheduled_state,
            pull_requests_state,
        }
    }

    pub fn page(&self) -> AutomationPage {
        self.page
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn render_placeholder(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let title = Text::new_inline(
            self.page.title(),
            appearance.ui_font_family(),
            type_ramp::HEADING.size,
        )
        .with_line_height_ratio(type_ramp::HEADING.line_height)
        .with_color(theme.main_text_color(theme.background()).into())
        .finish();

        let body = Text::new_inline(
            self.page.placeholder_body(),
            appearance.ui_font_family(),
            type_ramp::UI.size,
        )
        .with_line_height_ratio(type_ramp::UI.line_height)
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish();

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(title)
                .with_child(body)
                .finish(),
        )
        .with_background(theme.background())
        .with_uniform_padding(spacing::XL)
        .finish()
    }
}

impl Entity for AutomationView {
    type Event = PaneEvent;
}

impl View for AutomationView {
    fn ui_name() -> &'static str {
        "AutomationView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if let Some(state) = &self.mcps_state {
            return state.render(app);
        }
        if let Some(state) = &self.skills_state {
            return state.render(app);
        }
        if let Some(state) = &self.scheduled_state {
            return state.render(app);
        }
        if let Some(state) = &self.pull_requests_state {
            return state.render(app);
        }
        self.render_placeholder(app)
    }
}

/// Actions supported by the automation pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationViewAction {
    /// MCPs page controls (twarp 20b).
    Mcps(McpsPageAction),
    /// Skills page controls (twarp 20c).
    Skills(SkillsPageAction),
    /// Scheduled Tasks page controls (twarp 20d).
    ScheduledTasks(ScheduledTasksPageAction),
    /// Pull Requests page controls (twarp 21a).
    PullRequests(PullRequestsPageAction),
}

impl TypedActionView for AutomationView {
    type Action = AutomationViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AutomationViewAction::Mcps(action) => {
                if let Some(state) = self.mcps_state.as_mut() {
                    state.handle_action(action, ctx);
                }
            }
            AutomationViewAction::Skills(action) => {
                if let Some(mut state) = self.skills_state.take() {
                    state.handle_action(action, ctx);
                    self.skills_state = Some(state);
                }
            }
            AutomationViewAction::ScheduledTasks(action) => {
                if let Some(mut state) = self.scheduled_state.take() {
                    state.handle_action(action, ctx);
                    self.scheduled_state = Some(state);
                }
            }
            AutomationViewAction::PullRequests(action) => {
                if let Some(mut state) = self.pull_requests_state.take() {
                    state.handle_action(action, ctx);
                    self.pull_requests_state = Some(state);
                }
            }
        }
    }
}

impl BackingView for AutomationView {
    type PaneHeaderOverflowMenuAction = AutomationViewAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // No overflow menu items are registered.
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, _ctx: &mut ViewContext<Self>) {
        // The page has no single natural focus target.
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::simple(self.page.title())
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
