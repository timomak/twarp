# Feature 15 — Computer Control Overlay Technical Spec

## Context

`PRODUCT.md` defines a macOS-first Claude computer-control mode with self-excluded capture, visible overlay/glow lifecycle, permissions onboarding, an agent loop, and safety defaults. The current codebase already has the low-level "eyes and hands" backbone, but it does not yet have the twarp overlay, self-exclusion proof, permission UX, or Claude computer-use loop.

Relevant current code:

1. `crates/computer_use/src/lib.rs:32` exposes `is_supported_on_current_platform()`, and `crates/computer_use/src/lib.rs:40` exposes `create_actor()`.
2. `crates/computer_use/src/lib.rs:49` defines the `Actor` trait. `perform_actions()` accepts a list of `Action`s plus `Options` and returns an `ActionResult`.
3. `crates/computer_use/src/lib.rs:74` defines supported actions: wait, mouse down/up/move/wheel, type text, key down, and key up.
4. `crates/computer_use/src/lib.rs:126` defines screenshot regions in physical pixels, and `crates/computer_use/src/lib.rs:168` defines `ScreenshotParams` with `max_long_edge_px`, `max_total_px`, and optional region.
5. `crates/computer_use/src/mac/mod.rs:36` executes actions sequentially on macOS and captures a screenshot after the action list when `Options.screenshot_params` is present.
6. `crates/computer_use/src/mac/screenshot.rs:6` currently captures the main display by shelling out to `/usr/sbin/screencapture`; `crates/computer_use/src/mac/screenshot.rs:13` passes `-x -tpng -m`, and `crates/computer_use/src/mac/screenshot.rs:20` maps optional physical-pixel regions to `screencapture -R` point coordinates.
7. `crates/computer_use/src/screenshot_utils.rs:31` handles downscaling and PNG encoding shared by screenshot backends.
8. `crates/computer_use/src/mac/mouse.rs` and `crates/computer_use/src/mac/keyboard.rs` already post CoreGraphics mouse and keyboard events, but there is no visible permission preflight or onboarding around them.
9. `app/Cargo.toml:84` already depends on `computer_use`.
10. `crates/warp_features/src/lib.rs:579` has `AgentModeComputerUse`, and `crates/warp_features/src/lib.rs:582` has `LocalComputerUse`. `crates/warp_features/src/lib.rs:842` enables `LocalComputerUse` for dogfood builds. `app/Cargo.toml:434` and `app/Cargo.toml:704` define matching cargo features.
11. `app/src/bin/oss.rs:23` force-enables a small set of twarp OSS flags. Implementers should decide per sub-phase whether `LocalComputerUse` needs to be included there for `warp-oss` smoke testing, while keeping incomplete work dark until the feature is safe.
12. `crates/claude_code/src/driver.rs:83` defines `SpawnOptions`; `crates/claude_code/src/driver.rs:134` spawns `claude` with stream-json IO; `crates/claude_code/src/driver.rs:269` already has an outgoing image content type accepted by the Claude stream.
13. `app/src/pane_group/pane/claude_code_pane.rs:25` wraps `ClaudeCodeView` as pane content, and `app/src/pane_group/pane/claude_code_pane.rs:110` forwards `ClaudeCodeViewEvent`s to the pane group.
14. `app/src/workspace/view.rs:4194` exposes `Workspace::active_tab_color()`. `app/src/claude_code_view.rs:1121` shows the existing pattern for resolving the active tab color to a `ColorU` with theme fallback behavior nearby.
15. `app/src/workspace/view.rs:608` documents the floating-panel treatment shared by side panels; `app/src/workspace/view.rs:633` currently resolves `floating_panel_surface_fill()` to the theme background, so the glow should use active-tab color APIs rather than assuming this fill returns the tab color.
16. `crates/warpui/src/platform/mac/objc/window.m:115` defines the existing `WarpWindow` AppKit bridge, and `crates/warpui/src/platform/mac/objc/host_view.m:1` demonstrates native AppKit/WebKit view integration beside the Metal host view.

## Proposed Changes

### Overall Architecture

1. Keep `computer_use` as the single low-level input/capture actor. Do not create a parallel mouse, keyboard, or screenshot abstraction in `app`.
2. Add a macOS-only computer-control coordinator in `app` that owns session state: stopped, blocked on permissions, starting, active-confirming, active-auto, stopping, and failed.
3. Expose coordinator commands/events to `ClaudeCodeView` or a sibling model owned by the Claude pane. The Claude session remains the identity anchor; computer control is an optional mode attached to one live Claude session.
4. Add a macOS overlay/glow host beside warpui rather than inside the Metal scene. The overlay and glow are AppKit windows so they can be non-activating, all-Spaces/fullscreen auxiliary, click-through where needed, and configurable for capture exclusion.
5. Gate all user-visible entry points and runtime work behind `FeatureFlag::LocalComputerUse` and, where agent-mode integration is reused, `FeatureFlag::AgentModeComputerUse`. Until 15e safety is complete, do not enable the full entry point by default for stable/release users.
6. Keep the `PRODUCT.md` smoke tests as the manual UX gate. This work cannot be fully proven on the headless fleet node because the highest-risk behavior depends on macOS window server capture, TCC, Spaces, and fullscreen behavior.

