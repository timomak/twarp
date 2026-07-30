use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::{json, Value};

use crate::driver::OutgoingMessage;

pub const CODEX_PROGRAM: &str = "codex";
pub const MIN_CODEX_CLI_VERSION: &str = "0.135.0";

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_request_id() -> String {
    format!("twarp-{}", NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRpcRequest<T> {
    pub id: String,
    pub method: &'static str,
    pub params: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<InitializeCapabilities>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeCapabilities {
    pub experimental_api: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams<'a> {
    pub cwd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    /// twarp 20b: `config.toml`-style overrides (e.g. `mcp_servers`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<&'a Value>,
    pub sandbox: &'static str,
    pub approval_policy: &'static str,
    pub approvals_reviewer: &'static str,
    pub thread_source: &'static str,
    pub session_start_source: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeParams<'a> {
    pub thread_id: &'a str,
    pub cwd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    /// twarp 20b: `config.toml`-style overrides (e.g. `mcp_servers`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<&'a Value>,
    pub sandbox: &'static str,
    pub approval_policy: &'static str,
    pub approvals_reviewer: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams<'a> {
    pub thread_id: &'a str,
    pub input: Vec<UserInput<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum UserInput<'a> {
    #[serde(rename = "text", rename_all = "camelCase")]
    Text { text: &'a str },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptParams<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
}

pub fn initialize_request(id: String) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "initialize",
        params: InitializeParams {
            client_info: ClientInfo {
                name: "twarp",
                title: "twarp",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: Some(InitializeCapabilities {
                experimental_api: true,
            }),
        },
    })
}

pub fn thread_start_request(
    id: String,
    cwd: &Path,
    model: Option<&str>,
    config: Option<&Value>,
    sandbox: &'static str,
    approval_policy: &'static str,
) -> Value {
    let cwd = cwd.to_string_lossy();
    json!(JsonRpcRequest {
        id,
        method: "thread/start",
        params: ThreadStartParams {
            cwd: cwd.as_ref(),
            model,
            config,
            sandbox,
            approval_policy,
            approvals_reviewer: "user",
            thread_source: "user",
            session_start_source: "startup",
        },
    })
}

pub fn thread_resume_request(
    id: String,
    thread_id: &str,
    cwd: &Path,
    model: Option<&str>,
    config: Option<&Value>,
    sandbox: &'static str,
    approval_policy: &'static str,
) -> Value {
    let cwd = cwd.to_string_lossy();
    json!(JsonRpcRequest {
        id,
        method: "thread/resume",
        params: ThreadResumeParams {
            thread_id,
            cwd: cwd.as_ref(),
            model,
            config,
            sandbox,
            approval_policy,
            approvals_reviewer: "user",
        },
    })
}

pub fn turn_start_request(
    id: String,
    thread_id: &str,
    message: &OutgoingMessage,
    model: Option<&str>,
    effort: Option<&str>,
) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "turn/start",
        params: TurnStartParams {
            thread_id,
            input: vec![UserInput::Text {
                text: message.text.as_str(),
            }],
            model,
            effort,
        },
    })
}

pub fn turn_interrupt_request(id: String, thread_id: &str, turn_id: &str) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "turn/interrupt",
        params: TurnInterruptParams { thread_id, turn_id },
    })
}

// --- twarp 25: realtime voice conversation (TECH 25 "Verified protocol facts") ---

/// The `-c` override that unlocks `thread/realtime/*`. Without it every
/// realtime method is rejected; the `experimentalApi` capability in
/// [`initialize_request`] is the other half of the gate.
pub const REALTIME_FEATURE_FLAG: &str = "features.realtime_conversation=true";

/// Codex's default v2 realtime voice (`thread/realtime/listVoices`).
pub const DEFAULT_REALTIME_VOICE: &str = "marin";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeStartParams<'a> {
    pub thread_id: &'a str,
    /// `audio` or `text`.
    pub output_modality: &'static str,
    pub transport: RealtimeTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<&'a str>,
    pub version: &'static str,
    /// Deliver the coding agent's own responses into the voice conversation as
    /// items — Codex's bridge between the thread and the spoken session.
    pub codex_responses_as_items: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum RealtimeTransport {
    /// Codex owns the upstream socket, so twarp never terminates media itself
    /// and needs no WebRTC stack.
    #[serde(rename = "websocket")]
    Websocket,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeAppendAudioParams<'a> {
    pub thread_id: &'a str,
    pub audio: RealtimeAudioChunk<'a>,
}

/// A microphone chunk. Codex resamples, so these are the *device's* real
/// values — twarp never resamples on the UI path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeAudioChunk<'a> {
    /// base64 s16le samples.
    pub data: &'a str,
    pub sample_rate: u32,
    pub num_channels: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples_per_channel: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeAppendSpeechParams<'a> {
    pub thread_id: &'a str,
    pub text: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeThreadParams<'a> {
    pub thread_id: &'a str,
}

pub fn realtime_start_request(
    id: String,
    thread_id: &str,
    voice: Option<&str>,
    speak: bool,
) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "thread/realtime/start",
        params: RealtimeStartParams {
            thread_id,
            output_modality: if speak { "audio" } else { "text" },
            transport: RealtimeTransport::Websocket,
            voice,
            version: "v2",
            codex_responses_as_items: true,
        },
    })
}

pub fn realtime_append_audio_request(
    id: String,
    thread_id: &str,
    data: &str,
    sample_rate: u32,
    num_channels: u16,
    samples_per_channel: Option<u32>,
) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "thread/realtime/appendAudio",
        params: RealtimeAppendAudioParams {
            thread_id,
            audio: RealtimeAudioChunk {
                data,
                sample_rate,
                num_channels,
                samples_per_channel,
            },
        },
    })
}

/// The **turn trigger**. `appendText` only injects a conversation item and
/// never produces a reply; `appendSpeech` drives a spoken turn, and the model
/// answers the text conversationally rather than reading it back.
pub fn realtime_append_speech_request(id: String, thread_id: &str, text: &str) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "thread/realtime/appendSpeech",
        params: RealtimeAppendSpeechParams { thread_id, text },
    })
}

pub fn realtime_stop_request(id: String, thread_id: &str) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "thread/realtime/stop",
        params: RealtimeThreadParams { thread_id },
    })
}
