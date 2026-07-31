# Voice conversation (Codex realtime) — TECH

Companion to [PRODUCT.md](PRODUCT.md); §N references its Behavior invariants.

## Verified protocol facts

Everything in this section was established by driving the installed `codex-cli 0.144.5` binary (`/opt/homebrew/bin/codex`) on 2026-07-30, not inferred from docs. Implementation should not re-derive it.

**Two gates, both mandatory:**

1. Spawn with `-c features.realtime_conversation=true`.
2. `initialize` must send `capabilities: { experimentalApi: true }` — otherwise every realtime method returns `-32600 … requires experimentalApi capability`.

**Auth:** realtime requires **API-key auth**. With only the Codex CLI's ChatGPT login present, `thread/realtime/start` succeeds but immediately emits `thread/realtime/error` with `realtime conversation requires API key auth`. Setting `OPENAI_API_KEY` in the app-server's environment clears it.

**Trap that cost a debugging cycle:** `thread/start` returns the thread id at **`result.thread.id`**, not `result.threadId`. Passing a null `threadId` surfaces as the unhelpful `-32600 Invalid request: invalid type: null, expected a string` — with no hint which field is at fault. Encode this in the response type so it can't recur.

**Methods** (v2 protocol — the one `crates/claude_code/src/codex/protocol.rs` already speaks):

| Method | Notes |
|---|---|
| `thread/realtime/start` | Requires `threadId` + `outputModality` (`text`\|`audio`). Optional: `transport`, `voice`, `model`, `version` (`v1`\|`v2`), `prompt`, `includeStartupContext`, `realtimeSessionId`, and the handoff controls below. Returns `{}`. |
| `thread/realtime/appendSpeech` | **The turn trigger.** `appendSpeech(text)` drove a full spoken turn: first audio **1.98s**, 264KB PCM, 26 `transcript/delta` + `transcript/done`. The model *responds conversationally* to the text — it does not read it back. Empty text is a no-op. |
| `thread/realtime/appendText` | Adds a conversation item and produces **no response**. Context injection, not a turn. |
| `thread/realtime/appendAudio` | `audio: { data (base64), sampleRate, numChannels, samplesPerChannel?, itemId? }`. **Codex resamples** — we send the device's native rate, no resampler needed. |
| `thread/realtime/stop` | Ends the session (§9). |
| `thread/realtime/listVoices` | Returned live: v2 = `alloy ash ballad coral echo sage shimmer verse marin cedar`, `defaultV2: marin`; v1 = `juniper maple spruce ember vale breeze arbor sol cove`, `defaultV1: cove`. Drives §29. |

**Notifications:** `thread/realtime/started`, `itemAdded`, `transcript/delta`, `transcript/done`, `outputAudio/delta`, `sdp`, `error`, `closed`.

**Transport:** `{"type":"websocket"}` works and is what we use. The `webrtc` variant requires a client-generated SDP offer and exists for clients that terminate media themselves — not needed here, and `sdp` notifications can be ignored (§ non-goals).

**Handoff controls on `start`** — Codex's own bridge between the *coding agent* and the *voice conversation*: `codexResponsesAsItems` (send agent responses as realtime conversation items), `codexResponseItemPrefix`, `codexResponseHandoffPrefix`, `clientManagedHandoffs`, `flushTranscriptTailOnSessionEnd`. We start with `codexResponsesAsItems: true`.

## Context in our codebase

