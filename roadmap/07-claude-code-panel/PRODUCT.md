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

8a. **(7m) Restore on relaunch.** A Claude Code pane that had a live session at quit or crash **reopens in its tab** on next launch — same tab/split position as any restored terminal or editor pane. twarp persists only the `claude` session id and the pane's cwd (no transcript of its own, §42); restore is a `claude --resume` of that session, so the reopened pane renders the conversation from the `.jsonl` `claude` wrote and continues live only on the next message (it does **not** respawn a process on launch, consistent with §6). A pane whose first turn never completed (no session on disk yet, §6) has nothing to resume and is not restored. Restore failures (the `.jsonl` was deleted out from under us, an unreadable session) degrade to an empty pane rather than dropping the tab.

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

26. **Risk/degradation:** the wire channel `claude` uses to request a permission decision over stdio is undocumented and may change between versions (TECH.md §Risks). **Update (claude 2.1.195):** the interactive channel was found to exist after all — `--permission-prompt-tool stdio` makes `claude` raise a `can_use_tool` control_request that the pane answers Allow/Deny inline (§24 implemented as specified). `AskUserQuestion` rides the **same** `can_use_tool` channel, not a separate `request_user_dialog`: claude raises a `can_use_tool` for the `AskUserQuestion` tool and **blocks the turn** on it. The pane holds that request open and renders the inline question card; submitting answers it with `{behavior:"allow", updatedInput:{…, answers:{"<question>":"<label>"}}}`, so claude reads the picks back as the tool result and continues in the **same turn** (§1). The user can always **Stop** instead (the interrupt releases the parked tool and ends the turn cleanly). If a future `claude` removes or changes this, the pane degrades safely: a missing prompt means tools simply deny after the fact (the pre-2.1.195 behavior), and a held question with no `tool_use_id` falls back to auto-allowing — never a crash or hang. The permission-mode selector (§25) remains the always-available fallback.

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

### Raw-CLI toggle — 7i *(amendment 2026-06-11, owner-requested)*

> Some sessions want the real thing: `claude`'s own TUI has surfaces twarp's pane doesn't render (slash commands, interactive pickers, the statusline). The toggle makes the rendered pane and the raw CLI two views of the **same conversation**, switchable at will.

39. The Claude Code pane shows a **"Raw CLI" control in its header's top-right**. Activating it swaps the rendered chat for the **raw interactive `claude` CLI** running in a real twarp terminal, in the same tab/pane position. The conversation continues: the raw CLI starts with `--resume <session id>`, so the full history is available in it.

40. While in raw mode, a **floating top-right button** persists over the terminal. Clicking it returns to the rendered Claude Code pane in the same position; the pane re-reads the session's on-disk history, so turns produced in raw mode appear in the rendered transcript, and the next message continues the same conversation.

41. Every pane-born session has a stable identity from birth: the pane generates the session id itself (passed via `--session-id` on the first spawn), so the toggle never hits a "no session id yet" window. A pane in the zero state (no first message sent) toggles into a **fresh** raw `claude` carrying that pre-assigned id; toggling back picks up whatever conversation the raw session started, under the same id.

42. Mode handoff never runs two drivers at once: entering raw mode ends the pane's headless process first; returning ends the raw CLI's process. The toggle is **disabled while a rendered turn is streaming** (same rule as the permission-mode selector, §25); leaving raw mode while the raw CLI is mid-turn interrupts it — `claude` persists what it has, and the re-read (§40) renders it.

43. Raw mode is a **real terminal session**: full keyboard interactivity, `claude`'s own TUI behaviors (slash commands, shift+tab mode cycling, its statusline), twarp's standard terminal rendering and theming. twarp chrome stays out of the way except the floating return button (§40). The raw CLI runs **vanilla** `claude --resume <id>` — pane-side settings (the §25 mode pill, launch-flag effort) configure the *headless* session only; the interactive CLI applies its own saved settings, exactly as if launched by hand.

44. If the raw CLI process **exits** (the user types `/exit`, or it crashes), the pane returns to the rendered chat automatically and re-reads history (§40). A failed raw launch (bad resume id) shows the CLI's own error in the terminal; the return button still works and the rendered pane stays usable (§37's never-stuck rule).

