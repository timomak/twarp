use claude_code::driver::PermissionMode;
use settings::{
    macros::define_settings_group, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};

use crate::app_state::CLIAgent;

pub const DEFAULT_BACKEND: &str = "claude";
pub const DEFAULT_CHAT_PROVIDER: &str = "claude";
pub const DEFAULT_CHAT_MODEL: &str = "";
pub const DEFAULT_CHAT_EFFORT: &str = "";
pub const DEFAULT_CHAT_PERMISSION_MODE: &str = "default";

define_settings_group!(AgentSettings, settings: [
    backend: AgentBackend {
        type: String,
        default: DEFAULT_BACKEND.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.backend",
        description: "The selected CLI agent backend.",
    },
    chat_provider: AgentChatProvider {
        type: String,
        default: DEFAULT_CHAT_PROVIDER.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.chat.provider",
        description: "The provider used for chat and history.",
    },
    chat_model: AgentChatModel {
        type: String,
        default: DEFAULT_CHAT_MODEL.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.chat.model",
        description: "The model used for chat and history. Empty means the provider default.",
    },
    chat_effort: AgentChatEffort {
        type: String,
        default: DEFAULT_CHAT_EFFORT.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.chat.effort",
        description: "The reasoning effort used for chat and history. Empty means the provider default.",
    },
    chat_permission_mode: AgentChatPermissionMode {
        type: String,
        default: DEFAULT_CHAT_PERMISSION_MODE.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.chat.permission_mode",
        description: "The permission mode used for new chat panes.",
    },
]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChatLaunchConfig {
    pub provider: CLIAgent,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: PermissionMode,
}

impl AgentSettings {
    pub fn backend_agent(&self) -> CLIAgent {
        enabled_agent_or_default(self.backend.value())
    }

    pub fn chat_provider_agent(&self) -> CLIAgent {
        enabled_agent_or_default(self.chat_provider.value())
    }

    pub fn chat_launch_config(&self) -> AgentChatLaunchConfig {
        AgentChatLaunchConfig {
            provider: self.chat_provider_agent(),
            model: valid_chat_model(self.chat_model.value()),
            effort: valid_chat_effort(self.chat_effort.value()),
            permission_mode: PermissionMode::from_cli_arg(self.chat_permission_mode.value())
                .unwrap_or(PermissionMode::Default),
        }
    }
}

pub fn enabled_agent_or_default(serialized_name: &str) -> CLIAgent {
    let agent = CLIAgent::from_serialized_name(serialized_name);
    if agent.is_agent_settings_enabled() {
        agent
    } else {
        CLIAgent::Claude
    }
}

pub fn valid_chat_model(model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    if crate::claude_code_models::FALLBACK_MODEL_ALIASES.contains(&model) {
        return Some(model.to_owned());
    }

    crate::claude_code_models::discovered()
        .is_some_and(|models| models.iter().any(|entry| entry.id == model))
        .then(|| model.to_owned())
}

pub fn valid_chat_effort(effort: &str) -> Option<String> {
    match effort.trim() {
        "" | "default" => None,
        "low" | "medium" | "high" | "max" => Some(effort.trim().to_owned()),
        _ => None,
    }
}