- **Codex protocol layer.** `crates/claude_code/src/codex/protocol.rs` defines requests via a `method:` struct pattern and currently covers only `initialize`, `thread/start`, `thread/resume`, `turn/start`, `turn/interrupt`. `codex/mod.rs` + `driver.rs` translate Codex events into the shared `TranscriptEvent` seam (`crates/claude_code/src/lib.rs:171`).
- **Composer controls.** `render_input` in `app/src/claude_code_view.rs`; the footer control row holds the permission control, paperclip, and (until 25a) the mic and speaker. The call button (§5) joins this row. Plain `ClaudeCodeViewAction` variants work here — the `PaneHeaderAction` indirection is only for header buttons.
- **Audio.** `app/src/voice/capture.rs` (cpal input) and `app/src/voice/playback.rs` (cpal output, `play`/`append`/`stop`/`is_active`/`position_secs`) both survive 25a and are reused directly. `playback.rs` must stop assuming a fixed 24 kHz: `TTS_SAMPLE_RATE` leaves with `tts.rs`, and each `outputAudio/delta` chunk declares its own rate.
- **Floating card anatomy to copy.** The pane's header menu (`claude_code_view.rs` ~`:7045–7069`): `Container` + `surface_1` + `Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline())` + `radius::PANEL`, wrapped in `Dismiss`, with a `DropShadow` from the elevation token and **both** min and max width — a positioned overlay child is measured against the whole pane's constraints and will otherwise stretch to full width.
- **Tab accent.** `crate::workspace::view::floating_panel_surface_fill(app)` (used at `claude_code_view.rs:5330`) for §28.
- **Retry precedent.** Computer control's four-state button (`claude_code_view.rs` ~`:5455–5511`): live / blocked / failed → "Retry control", with the idle state deliberately inert. This is the model for §20.
- **Keychain.** `twarpui_extras::secure_storage` via the feature-16 pattern; read with the existing `voice_api_key`-style helper and the Agent settings account key (§6).

## Proposed changes

### 25a — remove TTS

Delete `app/src/voice/tts.rs` and `app/src/voice/prose.rs`; the speaker button, `ToggleSpeakReplies`, `speak_replies`, all `voice_tts_*` and karaoke state and `active_karaoke` in `claude_code_view.rs` (~69 references); the six `voice_tts_*` fields in `app/src/settings/agent.rs`; the TTS rows, dropdowns, key editors and `TestVoice` in `app/src/settings_view/agent_page.rs`. Add a one-shot removal of the `agent.api_key.voice_tts` keychain entry (§3). Keep `capture.rs`, `stt.rs`, `wav.rs`, `playback.rs`, and the STT half of `config.rs`. Mark feature 17 §12–§18 and §32 removed in its PRODUCT.md with a pointer here.

### 25b — protocol + audio

Extend `codex/protocol.rs` with the `experimentalApi` capability, the five realtime requests, and typed notifications; add `-c features.realtime_conversation=true` and `OPENAI_API_KEY` to the spawn. Fix the `result.thread.id` shape in the `thread/start` response type.

Route in `driver.rs`: `transcript/delta` / `transcript/done` / `itemAdded` → transcript items (§15, §16); `error` / `closed` → session state (§20). `outputAudio/delta` is audio, not transcript content — it needs a sibling channel to the view rather than being forced through `TranscriptEvent`.

Audio: `capture.rs` → base64 → `appendAudio` with the device's real `sampleRate`/`numChannels`; `outputAudio/delta` → `playback.rs` using each chunk's declared rate. Barge-in (§14) reuses the generation-counter discipline the old TTS path used for latest-reply-wins. Composer send during a call maps to `appendSpeech` (§18) — which is also how this is smoke-tested without a microphone.

### 25c — the panel

Per §25–§30, following the header-menu anatomy above. State line driven strictly by `started` / `outputAudio` / `closed` / `error` (§8) — no optimistic "Listening". Level meter from `capture.rs`; a self-rearming `notify()` timer is required because `repaint_after` never re-runs `render()`. Voice picker from `listVoices`. `SavePosition` for drag (§26), with `with_propagate_drag` / `with_propagate_mousewheel_if_not_handled(true)` so the transcript beneath keeps its gestures (§27).

### 25d — retry and logging

Failed state + Retry per the computer-control pattern (§20), bounded reconnect with backoff (§21), and lifecycle logging (§23). Voice code logs nothing today — `grep -i tts ~/Library/Logs/twarp-oss.log` returns zero hits, which is exactly why the original TTS misconfiguration could not be diagnosed from logs.

## Testing

- A checked-in protocol probe (adapted from the session's scripts) that drives `initialize` → `thread/start` → `realtime/start` → `appendSpeech` and asserts audio + transcript arrive. This runs without the GUI and would have caught both the `experimentalApi` gate and the `result.thread.id` trap.
- Unit coverage for the notification → `TranscriptEvent` mapping and for per-chunk sample-rate handling in playback.
- Manual: the §Validation list in PRODUCT.md.
- `./script/presubmit` can't run fully on this Mac (clang-format off PATH, wgslfmt/nextest missing); `cargo test` gives false failures versus nextest.