### Composer intelligence — 7j *(shipped in #76; documenting §15a–§15b)*

> These two invariants describe behavior that shipped with 7j (PR #76) ahead of a spec; folded in here per the STATUS note so the spec matches the code.

15a. The composer offers **in-line suggestions**. Typing `/` opens a panel above the input listing the session's real slash commands (parsed from the stream-json `init`) merged with twarp built-ins; `@` opens a fuzzy file picker over the cwd (gitignore-aware `ignore` walk, capped and cached, `fuzzy_match` ranking). Enter accepts the highlighted suggestion; click accepts; Esc dismisses; with no panel open Enter sends (§15). The panel never blocks typing.

15b. `@`-mentioning an **image** file attaches it as a preview chip; on send the image ships as a base64 `image` content block alongside the text (`OutgoingMessage { text, images }`). Each chip has a `✕` drop control; an oversized/unreadable image degrades gracefully (the mention stays as text, no crash).

## Phase 2 — fidelity & rich input *(amendment 2026-06-15, owner-directed; sub-phases 7k–7n)*

> Owner-directed addition on top of the merged phase-1 panel ("add them to the roadmap as the next step"), after a triple-checked feasibility pass against `claude` 2.1.175 (receipts in STATUS.md §Phase 2 feasibility). Each sub-phase is spec-first, like 7i. These **extend** the existing invariants — the numbering continues at §45 so nothing above renumbers. Sub-phase 7n carries a documented headless-approval caveat (same wall as §24/§26). All four precede 09-rebrand for the same crate-churn reason as phase 1.

### Token streaming, thinking duration, per-turn metrics — 7k *(amends §12–§14)*

45. Assistant text, extended-thinking content, and tool-call arguments render **incrementally, token-by-token** as `claude` streams them (the pane opts into `--include-partial-messages` and consumes `content_block_delta` — `text_delta` / `thinking_delta` / `input_json_delta`). The whole-message updates §12 permitted as a floor are superseded by true streaming where the stream provides it.

46. The consolidated end-of-turn `assistant` event is treated as the **done-marker only**: text already rendered from deltas is **not re-rendered** (no duplicated paragraphs, no flicker). A turn that emits no partial deltas still renders correctly from the consolidated event.

47. A completed thinking card is labeled with its **measured wall-clock duration** — "Thought for N s", computed from the block's streamed start/stop — replacing the §22 unlabeled "Thinking" fallback. When the stream genuinely carries no timing, the unlabeled fallback stands (§22's honesty rule).

48. On turn completion the pane shows a **per-turn metrics line**: cost in USD, wall-clock duration, and time-to-first-token, sourced from the `result` event (`total_cost_usd` / `duration_ms` / `ttft_ms`). Any field the stream omits is left out — never shown as `0` or placeholder text. This is session-local display only; twarp meters nothing (§34).

### Rich input: paste, drag-drop, file picker — 7l *(amends §15b)*

49. **Pasting an image** into the composer (clipboard image content) attaches it as an image chip via the 7j attachment path (§15b).

50. **Dropping files** onto the pane attaches them: image files become image chips; non-image files become `@`-mention text in the composer. Dropping is accepted anywhere on the pane body.

51. A composer **"＋ attach"** control opens the OS-native file picker; chosen images become chips, non-images become `@`-mentions. All three routes (paste, drop, picker) reuse 7j's single attachment-send path — no parallel send code. An unreadable/oversized selection degrades like §15b (no crash).

### Composer controls: model/effort selector + send-queue — 7m *(amends §25, §27)*

