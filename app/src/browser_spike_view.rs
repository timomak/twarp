use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use warpui::elements::Point;
use warpui::text_layout::ClipConfig;
use warpui::{
    AfterLayoutContext, AppContext, Element, Entity, EventContext, LayoutContext, PaintContext,
    SizeConstraint, TypedActionView, View, ViewContext, WindowId,
};

#[cfg(target_os = "macos")]
use warpui::platform::mac::{BrowserWebViewId, Window as MacWindow};

use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent,
};

const BROWSER_SPIKE_TITLE: &str = "Browser spike";
const BROWSER_SPIKE_TEST_URL: &str = "https://example.com";

#[cfg(not(target_os = "macos"))]
type BrowserWebViewId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserSpikeViewEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserSpikeViewAction {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserSpikeViewCustomAction {}

pub struct BrowserSpikeView {
    window_id: WindowId,
    webview_id: Option<BrowserWebViewId>,
    pane_configuration: warpui::ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
}

impl BrowserSpikeView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(BROWSER_SPIKE_TITLE));
        let window_id = ctx.window_id();
        let webview_id = Self::create_webview(window_id);

        Self {
            window_id,
            webview_id,
            pane_configuration,
            focus_handle: None,
        }
    }

    pub fn pane_configuration(&self) -> warpui::ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&self) {
        #[cfg(target_os = "macos")]
        if let Some(webview_id) = self.webview_id {
            MacWindow::focus_browser_webview(self.window_id, webview_id);
        }
    }

    pub fn destroy_webview(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(webview_id) = self.webview_id.take() {
            MacWindow::destroy_browser_webview(self.window_id, webview_id);
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.webview_id = None;
        }
    }

    fn create_webview(window_id: WindowId) -> Option<BrowserWebViewId> {
        #[cfg(target_os = "macos")]
        {
            let webview_id = MacWindow::create_browser_webview(window_id)?;
            MacWindow::load_browser_webview_url(window_id, webview_id, BROWSER_SPIKE_TEST_URL);
            Some(webview_id)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = window_id;
            None
        }
    }
}

impl Drop for BrowserSpikeView {
    fn drop(&mut self) {
        self.destroy_webview();
    }
}

impl Entity for BrowserSpikeView {
    type Event = BrowserSpikeViewEvent;
}

impl View for BrowserSpikeView {
    fn ui_name() -> &'static str {
        "BrowserSpikeView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        NativeBrowserElement::new(self.window_id, self.webview_id).finish()
    }

    fn on_window_closed(&mut self, _ctx: &mut ViewContext<Self>) {
        self.destroy_webview();
    }

    fn on_window_transferred(
        &mut self,
        _source_window_id: WindowId,
        target_window_id: WindowId,
        _ctx: &mut ViewContext<Self>,
    ) {
        self.destroy_webview();
        self.window_id = target_window_id;
        self.webview_id = Self::create_webview(target_window_id);
    }
}

impl TypedActionView for BrowserSpikeView {
    type Action = BrowserSpikeViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl BackingView for BrowserSpikeView {
    type PaneHeaderOverflowMenuAction = BrowserSpikeViewAction;
    type CustomAction = BrowserSpikeViewCustomAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.destroy_webview();
        ctx.emit(BrowserSpikeViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, _ctx: &mut ViewContext<Self>) {
        self.focus();
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: BROWSER_SPIKE_TITLE.to_string(),
            title_secondary: Some(BROWSER_SPIKE_TEST_URL.to_string()),
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: StandardHeaderOptions {
                always_show_icons: true,
                ..StandardHeaderOptions::default()
            },
            title_on_double_click: None,
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

struct NativeBrowserElement {
    window_id: WindowId,
    webview_id: Option<BrowserWebViewId>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl NativeBrowserElement {
    fn new(window_id: WindowId, webview_id: Option<BrowserWebViewId>) -> Self {
        Self {
            window_id,
            webview_id,
            size: None,
            origin: None,
        }
    }

    fn finish(self) -> Box<dyn Element> {
        Box::new(self)
    }
}

impl Element for NativeBrowserElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = Vector2F::new(
            if constraint.max.x().is_finite() {
                constraint.max.x()
            } else {
                constraint.min.x()
            },
            if constraint.max.y().is_finite() {
                constraint.max.y()
            } else {
                constraint.min.y()
            },
        );
        self.size = Some(size);
        size
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        #[cfg(target_os = "macos")]
        if let (Some(webview_id), Some(size)) = (self.webview_id, self.size) {
            if size.x() > 0.0 && size.y() > 0.0 {
                MacWindow::set_browser_webview_frame(
                    self.window_id,
                    webview_id,
                    RectF::new(origin, size),
                );
                MacWindow::set_browser_webview_hidden(self.window_id, webview_id, false);
            }
        }
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn dispatch_event(
        &mut self,
        _event: &warpui::event::DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
