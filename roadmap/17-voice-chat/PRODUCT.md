# Voice chat in the Claude pane — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers.

## Summary

Talk to the Claude pane. A **microphone button** in the composer (next to the attach/paperclip button) records speech and transcribes it into the composer via a **speech-to-text provider** (`gpt-4o-transcribe` on Azure AI Foundry, or any OpenAI-compatible endpoint). A **speaker toggle** next to it makes twarp **speak Claude's replies aloud** via an OpenAI-voice **text-to-speech provider**. Both providers are configured on the existing **Agent settings page** (feature 16), with API keys in the OS keychain.

> **Placement note (owner review):** the request said "next to the pin icon". The Claude pane has no pin icon; the closest match is the composer's **paperclip (attach) button**, so the mic + speaker buttons go in the composer's left control row beside it. Flag if a different spot was meant (e.g. the pane-header icon cluster).

Figma: none provided.

## Goals / Non-goals

**Goals**

- Push-to-toggle voice input: record → transcribe → text lands in the composer for review; Enter sends as usual.
- Spoken replies: opt-in per pane; prose is spoken, code/tool output is skipped.
- Provider-agnostic config: Azure AI Foundry deployments **and** generic OpenAI-compatible endpoints, independent STT and TTS entries, keys in the OS keychain (feature 16 pattern).
- Optional **auto-send** setting (off by default) so a transcription submits immediately — combined with spoken replies this gives a hands-free loop.

**Non-goals**

- No realtime/streaming transcription while speaking (single-shot transcribe on stop).
- No wake word, no always-listening mode, no local/on-device models.
- No sentence-by-sentence TTS during streaming — speech starts when the turn's text is final.
- No voice features in the Raw CLI mode of the pane, the terminal, or other panes this pass.
- macOS is the only supported platform this pass (matches twarp's mac-first posture).

## Behavior

### Voice input (STT)

1. The Claude pane composer's left control row (currently permission control + paperclip) gains a **microphone button** immediately right of the paperclip, same size/hover treatment as the paperclip, tooltip "Voice input". It is present in both new-session and active-session composers.
2. If no STT provider is configured, clicking the mic shows a non-blocking notice ("Configure voice in Settings → Agent") and does not record. The button renders slightly dimmed in this state but stays visible (discoverability).
3. If macOS microphone permission is undetermined, the first click triggers the system permission prompt; if denied, clicking shows a notice directing the user to System Settings → Privacy & Security → Microphone. No recording starts in either failure case.
4. Clicking the mic (configured + permitted) starts recording: the button switches to an accent-colored recording state with a subtle pulse, and an elapsed-time label (m:ss) appears beside it. The composer stays fully editable while recording.
5. Clicking the mic again stops recording and submits the audio for transcription. While transcribing, the button shows a spinner state; a second click during transcription is a no-op.
6. Pressing **Esc** while recording cancels: audio is discarded, no request is made, the button returns to idle. Esc retains its existing composer behaviors when not recording.
7. On successful transcription, the text is inserted at the composer cursor (with a separating space if the composer already has adjacent text), the composer keeps focus, and nothing is auto-sent — **unless** the auto-send setting (§19) is on and the composer was empty when recording started, in which case the transcript is submitted as a message immediately.
8. On transcription failure (network error, HTTP error, timeout, empty transcript), a non-blocking error notice appears with the provider's error text where available; the composer content is unchanged and the recorded audio is discarded.
9. Recording auto-stops (and transcribes) at a 5-minute cap. Closing the pane, switching it to Raw CLI mode, or quitting the app while recording cancels the recording silently.
10. Only one recording may be active across the app. The mic button in other Claude panes is disabled (dimmed, tooltip "Recording in another pane") while one records.
11. Recording captures from the system default input device. Device changes mid-recording end the recording as a stop-and-transcribe (best effort), not a crash.

### Spoken replies (TTS)

12. A **speaker button** sits immediately right of the mic button. It toggles "speak replies" for that pane; default off; per-pane; not persisted across app restarts. If no TTS provider is configured, clicking it shows the same settings notice as §2 and stays off.
13. While on, when a Claude turn completes (final assistant text for that turn), the pane speaks that text. Only prose is spoken: markdown syntax is stripped, and fenced code blocks, tool cards, and other non-prose items are skipped entirely.
14. The speaker button shows an active state while audio plays. Clicking it while speaking stops playback immediately **and** turns the toggle off; clicking again re-enables it for subsequent turns.
15. If a new turn completes while a previous reply is still playing, the previous playback stops and the new reply is spoken (only the latest reply plays).
16. Replies longer than the provider's per-request input cap are chunked at sentence boundaries and played back seamlessly in order.
17. TTS failures (network/HTTP errors) show a non-blocking notice once per turn and never block or delay the rendered text. Starting a voice recording (§4) pauses nothing — recording and playback may overlap, but see §18.
18. Turns that were interrupted (Stop) are not spoken. Closing the pane stops its playback.

### Settings (Agent page)

19. The Agent settings page gains a **Voice** section with three parts: **Speech-to-text**, **Text-to-speech**, and an **Auto-send transcriptions** toggle (default off, governs §7).
20. Speech-to-text row set: provider kind (**Azure AI Foundry** | **OpenAI-compatible**), endpoint/base URL, model/deployment name (default `gpt-4o-transcribe`), API version (Azure kind only; sensible default prefilled), and an API key field using the same masked save/remove affordance as the existing Claude key row. The key lives in the OS keychain; settings persist only a presence flag.
21. Text-to-speech row set: same provider-kind/endpoint/model fields (model default `gpt-4o-mini-tts`), a **voice** dropdown (the OpenAI voice set: alloy, ash, ballad, coral, echo, fable, onyx, nova, sage, shimmer, verse), a "use speech-to-text key" toggle (default on) with its own key field when off, and a **Test voice** button that speaks a short sample using current values and surfaces any error inline.
22. Voice settings are local-only (never cloud-synced), consistent with all feature-16 fields. Removing a key clears it from the keychain and flips the presence flag; the mic/speaker buttons immediately fall back to the unconfigured behavior (§2, §12).
23. Endpoint values are used as entered: Azure kind expects a resource endpoint (deployment + api-version appended per Azure's audio API); OpenAI-compatible kind expects a base URL against which the standard `/audio/transcriptions` and `/audio/speech` paths are called with bearer auth. This is what makes "or any other" providers work.

### Cross-cutting

24. No audio, transcripts, or keys are ever written to logs or plaintext settings. Recorded audio exists only in memory for the duration of the request.
25. All voice affordances live behind the same build channels as the Claude pane itself; nothing about existing composer behavior (attach, pills, Enter-to-send, suggestions) changes when voice is unconfigured or idle.
