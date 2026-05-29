# Claude Code panel — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers.

## Summary

Bring back the **rendering layer** of Warp's Agent Mode — streaming assistant text, collapsible thinking blocks, structured tool-call cards, inline diff cards, and a task/todo list — as a panel that hosts **only** the local `claude` CLI (Anthropic's Claude Code) running on the user's own machine. The panel spawns `claude` as a subprocess, parses its streaming-JSON output, and renders it in the same visual shape Warp's Agent Mode used. Authentication, model selection, and billing are entirely the `claude` binary's concern; twarp adds no account, no LLM client, no cloud sync.

## Problem

Feature 02 removed Warp's AI **service** (accounts, LLM clients, billing, cloud conversation storage) and, with it, the conversation **rendering** layer. Users who run Claude Code in a terminal still get only raw scrolling text — no structured tool cards, no collapsible thinking, no diff previews, no task list. The polished surface that made those legible already existed in the codebase and was deleted alongside the service. This feature resurrects that surface and points it at the `claude` binary the user already runs, so twarp renders Claude Code sessions the way Warp once rendered its own agent — without re-introducing anything feature 02 deliberately removed.

## Goals / Non-goals

**Goals**

- A left-panel surface that renders a live Claude Code session in Warp's Agent-Mode visual shape.
- Drive sessions through the local `claude` binary; reuse the user's existing Claude Code login.
- Render the event stream legibly: assistant text, thinking blocks, tool-call cards, diff cards, todos.
- Let the user send messages, answer permission prompts, and resume prior `claude` sessions.
- Tolerate an evolving, partially-undocumented JSON interface without crashing.

