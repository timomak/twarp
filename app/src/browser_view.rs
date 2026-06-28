use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use browser::BrowserEngine;
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use url::Url;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable,
};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::text_input::TextInput;
use warpui::SingletonEntity;
use warpui::{
    r#async::Timer, AfterLayoutContext, AppContext, Entity, EventContext, LayoutContext,
    PaintContext, SizeConstraint, TypedActionView, View, ViewContext, WindowId,
};

#[cfg(target_os = "macos")]
use warpui::platform::mac::{BrowserWebViewId, Window as MacWindow};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent,
};
use crate::themes::theme::Fill;
use crate::ui_components::{
    buttons,
    icons::{self, Icon},
};

const BROWSER_TITLE: &str = "Browser";
const OMNIBAR_PLACEHOLDER: &str = "Enter URL";
const TOOLBAR_HEIGHT: f32 = 36.;
const TOOLBAR_PADDING: f32 = 6.;
const LOADING_INDICATOR_HEIGHT: f32 = 2.;
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(250);
static NEXT_BROWSER_FOCUS_SEQ: AtomicU64 = AtomicU64::new(1);

#[cfg(not(target_os = "macos"))]
type BrowserWebViewId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserViewEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserViewAction {
    Back,
    Forward,
    ReloadOrStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserViewCustomAction {}

#[derive(Clone, Debug)]
pub(crate) struct BrowserAutomationTarget {
    pub engine: BrowserEngine,
    pub title: String,
    pub url: Option<String>,
    pub last_focus_seq: u64,
}

impl BrowserAutomationTarget {
    pub fn label(&self) -> String {
        match self.url.as_deref() {
            Some(url) if !url.is_empty() => format!("{} ({url})", self.title),
            _ => self.title.clone(),
        }
    }
}

#[derive(Default, PartialEq, Eq)]
struct NativeBrowserState {
    url: Option<String>,
    title: Option<String>,
    can_go_back: bool,
    can_go_forward: bool,
    is_loading: bool,
}

pub struct BrowserView {
    window_id: WindowId,
    engine: Option<BrowserEngine>,
    omnibar_editor: warpui::ViewHandle<EditorView>,
    pane_configuration: warpui::ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    current_url: Option<String>,
    title: String,
    can_go_back: bool,
    can_go_forward: bool,
    is_loading: bool,
    back_button: MouseStateHandle,
    forward_button: MouseStateHandle,
    reload_button: MouseStateHandle,
    last_focus_seq: u64,
}

impl BrowserView {
    pub fn new(initial_url: Option<String>, ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let editor_options = SingleLineEditorOptions {
            text: TextOptions::ui_font_size(appearance),
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Always,
            select_all_on_focus: true,
            clear_selections_on_blur: true,
            ..Default::default()
        };
        let omnibar_editor =
            ctx.add_typed_action_view(|ctx| EditorView::single_line(editor_options, ctx));
        ctx.subscribe_to_view(&omnibar_editor, Self::handle_omnibar_editor_event);
        omnibar_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(OMNIBAR_PLACEHOLDER, ctx);
        });

        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(BROWSER_TITLE));
        let window_id = ctx.window_id();
        let engine = BrowserEngine::new(window_id);

        let mut view = Self {
            window_id,
            engine,
            omnibar_editor,
            pane_configuration,
            focus_handle: None,
            current_url: None,
            title: BROWSER_TITLE.to_owned(),
            can_go_back: false,
            can_go_forward: false,
            is_loading: false,
            back_button: Default::default(),
            forward_button: Default::default(),
            reload_button: Default::default(),
            last_focus_seq: 0,
        };

        if let Some(url) = initial_url {
            view.navigate_to_normalized_url(url, ctx);
        }
        view.focus_omnibar(ctx);
        view.schedule_state_poll(ctx);
        view
    }

    pub fn new_restore(url: Option<String>, ctx: &mut ViewContext<Self>) -> Self {
        Self::new(url, ctx)
    }

    pub fn pane_configuration(&self) -> warpui::ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn snapshot_url(&self) -> Option<String> {
        self.current_url.clone()
    }

    pub fn focus_omnibar(&mut self, ctx: &mut ViewContext<Self>) {
        self.touch_focus();
        ctx.focus(&self.omnibar_editor);
    }

    pub fn focus_webview(&mut self) {
        self.touch_focus();
        if let Some(engine) = &self.engine {
            engine.focus();
        }
    }

    pub fn destroy_webview(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.destroy();
        }
    }

    pub fn browser_engine(&self) -> Option<&BrowserEngine> {
        self.engine.as_ref()
    }

    pub(crate) fn automation_target(&self) -> Option<BrowserAutomationTarget> {
        Some(BrowserAutomationTarget {
            engine: *self.engine.as_ref()?,
            title: self.title.clone(),
            url: self.current_url.clone(),
            last_focus_seq: self.last_focus_seq,
        })
    }

    fn touch_focus(&mut self) {
        self.last_focus_seq = NEXT_BROWSER_FOCUS_SEQ.fetch_add(1, Ordering::Relaxed);
    }

    fn handle_omnibar_editor_event(
        &mut self,
        _handle: warpui::ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Enter => {
                let input = self
                    .omnibar_editor
                    .as_ref(ctx)
                    .buffer_text(ctx)
                    .trim()
                    .to_owned();
                if let Some(url) = normalize_browser_url(&input) {
                    self.navigate_to_normalized_url(url, ctx);
                }
            }
            EditorEvent::Escape => {
                self.sync_omnibar_to_current_url(ctx);
            }
            _ => {}
        }
    }

    fn navigate_to_normalized_url(&mut self, url: String, ctx: &mut ViewContext<Self>) {
        if let Some(engine) = &self.engine {
            engine.load_url(&url);
        }

        self.current_url = Some(url.clone());
        self.is_loading = true;
        self.set_omnibar_text(&url, ctx);
        self.update_title_from_state(ctx);
        ctx.notify();
    }

    fn go_back(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(engine) = &self.engine {
            engine.go_back();
        }
        self.is_loading = true;
        ctx.notify();
    }

    fn go_forward(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(engine) = &self.engine {
            engine.go_forward();
        }
        self.is_loading = true;
        ctx.notify();
    }

    fn reload_or_stop(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(engine) = &self.engine {
            if self.is_loading {
                engine.stop_loading();
            } else {
                engine.reload();
            }
        }
        self.is_loading = !self.is_loading;
        ctx.notify();
    }

    fn schedule_state_poll(&self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(STATE_POLL_INTERVAL).await;
            },
            |me, _, ctx| {
                if me.engine.is_none() {
                    return;
                }

                if me.sync_native_state(ctx) {
                    ctx.notify();
                }
                me.schedule_state_poll(ctx);
            },
        );
    }

    fn sync_native_state(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let mut state = self.read_native_state();
        if state.url.is_none() && state.is_loading {
            state.url = self.current_url.clone();
        }
        let changed = self.current_url != state.url
            || self.title != self.title_for_state(&state)
            || self.can_go_back != state.can_go_back
            || self.can_go_forward != state.can_go_forward
            || self.is_loading != state.is_loading;

        self.current_url = state.url;
        self.can_go_back = state.can_go_back;
        self.can_go_forward = state.can_go_forward;
        self.is_loading = state.is_loading;

        if changed {
            self.update_title_from_state(ctx);
            if !self.omnibar_editor.as_ref(ctx).is_focused() {
                self.sync_omnibar_to_current_url(ctx);
            }
        }

        changed
    }

    fn read_native_state(&self) -> NativeBrowserState {
        if let Some(engine) = &self.engine {
            return NativeBrowserState {
                url: engine.current_url(),
                title: engine.title(),
                can_go_back: engine.can_go_back(),
                can_go_forward: engine.can_go_forward(),
                is_loading: engine.is_loading(),
            };
        }

        NativeBrowserState::default()
    }

    fn update_title_from_state(&mut self, ctx: &mut ViewContext<Self>) {
        let state = self.read_native_state();
        let title = self.title_for_state(&state);
        self.title = title.clone();
        self.pane_configuration
            .update(ctx, |config, ctx| config.set_title(title, ctx));
    }

    fn title_for_state(&self, state: &NativeBrowserState) -> String {
        state
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                state
                    .url
                    .as_deref()
                    .or(self.current_url.as_deref())
                    .and_then(host_from_url)
            })
            .unwrap_or_else(|| BROWSER_TITLE.to_owned())
    }

    fn sync_omnibar_to_current_url(&self, ctx: &mut ViewContext<Self>) {
        let text = self.current_url.as_deref().unwrap_or_default();
        self.set_omnibar_text(text, ctx);
    }

    fn set_omnibar_text(&self, text: &str, ctx: &mut ViewContext<Self>) {
        self.omnibar_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text_ignoring_undo(text, ctx)
        });
    }

    fn render_toolbar_button(
        &self,
        appearance: &Appearance,
        icon: Icon,
        enabled: bool,
        mouse_state: MouseStateHandle,
        action: BrowserViewAction,
    ) -> Box<dyn Element> {
        let mut button = buttons::icon_button(appearance, icon, false, mouse_state);
        if !enabled {
            button = button.disabled();
        }

        let mut hoverable = button.build();
        if enabled {
            hoverable = hoverable.on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            });
        }
        hoverable.finish()
    }

    fn render_toolbar(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let text_input = TextInput::new(
            self.omnibar_editor.clone(),
            UiComponentStyles::default()
                .set_background(theme.surface_1().into())
                .set_border_color(theme.outline().into())
                .set_border_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .set_height(24.),
        )
        .build()
        .finish();

        let reload_icon = if self.is_loading {
            icons::Icon::Stop
        } else {
            icons::Icon::RefreshCw04
        };

        ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(self.render_toolbar_button(
                        appearance,
                        icons::Icon::ArrowLeft,
                        self.can_go_back,
                        self.back_button.clone(),
                        BrowserViewAction::Back,
                    ))
                    .with_child(self.render_toolbar_button(
                        appearance,
                        icons::Icon::ArrowRight,
                        self.can_go_forward,
                        self.forward_button.clone(),
                        BrowserViewAction::Forward,
                    ))
                    .with_child(self.render_toolbar_button(
                        appearance,
                        reload_icon,
                        self.current_url.is_some() || self.is_loading,
                        self.reload_button.clone(),
                        BrowserViewAction::ReloadOrStop,
                    ))
                    .with_child(Shrinkable::new(1., text_input).finish())
                    .finish(),
            )
            .with_horizontal_padding(TOOLBAR_PADDING)
            .with_background(theme.background())
            .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
            .finish(),
        )
        .with_height(TOOLBAR_HEIGHT)
        .finish()
    }

    fn render_loading_indicator(&self, app: &AppContext) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        let fill = if self.is_loading {
            theme.accent()
        } else {
            Fill::Solid(pathfinder_color::ColorU::transparent_black())
        };

        ConstrainedBox::new(
            Container::new(Flex::row().finish())
                .with_background(fill)
                .finish(),
        )
        .with_height(LOADING_INDICATOR_HEIGHT)
        .finish()
    }
}

