//! Text-to-speech client (twarp 17, PRODUCT §13–§17, §21, §23).
//!
//! `synthesize` returns raw PCM (24 kHz s16le mono — `response_format: "pcm"`,
//! so no decoder dependency); [`super::playback`] resamples to the output
//! device. Long replies are chunked at sentence boundaries under the
//! provider's input cap (PRODUCT §16) and synthesized sequentially.

use std::time::Duration;

use super::config::{trimmed_endpoint, VoiceProviderKind, VoiceTtsConfig};
use super::stt::{api_error_message, auth_header};
use super::VoiceError;

/// Sample rate of `response_format: "pcm"` on the OpenAI audio API.
pub const TTS_SAMPLE_RATE: u32 = 24_000;

/// The OpenAI speech endpoint caps `input` at 4096 characters.
pub const TTS_INPUT_CAP: usize = 4096;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) fn tts_url(config: &VoiceTtsConfig) -> String {
    let endpoint = trimmed_endpoint(&config.endpoint);
    match config.kind {
        VoiceProviderKind::AzureFoundry => format!(
            "{endpoint}/openai/deployments/{}/audio/speech?api-version={}",
            config.model, config.api_version
        ),
        VoiceProviderKind::OpenAiCompatible => format!("{endpoint}/audio/speech"),
    }
}

/// Synthesize one chunk of text to 24 kHz s16le mono PCM.
pub async fn synthesize(config: &VoiceTtsConfig, text: &str) -> Result<Vec<u8>, VoiceError> {
    let url = tts_url(config);
    let (auth_name, auth_value) = auth_header(config.kind, &config.api_key);

    let mut body = serde_json::json!({
        "model": config.model,
        "voice": config.voice,
        "input": text,
        "response_format": "pcm",
    });
    if config.kind == VoiceProviderKind::AzureFoundry {
        // The deployment path already picks the model, but Azure accepts (and
        // some api-versions require) it in the body too — harmless to send.
        body["model"] = serde_json::Value::String(config.model.clone());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| VoiceError::Http(e.to_string()))?;
    let response = client
        .post(url)
        .header(auth_name, auth_value)
        .json(&body)
        .send()
        .await
        .map_err(|e| VoiceError::Http(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(VoiceError::Api {
            status: status.as_u16(),
            message: api_error_message(&body),
        });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| VoiceError::Http(e.to_string()))?;
    Ok(bytes.to_vec())
}

/// Split `text` into chunks of at most `cap` characters, preferring sentence
/// boundaries (`.` `!` `?` and newlines), falling back to whitespace, and
/// hard-splitting only when a single token exceeds the cap (PRODUCT §16).
pub fn chunk_text(text: &str, cap: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        if current.len() + sentence.len() <= cap {
            current.push_str(sentence);
            continue;
        }
        if !current.trim().is_empty() {
            chunks.push(std::mem::take(&mut current).trim().to_owned());
        }
        if sentence.len() <= cap {
            current.push_str(sentence);
        } else {
            // A single monster sentence: split on char boundaries under the cap.
            let mut rest = sentence;
            while rest.len() > cap {
                let mut cut = cap;
                while !rest.is_char_boundary(cut) {
                    cut -= 1;
                }
                let split_at = rest[..cut].rfind(char::is_whitespace).unwrap_or(cut).max(1);
                chunks.push(rest[..split_at].trim().to_owned());
                rest = &rest[split_at..];
            }
            current.push_str(rest);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_owned());
    }
    chunks
}

/// Iterate sentence-ish pieces, each keeping its terminator.
fn split_sentences(text: &str) -> impl Iterator<Item = &str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if matches!(c, '.' | '!' | '?' | '\n') {
            let end = i + c.len_utf8();
            pieces.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        pieces.push(&text[start..]);
    }
    pieces.into_iter()
}

#[cfg(test)]
mod tests {
    use super::super::config::*;
    use super::*;

    #[test]
    fn urls_per_kind() {
        let mut config = VoiceTtsConfig {
            kind: VoiceProviderKind::AzureFoundry,
            endpoint: "https://my-res.openai.azure.com".into(),
            model: "gpt-4o-mini-tts".into(),
            voice: "alloy".into(),
            api_version: "2025-03-01-preview".into(),
            api_key: "k".into(),
        };
        assert_eq!(
            tts_url(&config),
            "https://my-res.openai.azure.com/openai/deployments/gpt-4o-mini-tts/audio/speech?api-version=2025-03-01-preview"
        );
        config.kind = VoiceProviderKind::OpenAiCompatible;
        config.endpoint = "https://api.openai.com/v1/".into();
        assert_eq!(tts_url(&config), "https://api.openai.com/v1/audio/speech");
    }

    #[test]
    fn chunking_respects_cap_and_sentences() {
        let text = "One two three. Four five six! Seven eight nine?";
        let chunks = chunk_text(text, 20);
        assert_eq!(
            chunks,
            vec!["One two three.", "Four five six!", "Seven eight nine?"]
        );
        assert!(chunks.iter().all(|c| c.len() <= 20));
    }

    #[test]
    fn chunking_packs_short_sentences() {
        let chunks = chunk_text("A. B. C.", 4096);
        assert_eq!(chunks, vec!["A. B. C."]);
    }

    #[test]
    fn chunking_hard_splits_monster_sentences() {
        let text = "word ".repeat(100); // 500 chars, no terminator
        let chunks = chunk_text(&text, 60);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.len() <= 60 && !c.is_empty()));
    }

    #[test]
    fn chunking_empty() {
        assert!(chunk_text("   ", 10).is_empty());
    }
}
