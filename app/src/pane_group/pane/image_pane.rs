//! The main-content pane hosting an [`ImageView`] (in-app image viewer).

use std::path::PathBuf;

use twarpui::{AppContext, ModelHandle, View, ViewContext, ViewHandle};

use crate::app_state::{LeafContents, NotebookPaneSnapshot};
use crate::image_view::{ImageView, ImageViewEvent};

use super::{
    view::PaneView, DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};

pub struct ImagePane {
    view: ViewHandle<PaneView<ImageView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl ImagePane {
    fn from_view(image_view: ViewHandle<ImageView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = image_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(image_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_image_pane_ctx(ctx);
            PaneView::new(pane_id, image_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn new<V: View>(path: PathBuf, ctx: &mut ViewContext<V>) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| ImageView::new(path, ctx));
        Self::from_view(view, ctx)
    }

    pub fn image_view(&self, ctx: &AppContext) -> ViewHandle<ImageView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for ImagePane {
    fn id(&self) -> PaneId {
        PaneId::from_image_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let image_view = self.image_view(ctx);
        let pane_id = self.id();

        ctx.subscribe_to_view(&image_view, move |pane_group, _, event, ctx| {
            let ImageViewEvent::Pane(pane_event) = event;
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
        // Always unsubscribe from views.
        let image_view = self.image_view(ctx);
        ctx.unsubscribe_to_view(&image_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        // Reuse the local-file-notebook snapshot: it already persists a path,
        // and restoration re-resolves the viewer by file type (an image path
        // restores into an `ImagePane`; see `PaneGroup::restore`).
        let path = Some(self.image_view(app).as_ref(app).path().clone());
        LeafContents::Notebook(NotebookPaneSnapshot::LocalFileNotebook { path })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.image_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
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
