# Claude Code panel — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers.

> **Re-spec (2026-06-02), after the 7b sidebar build (PR #69) was rejected on sight.** The original spec made the Claude Code surface a **left-panel sidebar tab** opened by a toolbelt button / ⌘⌥K. That was the wrong product direction. The corrected design (owner-confirmed):
>
> - **Trigger:** running `claude` in a terminal opens the rich UI — there is no sidebar button or chord for the chat.
> - **Surface:** the chat is a **main-content pane**, like an opened file or a terminal tab — never the sidebar.
> - **Sidebar:** lists **past sessions only**, and only when some exist.
> - **Visual bar:** match the **Claude desktop / Claude Code app** UI.
>
> The *rendering layer* (markdown transcript, tool/diff/thinking/todo cards) and the headless driver are unchanged in intent and carry over from PR #69; only the **entry point and host surface** change. See TECH.md §Re-spec for what's kept vs dropped.

## Summary

When you run `claude` in a twarp terminal, twarp opens a dedicated **Claude Code pane** — a first-class main-content pane (a tab alongside your terminals and editors) — instead of running the raw CLI in the scrollback. The pane drives the local `claude` binary, parses its streaming-JSON output, and renders the session in a polished chat UI modeled on Anthropic's Claude desktop / Claude Code app: streaming assistant markdown, collapsible thinking, structured tool-call cards, inline diff cards, and a task list. A left-sidebar entry lists your **past** Claude Code sessions for the current directory so you can reopen one. Authentication, model selection, and billing are entirely the `claude` binary's concern; twarp adds no account, no LLM client, no cloud sync.

## Problem

Feature 02 removed Warp's AI **service** and, with it, the conversation **rendering** layer. Running Claude Code in a terminal today gives only raw scrolling text — no structured tool cards, no collapsible thinking, no diff previews, no task list. The polished surface that made those legible already existed in the codebase and was deleted alongside the service.

The first attempt to bring it back (PR #67, #69) put it in the **left sidebar** behind a toolbelt button — the wrong home. The natural way to start Claude Code is to type `claude`, and the natural home for a rich, long-running agent session is a **main pane** you can resize, split, and tab through like a file or a terminal — not a cramped sidebar you open separately. This feature resurrects the deleted renderer, points it at the `claude` binary the user already runs, and hosts it where it belongs: a main-content pane entered by the command you already type.

## Goals / Non-goals

**Goals**

- **Terminal-triggered:** typing `claude` in a terminal opens the rich Claude Code pane; nothing else to discover.
- **Main-content pane:** the session is a resizable/splittable/tabbable pane like an editor or terminal — not the sidebar.
- **Polished, Claude-app-like UI:** the session renders in Warp's Agent-Mode shape, styled to match Anthropic's Claude desktop / Claude Code app.
- **Sidebar = history:** a left-panel entry lists past sessions for the cwd (only when some exist) so you can reopen one.
- Drive sessions through the local `claude` binary, reusing the user's existing Claude Code login.
- Render the event stream legibly: assistant markdown, thinking blocks, tool-call cards, diff cards, todos.
- Let the user send messages, answer permission prompts, and resume prior `claude` sessions.
- Tolerate an evolving, partially-undocumented JSON interface without crashing.

**Non-goals (these are feature 02's removals and must stay removed)**

- No Warp AI account, no sign-in to any Warp/Anthropic service from within twarp, no API-key entry UI. `claude` owns auth.
- No LLM client, no direct model API calls, no request/response code talking to Anthropic. twarp only ever talks to the local `claude` process.
- No billing UI, no usage metering, no cloud conversation storage, no Warp Drive sync of conversations.
- No twarp-side session database. Sessions are the `.jsonl` files `claude` already writes under `~/.claude/`; twarp reads and resumes those, never duplicates them.
- Not a general agent framework. The pane drives exactly one external program (`claude`).
- No telemetry carrying conversation content.

**Constraint the user should know (surfaced, not handled):** Anthropic has announced that **starting 2026-06-15**, `claude -p` / Agent-SDK usage on subscription plans draws from a *separate* monthly Agent-SDK credit rather than interactive limits. The pane does not meter or manage this; it surfaces whatever auth/billing/limit error `claude` itself reports (§30). The feature's value ("uses your existing Claude login") still holds, but the quota it draws is `claude`'s to define.

## Figma

Figma: none provided. The **visual target is Anthropic's Claude desktop / Claude Code app** — its chat layout (user/assistant turns, a docked message composer, structured tool/diff/thinking cards, a task list). The underlying renderer is Warp's deleted Agent Mode surface (recoverable from the pre-removal commit; see TECH.md), reused and restyled to match the Claude app. Net-new pane chrome (header, session list) follows the existing twarp pane conventions used by terminal/editor panes.

**Visual consistency with the Claude app is the acceptance gate** for this feature, not a nice-to-have. Each rendered surface — assistant markdown, tool cards, diff cards, thinking blocks, the task list, the composer — must look like a modern Claude chat UI (themed cards, +/- tinted diffs with hunk headers, collapsible thinking, a real task list, a docked input), achieved by *porting/reusing* the deleted renderer per TECH.md's decision matrix and restyling. A surface that renders the right data in a primitive/plain-text shape is **not** done. (This is the gate the first implementation, PR #67/#69, failed by rebuilding from GPUI primitives and by putting the chat in the sidebar; see TECH.md §Re-spec.)

## Load-bearing decisions (surfaced for review)

These shape everything below; flagged here rather than buried so they're easy to veto. Defaults are marked **(provisional)** and re-decidable in review.

1. **Trigger = the `claude` command, intercepted at submit.** Pressing Enter on a terminal command whose program is `claude` opens the pane instead of writing `claude` to the PTY (§1–§4). Other ways to launch the program raw (a script that calls `claude`, `/usr/bin/claude` by full path) are out of scope for interception — only a top-level interactive `claude [args]` is caught (§3).
2. **Pane placement = new tab (provisional).** The pane opens as a new tab in the active tab's pane group and is focused, like opening a file. Split-pane / replace-the-terminal alternatives stay open (§5).
3. **Args/prompt are forwarded (provisional).** `claude "fix the bug"` starts the session with `fix the bug` as the first turn; bare `claude` opens an empty composer (§2).
4. **Sidebar = read-only session list.** The left-panel entry from #69 is repurposed to list past sessions for the cwd; it hosts no chat. It appears only when sessions exist (§35–§38).
5. **No feature flag (always-on).** Acceptable on a personal fork; degrades cleanly when `claude` is absent (§4). (TECH.md §Feature flag.)

## Behavior

Invariants are grouped by area; each group is annotated with the sub-phase that delivers it (7b–7h, re-derived in TECH.md). Chord names are macOS.

### Trigger & pane lifecycle — 7b/7c

1. Running a command whose program is **`claude`** in a terminal (pressing Enter on it) opens a **Claude Code pane** in the main content area and focuses it, instead of executing `claude` in the terminal block. The terminal block does not run the raw CLI; it shows a brief inline note that the session opened in a pane (so the command is not silently swallowed).

2. Arguments are forwarded: `claude <prompt>` starts the session with `<prompt>` as the first user turn; bare `claude` opens the pane with an empty composer and no session yet. (Flags twarp doesn't understand are passed through to `claude` where safe; see TECH.md §Trigger.)

3. Only a **top-level interactive `claude` invocation** typed at the prompt is intercepted. A pipeline, a `claude` inside a subshell/script, a full path, or `claude` used as an argument to another program is **not** intercepted and runs normally. Detection is conservative: when in doubt, run it raw (never break a command the user meant for the shell).

4. The pane's working directory is the cwd of the terminal the command was run in. If the `claude` binary is not found on `PATH`, twarp does not intercept (the shell's own "command not found" stands), or — if twarp opened the pane — the pane shows a clear unavailable state naming the missing binary with an install hint and no composer.

5. The pane is a **first-class main-content pane**: it opens as a new tab (provisional, §load-bearing-2), has a tab title ("Claude Code", with the cwd or first-message snippet), and can be focused, resized, split, moved, and closed exactly like a terminal or editor pane. Closing the pane ends its session (§8).

6. Opening the pane with no prompt starts **no** subprocess until the user sends a first message. Running `claude <prompt>` (with a prompt) starts the session immediately (§2).

7. A session is a single long-lived `claude` process driven over its lifetime (multi-turn): the user sends further messages into the same session without it restarting. Exactly one live session per pane. Multiple Claude Code panes (multiple tabs) run independent sessions.

8. Closing the pane, or quitting twarp, terminates that pane's `claude` subprocess (killed on drop, not orphaned). The session remains resumable on next launch because `claude` persisted it (§36). Switching to another tab does **not** kill the session; output that arrives while the pane is backgrounded is rendered when you return to it.

### Messages & streaming — 7c

9. While `claude` is producing output the pane is in a **streaming** state: a visible activity indicator shows work in progress and a **Stop** affordance is available. Sending a new message is disabled until the turn completes.

10. **Stop** interrupts the current turn (equivalent to interrupting Claude Code). The session stays alive; partial output already rendered remains, marked interrupted.

11. When a turn completes, the indicator clears, the composer re-enables and refocuses, and the view auto-scrolls to the newest content unless the user has scrolled up (§14).

12. User messages render as visually distinct user turns in send order. Assistant text renders as it arrives (incremental where available; whole-message updates acceptable, but the indicator must still reflect in-progress work).

13. Assistant text renders as **Markdown** using twarp's shared Markdown treatment (headings, lists, inline code, fenced code blocks with syntax highlighting, links, tables), consistent with feature 03. Multiple content pieces within one turn (text, tool call, more text) render in emitted order, interleaved with tool cards (§16) and diffs (§20).

14. The transcript is one chronological stream. It stays responsive as the session grows (target: multi-hundred-message sessions remain usable) and uses bottom-stick **auto-scroll**: while content streams the view sticks to the bottom; if the user scrolls up, auto-scroll pauses and a "jump to latest" affordance appears.

15. The composer is **docked at the bottom** of the pane (Claude-app style): multi-line (Shift+Enter newline, Enter sends), placeholder guidance when empty, disabled with a clear indication while streaming (§9) and re-enabled on completion (§11). Empty/whitespace-only messages are a no-op.

### Tool-call cards — 7d

16. Each tool invocation renders as a structured **tool card**, not raw JSON: tool name, a concise human-readable summary of the key input, and a status advancing running → completed/failed.

17. Known tools render tool-specific summaries: `Read`/`Write`/`Edit`/`MultiEdit`/`NotebookEdit` show the file path; `Bash` shows the command (and its `description` when present); `Grep`/`Glob` show the pattern; `WebFetch`/`WebSearch` show the URL/query; `Task` shows the sub-agent description; `TodoWrite` routes to the task list (§22) rather than a generic card.

18. A tool whose name twarp does not specifically map (including `mcp__<server>__<tool>` and any tool added to Claude Code after twarp shipped) renders as a **generic card**: tool name plus a compact, readable rendering of its input. Unknown tools never crash or render blank.

19. A tool card shows its **result** when reported: success collapses to a short summary (byte/line/match count, exit status) with expand for full output; failure shows the error and is marked failed. Large output is truncated in the collapsed view with an expand affordance; expanding never blocks the UI. A `Task` (sub-agent) card visually groups the child activity it spawned. Cards are themed (no hard-coded colors).

### Diff rendering — 7e

20. `Edit`, `MultiEdit`, and `Write` tool calls render as **diff cards**: the file path as a header and a unified diff (old → new) with +/- line tinting from the active theme. A `Write` to a new file renders as an all-additions diff; `MultiEdit` renders each edit in order against the same file.

21. Diff cards reuse the diff-rendering treatment established by feature 05's Open Changes panel (hunk headers, +/- tinting, monospace, expand/collapse) so diffs look the same wherever twarp shows them. Diff cards are **read-only** here — no staging/discard affordances.

### Thinking & todos — 7f

22. Extended-thinking content renders as a **collapsible thinking card** labeled with a duration when available ("Thought for N seconds"), collapsed by default, visually subordinate to assistant text. A turn with no thinking shows no card.

23. `TodoWrite` updates render as a **task list**: each item shows text and status (pending / in-progress / completed) with status-appropriate styling. The list updates **in place** as `claude` revises it within the session rather than appending a new list each time; completed items remain visible (struck through / checked).

### Permissions & input — 7g

24. When `claude` requests permission to use a tool (in a prompting mode), the pane renders an in-transcript **permission prompt** showing the tool and the specific action, with **Allow** and **Deny**. The session pauses on that turn until the user responds; **Allow** proceeds, **Deny** rejects and lets the session continue.

25. The pane exposes a **permission-mode** selector (at least: prompt-for-everything default, auto-accept edits, plan/read-only, skip-prompts), defaulting to the prompting mode so nothing runs unprompted on a fresh session. The selected mode applies to the current and subsequent turns.

26. **Risk/degradation:** the wire channel `claude` uses to request a permission decision over stdio is undocumented and may change between versions (TECH.md §Risks). If interactive prompts (§24) prove unreliable against the pinned `claude` version, the pane falls back to permission-mode pre-selection (§25) only, and §24 degrades to surfacing denials after the fact rather than blocking prompts. This must not crash or hang the session.

27. If the richer command editor (the Ctrl+G-style input used elsewhere in twarp) can host the composer without significant extra work it is offered; otherwise a plain multi-line input is acceptable. Either way §15 holds.

### Errors & resilience — 7c/all

28. If `claude` exits unexpectedly mid-turn (crash, killed, non-zero exit), the pane shows an error card explaining the session ended, preserves the transcript, and offers to resume (§37) or start anew. It never silently appears idle as if the turn completed.

29. The pane parses `claude`'s output **defensively**: an unrecognized event type, an unknown content-block type, or a missing optional field is tolerated and skipped rather than crashing or stalling. A non-JSON line is dropped (and noted for diagnostics) without breaking the stream. A partial/streamed event that hasn't fully arrived does not render as broken text.

30. Auth, rate-limit, and billing errors reported by `claude` (including the post-2026-06-15 subscription-credit behavior) surface **verbatim** as an error card with a copy affordance. twarp neither interprets nor hides them, and never offers an account/billing remedy of its own.

31. If a turn produces no output for a long time, the activity indicator keeps reflecting "working"; the user can always Stop. The pane imposes no turn timeout that would contradict `claude`. Two panes' sessions/processes never affect each other.

### Visual fidelity (acceptance gate) — every sub-phase

32. The pane is **visually consistent with Anthropic's Claude desktop / Claude Code app**: a clean chat layout with distinct user/assistant turns, a docked composer, structured tool cards (icon + name + summary + status), +/- tinted diff cards with hunk headers (feature 05's look), collapsible "Thought for N seconds" cards, and a real task list. A surface that shows the same information as plain text rows **fails** this gate even if every other invariant passes.

33. All pane visuals — card backgrounds, status colors, diff tints, thinking-card styling, focus highlight — come from the active theme. No hard-coded colors.

### Privacy, theming, accessibility — all

34. No conversation content leaves the machine via twarp. The only process twarp sends conversation data to is the local `claude` binary; the only place sessions are stored is `claude`'s own store. The pane never shows a Warp/Anthropic account, sign-in, API-key field, usage meter, or upgrade prompt; anything of that nature `claude` itself prints is surfaced as plain session output, not adopted into twarp chrome. The pane is keyboard-navigable and respects twarp's theming/accessibility conventions; it introduces no mouse-only surface.

### Sidebar: session list & resume — 7h

35. A left-panel entry offers a **session list** for the current cwd, populated from the sessions `claude` itself stores under `~/.claude/`. It hosts **no chat** — only a list. It appears only when prior sessions exist for the cwd (when none exist, it is absent/empty, not an empty chat). Each entry shows enough to identify it (a title or first-message snippet and a relative timestamp).

36. Selecting a stored session **resumes** it: twarp opens a Claude Code pane against that session id (`claude --resume <id>`), renders its existing history, and continues live. Resume reads `claude`'s own store; twarp keeps no parallel copy. Resume is scoped to the cwd a session was created in (that is how `claude` stores them); sessions from other directories are not shown in this cwd's list.

37. If resuming fails (file missing, corrupt, or `claude` refuses), the pane shows the error and returns to a state from which the user can start fresh or pick a different session — it does not get stuck.

38. Quitting and relaunching twarp preserves the list (these are `claude`'s own on-disk sessions). Opening a fresh `claude` from the terminal (§1) and resuming from the list both produce the same kind of Claude Code pane.

## Smoke test

Run against a freshly built twarp binary. Most steps require the `claude` CLI installed and logged in (a Claude Code account/subscription). Pin the tested `claude` version per TECH.md. Chord names are macOS.

### 7b — Pane shell + ported transcript (no live session needed)

1. In a terminal, run `claude` → a **Claude Code pane** opens as a new tab in the main content area and is focused; the terminal block shows a brief "opened in a pane" note and does **not** run raw `claude`. (7b may gate this behind a synthetic/stub session that renders a sample transcript — see TECH.md; the live driver is 7c.)
2. The pane shows the chat layout: a transcript area and a **docked composer** at the bottom. Type a message → it renders as a user turn and a sample assistant reply renders as **themed Markdown** (headings, list, inline code, a fenced code block), not plain text.
3. Resize/split the pane and open a second Claude Code pane in another tab — both behave like normal panes; the tab title reads "Claude Code".
4. Temporarily remove `claude` from `PATH` and run `claude` → twarp does not intercept (shell shows command-not-found), or the pane shows the unavailable state with an install hint. Restore `claude`.

### 7c — Live session & streaming

5. Run `claude list the files here` → a session starts (one `claude` process), the prompt is the first user turn, assistant text streams in with an activity indicator and a Stop control; on completion the composer refocuses and the view is at the latest content.
6. Send a second message → same session answers (no second process). Send a long request and click **Stop** → output stops, turn marked interrupted, session still alive.
7. Switch to another tab and back → transcript intact, output that arrived while away is present. Quit twarp → the `claude` process is gone (not orphaned).

### 7d–7f — Cards, diffs, thinking, todos

8. Ask Claude to read a file / run a shell command / make an edit / create a file / do a multi-step task with TodoWrite, and trigger an MCP/unmapped tool. Confirm: `Read`/`Bash` tool cards with summaries + expandable results; `Edit`/`Write` as themed +/- diff cards with hunk headers (feature 05's look, read-only); a collapsed "Thought for N seconds" card; an in-place task list; and the unmapped tool as a readable generic card (not a crash/blank).

### 7g — Permissions & input

9. In the default (prompting) mode, ask Claude to run a command needing permission → an in-transcript Allow/Deny prompt; Allow proceeds, Deny continues. Switch to auto-accept-edits and plan/read-only and confirm behavior. In the composer, Shift+Enter inserts a newline, Enter sends, empty message is a no-op, input disabled while streaming. If interactive prompts are unreliable against the pinned `claude`, confirm graceful degradation to mode pre-selection (§26) without hanging.

### 7h — Sidebar session list & resume

10. After running a couple of sessions in this cwd, open the left-panel **session list** → prior sessions show with a snippet + timestamp; it contains a list, **no chat**. In a directory with no prior sessions, the list is absent/empty.
11. Click a session → it resumes in a Claude Code pane and continues live. Quit/relaunch twarp → the list still shows them. Corrupt/remove a session file and resume → the pane shows the error and lets you start fresh.

### Cross-cutting

12. Throughout, no Warp/Anthropic sign-in, API-key field, usage meter, or billing UI appears in twarp chrome; auth/limit errors from `claude` show verbatim as copyable error cards. Toggle the theme → all cards/diffs/thinking/status colors follow it (no hard-coded colors).
13. **(Acceptance gate)** Side-by-side with the Claude desktop / Claude Code app, the pane is *visually consistent* — chat turns, docked composer, structured tool cards, +/- tinted diffs with hunk headers, collapsible thinking, a real task list. A pane that shows the same information as plain text rows fails this step even if every other step passes.
