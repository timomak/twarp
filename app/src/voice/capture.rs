//! Microphone capture on a dedicated thread (twarp 17, PRODUCT §4–§6, §9–§11).
//!
//! cpal streams are `!Send` and CoreAudio must never be driven from the UI
//! thread, so the recorder spawns one thread that owns the input stream and
//! accumulates samples; the view talks to it through channels and shared
//! atomics. `stop()` finalizes in-memory (downmix → 16 kHz resample → WAV) and
//! hands back the upload bytes; `cancel()`/drop discards everything.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::wav;
use super::VoiceError;

/// PRODUCT §9: recording auto-stops (and transcribes) at this cap.
pub const MAX_RECORDING: Duration = Duration::from_secs(5 * 60);

enum Command {
    /// Finalize and reply with the WAV bytes.
    Stop(mpsc::Sender<Result<Vec<u8>, VoiceError>>),
    /// Encode everything captured so far WITHOUT stopping the stream — the
    /// live-transcription upload (PRODUCT §30).
    Snapshot(mpsc::Sender<Result<Vec<u8>, VoiceError>>),
    /// Discard everything (PRODUCT §6).
    Cancel,
}

struct Shared {
    /// Set by the cpal error callback or a failed stream build (§11).
    error: Mutex<Option<String>>,
    /// Set by the capture thread when the §9 cap is reached.
    capped: AtomicBool,
    /// The stream ended on its own (device lost); the view should
    /// stop-and-transcribe best effort (§11).
    ended: AtomicBool,
}

pub struct Recorder {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    started: Instant,
}

impl Recorder {
    /// Open the default input device and start capturing. Errors (no device,
    /// unsupported format, TCC denial surfacing as a build failure) come back
    /// immediately (§3/§11).
    pub fn start() -> Result<Recorder, VoiceError> {
        let shared = Arc::new(Shared {
            error: Mutex::new(None),
            capped: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });
        let (commands, command_rx) = mpsc::channel::<Command>();
        // The stream build happens on the capture thread; this channel reports
        // whether it came up so `start()` can fail fast.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), VoiceError>>();

        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("voice-capture".into())
            .spawn(move || capture_thread(command_rx, ready_tx, thread_shared))
            .map_err(|e| VoiceError::Audio(format!("capture thread: {e}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Recorder {
                commands,
                shared,
                started: Instant::now(),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(VoiceError::Audio("microphone did not start".into())),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// §9: the capture thread reached the recording cap.
    pub fn hit_cap(&self) -> bool {
        self.shared.capped.load(Ordering::SeqCst)
    }

    /// §11: the input stream died (device unplugged / error callback).
    pub fn stream_ended(&self) -> bool {
        self.shared.ended.load(Ordering::SeqCst) || self.shared.error.lock().unwrap().is_some()
    }

    /// Stop and finalize: 16 kHz mono WAV bytes ready for upload (§5).
    pub fn stop(self) -> Result<Vec<u8>, VoiceError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.commands
            .send(Command::Stop(reply_tx))
            .map_err(|_| VoiceError::Audio("capture thread exited".into()))?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| VoiceError::Audio("capture did not finalize".into()))?
    }

    /// Ask the capture thread to encode the audio captured so far (16 kHz
    /// mono WAV) while the stream keeps recording — feeds the §30 live
    /// transcription requests. Non-blocking: the encode (downmix + resample
    /// of the whole buffer) happens on the capture thread and the result
    /// arrives on the returned channel; the caller polls it with `try_recv`
    /// so the UI thread never waits on audio work.
    pub fn request_snapshot(&self) -> Option<mpsc::Receiver<Result<Vec<u8>, VoiceError>>> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.commands.send(Command::Snapshot(reply_tx)).ok()?;
        Some(reply_rx)
    }

    /// Discard the recording (§6). Dropping the recorder has the same effect.
    pub fn cancel(self) {
        let _ = self.commands.send(Command::Cancel);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // The capture thread exits on channel disconnect; make sure the
        // app-wide recording slot (PRODUCT §10) never leaks when the owning
        // pane is torn down mid-recording. Releasing twice is harmless.
        super::end_recording();
    }
}

fn capture_thread(
    commands: mpsc::Receiver<Command>,
    ready: mpsc::Sender<Result<(), VoiceError>>,
    shared: Arc<Shared>,
) {
    let built = build_stream(&shared);
    let (stream, buffer, channels, sample_rate) = match built {
        Ok(parts) => parts,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(VoiceError::Audio(format!("stream start: {e}"))));
        return;
    }
    let _ = ready.send(Ok(()));

    // Wait for a command, watching the cap so the view can auto-stop (§9).
    let max_samples = MAX_RECORDING.as_secs() as usize * sample_rate as usize * channels as usize;
    loop {
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Stop(reply)) => {
                drop(stream); // stop capturing before reading the buffer
                let samples = buffer.lock().unwrap();
                let mono = wav::downmix_resample_to_s16(
                    &samples,
                    channels,
                    sample_rate,
                    wav::STT_SAMPLE_RATE,
                );
                let _ = reply.send(Ok(wav::encode_mono_s16(&mono, wav::STT_SAMPLE_RATE)));
                return;
            }
            Ok(Command::Snapshot(reply)) => {
                let mono = {
                    let samples = buffer.lock().unwrap();
                    wav::downmix_resample_to_s16(
                        &samples,
                        channels,
                        sample_rate,
                        wav::STT_SAMPLE_RATE,
                    )
                };
                let _ = reply.send(Ok(wav::encode_mono_s16(&mono, wav::STT_SAMPLE_RATE)));
            }
            // Cancel, or the Recorder was dropped: discard everything.
            Ok(Command::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if buffer.lock().unwrap().len() >= max_samples {
                    shared.capped.store(true, Ordering::SeqCst);
                }
            }
        }
    }
}

type StreamParts = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u16, u32);

fn build_stream(shared: &Arc<Shared>) -> Result<StreamParts, VoiceError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| VoiceError::Audio("no input device".into()))?;
    let supported = device
        .default_input_config()
        .map_err(|e| VoiceError::Audio(format!("input config: {e}")))?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels;
    let sample_rate = config.sample_rate.0;

    let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
    let error_shared = shared.clone();
    let err_fn = move |e: cpal::StreamError| {
        *error_shared.error.lock().unwrap() = Some(e.to_string());
        error_shared.ended.store(true, Ordering::SeqCst);
    };

    let stream =
        match sample_format {
            cpal::SampleFormat::F32 => {
                let sink = buffer.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _| sink.lock().unwrap().extend_from_slice(data),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let sink = buffer.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        sink.lock()
                            .unwrap()
                            .extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let sink = buffer.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        sink.lock().unwrap().extend(data.iter().map(|&s| {
                            (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0)
                        }));
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                return Err(VoiceError::Audio(format!(
                    "unsupported input format {other:?}"
                )))
            }
        }
        .map_err(|e| VoiceError::Audio(format!("input stream: {e}")))?;

    Ok((stream, buffer, channels, sample_rate))
}
