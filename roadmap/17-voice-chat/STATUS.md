# 17 — Voice chat in the Claude pane

**Phase:** impl-in-review
**Spec PR:** [#205](https://github.com/timomak/twarp/pull/205)
**Impl PRs:** 17a–17c bundled (this PR — owner asked for end-to-end in one pass, 2026-07-11)

## Scope

Talk to the Claude pane, and have it talk back. Owner-requested 2026-07-11.

- **Mic button** in the Claude pane composer (next to the paperclip): record → transcribe → transcript lands in the composer. STT = `gpt-4o-transcribe` on Azure AI Foundry **or any OpenAI-compatible endpoint**.
- **Speaker toggle** beside it: Claude's replies spoken aloud via OpenAI-voice TTS (`gpt-4o-mini-tts` default), prose only.
- **Voice section on the Agent settings page** (feature 16 infra): provider kind / endpoint / model / voice, API keys in the OS keychain, optional auto-send.

> Owner said "next to the pin icon" — the pane has no pin; interpreted as the composer **paperclip**. Flag in review if a different placement was meant.

## Sub-phases

- [x] **17a — providers + settings.** STT/TTS clients, settings fields + keychain, Voice settings UI incl. Test voice.
- [x] **17b — talk to the chat.** cpal capture, mic button, transcript insertion, permission/error paths.
- [x] **17c — spoken replies.** Playback, speaker toggle, prose extraction + chunking.

All three bundled into one impl PR (owner-directed end-to-end pass, 2026-07-11 — supersedes the spec's two-PR plan).

## Smoke test

See TECH.md "Manual smoke test" — configure Azure Foundry key, dictate a prompt into the composer, hear the reply spoken with the speaker toggle on.
