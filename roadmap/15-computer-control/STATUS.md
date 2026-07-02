# 15 — Computer control overlay (Claude drives the Mac)

**Phase:** impl-pending
**Spec PR:** —
**Impl PRs:** —

## Scope

Let a Claude session in twarp see and control the user's Mac, like Anthropic's
Claude desktop app: an always-on-top corner overlay, full-screen screenshots as
Claude's "eyes," twarp's own chrome excluded from those screenshots, a glow
border signaling capture is live (tinted to the active tab's color, not a fixed
orange), and mouse/keyboard control in a screenshot → action → screenshot agent
loop.

See [PLAN.md](PLAN.md) for the full pre-spec plan, the capture-path gap, and risks.

## Why this slot

Owner-requested. Builds directly on the just-shipped Claude pane (feature 07) —
the agent loop is another consumer of a Claude session. Sequenced after the
built-in browser (14) in the active queue; placement vs. the rebrand (09) to be
decided when it goes active. Touches a new overlay surface plus the existing
`computer_use` crate and `claude_code*` crates the rebrand hasn't renamed yet, so
low cherry-pick risk.

## Sub-phases (from PLAN.md)

- [x] **15a — Self-excluding capture spike.** Corner `NSPanel` overlay + click-through glow window; prove twarp's chrome is absent from a capture (`sharingType = .none`, else a ScreenCaptureKit exclusion-filter backend). **Highest-risk gate.**
- [x] **15b — Overlay chrome + lifecycle.** Non-activating, corner-pinned, all-Spaces/fullscreen-auxiliary panel; glow toggled with state and tinted to the active tab's color (reuse `floating_panel_surface_fill`), re-tinting on tab/color change; start/stop; feature flag.
- [x] **15c — Permissions onboarding.** Screen Recording + Accessibility TCC grants (restart-gated) with pre-flight prompts and blocked-state UI.
- [x] **15d — Agent loop.** Bridge a Claude session to `computer_use::Actor`: screenshot → Claude computer-use tool → action → execute → repeat; region/downscale captures.
- [x] **15e — Safety + polish.** Always-available Stop, confirm-before-act default, action log, idle auto-stop. (Partly folded into 15b/15d.)

## What's already built

- **Full cross-platform input + capture backbone:** `crates/computer_use/`
  (`Actor` trait, `Action` enum, `ScreenshotParams`/region/downscale; mac impl in
  `src/mac/{screenshot,mouse,keyboard}.rs`). `app/Cargo.toml` already depends on it.
- Native-view-over-Metal + window plumbing: `crates/warpui/src/platform/mac/objc/{host_view,window}.m`.
- `objc2` / `objc2-app-kit` / `objc2-core-graphics` / `core-graphics` deps in the workspace.
- Claude session + pane + terminal-trigger precedent: feature 07.

## Notes

- **Capture-path gap:** the mac screenshot impl shells out to `/usr/sbin/screencapture`, which has no window-exclusion flag. Self-exclusion needs `NSWindow.sharingType = .none` (verify it's honored) or a ScreenCaptureKit backend with `excludingWindows:`. Resolved in 15a.
- **Two restart-gated TCC grants** (Screen Recording + Accessibility) — onboarding UX matters.
- Broad capture + CGEvent injection rule out App Store sandbox; fine for `WarpOss.app`, but confirm signing/entitlements don't regress.
- High blast radius — Stop + confirm-before-act are required, not optional.