### 15a — Self-Excluding Capture Spike

1. Add a small macOS overlay/glow spike host, preferably under a new AppKit bridge module such as `app/src/computer_control/mac_overlay.rs` plus Objective-C helpers near `crates/warpui/src/platform/mac/objc/` if Rust `objc2` coverage is insufficient.
2. Create two windows:
   - A non-activating corner `NSPanel` for the overlay.
   - A border-only, transparent, click-through `NSWindow` for the glow.
3. Set capture-exclusion attributes on both windows. Try `NSWindow.sharingType = NSWindowSharingNone` first because it is the smallest change. The spike must produce an explicit result documenting whether `/usr/sbin/screencapture` honors it on the target macOS version.
4. Add a test/spike capture path that calls the existing `computer_use::create_actor().perform_actions(&[], Options { screenshot_params: Some(...) })` and writes or surfaces the captured PNG for inspection.
5. If `screencapture` does not honor `sharingType`, add a macOS ScreenCaptureKit backend in `crates/computer_use/src/mac/`, selected only for computer-control capture. Use an exclusion filter for the overlay/glow windows, and keep `screenshot_utils::process_screenshot()` as the shared downscale/PNG encoder.
6. Do not proceed to the general agent loop until the spike can show overlay/glow absence in the captured image. If main twarp windows cannot yet be excluded, represent that honestly in the spike result and product UI.

### 15b — Overlay Chrome + Lifecycle

1. Replace the spike entry point with the feature-flagged lifecycle entry point attached to `ClaudeCodeView`.
2. Add coordinator state transitions:
   - `Stopped -> Starting` after a visible user start action.
   - `Starting -> Blocked` when permissions or capture self-exclusion fail.
   - `Starting -> Active` after permissions and capture support pass.
   - `Active -> Stopping -> Stopped` after Stop, pane close, window close, quit, or feature disable.
   - `Active -> Failed` for unrecoverable capture/input/backend errors.
3. Build overlay controls in AppKit or a minimal warpui-hosted surface inside the `NSPanel`. The panel must include Stop, session identity, confirmation mode, and latest action status.
4. Keep the glow window click-through by ignoring mouse events. The overlay panel itself may accept clicks only for its controls.
5. Resolve glow color from the active workspace tab:
   - Reuse `Workspace::active_tab_color()` and the `AnsiColorIdentifier::to_tab_color()` pattern from `ClaudeCodeView::tab_accent()`.
   - Fall back to the theme accent when there is no tab color.
   - Subscribe to tab activation/color changes or refresh color during workspace updates so the glow re-tints live.
6. Ensure teardown is idempotent. Dropping the Claude pane or coordinator must close native windows and stop queued work even if Stop was already pressed.

### 15c — Permissions Onboarding

1. Add macOS permission probes around:
   - Screen Recording: use CoreGraphics preflight/request APIs where available.
   - Accessibility: use AX trust checks and prompt/open System Settings where available.
2. Keep permission probing separate from `computer_use::Actor` action execution so the UI can show blocked states before attempting real capture/input.
3. Model permissions as independent states: granted, missing, requested-restart-needed, denied/unknown. Do not collapse Screen Recording and Accessibility into one generic error.
4. On start, preflight both permissions. If either is missing, transition to `Blocked` and do not create an active agent loop.
5. After the user opens settings or returns to twarp, re-run preflight. If macOS requires restart before a grant takes effect, keep the UI blocked with restart copy.
6. Add Info.plist usage strings only if the macOS API path requires them for this app bundle. Confirm `warp-oss` embedding in `app/src/bin/oss.rs` and any external plist path remain consistent.

### 15d — Agent Loop

1. Add a computer-use tool bridge for Claude sessions. The bridge translates Claude computer-use tool requests into `computer_use::Action` values and sends screenshots back as image content.
2. Reuse the existing stream-json driver in `crates/claude_code/src/driver.rs` where possible. If Claude Code's stream protocol exposes computer-use tool calls differently from ordinary tool events, isolate that parsing/serialization in `crates/claude_code` and keep app UI on typed events.
3. Send the initial screenshot when control starts or when Claude first requests computer context. Subsequent loop iterations should be action result -> screenshot -> Claude response.
4. Use `ScreenshotParams` to limit payload size. Default to main-display capture with a max long edge / max total pixels suitable for latency and cost. Use regions when Claude asks for a focused area or when the UI can safely infer one.
5. Implement strict action validation before execution:
   - Reject coordinates outside captured bounds.
   - Reject invalid screenshot regions.
   - Bound wait durations.
   - Treat unsupported action names or malformed arguments as failed tool calls.
