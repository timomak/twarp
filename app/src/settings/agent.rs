use claude_code::driver::PermissionMode;
use settings::{
    macros::define_settings_group, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};

use crate::app_state::CLIAgent;
use crate::voice::config::{
    VoiceProviderKind, VoiceSttConfig, VoiceTtsConfig, DEFAULT_AZURE_API_VERSION,
    DEFAULT_STT_MODEL, DEFAULT_TTS_MODEL, DEFAULT_TTS_VOICE,
};

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
    // twarp 17: voice chat (PRODUCT §19–§23). Keys live in the keychain;
    // settings persist endpoints/models and key-presence flags only.
    voice_stt_kind: AgentVoiceSttKind {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.stt.kind",
        description: "Speech-to-text provider kind: azure_foundry or openai_compatible.",
    },
    voice_stt_endpoint: AgentVoiceSttEndpoint {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.stt.endpoint",
        description: "Speech-to-text endpoint: provider resource endpoint or OpenAI-compatible base URL.",
    },
    voice_stt_model: AgentVoiceSttModel {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.stt.model",
        description: "Speech-to-text model or provider deployment name. Empty means gpt-4o-transcribe.",
    },
    voice_stt_api_version: AgentVoiceSttApiVersion {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.stt.api_version",
        description: "Provider api-version for the transcription endpoint. Empty means the built-in default.",
    },
    voice_tts_kind: AgentVoiceTtsKind {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.tts.kind",
        description: "Text-to-speech provider kind: azure_foundry or openai_compatible.",
    },
    voice_tts_endpoint: AgentVoiceTtsEndpoint {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.tts.endpoint",
        description: "Text-to-speech endpoint: provider resource endpoint or OpenAI-compatible base URL. Empty means reuse the speech-to-text endpoint.",
    },
    voice_tts_model: AgentVoiceTtsModel {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.tts.model",
        description: "Text-to-speech model or provider deployment name. Empty means gpt-4o-mini-tts.",
    },
    voice_tts_voice: AgentVoiceTtsVoice {
        type: String,
        default: DEFAULT_TTS_VOICE.to_owned(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.tts.voice",
        description: "The OpenAI voice used for spoken replies.",
    },
    voice_tts_use_stt_key: AgentVoiceTtsUseSttKey {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.tts.use_stt_key",
        description: "Whether text-to-speech reuses the speech-to-text API key.",
    },
    voice_auto_send: AgentVoiceAutoSend {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.voice.auto_send",
        description: "Whether a voice transcription into an empty composer sends immediately.",
    },
    voice_stt_api_key_set: AgentVoiceSttApiKeySet {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.auth.voice_stt.api_key_set",
        description: "Whether a voice speech-to-text API key is stored in the OS keychain.",
    },
    voice_tts_api_key_set: AgentVoiceTtsApiKeySet {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "agent.auth.voice_tts.api_key_set",
        description: "Whether a dedicated voice text-to-speech API key is stored in the OS keychain.",
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
        let provider = self.chat_provider_agent();
        AgentChatLaunchConfig {
            provider,
            model: valid_model_for_provider(provider, self.chat_model.value()),
            effort: valid_effort_for_provider(provider, self.chat_effort.value()),
            permission_mode: valid_permission_mode_for_provider(
                provider,
                self.chat_permission_mode.value(),
            )
            .unwrap_or(PermissionMode::Default),
        }
    }

    pub fn terminal_suggest_config(&self) -> AgentSuggestionActionConfig {
        let chat_provider = self.chat_provider_agent();
        let provider = suggestion_provider(self.terminal_suggest_provider.value());
        let resolved_provider = provider.resolve(chat_provider);
        AgentSuggestionActionConfig {
            provider,
            model: valid_model_for_provider(resolved_provider, self.terminal_suggest_model.value()),
            effort: valid_effort_for_provider(
                resolved_provider,
                self.terminal_suggest_effort.value(),
            ),
        }
    }

    pub fn reply_suggest_config(&self) -> AgentSuggestionActionConfig {
        let chat_provider = self.chat_provider_agent();
        let provider = suggestion_provider(self.reply_suggest_provider.value());
        let resolved_provider = provider.resolve(chat_provider);
        AgentSuggestionActionConfig {
            provider,
            model: valid_model_for_provider(resolved_provider, self.reply_suggest_model.value()),
            effort: valid_effort_for_provider(resolved_provider, self.reply_suggest_effort.value()),
        }
    }
}

/// Keychain accounts for the voice keys (twarp 17, PRODUCT §17 pattern —
/// same service as `api_key_storage_key`, distinct accounts).
pub const VOICE_STT_API_KEY_STORAGE_KEY: &str = "agent.api_key.voice_stt";
pub const VOICE_TTS_API_KEY_STORAGE_KEY: &str = "agent.api_key.voice_tts";

