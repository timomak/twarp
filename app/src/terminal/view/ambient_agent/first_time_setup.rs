//! twarp 2c-d.3: cloud environments are no longer materialized client-side, so
//! the first-time cloud-agent setup view is reduced to a stub that simply
//! emits `Cancelled` whenever it is asked to do anything.

use warpui::{
    elements::{Element, Empty},
    AppContext, Entity, TypedActionView, View, ViewContext,
};

/// Events emitted by FirstTimeCloudAgentSetupView.
#[derive(Debug, Clone)]
pub enum FirstTimeCloudAgentSetupViewEvent {
    /// The user cancelled the setup (should pop from pane stack).
    Cancelled,
    /// The user created an environment and we should navigate to cloud agent mode.
    #[allow(dead_code)]
    EnvironmentCreated,
}

/// Stubbed first-time cloud-agent setup view.
pub struct FirstTimeCloudAgentSetupView;

impl FirstTimeCloudAgentSetupView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self
    }

    pub fn reset_form(&mut self, _ctx: &mut ViewContext<Self>) {}
}

impl Entity for FirstTimeCloudAgentSetupView {
    type Event = FirstTimeCloudAgentSetupViewEvent;
}

#[derive(Clone, Debug)]
pub enum FirstTimeCloudAgentSetupViewAction {}

impl TypedActionView for FirstTimeCloudAgentSetupView {
    type Action = FirstTimeCloudAgentSetupViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl View for FirstTimeCloudAgentSetupView {
    fn ui_name() -> &'static str {
        "FirstTimeCloudAgentSetupView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}
