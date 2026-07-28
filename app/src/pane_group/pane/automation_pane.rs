//! twarp 20a: pane wrapper for the automation surfaces (Scheduled Tasks /
//! Skills / MCPs). Mirrors [`super::network_log_pane::NetworkLogPane`], with
//! per-window/per-page tracking via
//! [`crate::automation::pane_manager::AutomationPaneManager`].

use twarpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

use crate::app_state::LeafContents;
use crate::automation::pane_manager::AutomationPaneManager;
use crate::automation::view::AutomationView;
use crate::automation::AutomationPage;
use crate::workspace::PaneViewLocator;

use super::{
    view::PaneView, DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};

pub struct AutomationPane {
    view: ViewHandle<PaneView<AutomationView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl AutomationPane {
    pub fn new<V: View>(page: AutomationPage, ctx: &mut ViewContext<V>) -> Self {
        let automation_view = ctx.add_typed_action_view(|ctx| AutomationView::new(page, ctx));
        let pane_configuration = automation_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_automation_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                automation_view,
                (),
                pane_configuration.clone(),
                ctx,
            )
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn automation_view(&self, ctx: &AppContext) -> ViewHandle<AutomationView> {
        self.view.as_ref(ctx).child(ctx)
    }

    pub fn page(&self, ctx: &AppContext) -> AutomationPage {
        self.automation_view(ctx).as_ref(ctx).page()
    }
}

impl PaneContent for AutomationPane {
    fn id(&self) -> PaneId {
        PaneId::from_automation_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let automation_view = self.automation_view(ctx);
        let pane_id = self.id();
        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        let page = self.page(ctx);

        ctx.subscribe_to_view(&automation_view, move |pane_group, _, event, ctx| {
            pane_group.handle_pane_event(pane_id, event, ctx)
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        AutomationPaneManager::handle(ctx).update(ctx, |manager, _ctx| {
            manager.register_pane(
                window_id,
                page,
                PaneViewLocator {
                    pane_group_id,
                    pane_id,
                },
            );
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let automation_view = self.automation_view(ctx);
        ctx.unsubscribe_to_view(&automation_view);
        ctx.unsubscribe_to_view(&self.view);

        let window_id = ctx.window_id();
        let page = self.page(ctx);
        AutomationPaneManager::handle(ctx).update(ctx, |manager, _| {
            manager.deregister_pane(window_id, page);
        });
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        LeafContents::Automation(self.page(app))
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        ctx.focus(&self.view);
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
