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

use super::mcps_page::{McpsPageAction, McpsPageState};
use super::AutomationPage;

/// A pane view showing one automation page as a centered full-page column.
pub struct AutomationView {
    page: AutomationPage,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    /// Present iff `page == AutomationPage::Mcps` (twarp 20b).
    mcps_state: Option<McpsPageState>,
}

impl AutomationView {
    pub fn new(page: AutomationPage, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(page.title()));
        let mcps_state = (page == AutomationPage::Mcps).then(|| McpsPageState::new(ctx));
        Self {
            page,
            pane_configuration,
            focus_handle: None,
            mcps_state,
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
        match &self.mcps_state {
            Some(state) => state.render(app),
            None => self.render_placeholder(app),
        }
    }
}

/// Actions supported by the automation pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationViewAction {
    /// MCPs page controls (twarp 20b).
    Mcps(McpsPageAction),
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
