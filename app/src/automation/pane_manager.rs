//! Tracks open automation panes (Scheduled Tasks / Skills / MCPs) so each
//! window shows at most one pane per page and reopening focuses the existing
//! one. Mirrors [`crate::server::network_log_pane_manager::NetworkLogPaneManager`],
//! keyed additionally by [`AutomationPage`].

use std::collections::HashMap;

use twarpui::{Entity, SingletonEntity, WindowId};

use crate::workspace::PaneViewLocator;

use super::AutomationPage;

/// Singleton mapping `(WindowId, AutomationPage) -> PaneViewLocator` for any
/// open automation panes.
#[derive(Default)]
pub struct AutomationPaneManager {
    panes: HashMap<(WindowId, AutomationPage), PaneViewLocator>,
}

impl AutomationPaneManager {
    pub fn find_pane(&self, window_id: WindowId, page: AutomationPage) -> Option<PaneViewLocator> {
        self.panes.get(&(window_id, page)).copied()
    }

    pub fn register_pane(
        &mut self,
        window_id: WindowId,
        page: AutomationPage,
        locator: PaneViewLocator,
    ) {
        self.panes.insert((window_id, page), locator);
    }

    pub fn deregister_pane(&mut self, window_id: WindowId, page: AutomationPage) {
        self.panes.remove(&(window_id, page));
    }
}

impl Entity for AutomationPaneManager {
    type Event = ();
}

impl SingletonEntity for AutomationPaneManager {}
