//! twarp 17: voice dictation for the agent pane.
//!
//! Speech-to-text (`gpt-4o-transcribe` on Azure AI Foundry or any
//! OpenAI-compatible endpoint) types into the composer. Capture and playback
//! each run on a dedicated thread owning its cpal stream (cpal streams are
//! `!Send`, and the UI thread must never block on CoreAudio); the async HTTP
//! clients marshal back to the view via `ctx.spawn`, the `agent_suggestions`
//! pattern.
//!
//! Feature 25 removed the text-to-speech path — spoken replies read the
//! *rendered* reply, which is lossy by construction for a coding agent (see
//! `roadmap/25-voice-conversation/PRODUCT.md`). `playback` stays: it is the
//! audio sink for the realtime conversation that replaces it.

pub mod capture;
pub mod config;
pub mod playback;
pub mod stt;
mod wav;

use std::sync::atomic::{AtomicBool, Ordering};

/// One recording at a time across the app (PRODUCT §10): the mic is a global
/// resource, so a second pane's mic button is disabled while any pane records.
static RECORDING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Claim the app-wide recording slot. Returns `false` if another pane holds it.
pub fn try_begin_recording() -> bool {
    RECORDING_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Release the app-wide recording slot (stop, cancel, or recorder drop).
pub fn end_recording() {
    RECORDING_ACTIVE.store(false, Ordering::SeqCst);
}

/// Whether any pane is currently recording (dims other panes' mic buttons).
pub fn recording_active() -> bool {
    RECORDING_ACTIVE.load(Ordering::SeqCst)
}

/// Voice-pipeline failure, shown verbatim in the composer status line
/// (PRODUCT §8/§17). Never contains the API key.
#[derive(Debug, Clone)]
pub enum VoiceError {
    /// Provider not configured / config incomplete (PRODUCT §2, §12).
    Config(String),
    /// Device / capture / playback failure (PRODUCT §11).
    Audio(String),
    /// Transport-level HTTP failure.
    Http(String),
    /// The provider answered with a non-success status.
    Api { status: u16, message: String },
    /// The provider returned an empty transcript (PRODUCT §8).
    EmptyTranscript,
}

impl std::fmt::Display for VoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoiceError::Config(message) => write!(f, "{message}"),
            VoiceError::Audio(message) => write!(f, "Audio error: {message}"),
            VoiceError::Http(message) => write!(f, "Request failed: {message}"),
            VoiceError::Api { status, message } => {
                write!(f, "Provider error ({status}): {message}")
            }
            VoiceError::EmptyTranscript => write!(f, "No speech recognized"),
        }
    }
}

impl std::error::Error for VoiceError {}
