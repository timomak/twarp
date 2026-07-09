use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use browser::{BrowserEngine, BrowserProfile};
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use twarpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Empty,
    Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    Shrinkable, Text,
};
use twarpui::text_layout::ClipConfig;
use twarpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use twarpui::ui_components::text_input::TextInput;
use twarpui::SingletonEntity;
use twarpui::{
    r#async::Timer, zoom::Scale, AfterLayoutContext, AppContext, Entity, EventContext,
    LayoutContext, PaintContext, SizeConstraint, TypedActionView, View, ViewContext, WindowId,
};
use url::Url;

#[cfg(target_os = "macos")]
use twarpui::platform::mac::{BrowserWebViewId, Window as MacWindow};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{
        self,
        header::{components::render_pane_header_buttons, PANE_HEADER_HEIGHT},
        HeaderContent,
    },
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
const TAB_TITLE_CLOSE_SPACING: f32 = 12.;
const TOOLBAR_PADDING: f32 = 6.;
const HISTORY_PANEL_ROW_HEIGHT: f32 = 28.;
const HISTORY_PANEL_MAX_ROWS: usize = 8;
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
    NewTab,
    NewPrivateTab,
    SelectTab(usize),
    CloseTab(usize),
    ToggleHistory,
    NavigateHistory(usize),
    ClearDefaultProfileData,
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

struct BrowserTab {
    engine: BrowserEngine,
    current_url: Option<String>,
    title: String,
    can_go_back: bool,
    can_go_forward: bool,
    is_loading: bool,
    tab_mouse_state: MouseStateHandle,
    close_button: MouseStateHandle,
}

impl BrowserTab {
    fn new(engine: BrowserEngine) -> Self {
        let title = match engine.profile() {
            BrowserProfile::Default => BROWSER_TITLE.to_owned(),
            BrowserProfile::Private => "Private Browser".to_owned(),
        };
        Self {
            engine,
            current_url: None,
            title,
            can_go_back: false,
            can_go_forward: false,
            is_loading: false,
            tab_mouse_state: Default::default(),
            close_button: Default::default(),
        }
    }
}

#[derive(Clone)]
struct BrowserHistoryEntry {
    url: String,
    title: String,
}

/// Tabs waiting to be re-created as native webviews. Engine creation fails
/// whenever the target native window isn't registered yet (during session
/// restore or a cross-window pane transfer), so instead of giving up we park
/// the tab URLs here and let the state poll retry until the window exists.
struct PendingTabRestore {
    tabs: Vec<(Option<String>, BrowserProfile)>,
    active_index: usize,
}

impl PendingTabRestore {
    fn single(url: Option<String>) -> Self {
        Self {
            tabs: vec![(url, BrowserProfile::Default)],
            active_index: 0,
        }
    }
}

pub struct BrowserView {
    window_id: WindowId,
    tabs: Vec<BrowserTab>,
    active_tab_index: usize,
    omnibar_editor: twarpui::ViewHandle<EditorView>,
    pane_configuration: twarpui::ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    history: Vec<BrowserHistoryEntry>,
    show_history: bool,
    pending_restore: Option<PendingTabRestore>,
    state_poll_active: bool,
    back_button: MouseStateHandle,
    forward_button: MouseStateHandle,
    reload_button: MouseStateHandle,
    new_tab_button: MouseStateHandle,
    new_private_tab_button: MouseStateHandle,
    history_button: MouseStateHandle,
    clear_profile_data_button: MouseStateHandle,
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
        let tabs: Vec<BrowserTab> = BrowserEngine::new(window_id)
            .map(BrowserTab::new)
            .into_iter()
            .collect();
        // Engine creation fails when the native window isn't registered yet
        // (session restore races window creation); park the URL and let the
        // state poll retry instead of leaving a permanently dead pane.
        let pending_restore = tabs
            .is_empty()
            .then(|| PendingTabRestore::single(initial_url.clone()));

