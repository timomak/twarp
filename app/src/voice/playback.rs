//! Speech playback on a dedicated thread (twarp 17, PRODUCT §13–§15, §18).
//!
//! One thread owns the cpal output stream (same `!Send` rationale as
//! [`super::capture`]) and drains a shared queue of s16 samples that arrive as
//! TTS chunks. `play` replaces whatever is queued (PRODUCT §15), `append`
//! extends the current utterance (§16 chunking), `stop` silences immediately
//! (§14/§18). `is_active` is polled by the view's tick timer to drive the
//! speaker button's speaking state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::wav::downmix_resample_to_s16;
use super::VoiceError;

enum Command {
    /// Replace the queue with this utterance's first chunk.
    Play(Vec<u8>, u32),
    /// Extend the current utterance with a follow-on chunk.
    Append(Vec<u8>, u32),
    Stop,
}

pub struct Player {
    commands: mpsc::Sender<Command>,
    active: Arc<AtomicBool>,
}

impl Player {
    /// Spawn the playback thread. Fails if there is no output device.
    pub fn start() -> Result<Player, VoiceError> {
        let active = Arc::new(AtomicBool::new(false));
        let (commands, command_rx) = mpsc::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), VoiceError>>();

        let thread_active = active.clone();
        std::thread::Builder::new()
            .name("voice-playback".into())
            .spawn(move || playback_thread(command_rx, ready_tx, thread_active))
            .map_err(|e| VoiceError::Audio(format!("playback thread: {e}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Player { commands, active }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(VoiceError::Audio("audio output did not start".into())),
        }
    }

    /// Start speaking `pcm` (s16le mono at `sample_rate`), replacing anything
    /// still playing (PRODUCT §15).
    pub fn play(&self, pcm: Vec<u8>, sample_rate: u32) {
        self.active.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Play(pcm, sample_rate));
    }

    /// Queue a follow-on chunk of the same utterance (PRODUCT §16).
    pub fn append(&self, pcm: Vec<u8>, sample_rate: u32) {
        self.active.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Append(pcm, sample_rate));
    }

    /// Silence immediately (PRODUCT §14/§18).
    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
        self.active.store(false, Ordering::SeqCst);
    }

    /// Whether audio is queued or audible.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

fn playback_thread(
    commands: mpsc::Receiver<Command>,
    ready: mpsc::Sender<Result<(), VoiceError>>,
    active: Arc<AtomicBool>,
) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        let _ = ready.send(Err(VoiceError::Audio("no output device".into())));
        return;
    };
    let supported = match device.default_output_config() {
        Ok(supported) => supported,
        Err(e) => {
            let _ = ready.send(Err(VoiceError::Audio(format!("output config: {e}"))));
            return;
        }
    };
    let config: cpal::StreamConfig = supported.into();
    let out_rate = config.sample_rate.0;
    let out_channels = config.channels as usize;

    // Mono s16 samples already resampled to the device rate; the output
    // callback fans each one out to every channel.
    let queue: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));

    let callback_queue = queue.clone();
    let callback_active = active.clone();
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            let mut queue = callback_queue.lock().unwrap();
            for frame in data.chunks_mut(out_channels) {
                let sample = match queue.pop_front() {
                    Some(s) => s as f32 / i16::MAX as f32,
                    None => 0.0,
                };
                for slot in frame {
                    *slot = sample;
                }
            }
            if queue.is_empty() {
                callback_active.store(false, Ordering::SeqCst);
            }
        },
        |_e: cpal::StreamError| {},
        None,
    );
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            let _ = ready.send(Err(VoiceError::Audio(format!("output stream: {e}"))));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(VoiceError::Audio(format!("output start: {e}"))));
        return;
    }
    let _ = ready.send(Ok(()));

    // Exits (dropping the stream) when the Player is dropped.
    while let Ok(command) = commands.recv() {
        match command {
            Command::Play(pcm, rate) => {
                let samples = resample_for_output(&pcm, rate, out_rate);
                let mut queue = queue.lock().unwrap();
                queue.clear();
                queue.extend(samples);
            }
            Command::Append(pcm, rate) => {
                let samples = resample_for_output(&pcm, rate, out_rate);
                queue.lock().unwrap().extend(samples);
            }
            Command::Stop => queue.lock().unwrap().clear(),
        }
    }
}

/// s16le bytes at `from_rate` → mono s16 samples at the device rate.
fn resample_for_output(pcm: &[u8], from_rate: u32, to_rate: u32) -> Vec<i16> {
    let as_f32: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / i16::MAX as f32)
        .collect();
    downmix_resample_to_s16(&as_f32, 1, from_rate, to_rate)
}