impl Drop for BrowserView {
    fn drop(&mut self) {
        self.destroy_webview();
    }
}

impl Entity for BrowserView {
    type Event = BrowserViewEvent;
}

impl View for BrowserView {
    fn ui_name() -> &'static str {
        "BrowserView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_toolbar(app))
            .with_child(self.render_loading_indicator(app))
            .with_child(
                Shrinkable::new(1., NativeBrowserElement::new(self.engine.as_ref()).finish())
                    .finish(),
            )
            .finish()
    }

    fn on_window_closed(&mut self, _ctx: &mut ViewContext<Self>) {
        self.destroy_webview();
    }

    fn on_window_transferred(
        &mut self,
        _source_window_id: WindowId,
        target_window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        let url = self.current_url.clone();
        self.destroy_webview();
        self.window_id = target_window_id;
        self.engine = BrowserEngine::new(target_window_id);
        if let Some(url) = url {
            self.navigate_to_normalized_url(url, ctx);
        }
    }
}

impl TypedActionView for BrowserView {
    type Action = BrowserViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BrowserViewAction::Back => self.go_back(ctx),
            BrowserViewAction::Forward => self.go_forward(ctx),
            BrowserViewAction::ReloadOrStop => self.reload_or_stop(ctx),
        }
    }
}