        let mut view = Self {
            window_id,
            tabs,
            active_tab_index: 0,
            omnibar_editor,
            pane_configuration,
            focus_handle: None,
            history: Vec::new(),
            show_history: false,
            pending_restore,
            state_poll_active: false,
            back_button: Default::default(),
            forward_button: Default::default(),
            reload_button: Default::default(),
            new_tab_button: Default::default(),
            new_private_tab_button: Default::default(),
            history_button: Default::default(),
            clear_profile_data_button: Default::default(),
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

    pub fn pane_configuration(&self) -> twarpui::ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn snapshot_url(&self) -> Option<String> {
        self.active_tab()
            .and_then(|tab| tab.current_url.clone())
            .or_else(|| {
                // Still waiting on the native window — persist the URL the
                // pending restore will load, not nothing.
                let pending = self.pending_restore.as_ref()?;
                pending.tabs.get(pending.active_index)?.0.clone()
            })
    }

    pub fn focus_omnibar(&mut self, ctx: &mut ViewContext<Self>) {
        self.touch_focus();
        ctx.focus(&self.omnibar_editor);
    }

    pub fn focus_webview(&mut self) {
        self.touch_focus();
        if let Some(tab) = self.active_tab() {
            tab.engine.focus();
        }
    }

    pub fn destroy_webview(&mut self) {
        self.pending_restore = None;
        for tab in self.tabs.drain(..) {
            tab.engine.destroy();
        }
    }

    pub fn browser_engine(&self) -> Option<&BrowserEngine> {
        self.active_tab().map(|tab| &tab.engine)
    }

    pub(crate) fn automation_target(&self) -> Option<BrowserAutomationTarget> {
        let tab = self.active_tab()?;
        Some(BrowserAutomationTarget {
            engine: tab.engine,
            title: tab.title.clone(),
            url: tab.current_url.clone(),
            last_focus_seq: self.last_focus_seq,
        })
    }

    fn touch_focus(&mut self) {
        self.last_focus_seq = NEXT_BROWSER_FOCUS_SEQ.fetch_add(1, Ordering::Relaxed);
    }

    fn active_tab(&self) -> Option<&BrowserTab> {
        self.tabs.get(self.active_tab_index)
    }

    fn active_tab_mut(&mut self) -> Option<&mut BrowserTab> {
        self.tabs.get_mut(self.active_tab_index)
    }

    fn active_title(&self) -> String {
        self.active_tab()
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| BROWSER_TITLE.to_owned())
    }

    fn active_current_url(&self) -> Option<String> {
        self.active_tab().and_then(|tab| tab.current_url.clone())
    }

    fn active_can_go_back(&self) -> bool {
        self.active_tab().is_some_and(|tab| tab.can_go_back)
    }

    fn active_can_go_forward(&self) -> bool {
        self.active_tab().is_some_and(|tab| tab.can_go_forward)
    }

    fn active_is_loading(&self) -> bool {
        self.active_tab().is_some_and(|tab| tab.is_loading)
    }

    fn handle_omnibar_editor_event(
        &mut self,
        _handle: twarpui::ViewHandle<EditorView>,
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
                if let Some(url) = resolve_omnibar_input(&input) {
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
        if let Some(tab) = self.active_tab_mut() {
            tab.engine.load_url(&url);
            tab.current_url = Some(url.clone());
            tab.is_loading = true;
        } else if let Some(pending) = self.pending_restore.as_mut() {
            // No live webview yet — remember the URL so the pending restore
            // loads it once the native window is available.
            if let Some(entry) = pending.tabs.get_mut(pending.active_index) {
                entry.0 = Some(url.clone());
            }
        }

        self.set_omnibar_text(&url, ctx);
        self.update_pane_title(ctx);
        ctx.notify();
    }

    fn go_back(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(tab) = self.active_tab_mut() {
            tab.engine.go_back();
            tab.is_loading = true;
        }
        ctx.notify();
    }

    fn go_forward(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(tab) = self.active_tab_mut() {
            tab.engine.go_forward();
            tab.is_loading = true;
        }
        ctx.notify();
    }

    fn reload_or_stop(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(tab) = self.active_tab_mut() {
            if tab.is_loading {
                tab.engine.stop_loading();
            } else {
                tab.engine.reload();
            }
            tab.is_loading = !tab.is_loading;
        }
        ctx.notify();
    }

    fn new_tab(&mut self, profile: BrowserProfile, ctx: &mut ViewContext<Self>) {
        let Some(engine) = BrowserEngine::new_with_profile(self.window_id, profile) else {
            return;
        };

        if let Some(tab) = self.active_tab() {
            tab.engine.set_hidden(true);
        }
        self.tabs.push(BrowserTab::new(engine));
        self.active_tab_index = self.tabs.len() - 1;
        self.set_omnibar_text("", ctx);
        self.update_pane_title(ctx);
        self.focus_omnibar(ctx);
        ctx.notify();
    }

    fn select_tab(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index >= self.tabs.len() || index == self.active_tab_index {
            return;
        }
        if let Some(tab) = self.active_tab() {
            tab.engine.set_hidden(true);
        }
        self.active_tab_index = index;
        if let Some(tab) = self.active_tab() {
            tab.engine.set_hidden(false);
        }
        self.sync_omnibar_to_current_url(ctx);
        self.update_pane_title(ctx);
        self.focus_webview();
        ctx.notify();
    }

    fn close_tab(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        tab.engine.destroy();
        if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len() - 1;
        } else if index < self.active_tab_index {
            self.active_tab_index -= 1;
        }
        if let Some(tab) = self.active_tab() {
            tab.engine.set_hidden(false);
        }
        self.sync_omnibar_to_current_url(ctx);
        self.update_pane_title(ctx);
        ctx.notify();
    }

