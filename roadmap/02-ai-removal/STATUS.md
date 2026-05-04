# 02 — AI removal

**Phase:** impl-pending
**Spec PR:** https://github.com/timomak/twarp/pull/4
**Impl PRs:** 2a — https://github.com/timomak/twarp/pull/6 (merged); 2b — https://github.com/timomak/twarp/pull/7 (merged); 2c — pending

## Scope

Rip out Warp's AI features: agentic mode UI, cloud-agent surfaces, inline AI suggestions, AI command palette, LLM-backed completion, and AI-only telemetry. See README §1.

## Strategy

Warp already supports an "AI disabled" path — onboarding lets the user decline AI, and the app gracefully degrades. Twarp piggybacks on that plumbing instead of cataloguing AI code by hand:

1. **Default to AI-disabled.** Set whatever flag / setting / onboarding-answer upstream uses to gate AI to default off. The onboarding question that lets the user pick "no AI" becomes the default (and may be reduced or removed once the alternative branch is gone).
2. **Remove the enable path.** Strip the code that turns AI on. With no caller, every AI module gated behind it is unreachable.
3. **Delete the dead code.** Iterate with `simplify` until nothing further collapses.

Smaller diff than auditing AI files one-by-one, and upstream cherry-picks touching AI features merge cleanly into the gated-off code path before being re-pruned.

## Sub-phases

- [x] **2a — Locate the gate.** Find the existing "AI disabled" mechanism upstream provides (likely a feature flag, settings key, or onboarding answer). Document where it's checked and what code it bypasses. Output: `roadmap/02-ai-removal/GATE.md`. Single PR, no behavior change.
- [x] **2b — Default AI off + remove the enable path.** Default the gate to disabled; remove the UI/code that flips it on (or strip the onboarding question entirely if it's a binary choice). Behavior change: every install boots in no-AI mode. Diff stays small.
- [ ] **2c — Delete dead AI code.** Sub-split into focused per-module PRs because the original "single PR multi-commit" plan in TECH §2c is too large to review in one shot (~204K lines across `app/src/ai/`, `app/src/ai_assistant/`, `crates/ai/`, `crates/natural_language_detection/`, plus the 2b deferrals `agent_slide.rs` and `ai_page.rs`, plus deep `app/src/workspace/view.rs` integration — 86 ai_assistant refs alone). Each sub-PR is one coherent module deletion that compiles and passes presubmit; `simplify` runs between sub-PRs.
    - [ ] **2c-a — Onboarding agent slide.** Delete `crates/onboarding/src/slides/agent_slide.rs`, prune `slides/mod.rs` re-exports, gut `OnboardingStateModel` of `agent_settings`/`disable_oz`/`AgentDevelopmentSettings`/`AgentAutonomy`/`OnboardingModelInfo` references, drop `OnboardingStep::Agent` variant, remove `agent_slide` field from `agent_onboarding_view`, delete `apply_agent_settings` + autonomy logic in `app/src/settings/onboarding.rs`, prune `AgentSlideUpgradeClicked` telemetry variant, fix the standalone `crates/onboarding/src/bin/main.rs` and `app/src/settings/onboarding_tests.rs`.
    - [ ] **2c-b — `app/src/ai_assistant/`.** Delete the directory, the `mod ai_assistant;` declaration in `lib.rs`, the `ai_assistant::panel::init(ctx)` call, the `ai_assistant_panel: ViewHandle<AIAssistantPanelView>` field on `WorkspaceView` and all 80+ associated touchpoints in `workspace/view.rs` (`should_show_ai_assistant_warm_welcome`, `is_ai_assistant_panel_open`, `workspace:toggle_ai_assistant`, `build_ai_assistant_panel_view`, etc.), the `AskWarpAI` telemetry call sites that referenced this panel, and the `app/src/auth/mod.rs` `REQUEST_LIMIT_INFO_CACHE_KEY` import.
    - [ ] **2c-c — Settings AI page.** Delete `app/src/settings_view/ai_page.rs`, prune all `settings_view/mod.rs` registration sites (`SettingsSection::{AI, WarpAgent, AgentProfiles, AgentMCPServers, Knowledge, ThirdPartyCLIAgents}`, `SettingsAction::AI`, `ai_page_handle`, navigation handlers), delete the `cli_agent_settings_widget_id` consumer in `app/src/ai/blocklist/agent_view/agent_input_footer/` (which is itself slated for 2c-d), `is_ai_disabled_due_to_remote_session_org_policy` in `app/src/settings/ai.rs`.
    - [ ] **2c-d — `app/src/ai/` subtree.** Delete in dependency order (leaves first): `predict/`, `blocklist/`, `ambient_agents/`, `execution_profiles/`, `cloud_agent_config/`, `voice/`, `skills/`, `agent/`, `agent_management/`, remaining subdirs, finally `ai/mod.rs` and the `mod ai;` re-export. May be split across multiple sub-PRs depending on cascading complexity in `workspace/view.rs`, `terminal/`, `settings_view/execution_profile_view.rs`, etc.
    - [ ] **2c-e — `crates/ai/` workspace crate.** Remove the crate from `Cargo.toml` workspace members, drop `ai.workspace = true` from `app/Cargo.toml` and `crates/onboarding/Cargo.toml` (the `ai::LLMId` import), delete the directory.
    - [ ] **2c-f — `crates/natural_language_detection/`.** Same pattern. Audit `crates/input_classifier/` — if its only purpose was AI routing, delete it too in this PR.
- [ ] **2d — Final sweep.** AI-only telemetry events, feature flags whose only consumer was AI, config keys nothing reads.

## Notes

- Run the `simplify` skill on each sub-PR to catch dead code the rip-out leaves behind.
- Cherry-pick conflict cost is **lower** under this strategy than under file-by-file removal: until 2c starts, AI files still exist (just gated off), so upstream AI patches still apply mechanically. Cost spikes once 2c starts physically deleting modules.
- After 2a merges, schedule a recurring upstream-watcher agent (weekly) to surface conflicting upstream commits early.
- The fork inherits Warp's MIT/AGPL split. Removing AI code shouldn't change licensing, but call out anything ambiguous in 2a's gate doc.

## Why this is feature 02 (not last)

The fork's identity is "no AI." Establishing that early matters more than minimizing cherry-pick conflict cost — and the conflict cost is unavoidable regardless of timing.
