// twarp: AI is permanently disabled, so the onboarding agent slide UI
// (`agent_slide.rs`) was deleted in 2c-a. These data types remain because
// non-slide consumers (the onboarding state model, the post-onboarding
// settings application, the AI-LLM bridge in `app/src/ai/onboarding.rs`)
// still reference them. They are scheduled for removal in 2c-d (when
// `app/src/ai/` is deleted) and 2c-e (when `crates/ai/` is deleted),
// after which the `OnboardingStateModel::agent_settings` field, the
// `SelectedSettings::AgentDrivenDevelopment` variant, and these types
// themselves can all go.

use ai::LLMId;
use warp_core::ui::icons::Icon;

/// Information about a model that was previously displayed on the onboarding
/// agent slide.
#[derive(Clone, Debug)]
pub struct OnboardingModelInfo {
    pub id: LLMId,
    pub title: String,
    pub icon: Icon,
    pub requires_upgrade: bool,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AgentAutonomy {
    Full,
    #[default]
    Partial,
    None,
}

impl std::fmt::Display for AgentAutonomy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentAutonomy::Full => write!(f, "full"),
            AgentAutonomy::Partial => write!(f, "partial"),
            AgentAutonomy::None => write!(f, "none"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDevelopmentSettings {
    /// The selected model's ID.
    pub selected_model_id: LLMId,
    pub autonomy: Option<AgentAutonomy>,
    /// Whether the CLI agent toolbar is enabled (maps to `should_render_cli_agent_footer`).
    pub cli_agent_toolbar_enabled: bool,
    /// The default session mode chosen during onboarding.
    pub session_default: crate::SessionDefault,
    /// Legacy AI-disable flag — twarp ignores this since AI is permanently
    /// off, but the field stays compiled until 2c-d/e remove the surrounding
    /// scaffolding.
    pub disable_oz: bool,
    /// Whether agent notifications (mailbox button, toasts, notification items) are shown.
    pub show_agent_notifications: bool,
}

impl AgentDevelopmentSettings {
    pub fn new(default_model_id: LLMId) -> Self {
        Self {
            selected_model_id: default_model_id,
            autonomy: Some(AgentAutonomy::default()),
            cli_agent_toolbar_enabled: true,
            session_default: crate::SessionDefault::Agent,
            disable_oz: false,
            show_agent_notifications: true,
        }
    }
}