52. The pane exposes a **model selector** (and, where the pinned CLI accepts it, an **effort** selector) changeable mid-session. Changing it detaches the current process and the **next message resumes the same conversation** under the new flag — the §25 permission-mode-pill detach→`--resume` mechanism, reused. The selector is **disabled while a turn is streaming** (same rule as §25). The model list is **self-updating**: once per app run twarp fetches the account's models from the Anthropic Models API (`GET /v1/models`, using `ANTHROPIC_API_KEY` from the environment or `~/.claude/settings.json`'s `env` block) and the dropdown lists them by display name, newest first — so newly launched models appear without a rebuild. Without a key (subscription-only auth — the CLI's OAuth token is not accepted by the Models API) or on any fetch failure, the dropdown falls back to the built-in tier aliases (`fable`, `opus`, `sonnet`, `haiku`), which the CLI resolves to the latest model per tier. A current selection not present in the list keeps its own row.

53. While a turn is streaming the composer **accepts type-ahead**: messages typed and sent during a turn are **queued client-side** and dispatched automatically, in order, when the turn completes — replacing §15's disabled-input behavior for text entry. (The actual send still waits for turn completion; only the input stays live.)

54. Queued-but-unsent messages are **visible and individually removable** before they dispatch. Clearing the queue while a turn streams is a no-op on the live turn.

### Plan-mode rendering — 7n *(amends §24, §26)*

55. An `ExitPlanMode` tool call renders as a **themed plan card** showing the plan's full markdown (carried in the tool input), visually distinct from an ordinary tool card (§16) — a readable plan, not raw JSON.

56. The plan card offers **Approve** and **Keep planning** affordances. Because headless `claude` exposes **no stdio approval channel** for plan exit (`ExitPlanMode`'s tool_result is `is_error:true "Exit plan mode?"` — the same wall as §24/§26), **Approve degrades** to switching the permission mode off `plan` and resuming (the §25 mode-pill path), not a one-click inline accept. The card must never hang the session; "Keep planning" simply leaves the session in plan mode for the next message.

### Background-scripts panel — 7o *(amendment 2026-06-25, owner-requested)*

57. When Claude launches a shell command **in the background** (a `Bash` call with `run_in_background: true` — a dev server, a watcher, a long build), the pane surfaces it in a **per-chat background-scripts panel**: a compact card floated at the pane's top-right, present only once this chat has launched at least one background script. Each chat (each `ClaudeCodeView`) shows only its own scripts; the panel is derived from that pane's transcript and needs no separate state or teardown.

58. Collapsed, the panel is a pill — terminal glyph, "Background scripts", and a count that reads "*N* · *M* running" when any are live. Expanded, it lists one row per script: a **status glyph** (running / finished / stopped / failed-to-start), the **command**, and a state label. A row expands again to its **captured output** (the launch acknowledgement plus any `BashOutput` polls Claude ran, matched by shell id), capped the same way tool-card results are so a chatty watcher can't stall layout.

59. State is derived best-effort from the transcript and is **read-only**: twarp observes the scripts Claude launched but does not itself start, poll, or kill them (those are the model's tool calls; ask Claude in chat to stop a script). A script reads **running** from a successful launch until a `KillShell` for its shell marks it **stopped** or a `BashOutput` status marker marks it **finished**; a launch whose `Bash` call failed reads **failed to start**. When the transcript doesn't make the state explicit, it stays **running** rather than guessing. The panel follows the active theme and adds no Warp/Anthropic chrome (§ cross-cutting).

### Attention signals: desktop notifications + tab status — 7p *(amendment 2026-07-07, owner-requested)*

60. **Desktop notifications.** When a turn finishes (success or error), or the session parks on a user-facing prompt (a permission Allow/Deny card, or an `AskUserQuestion`), **while twarp's window is not the active window**, the pane fires a desktop notification through the same pipeline as the terminal's command-completion notifications (sound setting, permission-failure handling included). The notification's **title is the pane's tab title** (the first-user-message snippet, or "Claude Code"), so it names the tab to come back to; the body is the tail of the assistant's reply, the error text, "Claude needs permission to use *{tool}*", or "Claude is asking you a question". Gating honors Settings → Notifications: the master mode must be **Enabled**; the existing "agent finished responding" toggle governs completion notifications and "needs attention" governs prompt ones. Unsupported platforms no-op. The mode-`Unset` discovery banner is a terminal-view inline surface the chat pane doesn't replicate — `Unset` simply sends nothing (the terminal flow is where the setting gets discovered). A user **Stop** (interrupt) and a bare process exit are silent: the former is the user's own action, the latter is either a mode-change restart or follows an error that already reported.

61. **Tab status dot.** A tab holding a Claude pane reuses the **agent status indicator** (the same dot Warp's Agent Mode used): a working state while a turn streams; **blocked** while the turn is parked on a permission/question (blocked outranks working); and once the turn ends, ✓ finished / ✗ failed that **persists until the user refocuses the pane** (a background tab keeps its dot until revisited; Stop sets none). A split shows the highest-urgency status across its Claude panes. The user's manual tab color and directory tab colors are **untouched** — the indicator dot is the signal, never the tab tint (feature 01 and directory colors own `SelectedTabColor`).

62. **Background-task completions are not desktop-notified.** A §57 background script finishing mid-turn is Claude's to react to, and the turn's own completion fires §60 — notifying both would be noise.

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

### 7i — Raw-CLI toggle

12. In a Claude Code pane with a live conversation, click the header's **Raw CLI** control → the same tab now hosts the real interactive `claude` with the conversation's history (a real terminal: arrow keys, slash commands, shift+tab all work).
13. Send a message in the raw CLI, then click the **floating top-right button** → the rendered pane returns with the raw-mode exchange in the transcript; sending another message continues the same conversation.
14. From a zero-state pane (no message sent), toggle to raw → a fresh interactive `claude` opens; have a short exchange, toggle back → the rendered pane shows it and continues that session.
15. While a rendered turn is streaming, the Raw CLI control is disabled. In raw mode, type `/exit` → the pane returns to the rendered chat automatically.

### Phase 2 — fidelity & rich input (7k–7n)

18. **(7k)** Ask a question that triggers thinking plus a tool call. Confirm: assistant text and thinking render **incrementally** (token-by-token, not a single late blob); the thinking card reads **"Thought for N s"** with a real number; on completion a **per-turn metrics line** shows cost / duration / time-to-first-token. When the turn finalizes, no already-shown text is duplicated or flickers.
19. **(7l)** Paste an image into the composer → a chip appears. Drag an image **and** a `.txt` file onto the pane → the image becomes a chip, the `.txt` becomes an `@`-mention. Click **＋ attach**, pick an image → a chip appears. Send → Claude receives the image(s). An oversized image degrades without crashing.
20. **(7m)** Change the **model** mid-session → the next message continues the **same conversation** under the new model (transcript intact). While a turn streams, type and send two messages → both **queue** (visible and removable) and dispatch in order on completion; the input was never disabled.
21. **(7n)** Ask Claude to enter plan mode and present a plan → an `ExitPlanMode` **plan card** renders the full plan markdown with **Approve** / **Keep planning**. Click **Approve** → the permission mode switches off `plan` and the session resumes (no hang). "Keep planning" leaves it in plan mode.
22. **(7o)** In a chat with no background scripts, no top-right panel shows. Ask Claude to start a long-running command in the background (e.g. "run `sleep 600` in the background", or start a dev server) → a **background-scripts pill** appears top-right reading "1 · 1 running". Expand it → a row with a running glyph + the command; expand the row → its captured output. Open a **second** Claude pane and confirm its panel is independent (empty unless that chat launched its own). Ask Claude to kill the script → the row reads **stopped**. The panel follows a theme toggle and shows no Warp/Anthropic chrome.

23. **(7p)** With notifications enabled in Settings, ask Claude a question, switch to another app before it finishes → a macOS notification arrives titled with the pane's tab title. Ask something that triggers a permission prompt while away → a "needs permission" notification. While a turn runs, the tab shows a working indicator; if it parks on a permission, the indicator reads blocked; after completion in a background tab, a ✓ dot persists until you click back into the pane, then clears. Stop a turn → no notification, no dot. The tab's custom color (right-click → color) is unaffected throughout.

### Cross-cutting

16. Throughout, no Warp/Anthropic sign-in, API-key field, usage meter, or billing UI appears in twarp chrome; auth/limit errors from `claude` show verbatim as copyable error cards. Toggle the theme → all cards/diffs/thinking/status colors follow it (no hard-coded colors).
17. **(Acceptance gate)** Side-by-side with the Claude desktop / Claude Code app, the pane is *visually consistent* — chat turns, docked composer, structured tool cards, +/- tinted diffs with hunk headers, collapsible thinking, a real task list. A pane that shows the same information as plain text rows fails this step even if every other step passes.
