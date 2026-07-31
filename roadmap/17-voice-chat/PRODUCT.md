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

**Goals (live-voice amendment, 2026-07-16 — owner request)**

- Live transcription: speech appears in the composer while still recording (§30–§31).
- Live readout: spoken replies start with the first complete sentence of a streaming turn, not at turn end (§32).
- Karaoke effect: while a reply is being spoken, the sentence currently audible is highlighted in the transcript with a progressive fill (§33).

**Non-goals**

- No wake word, no always-listening mode, no local/on-device models.
- No realtime/websocket STT or TTS APIs — live behavior is built from the same HTTP endpoints (§30 re-transcribes the growing buffer; §32 synthesizes per sentence). Word-accurate karaoke timing is out of scope: the API returns no word timings, so the fill is a linear estimate.
- No voice features in the Raw CLI mode of the pane, the terminal, or other panes this pass.
- macOS is the only supported platform this pass (matches twarp's mac-first posture).

> **Superseded in part by feature 25 (2026-07-30).** The text-to-speech half of
> this spec — the speaker toggle and spoken replies (§12–§18) plus the live
> readout and karaoke amendments (§32–§33) — was **removed**, not re-tuned:
> speaking the *rendered* reply is lossy by construction for a coding agent,
> because the prose pass must delete the code, diffs and tables that carry the
> meaning. Voice output now happens as a realtime spoken *conversation*; see
> [../25-voice-conversation/PRODUCT.md](../25-voice-conversation/PRODUCT.md).
> Everything else here — the mic button, dictation, live transcription
> (§1–§11, §19–§20, §22–§23, §30–§31) — remains current and unchanged.

## Behavior

### Voice input (STT)

1. The Claude pane composer's left control row (currently permission control + paperclip) gains a **microphone button** immediately right of the paperclip, same size/hover treatment as the paperclip, tooltip "Voice input". It is present in both new-session and active-session composers.
2. If no STT provider is configured, clicking the mic shows a non-blocking notice ("Configure voice in Settings → Agent") and does not record. The button stays visible in its muted idle style (discoverability).
3. If macOS microphone permission is undetermined, the first click triggers the system permission prompt; if denied, clicking shows a notice directing the user to System Settings → Privacy & Security → Microphone. No recording starts in either failure case.
4. Clicking the mic (configured + permitted) starts recording: the button switches to an accent-colored recording state with a subtle pulse, and an elapsed-time label (m:ss) appears beside it. The composer stays fully editable while recording.
5. Clicking the mic again stops recording and submits the audio for transcription. While transcribing, the button shows a spinner state; a second click during transcription is a no-op.
6. Pressing **Esc** while recording cancels: audio is discarded, no request is made, the button returns to idle. Esc retains its existing composer behaviors when not recording.
7. On successful transcription, the text is appended to the composer draft (with a separating space if the draft doesn't already end in whitespace), the composer keeps focus, and nothing is auto-sent — **unless** the auto-send setting (§19) is on and the composer was empty both when recording started and when the transcript lands, in which case the transcript is submitted as a message immediately.
8. On transcription failure (network error, HTTP error, timeout, empty transcript), a non-blocking error notice appears with the provider's error text where available; the composer content is unchanged and the recorded audio is discarded.
9. Recording auto-stops (and transcribes) at a 5-minute cap. Closing the pane, switching it to Raw CLI mode, or quitting the app while recording cancels the recording silently.
10. Only one recording may be active across the app. Clicking the mic in another Claude pane while one records shows a non-blocking "Already recording in another pane" notice and does not start a second recording.
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

### Live transcription (amendment, 2026-07-16)

30. While recording, the pane periodically (≈ every 2.5 s, stretching as the recording grows) transcribes **everything captured so far** and mirrors the result into the composer, so speech shows up while still talking. One live request is in flight at a time; live requests never stop the recording. Live-pass failures are silent (the §5 stop-transcription still reports errors for real); transcription costs scale with recording length, which the stretching cadence bounds.
31. Each live pass **replaces** the text the previous pass inserted (the "live region" — appended after any pre-existing draft with the §7 separator rule). Earlier words may therefore self-correct as more audio context arrives. If the user edits the live region mid-recording, live updates stop for that recording and the user's text wins (the final §5 transcription is then discarded too). Esc (§6) removes the live region along with discarding the audio. The §7 auto-send emptiness test ignores the live region.

### Live readout + karaoke (amendment, 2026-07-16)

32. With the speaker toggle on, a **streaming** turn is spoken sentence-by-sentence as text arrives: each newly completed sentence (terminator `.` `!` `?` or newline, evaluated over the §13 prose form of complete markdown lines only) is synthesized and queued in order. Turn completion flushes the unterminated tail. §13's prose-only rule, §15's latest-reply-wins, §16's chunking cap, and §17/§18's failure/interrupt behavior all apply unchanged.
33. While audio is playing, the transcript highlights the sentence currently being spoken (karaoke): a light accent wash over the whole sentence and a stronger wash sweeping over its estimated already-spoken prefix (linear chars-over-audio-time estimate — the TTS API returns no word timings). The highlight tracks playback (including across §16 chunks), disappears when playback ends or is stopped, and never appears on code blocks or tables (nothing non-prose is spoken). If a spoken sentence cannot be located in the rendered markdown (rare formatting divergence), the highlight is simply skipped — audio is never affected.

### Cross-cutting

24. No audio, transcripts, or keys are ever written to logs or plaintext settings. Recorded audio exists only in memory for the duration of the request.
25. All voice affordances live behind the same build channels as the Claude pane itself; nothing about existing composer behavior (attach, pills, Enter-to-send, suggestions) changes when voice is unconfigured or idle.