6. In confirm-before-act mode, pause before `Actor::perform_actions()` and surface the proposed action through the overlay. Only approved actions are sent to the actor.
7. After each executed action list, request a screenshot through `Options.screenshot_params`. If screenshot capture fails, stop or block the loop and return a visible error.
8. Keep all actor calls serialized per control session. The actor has mutable mouse state, and overlapping actions would make confirmation, Stop, and key/button release semantics unreliable.

### 15e — Safety + Polish

1. Make Stop a high-priority coordinator command that cancels pending Claude/tool work, prevents future queued actions, and closes capture/control after best-effort key/button release.
2. Track held keyboard and mouse state at the coordinator boundary if `computer_use` does not expose enough state to release on Stop. Prefer adding explicit release helpers to `computer_use` over duplicating low-level event code in `app`.
3. Keep confirm-before-act as the default for every new control session. If an auto-act toggle is added, it must be explicit per active session and visible in the overlay.
4. Add a bounded action log model storing start/stop, screenshots, proposals, approvals/rejections, executed actions, failures, and idle timeout. Render a concise overlay summary with a way to inspect the full active-session log.
5. Add idle auto-stop driven by coordinator activity timestamps. Activity includes screenshots, Claude tool requests, user confirmation decisions, and overlay interactions.
6. Add defensive teardown hooks for pane close, window close, app quit, display reconfiguration failure, feature disable, and actor/backend errors.

## Testing and Validation

1. Unit-test pure state transitions in the coordinator: start blocked by missing permissions, start success, Stop from every active/pending state, failure transitions, idle timeout, and idempotent teardown. These cover `PRODUCT.md` Behavior 1-8, 20-21, 28-31, and 34.
2. Unit-test action validation and Claude-tool translation without posting real input by using `computer_use`'s `test-util`/noop path or a local mock actor. Cover `PRODUCT.md` Behavior 22-27.
3. Unit-test screenshot parameter selection for default full-main-display capture, downscale constraints, and rejected invalid regions. Cover `PRODUCT.md` Behavior 19 and 35.
4. Unit-test glow color resolution with active tab color, no tab color fallback, and tab color changes. Cover `PRODUCT.md` Behavior 14-15.
5. Add headless tests for Claude driver parsing/serialization of computer-use tool events and image responses once the exact stream-json shape is known. Cover `PRODUCT.md` Behavior 22-23 and 33.
6. Add macOS manual smoke validation for self-exclusion because it depends on WindowServer behavior. Use the 15a `PRODUCT.md` smoke test before allowing later phases to proceed.
7. Add macOS manual smoke validation for overlay level, all-Spaces/fullscreen auxiliary behavior, click-through glow, non-activation, and live re-tint. Use the 15b smoke test.
8. Add macOS manual smoke validation for TCC onboarding on a clean user account or reset TCC database. Use the 15c smoke test.
9. Add macOS manual smoke validation for the live Claude loop against a harmless target app such as TextEdit or a local test page. Use the 15d smoke test.
10. Add macOS manual smoke validation for Stop, confirm-before-act default, auto-act indication if present, action log, and idle auto-stop. Use the 15e smoke test.
11. Required fleet checks for implementation PRs remain `cargo build --bin warp-oss`, `cargo fmt -- --check`, and `cargo clippy --workspace -- -D warnings`. The headless node should not launch the GUI or run real-display tests.

## Sub-Phase Breakdown

1. **15a — Self-excluding capture spike.** Own the native overlay/glow spike and capture backend proof. Output is a working proof or a ScreenCaptureKit fallback, plus an honest statement of exactly which twarp windows are excluded.
2. **15b — Overlay chrome + lifecycle.** Own the production overlay/glow lifecycle, feature-flagged entry point, active-tab tint, start/stop transitions, and teardown hooks.
3. **15c — Permissions onboarding.** Own Screen Recording and Accessibility preflight, blocked-state UI, System Settings handoff, restart-needed messaging, and retry behavior.
4. **15d — Agent loop.** Own Claude session tool bridging, action validation, serialized actor execution, screenshot payloads, and transcript/log events for tool progress.
5. **15e — Safety + polish.** Own confirm-before-act default, Stop hardening, held-input release behavior, action log UI, idle auto-stop, and final smoke-test closure.

## Risks and Mitigations

1. **Self-exclusion may not work with `screencapture`.** Gate later phases on 15a. If `NSWindowSharingNone` is not honored, use ScreenCaptureKit exclusion filters for computer-control capture.
2. **TCC behavior is restart-gated and hard to automate.** Keep permission states explicit and validate manually on clean macOS accounts.
3. **Claude stream-json computer-use support may not match existing tool parsing.** Isolate protocol-specific code in `crates/claude_code` and expose typed app events.
4. **Stop during in-flight input can leave held state behind.** Serialize actor calls and add best-effort release on Stop/failure.
5. **Full-screen and Space behavior can regress silently.** Keep it in the required manual smoke test for the primary Mac UX gate.
6. **Cargo feature and runtime flag names already exist.** Prefer reusing `LocalComputerUse`/`AgentModeComputerUse`; avoid adding a duplicate twarp-specific feature flag unless implementation proves a separate gate is necessary.
