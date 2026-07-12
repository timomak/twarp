# Voice chat in the Claude pane — TECH

Companion to [PRODUCT.md](PRODUCT.md); §N references its Behavior invariants.

## Context

- **Composer insertion point.** The Claude pane composer is `render_input` at `app/src/claude_code_view.rs:6423`; its footer control row is the `controls_for` closure at `:6520`, whose left `Flex::row` (`:6521`) holds the permission control + paperclip attach button (`make_attach`, `:6491`, `Icon::Paperclip`, dispatches `ClaudeCodeViewAction::AttachFromPicker`). Mic + speaker buttons (§1, §12) go in this left row. The composer lives inside the pane's own view tree, so plain `ClaudeCodeViewAction` variants work — the `PaneHeaderAction` indirection needed for header buttons (`claude_code_view.rs:468–495`) does **not** apply here.
- **Icon + permission scaffolding already exist.** `Icon::Microphone` (`crates/twarp_core/src/ui/icons.rs:159` → `app/assets/bundled/svg/microphone.svg`) survived the 2c-f voice_input deletion, as did the platform mic-permission API: `MicrophoneAccessState` / `microphone_access_state()` (`crates/twarpui_core/src/platform/mod.rs:186,:276`, macOS impl via `AVCaptureDevice` at `crates/twarpui/src/platform/mac/delegate.rs:430`, exposed on `AppContext` at `crates/twarpui_core/src/core/app.rs:4489`). The app bundle's Info.plist already carries `NSMicrophoneUsageDescription`, so the TCC prompt (§3) needs no bundling changes. A speaker glyph likely needs a new SVG (add variant per the `icons.rs:313` `From` impl).
- **No audio stack.** Upstream's `voice_input` crate was deleted (2c-f); `cpal`/`rodio`/`coreaudio` have zero hits in `Cargo.lock`. Capture and playback are net-new.
- **Feature 16 gives the settings + secrets pattern.** `define_settings_group!(AgentSettings, ...)` at `app/src/settings/agent.rs:17` (flat TOML-path fields, `SyncToCloud::Never`); keychain via `twarpui_extras::secure_storage` (`SecureStorage` trait, `secure_storage/mod.rs:93`) with account keys from `api_key_storage_key()` (`agent.rs:301`) and presence flags like `agent.auth.claude.api_key_set`; page UI in `app/src/settings_view/agent_page.rs` (render root `:854`, masked key row `render_api_key_row` `:1340`, save/remove `:538/:578`).
- **HTTP precedent.** `app/src/agent_suggestions.rs:176` (`suggest_with_anthropic_api`) shows the pattern: per-call `reqwest::Client` with `https_only` + timeouts, spawned on a tokio task, result marshalled back via context notify. `reqwest` is already an `app` dependency with multipart support available via feature flag.
- **UI gotchas that apply here:** a pulsing record state (§4) needs a self-rearming `notify()` timer — `repaint_after` never re-runs `render()` on its own (see the elapsed-label pattern already used in the pane).

## Proposed changes

### New module `app/src/voice/` (net-new, no new crate)

- `capture.rs` — mic capture via **cpal** (new workspace dep; CoreAudio-backed on mac, no objc2 additions). Open the default input device, accumulate samples in memory, downmix to mono and linearly resample to 16 kHz i16 on stop. Enforce the 5-minute cap (§9) in the callback. Device-loss ends the stream gracefully (§11).
- `wav.rs` — minimal RIFF/WAV encoder for 16 kHz mono s16le (a hand-rolled ~40-line writer; avoids a `hound` dep for one fixed format).
- `stt.rs` — transcription client. Two request shapes off one config (§23):
  - **Azure AI Foundry:** `POST {endpoint}/openai/deployments/{model}/audio/transcriptions?api-version={v}` with `api-key` header (api-version default `2025-03-01-preview`, user-editable per §20 since Azure moves this).
  - **OpenAI-compatible:** `POST {base}/audio/transcriptions` with `Authorization: Bearer`.
  Both: multipart body (`file` = WAV; `model` in the form for the OpenAI-compatible shape — Azure scopes it via the deployment path), parse `{ "text": ... }`.
- `tts.rs` — speech client, same dual shape (`.../audio/speech`), JSON body `{model, voice, input, response_format: "pcm"}` → 24 kHz s16le mono bytes, no decoder needed. Sentence-boundary chunking at the 4096-char input cap (§16). Markdown-to-prose stripping (§13, `prose.rs`) is a text-level pass over the turn's final markdown (`last_assistant_text`): drops fenced blocks / tables / rules, keeps inline markup's text content — no hooks into the rendered transcript needed.
- `playback.rs` — cpal output stream fed from a byte channel; `stop()` handle for §14/§15/§18.
- A small `VoiceController` (one per Claude pane view) owning the state machine Idle → Recording → Transcribing and Speaking, plus the **global single-recording guard** (§10) as a `static` slot.

### Settings (`app/src/settings/agent.rs` + `agent_page.rs`)

