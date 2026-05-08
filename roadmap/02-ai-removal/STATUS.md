# 02 — AI removal

**Phase:** merged
**Spec PR:** https://github.com/timomak/twarp/pull/4
**Impl PRs:** 2a [#6](https://github.com/timomak/twarp/pull/6), 2b [#7](https://github.com/timomak/twarp/pull/7), 2c-a [#9](https://github.com/timomak/twarp/pull/9), 2c-b [#10](https://github.com/timomak/twarp/pull/10), 2c-c [#11](https://github.com/timomak/twarp/pull/11), 2c-d.1 [#12](https://github.com/timomak/twarp/pull/12), 2c-d.2 [#13](https://github.com/timomak/twarp/pull/13), 2c-d.3 [#14](https://github.com/timomak/twarp/pull/14), 2c-d (collapsed .4/.5/.6) [#15](https://github.com/timomak/twarp/pull/15), 2c-e [#16](https://github.com/timomak/twarp/pull/16), 2c-f [#17](https://github.com/timomak/twarp/pull/17), 2d [#18](https://github.com/timomak/twarp/pull/18) — all merged.

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
- [x] **2c — Delete dead AI code.** Originally one PR; split into focused per-module sub-PRs after the single-PR attempt produced ~3000 cargo errors. Final sequence:
    - [x] **2c-a — Onboarding agent slide.**
    - [x] **2c-b — `app/src/ai_assistant/`.**
    - [x] **2c-c — Settings AI page.**
    - [x] **2c-d.1 — `app/src/ai/predict/` + `voice/` + `aws_credentials*`.**
    - [x] **2c-d.2 — `app/src/ai/blocklist/inline_action/` + `blocklist/usage/`.**
    - [x] **2c-d.3 — `app/src/ai/agent_management/` + `agent_events/` + `cloud_*`.**
    - [x] **2c-d (collapsed .4/.5/.6) — Rest of `app/src/ai/` + outside-tree AI files + `pub mod ai;` removal.** Sub-agent attempted 2c-d.4 alone and reported the four blocklist subdirs were architecturally inseparable from sibling AI modules; collapsed into one PR (#15, 742 files, +8052/-250963, 18 sub-agent rounds).
    - [x] **2c-e — `crates/ai/` workspace crate.**
    - [x] **2c-f — `crates/natural_language_detection/` + `input_classifier/` + `voice_input/` deletion.** All three were AI-only; voice_input cargo feature also removed.
- [x] **2d — Final sweep.** AI-only telemetry events, `TipAction::{WarpAI,AiCommandSearch}`, `CommandSearchItemAction::{OpenWarpAI,TranslateUsingWarpAI}`, `SettingsSection::WarpAgent`, AI-only `FeatureFlag` variants, `WARP.md` AI references all removed. The `agents` settings group is kept (its `is_any_ai_enabled` reader hard-codes `false`, and ~100 callers gate themselves on the reader rather than the schema field directly — full schema removal would cascade extensively without changing user-visible behavior).

## Notes

- Run the `simplify` skill on each sub-PR to catch dead code the rip-out leaves behind.
- Cherry-pick conflict cost is **lower** under this strategy than under file-by-file removal: until 2c starts, AI files still exist (just gated off), so upstream AI patches still apply mechanically. Cost spikes once 2c starts physically deleting modules.
- After 2a merges, schedule a recurring upstream-watcher agent (weekly) to surface conflicting upstream commits early.
- The fork inherits Warp's MIT/AGPL split. Removing AI code shouldn't change licensing, but call out anything ambiguous in 2a's gate doc.

## Why this is feature 02 (not last)

The fork's identity is "no AI." Establishing that early matters more than minimizing cherry-pick conflict cost — and the conflict cost is unavoidable regardless of timing.
