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
pub const DEFAULT_SUGGESTION_PROVIDER: &str = "";
pub const DEFAULT_SUGGESTION_MODEL: &str = "";
pub const DEFAULT_SUGGESTION_EFFORT: &str = "";

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
    terminal_suggest_provider: AgentTerminalSuggestProvider {
        type: String,
        default: DEFAULT_SUGGESTION_PROVIDER.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.terminal_suggest.provider",
        description: "The provider used for terminal suggestions. Empty means inherit the chat provider.",
    },
    terminal_suggest_model: AgentTerminalSuggestModel {
        type: String,
        default: DEFAULT_SUGGESTION_MODEL.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.terminal_suggest.model",
        description: "The model used for terminal suggestions. Empty means the provider default.",
    },
    terminal_suggest_effort: AgentTerminalSuggestEffort {
        type: String,
        default: DEFAULT_SUGGESTION_EFFORT.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.terminal_suggest.effort",
        description: "The reasoning effort used for terminal suggestions. Empty means the provider default.",
    },
    reply_suggest_provider: AgentReplySuggestProvider {
        type: String,
        default: DEFAULT_SUGGESTION_PROVIDER.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.reply_suggest.provider",
        description: "The provider used for chat reply suggestions. Empty means inherit the chat provider.",
    },
    reply_suggest_model: AgentReplySuggestModel {
        type: String,
        default: DEFAULT_SUGGESTION_MODEL.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.reply_suggest.model",
        description: "The model used for chat reply suggestions. Empty means the provider default.",
    },
    reply_suggest_effort: AgentReplySuggestEffort {
        type: String,
        default: DEFAULT_SUGGESTION_EFFORT.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.actions.reply_suggest.effort",
        description: "The reasoning effort used for chat reply suggestions. Empty means the provider default.",
    },
    enable_reply_suggestions: AgentEnableReplySuggestions {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.suggestions.reply.enabled",
        description: "Whether to suggest a reply after each agent response.",
    },
    enable_terminal_suggestions: AgentEnableTerminalSuggestions {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.suggestions.terminal.enabled",
        description: "Whether to show AI command suggestions in the terminal.",
    },
    claude_api_key_set: AgentClaudeApiKeySet {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.auth.claude.api_key_set",
        description: "Whether a Claude API key is stored in the OS keychain.",
    },
]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChatLaunchConfig {
    pub provider: CLIAgent,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: PermissionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentActionProvider {
    Inherit,
    Agent(CLIAgent),
}

impl AgentActionProvider {
    pub fn resolve(self, chat_provider: CLIAgent) -> CLIAgent {
        match self {
            Self::Inherit => chat_provider,
            Self::Agent(agent) => agent,
        }
    }

    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Inherit => DEFAULT_SUGGESTION_PROVIDER,
            Self::Agent(agent) => agent.serialized_name(),
        }
    }

    pub fn is_inherit(self) -> bool {
        matches!(self, Self::Inherit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSuggestionActionConfig {
    pub provider: AgentActionProvider,
    pub model: Option<String>,
    pub effort: Option<String>,
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

    pub fn terminal_suggest_config(&self) -> AgentSuggestionActionConfig {
        AgentSuggestionActionConfig {
            provider: suggestion_provider(self.terminal_suggest_provider.value()),
            model: valid_chat_model(self.terminal_suggest_model.value()),
            effort: valid_chat_effort(self.terminal_suggest_effort.value()),
        }
    }

    pub fn reply_suggest_config(&self) -> AgentSuggestionActionConfig {
        AgentSuggestionActionConfig {
            provider: suggestion_provider(self.reply_suggest_provider.value()),
            model: valid_chat_model(self.reply_suggest_model.value()),
            effort: valid_chat_effort(self.reply_suggest_effort.value()),
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

pub fn suggestion_provider(serialized_name: &str) -> AgentActionProvider {
    let serialized_name = serialized_name.trim();
    if serialized_name.is_empty() || serialized_name == "default" {
        return AgentActionProvider::Inherit;
    }

    let agent = CLIAgent::from_serialized_name(serialized_name);
    if agent.is_agent_settings_enabled() {
        AgentActionProvider::Agent(agent)
    } else {
        AgentActionProvider::Inherit
    }
}

pub fn valid_suggestion_provider_value(serialized_name: &str) -> String {
    suggestion_provider(serialized_name)
        .serialized_name()
        .to_owned()
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

pub fn api_key_storage_key(agent: CLIAgent) -> Option<String> {
    (!matches!(agent, CLIAgent::Unknown))
        .then(|| format!("agent.api_key.{}", agent.serialized_name()))
}

pub fn api_key_presence(settings: &AgentSettings, agent: CLIAgent) -> bool {
    match agent {
        CLIAgent::Claude => *settings.claude_api_key_set.value(),
        CLIAgent::Codex | CLIAgent::Gemini | CLIAgent::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestion_provider_defaults_to_inherit() {
        assert_eq!(suggestion_provider(""), AgentActionProvider::Inherit);
        assert_eq!(suggestion_provider("default"), AgentActionProvider::Inherit);
        assert_eq!(suggestion_provider("unknown"), AgentActionProvider::Inherit);
    }

    #[test]
    fn suggestion_provider_allows_enabled_agents_only() {
        assert_eq!(
            suggestion_provider("claude"),
            AgentActionProvider::Agent(CLIAgent::Claude)
        );
        assert_eq!(suggestion_provider("codex"), AgentActionProvider::Inherit);
        assert_eq!(suggestion_provider("gemini"), AgentActionProvider::Inherit);
    }

    #[test]
    fn suggestion_provider_resolves_inherit_to_chat_provider() {
        assert_eq!(
            AgentActionProvider::Inherit.resolve(CLIAgent::Claude),
            CLIAgent::Claude
        );
        assert_eq!(
            AgentActionProvider::Agent(CLIAgent::Claude).resolve(CLIAgent::Gemini),
            CLIAgent::Claude
        );
    }
}