impl BackingView for BrowserView {
    type PaneHeaderOverflowMenuAction = BrowserViewAction;
    type CustomAction = BrowserViewCustomAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.destroy_webview();
        ctx.emit(BrowserViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, _ctx: &mut ViewContext<Self>) {
        self.focus_webview();
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: self.title.clone(),
            title_secondary: self.current_url.clone(),
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
    webview: Option<(WindowId, BrowserWebViewId)>,
    size: Option<Vector2F>,
    origin: Option<warpui::elements::Point>,
}

impl NativeBrowserElement {
    fn new(engine: Option<&BrowserEngine>) -> Self {
        Self {
            webview: engine.map(|engine| (engine.window_id(), engine.webview_id())),
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
        self.origin = Some(warpui::elements::Point::from_vec2f(
            origin,
            ctx.scene.z_index(),
        ));

        #[cfg(target_os = "macos")]
        if let (Some((window_id, webview_id)), Some(size)) = (self.webview, self.size) {
            if size.x() > 0.0 && size.y() > 0.0 {
                MacWindow::set_browser_webview_frame(
                    window_id,
                    webview_id,
                    RectF::new(origin, size),
                );
                MacWindow::set_browser_webview_hidden(window_id, webview_id, false);
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

    fn origin(&self) -> Option<warpui::elements::Point> {
        self.origin
    }
}

pub(crate) fn normalize_browser_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return None;
    }

    if let Ok(url) = Url::parse(trimmed) {
        return Some(url.to_string());
    }

    if looks_like_localhost(trimmed) {
        return Url::parse(&format!("http://{trimmed}"))
            .ok()
            .map(|url| url.to_string());
    }

    if trimmed.contains('.') {
        return Url::parse(&format!("https://{trimmed}"))
            .ok()
            .map(|url| url.to_string());
    }

    None
}

fn looks_like_localhost(input: &str) -> bool {
    input == "localhost"
        || input
            .strip_prefix("localhost:")
            .is_some_and(|port| !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()))
}

fn host_from_url(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(ToOwned::to_owned))
}