    fn navigate_to_history(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(entry) = self.history.get(index) {
            self.navigate_to_normalized_url(entry.url.clone(), ctx);
            self.show_history = false;
        }
    }

    fn clear_default_profile_data(&mut self, ctx: &mut ViewContext<Self>) {
        BrowserEngine::clear_default_profile_data(self.window_id);
        self.history.clear();
        self.show_history = false;
        ctx.notify();
    }

    fn schedule_state_poll(&mut self, ctx: &mut ViewContext<Self>) {
        self.state_poll_active = true;
        ctx.spawn(
            async move {
                Timer::after(STATE_POLL_INTERVAL).await;
            },
            |me, _, ctx| {
                if me.try_restore_pending_tabs(ctx) {
                    ctx.notify();
                }
                if me.tabs.is_empty() && me.pending_restore.is_none() {
                    me.state_poll_active = false;
                    return;
                }

                if me.sync_native_state(ctx) {
                    ctx.notify();
                }
                me.schedule_state_poll(ctx);
            },
        );
    }

    fn ensure_state_poll(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.state_poll_active {
            self.schedule_state_poll(ctx);
        }
    }

    /// Attempts to turn `pending_restore` back into live webview tabs.
    /// Returns true when the tabs were (re)created. Engine creation keeps
    /// failing until the native window registers, so this is called from the
    /// state poll as a retry loop.
    fn try_restore_pending_tabs(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(pending) = self.pending_restore.as_ref() else {
            return false;
        };
        let Some((_, first_profile)) = pending.tabs.first() else {
            self.pending_restore = None;
            return false;
        };
        // Probe with the first tab; if the window isn't ready this fails and
        // the next poll tick retries.
        let Some(first_engine) = BrowserEngine::new_with_profile(self.window_id, *first_profile)
        else {
            return false;
        };

        let pending = self.pending_restore.take().expect("checked above");
        let mut first_engine = Some(first_engine);
        let mut tabs = Vec::with_capacity(pending.tabs.len());
        for (url, profile) in &pending.tabs {
            let engine = first_engine
                .take()
                .or_else(|| BrowserEngine::new_with_profile(self.window_id, *profile));
            let Some(engine) = engine else { continue };
            let mut tab = BrowserTab::new(engine);
            if let Some(url) = url {
                tab.engine.load_url(url);
                tab.current_url = Some(url.clone());
                tab.is_loading = true;
            }
            tabs.push(tab);
        }
        self.tabs = tabs;
        self.active_tab_index = pending
            .active_index
            .min(self.tabs.len().saturating_sub(1));
        for (index, tab) in self.tabs.iter().enumerate() {
            tab.engine.set_hidden(index != self.active_tab_index);
        }
        self.sync_omnibar_to_current_url(ctx);
        self.update_pane_title(ctx);
        !self.tabs.is_empty()
    }

    fn sync_native_state(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let mut changed = false;
        let mut history_to_record = None;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            let mut state = Self::read_native_state_for_engine(&tab.engine);
            if state.url.is_none() && state.is_loading {
                state.url = tab.current_url.clone();
            }
            let title =
                Self::title_for_state(&state, tab.current_url.as_deref(), tab.engine.profile());
            let tab_changed = tab.current_url != state.url
                || tab.title != title
                || tab.can_go_back != state.can_go_back
                || tab.can_go_forward != state.can_go_forward
                || tab.is_loading != state.is_loading;

            tab.current_url = state.url.clone();
            tab.title = title;
            tab.can_go_back = state.can_go_back;
            tab.can_go_forward = state.can_go_forward;
            tab.is_loading = state.is_loading;

            if tab_changed {
                changed = true;
                if index == self.active_tab_index {
                    if let Some(url) = &tab.current_url {
                        history_to_record = Some((url.clone(), tab.title.clone()));
                    }
                }
            }
        }

