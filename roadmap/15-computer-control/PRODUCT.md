# Feature 15 — Computer Control Overlay

## Summary

Computer control lets a Claude session in twarp see and operate the user's Mac through a visible, always-available overlay. The user starts control deliberately, sees a persistent live-capture indicator while it is active, can stop it at any time, and can keep action confirmation enabled so Claude never drives the real mouse or keyboard without consent.

## Goals

1. Provide a macOS-first computer-control mode for Claude sessions in twarp.
2. Make capture/control state obvious through a corner overlay and tab-colored screen glow.
3. Prevent twarp's control chrome from appearing in Claude's screenshots.
4. Keep the high-risk pieces reviewable by shipping in the sub-phases listed in `STATUS.md`.

## Non-goals

1. Windows and Linux overlay UX are out of scope for v1.
2. Remote or cloud-host computer control is out of scope for this feature.
3. Replacing Claude Code pane behavior outside computer-control mode is out of scope.
4. Removing the user's responsibility for macOS Screen Recording or Accessibility grants is out of scope; twarp can guide the user, but macOS owns the final permission prompts.

## Figma

Figma: none provided.

## Behavior

1. When the feature is disabled, no computer-control entry point is visible, no overlay or glow window is created, no screenshot capture is started for computer control, and no input events are sent on Claude's behalf.

2. On unsupported platforms, the computer-control entry point is hidden or disabled with a concise unavailable state. The user must not be led through macOS-only permission prompts on non-macOS platforms.

3. A user starts computer control from an existing Claude session in twarp. Starting control does not discard the Claude session, replace the pane, or create a second unrelated conversation.

4. Starting control requires a visible user action. A Claude response, tool request, or restored session must not silently start capture or input control.

5. If required permissions are missing, starting control opens a blocked state instead of beginning capture. The blocked state clearly names the missing permission or permissions: Screen Recording for screenshots, Accessibility for mouse and keyboard control.

6. If both permissions are missing, the blocked state presents both requirements. Granting one permission while the other remains missing keeps control blocked and updates the state to show only the remaining blocker.

7. Permission onboarding gives the user an action that opens the relevant macOS privacy pane when possible. If macOS requires twarp to restart before a newly granted permission is usable, the UI says so and does not pretend control can start immediately.

8. Declining or ignoring a permission prompt leaves control stopped. The user can retry from the same Claude session after changing permissions.

9. Once control starts, a small overlay panel appears pinned to a screen corner. The panel remains visible above normal windows, across Spaces, and over fullscreen apps where macOS allows auxiliary panels.

10. The overlay panel does not activate twarp, steal keyboard focus, or move focus away from the app the user is controlling. Opening menus or controls inside the overlay is allowed to receive focus only for the duration of that direct user interaction.

11. The overlay panel shows at minimum: that Claude computer control is live, the associated Claude session, a Stop control, the current confirmation mode, and the most recent or pending action status.

12. The overlay panel is compact enough not to obscure a meaningful portion of the desktop. It can be repositioned between corners if its current corner blocks the target app. The chosen corner persists for the active control session.

13. While capture or control is live, a click-through glow border is visible around the captured display. It is an indicator only: clicks, drags, scrolls, and keyboard input continue to reach the user's active app unless the user directly interacts with the overlay panel.

14. The glow border is tinted to the active tab's resolved color. If the active tab has no custom color, the glow falls back to twarp's normal accent color.

15. If the user switches tabs or changes the active tab's color while control is live, the glow re-tints without restarting the control session.

16. The overlay panel and glow border must not appear in screenshots sent to Claude. Claude should see the desktop and target applications, not twarp's computer-control chrome.

17. twarp's main app chrome should be excluded from screenshots where possible. If a macOS capture backend can only reliably exclude the overlay/glow in the current sub-phase, the UI must not claim that all twarp windows are hidden until that is true.

18. If twarp cannot prove self-exclusion for the active capture backend, computer control does not progress beyond the spike/blocked state. The user must not be allowed to run an agent loop that can see its own overlay or glow.

19. Screenshots are captured as Claude's view of the user's Mac. By default, the screenshot represents the controllable main display; region-limited or downscaled captures may be used to reduce latency and cost as long as Claude still receives enough context to perform the requested task.

20. Capture failures stop the agent loop and keep the overlay visible in an error state until the user stops or retries. twarp does not keep sending actions when it no longer has a fresh screenshot.

21. Accessibility/input failures stop action execution and show the failure in the overlay/action log. A failed click, key press, scroll, or type action must not be silently treated as successful.

22. Claude acts in a screenshot -> proposed action -> execute or confirm -> screenshot loop. Each action is based on the latest available screenshot, and a new screenshot is taken after executed actions unless the user stops the session.

23. Supported action types are mouse move, mouse down, mouse up, click-like combinations, scroll, text typing, key down, key up, and waits. Unsupported or malformed actions are rejected and shown as failed actions, not guessed.

24. The default safety mode is confirm-before-act. In this mode, Claude can propose the next action, but twarp waits for the user to approve before sending real mouse or keyboard events.

25. The confirmation prompt describes the proposed action in human-readable terms, including target coordinates or text when applicable. For typed text, the prompt shows the exact text unless it is too long, in which case it shows a safe preview and the full text is available before approval.

