// twarp: 2c-d — MCP UI removed. The settings page is reduced to an empty
// placeholder so the rest of the settings shell still compiles. The MCP
// gallery / installation / edit subviews lived under settings_view/mcp_servers/
// and used `crate::ai::mcp::*` types, all of which were deleted with the AI
// modules. We keep `MCPServersSettingsPageView`, `MCPServersSettingsPage`,
// `MCPServersSettingsPageEvent`, and `InstallOrigin` so external imports keep
// resolving; their behavior is now a no-op until MCP is reintroduced.
use uuid::Uuid;
use warpui::{elements::Empty, AppContext, Element, Entity, TypedActionView, View, ViewContext};

use crate::{
    appearance::Appearance,
    settings_view::{
        settings_page::{MatchData, PageType, SettingsPageMeta, SettingsWidget},
        SettingsSection,
    },
};

/// Describes where an MCP install request originated.
///
/// Used to decide whether an install request is allowed to bypass the
/// installation modal. In-app gestures (gallery card click, reinstall button)
/// are implicitly confirmed by the click itself. Deeplink-triggered installs
/// are untrusted and must always route through the installation modal so the
/// user can explicitly confirm before any installation or server spawn occurs.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum InstallOrigin {
    /// Triggered by a user gesture inside Warp (gallery card click,
    /// reinstall button, programmatic in-app flows, etc.).
    InApp,
    /// Triggered by a `warp://settings/mcp?autoinstall=...` deeplink; must be
    /// gated by an explicit in-app confirmation before install or spawn.
    Deeplink,
}

const PAGE_TITLE_TEXT: &str = "MCP Servers";

/// twarp: 2c-d — formerly carried `Edit { item_id: Option<ServerCardItemId> }`,
/// reduced to `List` only after the edit page was deleted.
#[derive(Debug, Default, Copy, Clone)]
pub enum MCPServersSettingsPage {
    #[default]
    List,
    Edit {
        item_id: Option<Uuid>,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MCPServersSettingsPageEvent {
    ShowModal,
    HideModal,
}

pub struct MCPServersSettingsPageView {
    page: PageType<Self>,
    current_page: MCPServersSettingsPage,
}

impl MCPServersSettingsPageView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            page: PageType::new_monolith(
                MCPServersSettingsWidget::default(),
                Some(PAGE_TITLE_TEXT),
                true,
            ),
            current_page: MCPServersSettingsPage::default(),
        }
    }

    pub fn update_page(&mut self, page: MCPServersSettingsPage, ctx: &mut ViewContext<Self>) {
        self.current_page = page;
        ctx.notify();
    }

    pub fn focus(&mut self, _ctx: &mut ViewContext<Self>) {
        // twarp: 2c-d — no subviews to focus
    }

    pub fn autoinstall_from_gallery(
        &mut self,
        _autoinstall_param: &str,
        _ctx: &mut ViewContext<Self>,
    ) {
        // twarp: 2c-d — MCP autoinstall removed
    }

    // twarp: 2c-d — modal content removed; stub returns None.
    pub fn get_modal_content(&self, _app: &AppContext) -> Option<Box<dyn Element>> {
        None
    }
}

impl Entity for MCPServersSettingsPageView {
    type Event = MCPServersSettingsPageEvent;
}

impl View for MCPServersSettingsPageView {
    fn ui_name() -> &'static str {
        "MCPServersSettingsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for MCPServersSettingsPageView {
    type Action = ();
}

impl SettingsPageMeta for MCPServersSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::MCPServers
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        // twarp: 2c-d — hide MCP page until reintroduced
        false
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget()
    }
}

#[derive(Default)]
pub struct MCPServersSettingsWidget;

impl SettingsWidget for MCPServersSettingsWidget {
    type View = MCPServersSettingsPageView;

    fn search_terms(&self) -> &str {
        "mcp servers"
    }

    fn render(
        &self,
        _view: &Self::View,
        _appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }
}