impl AgentSettings {
    /// The resolved STT provider, or `None` while unconfigured (no endpoint or
    /// no stored key — drives the mic button's PRODUCT §2 behavior).
    /// `api_key` comes back **empty**: settings can't reach the keychain, so
    /// the caller fills it via [`VOICE_STT_API_KEY_STORAGE_KEY`].
    pub fn voice_stt_config(&self) -> Option<VoiceSttConfig> {
        let endpoint = self.voice_stt_endpoint.value().trim().to_owned();
        if endpoint.is_empty() || !*self.voice_stt_api_key_set.value() {
            return None;
        }
        Some(VoiceSttConfig {
            kind: VoiceProviderKind::from_serialized_name(self.voice_stt_kind.value()),
            endpoint,
            model: non_empty_or(self.voice_stt_model.value(), DEFAULT_STT_MODEL),
            api_version: non_empty_or(
                self.voice_stt_api_version.value(),
                DEFAULT_AZURE_API_VERSION,
            ),
            api_key: String::new(),
        })
    }

    /// The resolved TTS provider, or `None` while unconfigured (PRODUCT §12).
    /// An empty TTS endpoint reuses the STT endpoint/kind/api-version, and
    /// `use_stt_key` (default on) reuses the STT key (PRODUCT §21). `api_key`
    /// comes back empty; the caller fills it from the keychain account named
    /// by [`Self::voice_tts_key_storage_key`].
    pub fn voice_tts_config(&self) -> Option<VoiceTtsConfig> {
        let stt_endpoint = self.voice_stt_endpoint.value().trim().to_owned();
        let own_endpoint = self.voice_tts_endpoint.value().trim().to_owned();
        let reuse_stt_endpoint = own_endpoint.is_empty();
        let endpoint = if reuse_stt_endpoint {
            stt_endpoint
        } else {
            own_endpoint
        };
        if endpoint.is_empty() {
            return None;
        }
        let key_set = if *self.voice_tts_use_stt_key.value() {
            *self.voice_stt_api_key_set.value()
        } else {
            *self.voice_tts_api_key_set.value()
        };
        if !key_set {
            return None;
        }
        let (kind, api_version) = if reuse_stt_endpoint {
            (
                VoiceProviderKind::from_serialized_name(self.voice_stt_kind.value()),
                non_empty_or(
                    self.voice_stt_api_version.value(),
                    DEFAULT_AZURE_API_VERSION,
                ),
            )
        } else {
            (
                VoiceProviderKind::from_serialized_name(self.voice_tts_kind.value()),
                DEFAULT_AZURE_API_VERSION.to_owned(),
            )
        };
        Some(VoiceTtsConfig {
            kind,
            endpoint,
            model: non_empty_or(self.voice_tts_model.value(), DEFAULT_TTS_MODEL),
            voice: non_empty_or(self.voice_tts_voice.value(), DEFAULT_TTS_VOICE),
            api_version,
            api_key: String::new(),
        })
    }

    /// Which keychain account holds the TTS key (PRODUCT §21's "use
    /// speech-to-text key" toggle).
    pub fn voice_tts_key_storage_key(&self) -> &'static str {
        if *self.voice_tts_use_stt_key.value() {
            VOICE_STT_API_KEY_STORAGE_KEY
        } else {
            VOICE_TTS_API_KEY_STORAGE_KEY
        }
    }
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
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
    valid_model_for_provider(CLIAgent::Claude, model)
}

pub fn valid_model_for_provider(provider: CLIAgent, model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    provider.is_valid_model(model).then(|| model.to_owned())
}

pub fn valid_chat_effort(effort: &str) -> Option<String> {
    valid_effort_for_provider(CLIAgent::Claude, effort)
}

pub fn valid_effort_for_provider(provider: CLIAgent, effort: &str) -> Option<String> {
    match effort.trim() {
        "" | "default" => None,
        effort if provider.is_valid_effort(effort) => Some(effort.to_owned()),
        _ => None,
    }
}

pub fn valid_permission_mode_for_provider(
    provider: CLIAgent,
    permission_mode: &str,
) -> Option<PermissionMode> {
    let permission_mode = PermissionMode::from_cli_arg(permission_mode)?;
    provider
        .permission_modes()
        .contains(&permission_mode)
        .then_some(permission_mode)
}

pub fn api_key_storage_key(agent: CLIAgent) -> Option<String> {
    agent
        .spawn_spec()
        .is_some()
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

    #[test]
    fn provider_validation_uses_capabilities() {
        assert_eq!(
            valid_model_for_provider(CLIAgent::Claude, "sonnet"),
            Some("sonnet".to_owned())
        );
        assert_eq!(valid_model_for_provider(CLIAgent::Unknown, "sonnet"), None);
        assert_eq!(
            valid_effort_for_provider(CLIAgent::Claude, "medium"),
            Some("medium".to_owned())
        );
        assert_eq!(valid_effort_for_provider(CLIAgent::Unknown, "medium"), None);
        assert_eq!(
            valid_permission_mode_for_provider(CLIAgent::Claude, "plan"),
            Some(PermissionMode::Plan)
        );
        assert_eq!(
            valid_permission_mode_for_provider(CLIAgent::Unknown, "plan"),
            None
        );
    }

    #[test]
    fn disabled_agents_have_no_adapter_backed_key_storage() {
        assert_eq!(
            api_key_storage_key(CLIAgent::Claude),
            Some("agent.api_key.claude".to_owned())
        );
        assert_eq!(api_key_storage_key(CLIAgent::Codex), None);
        assert_eq!(api_key_storage_key(CLIAgent::Gemini), None);
    }
}