- New `define_settings_group!` fields: `agent.voice.stt.{kind,endpoint,model,api_version}`, `agent.voice.tts.{kind,endpoint,model,voice,use_stt_key}`, `agent.voice.auto_send`, presence flags `agent.auth.voice_stt.api_key_set` / `agent.auth.voice_tts.api_key_set`. All `SyncToCloud::Never` (§22). Accessors `voice_stt_config()` / `voice_tts_config()` returning resolved structs (or `None` when unconfigured — drives §2/§12).
- Keychain accounts `agent.api_key.voice_stt` / `agent.api_key.voice_tts` alongside `api_key_storage_key()`; reuse the existing save/remove/presence-sync flow (`agent_page.rs:538–:609`).
- Voice section UI appended in `AgentSettingsWidget::render` (`agent_page.rs:854`) using the existing row/dropdown/key-row helpers; **Test voice** button (§21) calls `tts.rs` + `playback.rs` directly with current (unsaved-field-aware) values.

### Claude pane wiring (`app/src/claude_code_view.rs`)

- New `ClaudeCodeViewAction` variants: `ToggleVoiceRecording`, `CancelVoiceRecording` (Esc path, §6), `ToggleSpeakReplies`.
- Mic + speaker buttons in the `controls_for` left row (`:6521`), mirroring `make_attach`'s Hoverable/ConstrainedBox idiom; recording pulse + m:ss label via the self-rearming timer pattern; button states per §2–§5, §10, §12, §14.
- STT completion marshals back like `agent_suggestions` consumers do (tokio task → notify → insert into `self.input_editor` at cursor, §7; auto-send path reuses the existing `Submit` action only when the pre-record composer was empty).
- TTS hook: the pane already knows turn completion (the result event that finalizes a turn's blocks — same signal 7p used for attention notifications). On completion with speaker on and turn not interrupted (§18), extract prose and hand to `VoiceController`.

### Tradeoffs

- **cpal vs objc2/AVAudioEngine:** cpal keeps capture+playback in one cross-platform pure-Rust dep and avoids hand-rolled ObjC audio-session code; the repo's objc2 precedent (feature 14) was for WebKit where no Rust wrapper existed. Chosen: cpal.
- **PCM TTS vs mp3/wav:** `response_format: "pcm"` removes any decode dependency at the cost of larger transfers; fine for reply-length audio. Chosen: pcm.
- **Single-shot vs streaming STT:** streaming (realtime API / websockets) is a large surface for marginal v1 gain; single-shot matches "record → review → send". Deferred (non-goal).

## Sub-phases

- **17a — providers + settings.** `voice/` module (stt/tts/wav clients, no UI), settings fields, keychain rows, Voice section UI incl. Test voice.
- **17b — talk to the chat.** Capture, mic button, transcript insertion, auto-send, permission/error paths (§1–§11).
- **17c — spoken replies.** Playback, speaker toggle, prose extraction, chunking (§12–§18).

Per the owner's bundling rule (feature 07 precedent): 17a alone has little an end-to-end smoke test can validate beyond the Test-voice button, so **17a+17b ship as one PR**; 17c as a second.

## Testing and validation

- **Unit tests** (`rust-unit-tests` conventions): WAV header bytes for known input; request-builder URL/header/body shapes for both provider kinds (Azure §23 path/api-version/api-key vs bearer); sentence chunking at the 4096 cap (§16); markdown→prose stripping drops fences/tools and keeps prose (§13); resampler length/endpoint sanity.
- **Config resolution tests:** unconfigured → `None` (§2/§12); presence-flag flips on save/remove (§22).
- **Manual smoke test (must pass live before merge, per repo convention):**
  1. Settings → Agent → Voice: enter an Azure Foundry endpoint + `gpt-4o-transcribe` deployment + key; enter/inherit TTS key, pick a voice, press **Test voice** → hear the sample (§21).
  2. In a Claude pane: click mic, say "list the files in this directory", click stop → text appears in composer (§4–§7); Enter sends.
  3. Esc mid-recording cancels (§6); mic with key removed shows the settings notice (§2).
  4. Enable speaker, send a prompt → reply is spoken, code blocks skipped (§13); click speaker mid-speech → silence + toggle off (§14).
  5. Two panes: record in one, mic disabled in the other (§10).
- CI on the fork always ends "cancelled" — not a merge signal; validation is unit tests + the live smoke test.

## Risks and mitigations

- **Azure api-version churn** for gpt-4o-transcribe → user-editable api-version field (§20) rather than hardcoding.
- **TCC denial / no input device** → explicit §3/§11 paths; never panic on stream-build failure.
- **Secrets hygiene** → keys only transit `SecureStorage` reads into request headers; no logging of bodies/headers (§24).
- **cpal main-thread discipline** — build streams off the UI thread; callbacks only fill buffers (echoes the 07 focus-loop lesson: no sync heavy work on the main thread).

## Parallelization

Not proposed: the sub-phases are sequential by dependency (settings → capture/STT → TTS), each touches the same two files (`claude_code_view.rs`, `agent.rs`/`agent_page.rs`), and the fleet already serializes per-feature work.
