# 02 — AI removal

**Phase:** impl-pending
**Spec PR:** https://github.com/timomak/twarp/pull/4
**Impl PRs:** 2a [#6](https://github.com/timomak/twarp/pull/6), 2b [#7](https://github.com/timomak/twarp/pull/7), 2c-a [#9](https://github.com/timomak/twarp/pull/9), 2c-b [#10](https://github.com/timomak/twarp/pull/10), 2c-c [#11](https://github.com/timomak/twarp/pull/11) — all merged. 2c-d in flight (split further into 2c-d.1 … 2c-d.6 — see sub-phase notes).

## Scope

Rip out Warp's AI features: agentic mode UI, cloud-agent surfaces, inline AI suggestions, AI command palette, LLM-backed completion, and AI-only telemetry. See README §1.

## Strategy

Warp already supports an "AI disabled" path — onboarding lets the user decline AI, and the app gracefully degrades. Twarp piggybacks on that plumbing instead of cataloguing AI code by hand:

1. **Default to AI-disabled.** Set whatever flag / setting / onboarding-answer upstream uses to gate AI to default off. The onboarding question that lets the user pick "no AI" becomes the default (and may be reduced or removed once the alternative branch is gone).
2. **Remove the enable path.** Strip the code that turns AI on. With no caller, every AI module gated behind it is unreachable.
3. **Delete the dead code.** Iterate with `simplify` until nothing further collapses.

Smaller diff than auditing AI files one-by-one, and upstream cherry-picks touching AI features merge cleanly into the gated-off code path before being re-pruned.

## Sub-phases

- [x] **2a — Locate the gate.**
- [x] **2b — Default AI off + remove the enable path.**
- [ ] **2c — Delete dead AI code.** Originally one PR; split into focused per-module sub-PRs. After attempting 2c-d as a single PR a sub-agent surfaced ~3000 cargo errors after import-only cleanup and recommended further sub-splitting `app/src/ai/` (391 files) into 2c-d.1 … 2c-d.6.
    - [x] **2c-a — Onboarding agent slide.**
    - [x] **2c-b — `app/src/ai_assistant/`.**
    - [x] **2c-c — Settings AI page.**
    - [ ] **2c-d.1 — `app/src/ai/predict/` + `voice/` + `aws_credentials*`.** Smallest leaves; few consumers (server_api/predict callers).
    - [ ] **2c-d.2 — `app/src/ai/blocklist/inline_action/` + `blocklist/usage/`.** Medium leaves.
    - [ ] **2c-d.3 — `app/src/ai/agent_management/` + `agent_events/` + `cloud_*`.** Medium leaves; consumers in workspace + notifications.
    - [ ] **2c-d.4 — `app/src/ai/blocklist/agent_view/` + `blocklist/block/` + `blocklist/controller/`.** Large; deeply coupled to terminal.
    - [ ] **2c-d.5 — `app/src/ai/agent/` + `ambient_agents/` + `agent_sdk/`.** Large; coupled to CLI runner & terminal.
    - [ ] **2c-d.6 — Remaining `app/src/ai/` modules** (`mcp/`, `llms*`, `skills/`, `document/`, `execution_profiles/`, `facts/`, `onboarding.rs`, `active_agent_views_model*`, `restored_conversations.rs`, `persisted_workspace.rs`, etc.) plus the final `mod ai;` removal in `lib.rs` and the AI-only files outside the tree (`app/src/integration_testing/agent_mode/`, `app/src/terminal/cli_agent_sessions/`, `app/src/search/ai_context_menu/`, `app/src/search/ai_queries/`, `app/src/search/command_search/ai_queries/`, `app/src/terminal/view/ambient_agent/`, `app/src/terminal/view/use_agent_footer/`, `app/src/pane_group/pane/ai_document_pane.rs`, `app/src/terminal/view/load_ai_conversation.rs`, `app/src/server/server_api/ai.rs`, etc. — sub-agent inventory found ~50 such files).
    - [ ] **2c-e — `crates/ai/` workspace crate.**
    - [ ] **2c-f — `crates/natural_language_detection/`.** Audit `crates/input_classifier/` — likely AI-only.
- [ ] **2d — Final sweep.** AI-only telemetry events (`OpenedWarpAI`, `AskWarpAI`, `AIBlocklist`, `AICommandSearch`, `AgentManagementPopup`, `AgentManagementView`, `AIQueryTimeout`, `AICommandSearchOpened`, `InputAICommandSearch`, `InputAskWarpAI`, `WarpAIAction`, `ToggleWarpAI`); `TipAction::WarpAI` / `AiCommandSearch`; `CommandSearchItemAction::{OpenWarpAI, TranslateUsingWarpAI}`; `SettingsSection::WarpAgent`; AI-only `FeatureFlag` variants; `agents` settings group; `WARP.md` AI references.

## Notes

- Run the `simplify` skill on each sub-PR to catch dead code the rip-out leaves behind.
- Cherry-pick conflict cost is **lower** under this strategy than under file-by-file removal: until 2c starts, AI files still exist (just gated off), so upstream AI patches still apply mechanically. Cost spikes once 2c starts physically deleting modules.
- After 2a merges, schedule a recurring upstream-watcher agent (weekly) to surface conflicting upstream commits early.
- The fork inherits Warp's MIT/AGPL split. Removing AI code shouldn't change licensing, but call out anything ambiguous in 2a's gate doc.

## Why this is feature 02 (not last)

The fork's identity is "no AI." Establishing that early matters more than minimizing cherry-pick conflict cost — and the conflict cost is unavoidable regardless of timing.