        if let Some((url, title)) = history_to_record {
            self.record_history(url, title);
        }
        if changed {
            self.update_pane_title(ctx);
            if !self.omnibar_editor.as_ref(ctx).is_focused() {
                self.sync_omnibar_to_current_url(ctx);
            }
        }
        changed
    }

    fn read_native_state_for_engine(engine: &BrowserEngine) -> NativeBrowserState {
        NativeBrowserState {
            url: engine.current_url(),
            title: engine.title(),
            can_go_back: engine.can_go_back(),
            can_go_forward: engine.can_go_forward(),
            is_loading: engine.is_loading(),
        }
    }

    fn update_pane_title(&mut self, ctx: &mut ViewContext<Self>) {
        let title = self.active_title();
        self.pane_configuration
            .update(ctx, |config, ctx| config.set_title(title, ctx));
    }

    fn record_history(&mut self, url: String, title: String) {
        if self.history.last().is_some_and(|entry| entry.url == url) {
            return;
        }
        self.history.push(BrowserHistoryEntry { url, title });
        if self.history.len() > 100 {
            let excess = self.history.len() - 100;
            self.history.drain(0..excess);
        }
    }

    fn title_for_state(
        state: &NativeBrowserState,
        fallback_url: Option<&str>,
        profile: BrowserProfile,
    ) -> String {
        state
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                state
                    .url
                    .as_deref()
                    .or(fallback_url)
                    .and_then(host_from_url)
            })
            .unwrap_or_else(|| match profile {
                BrowserProfile::Default => BROWSER_TITLE.to_owned(),
                BrowserProfile::Private => "Private Browser".to_owned(),
            })
    }

    fn sync_omnibar_to_current_url(&self, ctx: &mut ViewContext<Self>) {
        let current_url = self.active_current_url();
        let text = current_url.as_deref().unwrap_or_default();
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

    // The tab strip doubles as the pane header (see `render_header_content`),
    // so it also carries the pane close/overflow buttons on the right edge.
    fn render_tab_strip_header(
        &self,
        header_ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);

        if header_ctx.header_left_inset > 0. {
            row.add_child(
                Container::new(Empty::new().finish())
                    .with_padding_left(header_ctx.header_left_inset)
                    .finish(),
            );
        }

        for (index, tab) in self.tabs.iter().enumerate() {
            let active = index == self.active_tab_index;
            let background = if active {
                theme.surface_2()
            } else {
                theme.background()
            };
            let title = if tab.engine.profile() == BrowserProfile::Private {
                format!("Private · {}", tab.title)
            } else {
                tab.title.clone()
            };
            let title = Text::new_inline(title, appearance.ui_font_family(), 12.)
                .with_color(theme.active_ui_text_color().into())
                .with_clip(ClipConfig::end())
                .finish();
            let close_button = self.render_toolbar_button(
                appearance,
                icons::Icon::X,
                self.tabs.len() > 1,
                tab.close_button.clone(),
                BrowserViewAction::CloseTab(index),
            );
            // SpaceBetween pins the title to the left edge and the close
            // button to the right edge of the fixed-width tab.
            let tab_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(TAB_TITLE_CLOSE_SPACING)
                .with_child(Shrinkable::new(1., title).finish())
                .with_child(close_button)
                .finish();
            let tab_element = Hoverable::new(tab.tab_mouse_state.clone(), move |_| {
                ConstrainedBox::new(
                    Container::new(tab_row)
                        .with_horizontal_padding(8.)
                        .with_background(background)
                        .with_border(Border::all(1.).with_border_fill(theme.outline()))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                )
                .with_width(180.)
                .with_height(24.)
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BrowserViewAction::SelectTab(index));
            })
            .finish();
            row.add_child(tab_element);
        }

        row.add_child(self.render_toolbar_button(
            appearance,
            icons::Icon::Plus,
            true,
            self.new_tab_button.clone(),
            BrowserViewAction::NewTab,
        ));
        row.add_child(self.render_toolbar_button(
            appearance,
            icons::Icon::Lock,
            true,
            self.new_private_tab_button.clone(),
            BrowserViewAction::NewPrivateTab,
        ));

        // Push the pane-level close/overflow buttons to the right edge.
        row.add_child(Shrinkable::new(1., Empty::new().finish()).finish());
        row.add_child(render_pane_header_buttons::<
            BrowserViewAction,
            BrowserViewCustomAction,
        >(header_ctx, appearance, true, None, None));

        ConstrainedBox::new(
            Container::new(row.finish())
                .with_horizontal_padding(TOOLBAR_PADDING)
                .with_background(theme.background())
                .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
                .finish(),
        )
        .with_height(PANE_HEADER_HEIGHT)
        .finish()
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
                .set_height(24.)
                // Center the single UI-font line inside the 24pt capsule and
                // keep the text off the rounded left edge.
                .set_padding(Coords::default().top(4.).left(6.).right(6.)),
        )
        .build()
        .finish();

        let reload_icon = if self.active_is_loading() {
            icons::Icon::Stop
        } else {
            icons::Icon::RefreshCw04
        };
        let current_url = self.active_current_url();

        ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(self.render_toolbar_button(
                        appearance,
                        icons::Icon::ArrowLeft,
                        self.active_can_go_back(),
                        self.back_button.clone(),
                        BrowserViewAction::Back,
                    ))
                    .with_child(self.render_toolbar_button(
                        appearance,
                        icons::Icon::ArrowRight,
                        self.active_can_go_forward(),
                        self.forward_button.clone(),
                        BrowserViewAction::Forward,
                    ))
                    .with_child(self.render_toolbar_button(
                        appearance,
                        reload_icon,
                        current_url.is_some() || self.active_is_loading(),
                        self.reload_button.clone(),
                        BrowserViewAction::ReloadOrStop,
                    ))
                    .with_child(self.render_toolbar_button(
                        appearance,
                        icons::Icon::History,
                        !self.history.is_empty(),
                        self.history_button.clone(),
                        BrowserViewAction::ToggleHistory,
                    ))
                    .with_child(self.render_toolbar_button(
                        appearance,
                        icons::Icon::Trash,
                        true,
                        self.clear_profile_data_button.clone(),
                        BrowserViewAction::ClearDefaultProfileData,
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

    fn render_history_panel(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self.show_history {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Max);
        for (index, entry) in self
            .history
            .iter()
            .enumerate()
            .rev()
            .take(HISTORY_PANEL_MAX_ROWS)
        {
            let label = if entry.title.trim().is_empty() {
                entry.url.clone()
            } else {
                format!("{} · {}", entry.title, entry.url)
            };
            let text = Text::new_inline(label, appearance.ui_font_family(), 12.)
                .with_color(theme.active_ui_text_color().into())
                .with_clip(ClipConfig::end())
                .finish();
            let row = Hoverable::new(Default::default(), move |_| {
                ConstrainedBox::new(
                    Container::new(text)
                        .with_horizontal_padding(TOOLBAR_PADDING)
                        .with_background(theme.surface_1())
                        .finish(),
                )
                .with_height(HISTORY_PANEL_ROW_HEIGHT)
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BrowserViewAction::NavigateHistory(index));
            })
            .finish();
            column.add_child(row);
        }

        Some(
            Container::new(column.finish())
                .with_background(theme.surface_1())
                .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
                .finish(),
        )
    }

    fn render_loading_indicator(&self, app: &AppContext) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        let fill = if self.active_is_loading() {
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
        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_toolbar(app))
            .with_child(self.render_loading_indicator(app));
        if let Some(history_panel) = self.render_history_panel(app) {
            column.add_child(history_panel);
        }
        let content: Box<dyn Element> = if self.browser_engine().is_some() {
            NativeBrowserElement::new(self.browser_engine()).finish()
        } else {
            // No live webview (waiting on the native window during restore or
            // a cross-window move) — show state instead of a dead blank area.
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            let label = Text::new_inline(
                "Reconnecting to browser…",
                appearance.ui_font_family(),
                12.,
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();
            Container::new(Align::new(label).finish())
                .with_background(theme.background())
                .finish()
        };
        column
            .with_child(Shrinkable::new(1., content).finish())
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
        // Webviews are native children of the source window, so they must be
        // re-created in the target window. Preserve every tab's URL/profile
        // and let the retry loop rebuild them — the target window may not be
        // registered yet at the moment of transfer.
        let pending = if self.tabs.is_empty() {
            self.pending_restore
                .take()
                .unwrap_or_else(|| PendingTabRestore::single(None))
        } else {
            PendingTabRestore {
                tabs: self
                    .tabs
                    .iter()
                    .map(|tab| (tab.current_url.clone(), tab.engine.profile()))
                    .collect(),
                active_index: self.active_tab_index,
            }
        };
        self.destroy_webview();
        self.window_id = target_window_id;
        self.pending_restore = Some(pending);
        if self.try_restore_pending_tabs(ctx) {
            ctx.notify();
        }
        self.ensure_state_poll(ctx);
    }
}

