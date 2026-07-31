# Voice conversation (Codex realtime) — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers.

## Summary

Replace feature 17's read-the-reply-aloud TTS with a real **spoken conversation**. In a Codex pane, a **call button** in the composer opens a realtime speech-to-speech session bound to the pane's Codex thread: you talk, the agent talks back, and it runs tools mid-conversation. A **floating call panel** over the pane carries the live state, level meter, voice picker, mute and End. The composer's **microphone button stays what it is** — push-to-talk dictation that types into the composer — and is unchanged by this feature.

Phase 2 (separate spec) ports the same panel and audio path to Claude panes over a direct Azure AI Foundry `gpt-realtime` socket.

Figma: none provided.

## Why the TTS approach is being removed, not fixed

Feature 17 §12–§18 and §32 spoke the *rendered reply*. A coding agent's reply is markdown, code fences, diffs and tool logs, so `voice/prose.rs` had to delete the code before speaking — the spoken output was lossy by construction, and no provider, voice or prompt change fixes that. Summarizing the reply instead can't begin until the turn ends, which for an agent turn is minutes. A realtime model composes *for the ear* in the first place. Codex reached the same conclusion: it ships no TTS at all.

## Goals / Non-goals

**Goals**

- A spoken, interruptible conversation with the Codex agent, sharing the pane's thread and tools.
- Everything said (both directions) lands in the transcript as normal items — scrollback, selection and copy keep working.
- A calm floating call panel that reports only real state, never optimistic state.
- Failure is visible and recoverable: a failed session shows why and offers Retry.
- Dictation (feature 17 §1–§11, §30–§31) survives untouched.

**Non-goals**

- No wake word, no always-listening mode, no local/on-device models.
- No WebRTC transport this pass (websocket only — Codex owns the upstream socket).
- No Claude-pane voice conversation this pass (phase 2).
- No voice for panes whose provider isn't Codex.

## Behavior

### Removal of TTS

1. The composer's speaker toggle is gone. No pane speaks reply text aloud.
2. The Voice section of Agent settings no longer offers TTS provider, endpoint, model, voice, key or "use STT key" controls, and no longer has a Test voice button that synthesizes speech.
3. Any stored TTS API key is removed from the keychain on first run after upgrade; no orphaned entry is left behind.
4. Dictation is unaffected: the mic button, its settings (STT kind/endpoint/model/api-version/key), auto-send, and live transcription all behave exactly as before.

### Starting and ending a call

5. A **call button** sits in the composer control row, to the right of the mic. It appears only when the pane's provider is Codex; other panes show only the mic.
6. With no realtime key configured, clicking the call button opens Agent settings rather than failing silently. (Realtime requires API-key auth; the Codex CLI's ChatGPT login is not sufficient.)
7. Clicking the call button starts a session and opens the floating call panel. The panel appears immediately in a **Connecting** state; it does not claim to be listening until the session is actually live.
8. The panel reports exactly one of: **Connecting**, **Listening**, **Speaking**, **Muted**, **Failed**. Each reflects a real event from the session, never an assumption about what should have happened by now.
9. **End** in the panel, closing the pane, or quitting stops the session, releases the microphone, and closes the panel. The transcript keeps everything already said.
10. Ending a call never ends the underlying Codex thread — the pane stays usable and typed turns continue to work.
11. Only one realtime session may be live per pane. Starting a call in a second pane does not disturb the first (each pane owns its own session).

### During a call

12. Speech is captured continuously while unmuted and streamed to the session; the agent replies in audio.
13. **Mute** stops sending microphone audio without ending the session. The agent can still finish speaking.
14. Speaking while the agent is speaking (barge-in) stops the agent's audio promptly rather than talking over the user.
15. The agent's speech is transcribed into the pane transcript as it streams, so the conversation is readable after the fact.
16. The user's speech is likewise represented in the transcript, attributed as user input.
17. Tool calls the agent makes during the conversation surface in the transcript with the same cards and approval flow as a typed turn. A pending approval does not silently stall the conversation.
18. The composer remains usable during a call: typing and sending a message injects it into the live conversation rather than queuing a separate turn.
19. A level meter in the panel reflects real input level, so a dead microphone is visibly dead.

### Failure and recovery

20. A session error shows the provider's message in the panel in a **Failed** state, with a **Retry** action that re-establishes the session.
21. A dropped connection is retried automatically a bounded number of times with backoff before falling back to the Failed state; retries are visible, not silent.
22. Retry never loses the transcript, and never requires closing and reopening the pane.
23. Realtime lifecycle transitions and failures are logged, so a silent failure can be diagnosed from logs after the fact.
24. Revoking or changing the API key mid-session surfaces as a Failed state with the provider's reason, not as a hang.

### Panel appearance

25. The panel is a detached floating card over the pane: `surface_1`, one hairline border, `radius::PANEL`, `elevation::PANEL`. The transcript behind it stays visible and scrollable.
26. The panel is draggable and remembers its position within the pane for the duration of the session.
27. Dragging or scrolling the transcript behind the panel is not captured by the panel.
28. The panel's accent is the active tab's colour. No other decorative hue is introduced; state colours appear only where they carry meaning (Failed).
29. The voice picker offers the voices the session actually supports, defaulting to the provider's default (`marin`).
30. All spacing, radius, and type values come from `tokens.rs` per `design/PHILOSOPHY.md`; the panel belongs to the **chrome** surface class.

## Validation

- Toggle every panel state by driving real events: connect, speak, be spoken to, mute, barge in, force an error, Retry, End.
- Confirm §4 by exercising dictation in both a Claude and a Codex pane after the removal.
- Confirm §17 by asking the agent to do something requiring approval mid-call.
- Confirm §24 by revoking the key while a call is live.
- Confirm §27 by scrolling the transcript with the pointer over the panel.
