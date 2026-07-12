//! Resolved voice-provider configuration (twarp 17, PRODUCT §19–§23).
//!
//! Settings (`app/src/settings/agent.rs`) persist the raw fields and the
//! keychain holds the keys; `AgentSettings::voice_stt_config()` /
//! `voice_tts_config()` resolve them into these structs, or `None` when the
//! provider is unconfigured (which drives the mic/speaker buttons'
//! unconfigured behavior, PRODUCT §2/§12).

/// Which request shape a voice endpoint speaks (PRODUCT §23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceProviderKind {
    /// Azure AI Foundry / Azure OpenAI: deployment-scoped path, `api-version`
    /// query parameter, `api-key` header.
    #[default]
    AzureFoundry,
    /// Any OpenAI-compatible base URL: standard `/audio/*` paths, bearer auth.
    OpenAiCompatible,
}

impl VoiceProviderKind {
    pub const ALL: [VoiceProviderKind; 2] = [
        VoiceProviderKind::AzureFoundry,
        VoiceProviderKind::OpenAiCompatible,
    ];

    pub fn serialized_name(self) -> &'static str {
        match self {
            VoiceProviderKind::AzureFoundry => "azure_foundry",
            VoiceProviderKind::OpenAiCompatible => "openai_compatible",
        }
    }

    pub fn from_serialized_name(name: &str) -> Self {
        match name {
            "openai_compatible" => VoiceProviderKind::OpenAiCompatible,
            _ => VoiceProviderKind::AzureFoundry,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            VoiceProviderKind::AzureFoundry => "Azure AI Foundry",
            VoiceProviderKind::OpenAiCompatible => "OpenAI-compatible",
        }
    }
}

/// Default Azure `api-version` for the audio endpoints; user-editable because
/// Azure moves this (PRODUCT §20).
pub const DEFAULT_AZURE_API_VERSION: &str = "2025-03-01-preview";

pub const DEFAULT_STT_MODEL: &str = "gpt-4o-transcribe";
pub const DEFAULT_TTS_MODEL: &str = "gpt-4o-mini-tts";
pub const DEFAULT_TTS_VOICE: &str = "alloy";

/// The OpenAI voice set (PRODUCT §21).
pub const TTS_VOICES: [&str; 11] = [
    "alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer", "verse",
];

/// Resolved speech-to-text provider (endpoint fields + keychain key).
#[derive(Debug, Clone)]
pub struct VoiceSttConfig {
    pub kind: VoiceProviderKind,
    /// Azure: resource endpoint (`https://<res>.openai.azure.com`).
    /// OpenAI-compatible: base URL (`https://api.openai.com/v1`).
    pub endpoint: String,
    /// Azure: deployment name. OpenAI-compatible: model name.
    pub model: String,
    /// Azure only.
    pub api_version: String,
    pub api_key: String,
}

/// Resolved text-to-speech provider.
#[derive(Debug, Clone)]
pub struct VoiceTtsConfig {
    pub kind: VoiceProviderKind,
    pub endpoint: String,
    pub model: String,
    pub voice: String,
    /// Azure only.
    pub api_version: String,
    pub api_key: String,
}

/// `endpoint` with any trailing slashes dropped, so path joins are uniform.
pub(crate) fn trimmed_endpoint(endpoint: &str) -> &str {
    endpoint.trim().trim_end_matches('/')
}
