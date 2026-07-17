# Multi-provider agent pane (Codex backend) — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers. Sub-phase tags (18a–18f) are defined in TECH.md and mirrored in STATUS.md.

## Summary

The agent pane (feature 07) becomes provider-generic: one pane, one timeline, one composer, one approval surface — with the provider (Claude / Codex) as an attribute of the session, the way model already is. Typing `codex` at a shell prompt opens the same pane driven by the OpenAI Codex CLI, exactly as `claude` does today. Feature 16's existing `CLIAgent::Codex` settings entry lights up. Claude behavior does not change at all.

## Goals / Non-goals

**Goals**

- A runtime provider abstraction behind the pane's existing normalized timeline, so a provider is an adapter — not a fork of the view.
- Codex as the second first-class provider: sessions, streaming, tool/command/edit cards, approvals, interrupt, resume-after-restart, fork, model+effort selection.
- Capability-driven UI: controls render from provider capabilities, never from `if provider == claude` branches in views.
- One trust vocabulary (the **Access** pill) mapped honestly onto each provider's native permission model.
- Feature-16 integration: Codex selectable and enabled on the Agent settings page with live auth/install probes; per-action matrix unchanged in shape.

**Non-goals**

- **No Gemini execution** — its entry stays disabled ("coming soon"); the adapter seam must make it cheap later.
- **No change to the suggestion generators** (16e/16f/16g/16h stay Claude-only; their matrix rows keep listing Codex as disabled).
- **No re-implementation of vendor auth** — login runs the vendor CLI's own flow; twarp detects the result.
- **No cost/billing math for Codex** — it reports tokens and plan quota, not dollars; twarp shows what the provider reports.
- **No "second opinion" features this phase** (asking the *other* provider to review a session's work stays a later-feature candidate). In-pane provider *switching* (a Cursor-style handoff) IS in scope — see §35–§44 / 18f.
- **No visual redesign** — feature 19 owns the pane's look; 18 must not move layout or styles.

## Load-bearing decisions (surfaced for review)

Owner-approved direction 2026-07-16 unless marked **(provisional)**.

1. **One pane, provider as attribute** — not a second pane type. Everything downstream (persistence, sidebar, notifications, voice, fork) works per-session regardless of provider.
2. **Codex integration surface = `codex app-server`** (the JSON-RPC-over-stdio API that powers OpenAI's own IDE/desktop surfaces), not `codex exec --json` — the pane needs streaming deltas, interactive approvals, interrupt, and resume, which only app-server provides. Minimum supported codex-cli version is pinned; older installs get an upgrade prompt (§34).
3. **Access pill vocabulary** (provisional wording): **Read-only / Ask to edit / Edits allowed / Full access**, mapped per provider (TECH §mapping). The pill shows the shared label; its popover shows the provider-native mode names. Never a lossy translation: a provider mode with no shared-stop equivalent renders under its native name.
4. **Provider identity stays quiet**: a small provider glyph plus the combined Model·Effort pill; no rebranding of the pane. Per-tab color remains the identity system.
5. **Claude regression bar is absolute**: recorded claude sessions must render identically before/after the refactor (golden-transcript tests in 18a); any behavior diff is a bug.
6. **(provisional)** One codex app-server process per pane (matching today's one-claude-per-pane lifecycle); consolidating to a shared multi-thread process is a later optimization.
7. **In-pane provider switching is a handoff, not a shared conversation** (owner-directed 2026-07-16, Cursor as the reference UX). Cursor can switch losslessly because *it* owns the conversation and replays it to any API. twarp's providers are CLI agents that own their **own** session state (claude's session files, codex's threads) — neither can resume the other's session. So switching mid-conversation starts a fresh session on the target provider **seeded with a digest of the conversation so far**, rendered as one continuous timeline with a visible switch divider. Same UX shape as Cursor; one honest seam. Switching is only available while no turn is running.

## Behavior

### Entry & identity — 18d

1. Typing `codex` (bare, or with supported flags) at an editable shell prompt opens the agent pane with provider=Codex, exactly like `claude` does today — including alias expansion (a user alias wrapping `codex` with flags is expanded and its flags parsed).
2. Supported launch flags map to session options (model, effort/reasoning, resume, access level); unsupported flags fall back to launching the raw CLI in the terminal instead of silently dropping them (same rule the `claude` trigger follows today).
3. The pane header and past-session rows show a small provider glyph; the composer's model pill reads as one combined **Model · Effort** control (e.g. "Sonnet 5 · Default", "GPT-5.5 · Extra High").
4. Nothing about the `claude` entry path changes: trigger, alias handling, flag parsing, raw-CLI toggle all behave as before (§5's regression bar).

### Session lifecycle — 18a / 18b / 18d

5. A Codex session streams into the same timeline surfaces as Claude: user turns, assistant prose (token-streamed), reasoning/thinking (collapsed by default, expandable), command runs with **live output**, file-change cards with diffs, MCP tool calls, web searches, and plan/todo updates.
6. A provider item the pane doesn't recognize renders as a generic expandable card (provider name + payload), never breaks the timeline, and never drops surrounding items.
7. Stop interrupts the running turn without killing the session: the partial turn is preserved, the pane stays live, and the next prompt works. (Codex: turn-level interrupt; Claude: today's interrupt control request. Neither path may signal-kill the process as its primary stop.)
8. Closing the pane (or quitting twarp) and relaunching restores the session lazily like Claude panes do today: the transcript reloads and the conversation resumes on next input, per provider. A Codex pane restores to its same thread; a Claude pane to its same session id.
9. Fork works for both providers where supported (both support it), producing a new session seeded with the history up to the fork point; turn-count parity rules (the 7-era fork fixes) hold for Codex too.
10. If the provider process dies unexpectedly mid-session, the pane shows a clear ended-state with the error the provider reported and offers resume/retry; it never wedges awaiting events that can't arrive.

### Approvals & access — 18c

11. The composer shows an **Access pill** with four stops: Read-only / Ask to edit / Edits allowed / Full access. Cycling it applies the provider-native mapping immediately to the session (as permission-mode changes do today on Claude).
12. The pill's popover lists the provider-native names for the current stop (e.g. Claude "acceptEdits"; Codex "workspace-write + on-request") so power users can verify the truth; a provider configuration that doesn't correspond to a shared stop displays its native name instead of a mislabel.
13. Approval requests from either provider render as one card anatomy: verb-first title ("Codex wants to run `cargo test`"), monospace detail (command + cwd, or file list for edits), and actions **Allow once / Always allow / Deny**. Keyboard shortcuts that answer approvals today keep working.
14. "Always allow" scopes to the session (Codex: its accept-for-session decision; Claude: today's always-allow semantics). It never silently persists beyond the session.
15. Denying an approval never wedges the turn: the provider receives a well-formed refusal and the turn continues or ends per the provider's semantics; there is **no code path where an approval request goes unanswered** (the AskUserQuestion-wedge class of bug is a spec-level invariant now).
16. Launching via a bypass alias (e.g. `codex --dangerously-bypass-approvals-and-sandbox`, `claude --dangerously-skip-permissions`) sets the pill to Full access, visibly.
17. A Codex session at Full access shows the same treatment the pane gives Claude's bypassPermissions today (the pill state is unmistakable); downgrading mid-session via the pill takes effect for subsequent actions.

### Auth & availability — 18d

18. Opening a Codex pane with the CLI not installed shows an install-guidance card (no crash, no blank pane); with the CLI installed but logged out, it shows a **Log in** card whose action runs the vendor's own login flow in a terminal split and auto-detects completion, after which the pane proceeds without restart.
19. Auth status on the Agent settings page (feature 16 §6–§13) works for Codex: installed/logged-in probes, "Logged in (local CLI)" / "Not authenticated" / "CLI not installed" states. The API-key path remains Claude-only unless Codex's key slot ships (out of scope; the settings row says so).
20. A Codex CLI older than the pinned minimum version gets an upgrade card naming the minimum; the pane does not attempt to drive an unsupported protocol.

### Usage & limits — 18e

21. After each turn, the usage line shows what the provider reports: Claude keeps today's cost + token display; Codex shows tokens (in/out/cached) and — when signed in with a ChatGPT plan — plan-quota status. No invented dollar figures for Codex.
22. Provider-reported failures (context window exceeded, usage limit exceeded, auth expired) render as readable ended-turn states with the provider's message, not raw JSON.

### Settings & capability gating — 18d / 18e

23. On the Agent settings page, Codex flips from disabled to enabled: selectable as chat backend, its model list and effort levels populate from the provider (models the provider reports, with their supported effort levels), and the Chat row's permission-mode control renders the Access vocabulary with the Codex mapping.
24. New panes seed from the Chat row exactly as Claude panes do today (launch flag → Chat row → fallback precedence, feature 16 §14–§18) — now provider-aware.
25. Controls a provider lacks are hidden or disabled per its capabilities, not shown broken: e.g. if a capability (fork, thinking display, steering, cost) is absent, its affordance is absent — with no dead buttons.
26. Gemini remains visibly disabled ("coming soon") everywhere it appears.
27. The suggestion generators (16e/16f/16g/16h) keep working unchanged on Claude regardless of the chat provider selection, per their own matrix rows.

### The regression bar — 18a (and every sub-phase)

28. A Claude pane after this feature behaves identically to before it: same timeline rendering for recorded sessions (golden transcripts), same approvals, same interrupt, same persistence/restore, same fork, same composer controls, same notifications and tab signals.
29. Existing persisted Claude panes (rows written before this feature) restore correctly after upgrade — absent provider metadata means Claude.
30. Mixed usage is first-class: Claude and Codex panes side-by-side in one window, in split panes, each driving its own provider without cross-talk (focus, stdin routing, notifications, and per-pane MCP config stay per-session).

### Past sessions — 18d

31. The sidebar past-sessions list shows sessions from both providers, each row carrying its provider glyph; resuming a row opens the pane with the right provider. A provider filter (All / Claude / Codex) is available.
32. Sessions recorded by the vendor CLIs outside twarp (e.g. `codex` run in a plain terminal) appear in the list the same way Claude's do today, when the provider's local store makes them discoverable for the cwd.

### Failure honesty — all sub-phases

33. Any provider error surface shows the provider's own message with enough context to act (stderr snippet, exit code, or protocol error), never a silent blank.
34. Version/compat problems (§20) and protocol mismatches produce actionable cards, never wedged spinners.

### In-pane provider switching (Cursor-style) — 18f

35. The composer's provider control (the glyph on the Model·Effort pill, or its menu) lets the user switch the session's provider **while no turn is running**: enabled at idle — brand-new pane, between turns, after Stop, after a turn ends — and visibly disabled while a turn is in flight (no queued switches).
36. Switching on a **fresh pane** (no completed turns yet) is seamless: the next message simply goes to the new provider; no divider, no seeding, nothing else changes. Composer text in progress is preserved.
37. Switching **mid-conversation** performs a handoff: the pane keeps the entire visible timeline, appends a subtle divider row ("Switched from Claude to Codex"), and the next message starts a fresh session on the target provider seeded with a digest of the conversation so far (decision 7). From the user's seat it reads as one continuous conversation.
38. The digest carries: user and assistant turns (text, verbatim within a size budget), and one-line summaries of tool runs and file edits (command + outcome; diffs elided). It does not re-send attachments/images from earlier turns; the divider's detail notes what was omitted. The new provider is instructed to continue seamlessly, not to re-introduce itself.
39. On switch, model + effort reset to the target provider's configured defaults (the feature-16 Chat row for that provider), shown immediately in the pill; the Access stop is preserved and re-mapped to the target's native modes (§11–§12).
40. Switching back (A→B→A) is a new handoff each time (a fresh A-session seeded with the full visible conversation, including B's turns) — never a silent resume of the earlier A-session, which would drop everything B did.
41. A mixed-provider pane persists and restores like any other: after relaunch, the full stitched timeline renders (each segment loaded from its own provider's store, dividers intact) and the conversation resumes on the **current** provider. A segment whose provider-side history is gone renders as a collapsed "history unavailable" marker instead of vanishing (§29-class honesty).
42. Fork from a turn in the **current** segment works as today (§9). Fork points in earlier segments are unavailable in this phase (their affordance is absent, not broken).
43. If the target provider isn't usable (not installed, logged out, below min version), the switch surfaces the same §18/§20 cards immediately at switch time — the pane never silently stays on the old provider after showing a successful switch.
44. The past-sessions sidebar lists a mixed pane once, under its current provider's glyph, with a mixed-history indicator; resuming restores per §41.

## Smoke test

Steps assume a built `warp-oss`, both CLIs installed and logged in (`claude`, `codex`), run from a repo cwd.

### 18a — driver extraction (no user-visible change)

1. Run `claude`, have a short tool-using conversation (one command, one edit, one approval): everything renders and behaves exactly as before — streaming, cards, approval buttons, Stop, usage line (§28).
2. Quit twarp mid-session and relaunch: the Claude pane restores and resumes on next input (§29).
3. Fork the session from an earlier turn: fork works as before (§28).

### 18b — codex driver (behind its feature flag)

1. In a dogfood/twarp-oss build (flag ships enabled there), run bare `codex` at a prompt: the agent pane opens with a Codex glyph and Model·Effort pill (§1, §3). Alias/flag handling and the rest of §1–§2 land in 18d.
2. Send "list the files here, then read one and summarize it": assistant text streams token-by-token; the command run shows live output; reasoning (if any) renders collapsed (§5).
3. Press Stop mid-turn: the turn stops with partial content preserved; sending a follow-up works (§7).
4. Quit twarp and relaunch **by running `open <path-to>/TwarpOss.app` from your shell** (on this rig the provider key reaches the app only via the shell env that `open` propagates — Dock/Spotlight launches lack it, and executing the binary directly also misbehaves): the Codex pane restores with its prior transcript visible; sending a message resumes the same thread (§8).

### 18c — approvals & access

1. Start a Codex session at "Ask to edit"; request an edit to a file: an approval card appears with Allow once / Always allow / Deny; Deny → the agent reports it couldn't edit; the turn does not wedge (§13, §15).
2. Ask it to run a command; choose Always allow: subsequent identical commands in this session run without prompting (§14).
3. Cycle the Access pill to Full access and request another edit: no prompt; the pill state is clearly visible (§11, §17).
4. Repeat step 1 on a Claude session: the approval card looks and behaves the same (§13).

### 18d — entry, settings, sessions

1. `which codex` → temporarily rename the binary → run `codex`: the pane shows install guidance, no crash; restore the binary (§18).
2. `codex logout`, then run `codex`: the Log in card opens the vendor login in a terminal split; complete it; the pane proceeds without restarting twarp (§18).
3. Open Agent settings: Codex is selectable; its models + efforts populate; set the Chat row to Codex with a specific model/effort; open a new pane via `codex`: it seeds with those values (§23–§24).
4. Sidebar past sessions: rows show provider glyphs; filter to Codex-only; resume a Codex row → correct provider and history (§31).
5. Open one Claude pane and one Codex pane side-by-side; interact with both: no cross-talk in input, output, or notifications (§30).

### 18e — capability polish

1. Fork a Codex session: a new pane continues from the fork point (§9).
2. Complete a Codex turn: the usage line shows tokens (and quota when on a ChatGPT plan), no dollar figure; a Claude turn still shows cost as today (§21).
3. Trigger a provider error (e.g. revoke auth mid-session or exceed a tiny context deliberately): a readable ended-state with the provider's message (§22, §33).

### 18f — in-pane provider switching

1. Open a fresh pane via `claude`; before sending anything, switch the provider control to Codex: the pill updates (Codex glyph, Codex default model/effort); send a message: it's answered by Codex; no divider appears (§36).
2. In a Claude conversation with a few turns (including one tool run), switch to Codex while idle: a "Switched from Claude to Codex" divider appears; ask "what did we do so far?": the answer correctly reflects the earlier conversation (§37–§38).
3. During a running Claude turn, the provider control is disabled; after Stop, it enables (§35).
4. Switch back to Claude and ask a follow-up referencing something Codex did: it's aware of it (§40).
5. Quit twarp and relaunch: the mixed timeline restores with the divider; the next message goes to the current provider (§41).
6. Log out of codex (`codex logout`), then try to switch to Codex: the Log in card appears at switch time; the pane clearly remains on Claude until auth succeeds (§43).
7. The usage/cost line and Access pill reflect the current provider after each switch (§39).