impl TypedActionView for BrowserView {
    type Action = BrowserViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BrowserViewAction::Back => self.go_back(ctx),
            BrowserViewAction::Forward => self.go_forward(ctx),
            BrowserViewAction::ReloadOrStop => self.reload_or_stop(ctx),
            BrowserViewAction::NewTab => self.new_tab(BrowserProfile::Default, ctx),
            BrowserViewAction::NewPrivateTab => self.new_tab(BrowserProfile::Private, ctx),
            BrowserViewAction::SelectTab(index) => self.select_tab(*index, ctx),
            BrowserViewAction::CloseTab(index) => self.close_tab(*index, ctx),
            BrowserViewAction::ToggleHistory => {
                self.show_history = !self.show_history;
                ctx.notify();
            }
            BrowserViewAction::NavigateHistory(index) => self.navigate_to_history(*index, ctx),
            BrowserViewAction::ClearDefaultProfileData => self.clear_default_profile_data(ctx),
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
        ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> HeaderContent {
        // The browser tab strip IS the pane header — a standard title row on
        // top of it would just duplicate the active tab's title.
        HeaderContent::Custom {
            element: self.render_tab_strip_header(ctx, app),
            has_custom_draggable_behavior: false,
        }
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

struct NativeBrowserElement {
    webview: Option<(WindowId, BrowserWebViewId)>,
    size: Option<Vector2F>,
    origin: Option<twarpui::elements::Point>,
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
        self.origin = Some(twarpui::elements::Point::from_vec2f(
            origin,
            ctx.scene.z_index(),
        ));

        #[cfg(target_os = "macos")]
        if let (Some((window_id, webview_id)), Some(size)) = (self.webview, self.size) {
            if size.x() > 0.0 && size.y() > 0.0 {
                // Paint coordinates are in zoom-scaled scene units (the scene
                // is laid out at window_size / zoom_factor); NSView frames are
                // in logical points, so scale back up by the zoom factor.
                let zoom = _app.zoom_factor();
                MacWindow::set_browser_webview_frame(
                    window_id,
                    webview_id,
                    RectF::new(origin.scale_up(zoom), size.scale_up(zoom)),
                );
                MacWindow::set_browser_webview_hidden(window_id, webview_id, false);
            }
        }
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn dispatch_event(
        &mut self,
        _event: &twarpui::event::DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<twarpui::elements::Point> {
        self.origin
    }
}

/// Resolves what the user typed in the omnibar: a URL when it parses as one,
/// otherwise a Google search for the raw input.
fn resolve_omnibar_input(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    normalize_browser_url(input).or_else(|| {
        Url::parse_with_params("https://www.google.com/search", &[("q", input)])
            .ok()
            .map(|url| url.to_string())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omnibar_urls_pass_through() {
        assert_eq!(
            resolve_omnibar_input("example.com").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            resolve_omnibar_input("http://localhost:3000").as_deref(),
            Some("http://localhost:3000/")
        );
    }

    #[test]
    fn omnibar_non_urls_become_google_searches() {
        assert_eq!(
            resolve_omnibar_input("weather").as_deref(),
            Some("https://www.google.com/search?q=weather")
        );
        assert_eq!(
            resolve_omnibar_input("rust borrow checker").as_deref(),
            Some("https://www.google.com/search?q=rust+borrow+checker")
        );
    }

    #[test]
    fn omnibar_empty_input_is_ignored() {
        assert_eq!(resolve_omnibar_input(""), None);
    }
}