26. The user can approve a single proposed action, reject it, stop the session, or switch to an auto-act mode if that mode is exposed. Rejecting an action returns control to Claude as a rejected/failed action and does not execute it.

27. If auto-act mode is available, enabling it requires an explicit user action while a control session is visible. Auto-act mode must be visibly indicated in the overlay for as long as it is active.

28. Stop is always available while control is live, including while Claude is thinking, while a screenshot is pending, while a confirmation prompt is open, and while an action is being executed.

29. Stopping control immediately prevents new screenshots from being sent to Claude and prevents queued future actions from executing. If a key or mouse button is held down by an in-flight action, stopping attempts to release it before the session finishes stopping.

30. Closing the Claude pane, closing the window that owns the session, quitting twarp, or disabling the feature stops computer control and removes overlay/glow chrome.

31. Computer control has at most one active control session per app instance in v1. If the user tries to start control from another Claude session while one is active, twarp focuses or identifies the active session instead of starting a second controller.

32. The overlay action log shows a concise ordered history of screenshots, proposed actions, approvals/rejections, executed actions, failures, and stop events for the active session. The log is visible or reachable from the overlay without exposing raw internal protocol details.

33. The Claude transcript should make clear when computer control starts, when it is stopped, when an action is rejected, and when a permission/capture/input failure blocks progress.

34. If no action occurs for an idle timeout, twarp stops computer control automatically and reports that it stopped due to idleness. User interaction with the confirmation prompt or overlay counts as activity.

35. If the display configuration changes while control is live, twarp refreshes the captured bounds and keeps the glow aligned with the captured display. If it cannot safely refresh, it stops control with an error instead of capturing the wrong region.

36. If the active app, Space, or fullscreen state changes while control is live, the overlay remains visible and the next screenshot reflects the user's current desktop state.

37. The feature does not hide or bypass macOS privacy indicators. If macOS shows its own capture indicator, twarp leaves it intact.

38. Errors are recoverable where possible. A retry from the overlay or Claude pane should re-check permissions and capture support rather than requiring the user to recreate the Claude session.

39. The UI copy must avoid implying that Claude has unrestricted or invisible control. While control is active, the user should always be able to tell that twarp is capturing the screen and may send input events.

40. The shipped v1 is acceptable only when Stop, confirm-before-act default, self-excluded overlay/glow capture, and permission-blocked states are present. These safety behaviors are required, not polish.

## Smoke test

### 15a — Self-Excluding Capture Spike

1. Build and run a macOS twarp binary with the computer-control feature flag enabled.
2. Start the self-excluding capture spike from a Claude session or the temporary spike entry point.
3. Verify a corner overlay panel and a full-screen glow border appear.
4. Trigger a screenshot capture while the overlay and glow are visible.
5. Inspect the captured image and verify the overlay panel and glow border are absent.
6. Place a recognizable twarp window under the capture area and note whether it is excluded; if it is still visible, verify the spike UI/report does not claim full twarp chrome exclusion.
7. Disable the exclusion mechanism or run on an unsupported capture backend and verify computer control stays blocked rather than allowing an agent loop.

### 15b — Overlay Chrome + Lifecycle

1. Enable the feature flag and open a Claude session in a tab with a custom tab color.
2. Start computer control and verify the overlay appears in a screen corner without moving keyboard focus away from the current app.
3. Verify the glow border is tinted to the active tab color.
4. Change the active tab color while control is live and verify the glow re-tints without restarting control.
5. Switch Spaces or enter a fullscreen app and verify the overlay remains available and the glow remains aligned to the captured display.
6. Click Stop and verify the overlay and glow disappear and no further screenshots or actions are attempted.

### 15c — Permissions Onboarding

1. Run on a macOS account where twarp lacks Screen Recording and Accessibility permissions.
2. Attempt to start computer control and verify capture/control does not start.
3. Verify the blocked state names both missing permissions and offers a path to the relevant macOS privacy settings.
4. Grant only Screen Recording, restart if macOS requires it, and verify the blocked state now names only Accessibility.
5. Grant Accessibility, restart if macOS requires it, and verify computer control can start.
6. Revoke either permission while testing and verify the next start or action returns to a blocked/error state instead of silently failing.

### 15d — Agent Loop

1. Start computer control from a Claude session with permissions granted.
2. Ask Claude to perform a simple visible action in another app, such as moving the cursor to a specific button or typing short text into a test document.
3. Verify Claude receives a screenshot, proposes an action, and twarp shows the action for confirmation before execution.
4. Approve the action and verify the real mouse/keyboard action occurs and a fresh screenshot is sent afterward.
5. Reject a subsequent proposed action and verify it is not executed and the transcript/log records the rejection.
6. Trigger a capture or input failure and verify the loop stops or blocks with a visible error.

### 15e — Safety + Polish

1. Start computer control and verify Stop is visible while Claude is thinking, while a confirmation prompt is open, and while an action is pending.
2. Click Stop during each of those states and verify no queued later action executes.
3. Verify confirm-before-act is the default for a new control session.
4. If auto-act mode is available, enable it and verify the overlay visibly indicates auto-act until it is disabled or stopped.
5. Leave a control session idle past the configured timeout and verify it stops automatically with an idle-stop message.
6. Open the action log and verify it lists start, screenshots, proposed actions, approvals/rejections, executed actions, failures, and stop events in order.
