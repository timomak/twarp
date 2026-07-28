//! twarp 20a: the full-page main-pane view backing the automation surfaces
//! (Scheduled Tasks / Skills / MCPs). Placeholder content only — later phases
//! replace the body with the real page content per [`AutomationPage`].

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

use super::AutomationPage;

/// A pane view showing one automation page as a centered full-page column.
pub struct AutomationView {
    page: AutomationPage,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
}

impl AutomationView {
    pub fn new(page: AutomationPage, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(page.title()));
        Self {
            page,
            pane_configuration,
            focus_handle: None,
        }
    }

    pub fn page(&self) -> AutomationPage {
        self.page
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
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

/// Actions supported by the pane header's overflow menu (currently none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationViewAction {}

impl TypedActionView for AutomationView {
    type Action = AutomationViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        // AutomationViewAction is currently uninhabited.
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
        // Placeholder page has no focusable content yet.
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
