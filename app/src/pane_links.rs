//! twarp 14n: cross-pane navigation between a Claude session and the Browser
//! pane bound to it (14j binding). Both directions resolve across every
//! window and tab, so the link keeps working after the user moves either
//! pane.

use twarpui::{AppContext, SingletonEntity, View, ViewContext, WindowId};

use crate::workspace::{PaneViewLocator, WorkspaceRegistry};

/// Locates the Browser pane bound to the given Claude session, anywhere.
pub(crate) fn locate_browser_pane_for_session(
    session_id: &str,
    ctx: &AppContext,
) -> Option<(WindowId, PaneViewLocator)> {
    WorkspaceRegistry::as_ref(ctx)
        .all_workspaces(ctx)
        .into_iter()
        .find_map(|(window_id, workspace)| {
            workspace
                .as_ref(ctx)
                .tab_views()
                .find_map(|pane_group| {
                    let pane_id = pane_group
                        .as_ref(ctx)
                        .find_browser_pane_bound_to_session(session_id, ctx)?;
                    Some(PaneViewLocator {
                        pane_group_id: pane_group.id(),
                        pane_id,
                    })
                })
                .map(|locator| (window_id, locator))
        })
}

/// Locates the Claude pane hosting the given session, anywhere.
pub(crate) fn locate_claude_pane_for_session(
    session_id: &str,
    ctx: &AppContext,
) -> Option<(WindowId, PaneViewLocator)> {
    WorkspaceRegistry::as_ref(ctx)
        .all_workspaces(ctx)
        .into_iter()
        .find_map(|(window_id, workspace)| {
            workspace
                .as_ref(ctx)
                .tab_views()
                .find_map(|pane_group| {
                    let pane_id = pane_group
                        .as_ref(ctx)
                        .find_claude_code_pane_by_session_id(session_id, ctx)?;
                    Some(PaneViewLocator {
                        pane_group_id: pane_group.id(),
                        pane_id,
                    })
                })
                .map(|locator| (window_id, locator))
        })
}

/// Focuses a located pane: raises its window, activates its tab, focuses the
/// pane (the root view's pane-navigation handler does tab + pane focus).
pub(crate) fn focus_located_pane<V: View>(
    window_id: WindowId,
    locator: PaneViewLocator,
    ctx: &mut ViewContext<V>,
) {
    ctx.windows().show_window_and_focus_app(window_id);
    if let Some(root_view_id) = ctx.root_view_id(window_id) {
        ctx.dispatch_action_for_view(
            window_id,
            root_view_id,
            "root_view:handle_pane_navigation_event",
            &locator,
        );
    }
}
