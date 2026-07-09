use twarpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use crate::app_state::{BrowserPaneSnapshot, LeafContents};
use crate::browser_view::{BrowserView, BrowserViewEvent};

use super::{
    view::PaneView, DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};

pub struct BrowserPane {
    view: ViewHandle<PaneView<BrowserView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl BrowserPane {
    pub fn from_view(browser_view: ViewHandle<BrowserView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = browser_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(browser_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_browser_pane_ctx(ctx);
            PaneView::new(pane_id, browser_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn new<V: View>(url: Option<String>, ctx: &mut ViewContext<V>) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| BrowserView::new(url, ctx));
        Self::from_view(view, ctx)
    }

    /// Opens without touching keyboard focus — the automation (browser MCP)
    /// path, so the user's typing focus is never hijacked.
    pub fn new_unfocused<V: View>(url: Option<String>, ctx: &mut ViewContext<V>) -> Self {
        let view =
            ctx.add_typed_action_view(move |ctx| BrowserView::new_with_focus(url, false, ctx));
        Self::from_view(view, ctx)
    }

    pub fn new_restore<V: View>(
        url: Option<String>,
        bound_claude_session: Option<String>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| {
            let mut view = BrowserView::new_restore(url, ctx);
            if let Some(session_id) = bound_claude_session {
                view.set_bound_claude_session(session_id);
            }
            view
        });
        Self::from_view(view, ctx)
    }

    pub(crate) fn browser_view(&self, ctx: &AppContext) -> ViewHandle<BrowserView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for BrowserPane {
    fn id(&self) -> PaneId {
        PaneId::from_browser_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let browser_view = self.browser_view(ctx);
        let pane_id = self.id();
        ctx.subscribe_to_view(&browser_view, move |pane_group, _, event, ctx| {
            let BrowserViewEvent::Pane(pane_event) = event;
            pane_group.handle_pane_event(pane_id, pane_event, ctx)
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let browser_view = self.browser_view(ctx);
        // A moved pane re-attaches with the same view (same-window moves keep
        // the webviews; cross-window moves rebuild them in
        // on_window_transferred) — only tear down the native webviews when
        // the pane is actually going away.
        if !matches!(detach_type, DetachType::Moved) {
            browser_view.update(ctx, |view, _ctx| view.destroy_webview());
        }
        ctx.unsubscribe_to_view(&browser_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let view = self.browser_view(app);
        let view = view.as_ref(app);
        LeafContents::Browser(BrowserPaneSnapshot {
            url: view.snapshot_url(),
            bound_claude_session: view.bound_claude_session().map(ToOwned::to_owned),
        })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.browser_view(ctx)
            .update(ctx, |view, ctx| view.focus_omnibar(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
