# Feature 15 — Computer control overlay (Claude drives the Mac)

> Pre-spec plan. The full `PRODUCT.md` / `TECH.md` are written via the spec
> workflow before implementation. This file captures the agreed direction so it
> survives across sessions.

## Goal

Let a Claude session in twarp **see and control the user's Mac**, the way
Anthropic's Claude desktop app does:

1. **Floating overlay** — a small always-on-top window pinned to a screen corner
   that follows the user across Spaces and over fullscreen apps.
2. **Screen capture** — Claude can screenshot the desktop as its "eyes."
3. **Self-exclusion** — twarp's own overlay/glow chrome does **not** appear in
   the captures, so Claude never sees its own UI and there's no feedback loop.
4. **Tab-colored glow border** — a full-screen, click-through ring that signals
   "capture/control is live." It is **tinted to match the active tab's color**
   (the per-tab custom color twarp already uses for the sidebar and code-review
   panel via `floating_panel_surface_fill`), not a fixed orange — and it
   re-tints live when the active tab / its color changes.
5. **Control** — Claude moves the mouse, clicks, scrolls, and types to drive any
   app, in a screenshot → action → screenshot agent loop.

## The big head start: `crates/computer_use` already exists

Warp upstream ships a complete, cross-platform **`computer_use`** crate
(`crates/computer_use/`) — the same actor Warp's cloud agents use. It already
provides the entire "eyes + hands" backbone:

- `Actor` trait + `create_actor()` (`src/lib.rs`) with `perform_actions(actions,
  options) -> ActionResult`.
- `Action`: `MouseDown/Up/Move`, `MouseWheel`, `TypeText`, `KeyDown/Up`, `Wait`.
- Screenshot capture with region + downscale params (`ScreenshotParams`,
  `ScreenshotRegion`), returning a `Screenshot { data, width, height, … }`.
- Mac/Windows/Linux(X11+Wayland) implementations already written.
- `app/Cargo.toml` already declares the `computer_use` dependency.

**So "control the Mac" is mostly already built.** This feature is not about
re-implementing input injection or screen capture — it's about (a) the overlay
UX, (b) self-exclusion from capture, (c) wiring the actor into a Claude session
as a computer-use tool loop, and (d) permissions + safety.

### Gap to close in the capture path

The mac screenshot impl shells out to `/usr/sbin/screencapture`
(`src/mac/screenshot.rs`), which has **no window-exclusion option**. Two ways to
get self-exclusion (resolve in the spike):

- **`NSWindow.sharingType = .none`** on the overlay + glow windows. Cheapest —
  if `screencapture` honors it, no capture-path change needed. Must be verified
  on current macOS; behavior has shifted across releases.
- **Move capture to ScreenCaptureKit** with an `SCContentFilter` built using
  `excludingWindows:` / `excludingApplications:`. Authoritative exclusion and
  the modern API, but a new capture backend alongside the CLI one.

## Decided direction (to confirm with owner during spec)

- **Reuse `computer_use`, don't rebuild.** Same lesson as feature 07
  (`[[twarp_07_port_not_rebuild]]`): port/extend what exists rather than writing
  a parallel input/capture stack.
- **Overlay is plain AppKit, beside warpui — not inside it.** The floating panel
  and glow window are `NSWindow`s living next to the Metal drawable, so the
  warpui "Metal-only, no native controls" constraint
  (`[[twarp_07_ui_idioms]]`) doesn't bite. Mirrors the native-view-over-Metal
  pattern in `crates/warpui/src/platform/mac/objc/host_view.m`.
- **Driven by a Claude session**, reusing the feature-07 Claude pane
  plumbing (`[[twarp_07_claude_stream_json]]`,
  `[[twarp_07_pane_and_trigger_wiring]]`). The computer-use loop is just another
  consumer of a Claude session; the new surface is the computer-use tool +
  screenshot images rather than text/diff rendering.
- **mac-first.** `computer_use` is cross-platform, but the overlay chrome,
  self-exclusion, and permission onboarding are macOS-specific in v1.
- **v1 scope: a working corner overlay that can screenshot (self-excluded) and
  execute Claude-issued actions, behind a feature flag, with a hard Stop and a
  confirm-before-act default.**

## Phases (spec-first, each a reviewable PR)

1. **Self-excluding capture spike (highest-risk gate).** Stand up the corner
   `NSPanel` overlay + click-through glow window, then prove twarp's chrome is
   absent from a capture. Try `sharingType = .none` first; fall back to a
   ScreenCaptureKit backend with an exclusion filter if the CLI ignores it.
   Nothing else proceeds until self-exclusion is real.
2. **Overlay chrome + lifecycle.** Non-activating, corner-pinned, all-Spaces /
   fullscreen-auxiliary floating panel; glow window toggled with
   capture/control state and **tinted to the active tab's color** (reuse the
   sidebar / code-review `floating_panel_surface_fill` source), re-tinting live
   on tab/color change; start/stop affordance; feature flag.
3. **Permissions onboarding.** Screen Recording + Accessibility TCC grants
   (each needs a restart on first grant) with clear pre-flight prompts and a
   blocked-state UI. Reuse `computer_use`'s support checks.
4. **Agent loop.** Bridge a Claude session to `computer_use::Actor`: screenshot
   → send to Claude with the computer-use tool → receive action → execute →
   repeat. Region/downscale screenshots for latency/cost.
5. **Safety + polish (partly folded into 2/4).** Visible always-available Stop,
   confirm-before-act mode, action log/transcript, idle auto-stop.

## Key risks

- **Self-exclusion** actually working against `screencapture` / SCK on current
  macOS (phase-1 gate; everything depends on it).
- **Two TCC grants**, each restart-gated — fragile first-run UX.
- **Sandboxing/signing**: broad capture + CGEvent injection are incompatible
  with App Store sandbox; fine for the self-distributed `WarpOss.app`, but
  confirm entitlements don't regress signing.
- **Latency/cost**: full-screen captures every step are large; need region +
  downscale (already supported by `ScreenshotParams`).
- **Safety/blast radius**: an agent driving real mouse/keyboard — Stop and
  confirm-before-act are not optional.
- **Overlay compositing**: corner panel over fullscreen apps and across Spaces
  without stealing focus or showing in captures.

## References

- Existing backbone: `crates/computer_use/` (`src/lib.rs`, `src/mac/{screenshot,
  mouse,keyboard}.rs`).
- Native-view-over-Metal pattern + window plumbing:
  `crates/warpui/src/platform/mac/objc/{host_view,window}.m`.
- Claude session + pane + trigger precedent: feature 07 (Claude Code pane).
