//! Speech-to-text client (twarp 17, PRODUCT §5, §7–§8, §23).
//!
//! One `transcribe` entry point over two request shapes: Azure AI Foundry's
//! deployment-scoped path with an `api-version` query + `api-key` header, and
//! the plain OpenAI-compatible `/audio/transcriptions` with bearer auth. The
//! URL/auth builders are pure so the shapes are unit-testable.

use std::time::Duration;

use super::config::{trimmed_endpoint, VoiceProviderKind, VoiceSttConfig};
use super::VoiceError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Header name + value carrying the credential (PRODUCT §23).
pub(crate) fn auth_header(kind: VoiceProviderKind, api_key: &str) -> (&'static str, String) {
    match kind {
        VoiceProviderKind::AzureFoundry => ("api-key", api_key.to_owned()),
        VoiceProviderKind::OpenAiCompatible => ("authorization", format!("Bearer {api_key}")),
    }
}

pub(crate) fn stt_url(config: &VoiceSttConfig) -> String {
    let endpoint = trimmed_endpoint(&config.endpoint);
    match config.kind {
        VoiceProviderKind::AzureFoundry => format!(
            "{endpoint}/openai/deployments/{}/audio/transcriptions?api-version={}",
            config.model, config.api_version
        ),
        VoiceProviderKind::OpenAiCompatible => format!("{endpoint}/audio/transcriptions"),
    }
}

/// Upload `wav` (16 kHz mono, from [`super::capture`]) and return the
/// transcript text. Runs on the tokio pool; marshal back with `ctx.spawn`.
pub async fn transcribe(config: VoiceSttConfig, wav: Vec<u8>) -> Result<String, VoiceError> {
    let url = stt_url(&config);
    let (auth_name, auth_value) = auth_header(config.kind, &config.api_key);

    let file = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| VoiceError::Http(e.to_string()))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("response_format", "json");
    if config.kind == VoiceProviderKind::OpenAiCompatible {
        // Azure scopes the model via the deployment path; OpenAI-compatible
        // endpoints take it in the form.
        form = form.text("model", config.model.clone());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| VoiceError::Http(e.to_string()))?;
    let response = client
        .post(url)
        .header(auth_name, auth_value)
        .multipart(form)
        .send()
        .await
        .map_err(|e| VoiceError::Http(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| VoiceError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(VoiceError::Api {
            status: status.as_u16(),
            message: api_error_message(&body),
        });
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| VoiceError::Http(format!("bad response: {e}")))?;
    let text = parsed
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(VoiceError::EmptyTranscript);
    }
    Ok(text)
}

/// Pull the human-readable message out of an OpenAI/Azure error body, falling
/// back to the (truncated) raw body.
pub(crate) fn api_error_message(body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.get("message")?.as_str().map(str::to_owned));
    message.unwrap_or_else(|| body.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::super::config::*;
    use super::*;

    fn azure_config() -> VoiceSttConfig {
        VoiceSttConfig {
            kind: VoiceProviderKind::AzureFoundry,
            endpoint: "https://my-res.openai.azure.com/".into(),
            model: "gpt-4o-transcribe".into(),
            api_version: DEFAULT_AZURE_API_VERSION.into(),
            api_key: "k".into(),
        }
    }

    #[test]
    fn azure_url_shape() {
        // Trailing slash on the endpoint must not double up.
        assert_eq!(
            stt_url(&azure_config()),
            "https://my-res.openai.azure.com/openai/deployments/gpt-4o-transcribe/audio/transcriptions?api-version=2025-03-01-preview"
        );
    }

    #[test]
    fn openai_url_shape() {
        let config = VoiceSttConfig {
            kind: VoiceProviderKind::OpenAiCompatible,
            endpoint: "https://api.openai.com/v1".into(),
            ..azure_config()
        };
        assert_eq!(
            stt_url(&config),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn auth_headers_per_kind() {
        assert_eq!(
            auth_header(VoiceProviderKind::AzureFoundry, "sekret"),
            ("api-key", "sekret".to_owned())
        );
        assert_eq!(
            auth_header(VoiceProviderKind::OpenAiCompatible, "sekret"),
            ("authorization", "Bearer sekret".to_owned())
        );
    }

    #[test]
    fn error_message_extraction() {
        assert_eq!(
            api_error_message(r#"{"error":{"message":"bad key","code":401}}"#),
            "bad key"
        );
        assert_eq!(api_error_message("plain failure"), "plain failure");
    }
}
