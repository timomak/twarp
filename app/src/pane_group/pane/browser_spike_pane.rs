use warpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use crate::app_state::LeafContents;
use crate::browser_spike_view::{BrowserSpikeView, BrowserSpikeViewEvent};

use super::{
    view::PaneView, DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};

pub struct BrowserSpikePane {
    view: ViewHandle<PaneView<BrowserSpikeView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl BrowserSpikePane {
    pub fn from_view(browser_view: ViewHandle<BrowserSpikeView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = browser_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(browser_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_browser_spike_pane_ctx(ctx);
            PaneView::new(pane_id, browser_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn new<V: View>(ctx: &mut ViewContext<V>) -> Self {
        let view = ctx.add_typed_action_view(BrowserSpikeView::new);
        Self::from_view(view, ctx)
    }

    fn browser_view(&self, ctx: &AppContext) -> ViewHandle<BrowserSpikeView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for BrowserSpikePane {
    fn id(&self) -> PaneId {
        PaneId::from_browser_spike_pane_view(&self.view)
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
            let BrowserSpikeViewEvent::Pane(pane_event) = event;
            pane_group.handle_pane_event(pane_id, pane_event, ctx)
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let browser_view = self.browser_view(ctx);
        browser_view.update(ctx, |view, _ctx| view.destroy_webview());
        ctx.unsubscribe_to_view(&browser_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        LeafContents::BrowserSpike
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.browser_view(ctx)
            .update(ctx, |view, _ctx| view.focus());
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