**Non-goals (these are feature 02's removals and must stay removed)**

- No Warp AI account, no sign-in to any Warp/Anthropic service from within twarp, no API-key entry UI. `claude` owns auth.
- No LLM client, no direct model API calls, no request/response code talking to Anthropic. twarp only ever talks to the local `claude` process.
- No billing UI, no usage metering, no cloud conversation storage, no Warp Drive sync of conversations.
- No twarp-side session database. Sessions are the `.jsonl` files `claude` already writes under `~/.claude/`; twarp reads and resumes those, never duplicates them.
- Not a general agent framework. The panel drives exactly one external program (`claude`); it is not a host for arbitrary agents or MCP orchestration UI.
- No telemetry carrying conversation content. (Existing panel-open/usage counters may be reused; message text, tool inputs, and outputs are never sent anywhere.)

**Constraint the user should know (surfaced, not handled):** Anthropic has announced that **starting 2026-06-15**, `claude -p` / Agent-SDK usage on subscription plans draws from a *separate* monthly Agent-SDK credit rather than interactive limits. The panel does not meter or manage this; it surfaces whatever auth/billing/limit error `claude` itself reports (§55). The feature's value ("uses your existing Claude login") still holds, but the quota it draws is `claude`'s to define.

## Figma

Figma: none provided. Visual reference is Warp's deleted Agent Mode renderer (recoverable from the pre-removal commit; see TECH.md) and the public shape at warp.dev/agents/claude-code. The resurrected cards/diffs/thinking blocks should match that shape; net-new chrome (panel header, session list) follows the existing twarp left-panel conventions used by Project Explorer / Global Search / Shortcuts.

**Visual consistency with Agent Mode is the acceptance gate** for this feature, not a nice-to-have. Each rendered surface — tool cards, diff cards, thinking blocks, the task list, assistant markdown — must look like Warp's Agent Mode (themed cards, +/- tinted diffs with hunk headers, collapsible thinking, a real task list), achieved by *porting/reusing* the deleted renderer per TECH.md's decision matrix. A surface that renders the right data in a primitive/plain-text shape is **not** done. (This is the gate the first implementation attempt, PR #67, failed by rebuilding from GPUI primitives; see TECH.md §Postmortem.)

## Behavior

Invariants are grouped by area; each group is annotated with the sub-phase that delivers it (7b–7h, per STATUS.md). Numbering is continuous so TECH.md and STATUS.md can cite a single invariant. Chord names are macOS; substitute Ctrl for ⌘ on Linux/Windows.

### Panel surface & entry — 7b

1. A new entry appears in the left-panel toolbelt alongside Project Explorer, Global Search, and Custom Shortcuts, with its own icon and label ("Claude Code"). Selecting it shows the Claude Code panel in the left-panel content area.

2. A default keyboard chord (**proposed ⌘⌥K**, conflict-checked and finalized in TECH.md) toggles the panel: if it is not the active left-panel view, the chord opens and focuses it; if it is already the active view, the chord returns focus to the previously focused surface (it does not collapse the whole left panel out from under other tabs). The chord is an `EditableBinding`, remappable through the same keybindings settings surface as features 01/04/06.

3. The panel is resizable along with the rest of the left panel and persists its width across restarts using the existing left-panel width persistence. No new width setting.

4. The panel's working directory is the cwd of the currently focused pane at the moment a session starts (§8). A session, once started, keeps the cwd it was started in even if pane focus later moves to a different directory; the panel header shows that cwd.

5. With no session ever started, the panel shows a **zero state**: a short explanation, a single-line message input, and a "Start session" affordance. The zero state also shows a "Resume…" entry point (§46) when prior sessions exist for the current cwd.

6. If the `claude` binary is not found on `PATH`, the panel shows a clear unavailable state naming the missing binary and a one-line hint to install Claude Code, with no input affordances. This state replaces the zero state and is re-checked each time the panel is opened.

7. Opening the panel never starts a session on its own. A session begins only on an explicit user action (§8). Merely showing the panel spawns no subprocess.

### Session lifecycle — 7c

8. Submitting a message from the zero state (Enter, or clicking "Start session") starts a session: twarp spawns a local `claude` process for the panel's cwd and sends the typed message as the first user turn. The input clears and the conversation view replaces the zero state.

9. A session is a single long-lived `claude` process driven over its lifetime (multi-turn): the user can send further messages into the same session without it being restarted. Exactly one session is active in the panel at a time.

10. While `claude` is producing output, the panel is in a **streaming** state: a visible activity indicator shows the session is working, and a **Stop** affordance is available. Sending a new message is disabled until the current turn completes (the input shows it is waiting).

11. **Stop** interrupts the current turn (equivalent to the user interrupting Claude Code). The session stays alive and ready for the next message; partial output already rendered remains visible, marked as interrupted.

12. When a turn completes, the activity indicator clears, the input re-enables, and focus returns to the input. The conversation auto-scrolls to the newest content unless the user has scrolled up (§22).

13. The user can **end** the session explicitly (a control in the panel header). Ending terminates the `claude` process. The conversation remains visible and read-only until a new session is started or one is resumed; ending does not clear the transcript from view.

14. Closing/hiding the panel (switching to another left-panel tab, or toggling the panel off) does **not** kill an active session: the process keeps running and the conversation is intact when the panel is shown again. Streaming that arrives while hidden is rendered into the transcript so reopening shows the up-to-date state.

15. Quitting twarp terminates any running `claude` subprocess (it is killed on drop, not orphaned). The session is still resumable on next launch because `claude` persisted it (§46).

### Messages & streaming — 7c

16. User messages render as right-aligned (or otherwise visually distinct) user turns in the transcript, in send order.

17. Assistant text renders as it arrives. Text appears incrementally during a turn rather than only at turn end; if incremental token streaming is unavailable, whole-message updates are acceptable but the indicator (§10) must still reflect in-progress work.

18. Assistant text renders as Markdown using the same Markdown treatment the rest of twarp uses (headings, lists, inline code, fenced code blocks with syntax highlighting, links, tables), consistent with feature 03's default-rendered Markdown.

19. Multiple content pieces within one assistant turn (e.g. text, then a tool call, then more text) render in the order `claude` emitted them, interleaved correctly with tool-call cards (§23) and diffs (§30).

20. The transcript is a single chronological stream from session start: user turns, assistant turns, tool cards, thinking blocks, todos, and permission prompts all appear in the order they occurred.

21. Long conversations stay responsive: the transcript scrolls smoothly and rendering does not degrade as the session grows (target: a multi-hundred-message session remains usable).

22. **Auto-scroll discipline:** while new content streams in, the view sticks to the bottom. If the user scrolls up, auto-scroll pauses and a "jump to latest" affordance appears; reaching the bottom (or activating the affordance) re-enables sticking to the bottom.

### Tool-call cards — 7d

23. Each tool invocation `claude` reports renders as a structured **tool card**, not as raw JSON. The card shows the tool name, a concise human-readable summary of the key input, and a status that advances from running → completed/failed.

24. Known tools render with tool-specific summaries: `Read`/`Write`/`Edit`/`MultiEdit`/`NotebookEdit` show the file path; `Bash` shows the command (and its `description` when present); `Grep`/`Glob` show the pattern; `WebFetch`/`WebSearch` show the URL/query; `Task` shows the sub-agent description; `TodoWrite` routes to the task list (§37) rather than a generic card.

25. A tool whose name twarp does not specifically map (including `mcp__<server>__<tool>` MCP tools and any tool added to Claude Code after twarp shipped) renders as a **generic card**: tool name plus a compact, readable rendering of its input. Unknown tools never crash the panel and never render as a blank or broken card.

26. A tool card shows its **result** when `claude` reports one: success collapses to a short summary (e.g. byte/line count, match count, exit status) with an expand affordance for the full output; failure shows the error and is visually marked as failed.

27. Tool output that is large is truncated in the collapsed view with an affordance to expand; expanding never blocks the UI.

28. A `Task` (sub-agent) tool card visually groups the child activity it spawned, so nested tool calls are attributable to their parent rather than appearing at top level.

29. Tool cards are themed (no hard-coded colors) and use the same iconography family as the rest of the panel.

### Diff rendering — 7e

30. `Edit`, `MultiEdit`, and `Write` tool calls render as **diff cards**: the file path as a header and a unified diff of the change (old → new), with +/- line tinting from the active theme.

31. A `Write` to a new file renders as an all-additions diff; a `Write` that replaces an existing file renders as a replacement diff where the prior content is known, otherwise as additions with a "new content" label.

32. `MultiEdit` renders each edit within the one card in order, against the same file path.

33. Diff cards reuse the diff-rendering treatment established by feature 05's Open Changes panel (hunk headers, +/- tinting, monospace, expand/collapse) so diffs look the same wherever twarp shows them. Diff cards are read-only here — no staging/discard affordances (that is feature 05's surface, not this one).

### Thinking & todos — 7f

34. Extended-thinking content renders as a **collapsible thinking card** labeled with a duration when available ("Thought for N seconds"), collapsed by default, expandable to show the thinking text.

35. Thinking cards are visually subordinate to assistant text (dimmer / smaller) so the main answer remains the focus.

36. If a turn has no thinking content, no thinking card appears (no empty placeholder).

37. `TodoWrite` updates render as a **task list**: each item shows its text and status (pending / in-progress / completed) with status-appropriate styling. The list updates in place as `claude` revises it within the session rather than appending a new list each time.

38. The most recent todo state is shown; completed items remain visible (struck through or checked) so the user can see the full plan and its progress.

### Permissions & input — 7g

39. When `claude` requests permission to use a tool (in a permission mode that prompts), the panel renders an in-transcript **permission prompt** showing the tool and the specific action (e.g. the command to run or the file to edit) with **Allow** and **Deny** actions. The session pauses on that turn until the user responds.

40. **Allow** lets the action proceed; **Deny** rejects it and lets the session continue (the assistant sees the denial and may adapt). Responding resumes the streaming state.

41. The panel exposes a **permission mode** selector with the modes `claude` supports (at least: prompt-for-everything default, auto-accept edits, plan/read-only, and a skip-prompts mode), defaulting to the prompting mode so nothing runs unprompted on a fresh session. The selected mode applies to the current and subsequent turns of the session.

42. **Open question / risk:** the wire-level channel `claude` uses to request a permission decision over stdio is not part of its documented public interface and may change between versions (see TECH.md §Risks). If interactive prompts (§39–§40) prove unreliable against the pinned `claude` version, the panel falls back to permission-mode pre-selection (§41) only, and §39–§40 degrade to surfacing denials after the fact rather than blocking prompts. This degradation must not crash or hang the session.

43. The message input supports multi-line composition (Shift+Enter inserts a newline; Enter sends). When empty and not streaming, it shows placeholder guidance. The input is disabled with a clear indication while a turn is streaming (§10) and re-enabled when it completes (§12).

44. Sending an empty or whitespace-only message is a no-op.

45. If the richer command editor (the Ctrl+G-style input used elsewhere in twarp) can host the message input without significant extra work, it is offered; otherwise a plain multi-line input is acceptable for this feature. Either way §43–§44 hold.

### Session list & resume — 7h

46. The panel offers a **session list** for the current cwd, populated from the sessions `claude` itself stores (under `~/.claude/`). Each entry shows enough to identify it (a title or first-message snippet and a relative timestamp). A **New session** action is always present.

47. Selecting a stored session **resumes** it: twarp starts `claude` against that session id and renders its existing history, then the session continues live from there. Resume reads `claude`'s own session store; twarp keeps no parallel copy.

48. Resume is scoped to the cwd a session was created in (that is how `claude` stores them). Sessions created in other directories are not shown in the current cwd's list. The panel does not attempt to resume a session from the wrong cwd.

49. Starting a **New session** while one is active prompts to end the current session first (or switches to it after the active one is ended); two live `claude` processes are never driven from one panel simultaneously.

50. If resuming fails (the session file is missing, corrupt, or `claude` refuses), the panel shows the error and returns to a state from which the user can start a new session or pick a different one — it does not get stuck.

51. **Open question:** whether the session list is in-panel (a switcher within the Claude Code tab) or a separate surface, and whether multiple sessions can be held open as in-panel sub-tabs, is a 7h decision. The invariant that survives either choice: one live process per panel at a time (§9, §49), and the list reflects `claude`'s real on-disk sessions.

### Errors & resilience

52. If `claude` exits unexpectedly mid-turn (crash, killed, non-zero exit), the panel shows an error card explaining the session ended, preserves the transcript so far, and offers to resume (§47) or start anew. It never silently appears idle as if the turn completed.

53. The panel parses `claude`'s output **defensively**: an unrecognized event type, an unknown content-block type, or a missing optional field is tolerated and skipped rather than crashing or stalling the stream. A line that is not valid JSON is dropped (and noted for diagnostics) without breaking the rest of the stream.

54. A partial/streamed JSON event that has not yet fully arrived does not render as broken text; the panel waits for the complete event (or renders the corresponding incremental delta if it is a recognized streaming delta).

55. Auth, rate-limit, and billing errors reported by `claude` (including the post-2026-06-15 subscription-credit behavior) surface **verbatim** to the user as an error card with a copy affordance. twarp neither interprets nor hides them, and never offers an account/billing remedy of its own.

56. If a turn produces no output for an unusually long time, the activity indicator (§10) keeps reflecting "working"; the user can always Stop (§11). The panel does not impose its own turn timeout that would contradict `claude`'s behavior.

57. Two twarp windows or panels each starting their own session operate independently; one panel's session and process are never affected by another's.

### Identity, privacy, theming, accessibility

58. No conversation content (messages, tool inputs, tool outputs, diffs) leaves the machine via twarp. The only process twarp sends conversation data to is the local `claude` binary; the only place sessions are stored is `claude`'s own store.

59. The panel never displays a Warp/Anthropic account, sign-in, API-key field, usage meter, or upgrade prompt. Anything of that nature that `claude` itself prints is surfaced as plain session output, not adopted into twarp chrome.

60. All panel visuals — card backgrounds, status colors, diff tints, thinking-card styling, focus highlight — come from the active theme. No hard-coded colors.

61. The panel is keyboard-navigable: the input is focusable, Enter/Shift+Enter behave per §43, permission prompts (§39) are reachable and answerable from the keyboard, and the session/zero-state actions are reachable by Tab.

62. The panel respects the same accessibility and theming conventions as the existing left-panel tabs; it introduces no surface that is mouse-only.

## Smoke test

Run against a freshly built twarp binary. Most steps require the `claude` CLI installed and logged in (a Claude Code account/subscription). Pin the tested `claude` version per TECH.md. Chord names are macOS.

### 7b — Panel surface (no live session needed)

1. Launch twarp. The left-panel toolbelt shows a **Claude Code** entry alongside Project Explorer / Global Search / Shortcuts. Click it → the Claude Code panel shows.
2. Press ⌘⌥K (or the finalized chord). The panel toggles: opens+focuses when not active, returns focus when already active. Rebind it in keybinding settings; the new chord works and the old one no longer does.
3. Resize the left panel wider, restart twarp, reopen the panel — the width persisted.
4. With no session and no prior sessions for this cwd, the panel shows the zero state (explanation + input + "Start session"), and opening the panel started **no** subprocess (verify: no `claude` process for this cwd in Activity Monitor / `ps`).
5. Temporarily rename/hide the `claude` binary on `PATH` and reopen the panel → it shows the unavailable state naming `claude` with an install hint and no input. Restore `claude`.

### 7c — Session lifecycle & streaming

6. From the zero state, type "list the files here" and press Enter. A `claude` session starts (one process appears), the zero state is replaced by the conversation, your message shows as a user turn, and assistant text streams in with a visible activity indicator and a Stop control.
7. When the turn completes, the indicator clears, the input re-enables and is focused, and the view is scrolled to the latest content.
8. Send a second message in the same session — it does **not** spawn a second process; the same session answers.
9. Send a long-running request and click **Stop** mid-stream — output stops, the turn is marked interrupted, the session stays alive, and you can send another message.
10. Switch to the Project Explorer tab and back — the conversation is intact and any output that arrived while away is present. Quit twarp — the `claude` process is gone (not orphaned).

### 7d — Tool-call cards

11. Ask "read README.md and tell me what this project is." A `Read` tool card renders showing the file path and a result summary; expanding it shows the content. The following assistant text renders after the card, in order.
12. Ask something that runs a shell command. A `Bash` card shows the command; on completion it shows exit status / output summary, expandable.
13. Trigger an MCP or otherwise-unmapped tool (or simulate one per TECH.md) → it renders as a readable generic card, not a crash or blank.

### 7e — Diff rendering

14. Ask Claude to make a small edit to a file. The `Edit` renders as a diff card: file path header, unified diff, themed +/- tints — matching feature 05's diff look. It is read-only (no stage/discard buttons).
15. Ask Claude to create a new file → renders as an all-additions diff card.

### 7f — Thinking & todos

16. Ask a question that triggers extended thinking → a collapsed "Thought for N seconds" card appears, dimmer than the answer; expand/collapse works. A turn with no thinking shows no such card.
17. Ask for a multi-step task that makes Claude use TodoWrite → a task list renders and updates **in place** (items move pending → in-progress → completed) rather than stacking duplicate lists.

### 7g — Permissions & input

18. Start a session in the default (prompting) permission mode and ask Claude to run a command that needs permission → an in-transcript permission prompt shows the action with Allow/Deny; the session pauses. Click **Allow** → it proceeds. Repeat and click **Deny** → it is rejected and the session continues.
19. Switch the permission-mode selector to auto-accept-edits and confirm edits no longer prompt; switch to plan/read-only and confirm it does not modify files.
20. In the input, Shift+Enter inserts a newline; Enter sends; an empty/whitespace message does nothing; the input is disabled while streaming and re-enabled after.
21. (Resilience) Against the pinned `claude` version, confirm interactive prompts work; if they don't, confirm the panel degrades to mode-pre-selection per §42 without hanging.

### 7h — Session list & resume

22. After running a couple of sessions in this cwd, open the session list → prior sessions show with a snippet + timestamp, plus a New session action. Quit and relaunch twarp; the list still shows them (they are `claude`'s own stored sessions).
23. Resume a prior session → its history renders and you can continue it live. Sessions created in a different directory do not appear in this cwd's list.
24. With a live session, choose New session → you are prompted to end the current one first; confirm two `claude` processes are never driven at once.
25. Corrupt or remove a session file and try to resume it → the panel shows the error and lets you start fresh instead of getting stuck.

### Cross-cutting

26. Throughout, no Warp/Anthropic sign-in, API-key field, usage meter, or billing UI ever appears in twarp chrome. Auth/limit errors from `claude` show verbatim as copyable error cards.
27. Toggle the app theme → all cards, diffs, thinking blocks, and status colors follow the theme (no hard-coded colors).
28. **(Acceptance gate)** Side-by-side with `warp.dev/agents/claude-code` (or a memory of Warp's Agent Mode), the rendered panel is *visually consistent* with Agent Mode — structured tool cards (icon + name + summary + status, not raw text), +/- tinted diffs with hunk headers (feature 05's look), collapsible "Thought for N seconds" cards, and a real task list. A panel that shows the same information as plain `Flex`/text rows fails this step even if every other step passes.
