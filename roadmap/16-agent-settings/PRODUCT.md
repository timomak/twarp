# Agent settings — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers.

## Summary

A new **Agent** settings page is the one home for configuring twarp's agent behavior — today scattered across composer pills, per-session SQLite defaults, and hardcoded spawn flags. From this page a user:

- picks the **agent CLI backend** (Claude enabled; Codex / Gemini shown but disabled), with twarp reporting whether that CLI is installed and already logged in so existing local auth "just works";
- optionally provides an **API key** (stored in the OS keychain, never in plaintext settings) as an alternative to subscription/local-CLI auth;
- configures a **per-action model matrix** — **Chat & history**, **Terminal suggestions**, and **Chat reply suggestions** — where each action independently picks a **provider + model + effort**, plus the chat action's **permission-mode** default;

and, because the same page owns the config, feature 16 also **ships the two suggestion consumers** that read that config:

- **Chat reply suggestions** — after a Claude turn, a suggested next message appears as dim ghost text in the composer; Tab/→ accepts it, any other key dismisses it;
- **Terminal suggestions** — an AI-generated command suggestion appears as ghost text in the terminal input (layered under the existing instant history suggestion), accepted with the existing accept-autosuggestion key.

Phase 1 is **Claude-only for the chat backend** but the schema and UI are **multi-provider and capability-aware** — the selector and per-action rows persist provider-tagged config now; Codex/Gemini are disabled entries lit up later by adapters.

## Problem

After feature 07 shipped the Claude Code pane, agent configuration ended up in three disconnected places: composer pills (runtime cycling), a `claude_session_defaults` SQLite row (last-used), and hardcoded fallbacks. There is no single surface to set what a *new* chat starts with, no way to choose an agent CLI, no place to supply an API key, and nowhere to say which model powers each distinct action. Separately, twarp has no reply-suggestion or terminal-AI-suggestion feature at all, even though the ghost-text rendering + accept plumbing already exists (feature 07's editor; Warp's terminal `Autosuggester`).

This feature creates the config surface **and** builds the two suggestion consumers on top of it, so the per-action matrix is live on ship rather than inert.

## Goals / Non-goals

**Goals**

- One **Agent** page in the existing settings surface, following twarp's `SettingsSection` / page-view conventions.
- **Backend selector** over `CLIAgent` (Claude / Codex / Gemini), Codex/Gemini shown **disabled** ("coming soon"), with a live **auth/availability probe** per backend.
- **Local-auth reuse** as the default path: shell out to the vendor CLI headless; it uses whatever it is already logged into — no re-implementation of vendor auth.
- **API-key entry** as an explicit alternative, stored in the **OS keychain**.
- A **per-action model matrix** — chat, terminal suggestions, reply suggestions — each with an **independent provider + model + effort** (decision #3).
- The **chat action row is the single source of truth** for what a new Claude Code pane starts with (model + effort + permission mode); there is **no separate "defaults" block** (decision, "unify").
- Those settings are **authoritative** for new panes — they define the starting point, not merely a floor under the pills' last-used memory (decision, "authoritative").
- **Ship both suggestion consumers**: chat reply ghost-text and terminal AI-suggestion, each reading its matrix row and each behind its own enable toggle.
- **Capability-aware UI**: controls that don't apply to the selected backend are hidden/disabled, not shown broken.
- **Multi-provider-shaped from day one**, so lighting up Codex/Gemini is an adapter, not a schema rewrite.

**Non-goals**

- **No Codex/Gemini execution this phase.** Selector lists them disabled; no driver adapter ships in Phase 1.
- **No cloud sync of settings, no twarp account.** Config is local; the keychain is the OS keychain.
- **No billing/usage metering.** Auth and quota stay the vendor CLI's / API's concern; twarp surfaces whatever error it reports (feature 07 §30 rule carries over).
- **No re-implementation of vendor auth flows** (no OAuth UI, no in-app login). "Log in" = run the vendor CLI's own login; twarp only *detects* the result.
- **No auto-send of suggestions.** Accepting a reply/terminal suggestion only fills the input; the user still presses Enter.
- **Not removing the composer pills.** Runtime per-pane cycling (7g/7m) stays; this page sets the authoritative *starting* values.

## Load-bearing decisions (surfaced for review)

Owner-confirmed 2026-07-01 unless marked **(provisional)**.

1. **New roadmap feature 16, not a 07 sub-phase** (07 is merged; this spans settings + provider abstraction + keychain + terminal + generators).
2. **Phase 1 = Claude-only chat backend, schema = multi-provider.** Selector shows Codex/Gemini **disabled** ("coming soon").
3. **Provider scope = independent per action.** Each action stores its own `{provider, model, effort}`; they need not match (e.g. Claude-subscription chat + an API-key Haiku suggester).
4. **API key + keychain in Phase 1.** Subscription/local-CLI auth remains the zero-config default.
5. **Auth reuse via CLI probe, not re-implementation** (§10–§13).
6. **Unified config, authoritative.** The chat matrix row *is* the new-chat model/effort/mode — no duplicate "defaults" block — and it **overwrites** the pills' last-used memory for new panes (§14–§18).
7. **Both suggestion generators are in scope**, each behind its own enable toggle (§30–§43).
8. **No feature flag for the page itself** (provisional); each suggestion consumer keeps its own enable toggle.

## Behavior

Grouped by area, annotated with the delivering sub-phase (16a–16f, defined in TECH.md).

### The page & backend selector — 16a/16d

1. A new **Agent** entry appears in the settings navigation sidebar, opening a dedicated Agent page.
2. A **Backend** control lists the `CLIAgent` options: **Claude** (enabled), **Codex** / **Gemini** (visible, **disabled**, "coming soon"). This is the top-level chat-backend choice.
3. Claude is the default selected backend on first open.
4. Selecting a disabled backend is a no-op with an inline "coming soon" affordance; persisted state is unchanged.
5. The page renders **capability-aware**: only controls the selected backend supports are shown/enabled (permission-mode Ask/Plan/Accept-edits/Bypass render only for backends that support them; a backend with no effort concept hides the effort control).

### Auth & availability — 16b

6. For the selected backend, an **auth status** row shows one of: **Logged in (local CLI)**, **Using API key**, **Not authenticated**, or **CLI not installed**.
7. Status derives from two probes: is the vendor CLI **installed** (resolvable on PATH), and is it **logged in** (a cheap, side-effect-free probe — §11).
8. When installed and logged in, twarp uses that **local auth by default**; no key required; the row reads "Logged in (local CLI)".
9. When not installed, the row reads "CLI not installed" with install guidance; the backend can't be used until installed.
10. An **API key** field per backend; saving stores it in the **OS keychain** (never in plaintext settings) and flips auth to "Using API key" for actions configured to use that key.
11. The auth probe is **read-only, cheap, off-thread** — never starts a chat, consumes quota, or blocks the UI; the row shows a transient "checking…" state.
12. Saving/clearing a key updates keychain + status immediately; clearing a key with local login present falls back to "Logged in (local CLI)".
13. The stored key is **never displayed back** — a masked "key set" indicator with Replace/Remove.

### Per-action model matrix (unified config) — 16a/16c

14. A **Models by action** section lists three rows: **Chat & history**, **Terminal suggestions**, **Chat reply suggestions**. Each independently selects **provider**, **model**, and (where supported) **effort** (decision #3). There is **no separate "defaults for new chats" block** — the Chat row is that config.
15. The **Chat & history** row also carries the **permission-mode** default (Ask=`default` / Accept-edits=`acceptEdits` / Plan=`plan` / Bypass=`bypassPermissions`) and is the **single source of truth** for what a newly opened Claude Code pane starts with.
16. These chat values are **authoritative** for new panes: a new pane starts at exactly the Chat row's `{model, effort, permission_mode}`, **overwriting** the pills' last-used memory. (An explicit `claude --model …` launch flag / alias still wins — precedence is **launch flag → Chat row → hardcoded fallback**.)
17. Per-pane composer pills (7g/7m) still override at runtime **for that pane**; changing a pill no longer changes the starting point of the *next* pane (that's now the settings' job).
18. Changing the Chat row affects **subsequently opened** panes only; open panes are unaffected.
19. Model/effort choices are constrained to the provider's known-valid values; an unknown/removed value degrades to the hardcoded fallback rather than spawning an invalid flag.
20. A suggestion row (Terminal / Reply) left at **"Default"** provider means "reuse the Chat backend's choice" — an explicit inherit.
21. Each row shows its **auth source** (local CLI vs API key) resolved per §6–§13 for that row's provider, and warns inline if that provider is unauthenticated.
22. Selecting a provider with no valid auth surfaces the §21 warning; a live consumer degrades to no-suggestion rather than erroring (feature 07 §26 degradation).

### Persistence & safety — 16a/16b

23. All non-secret config (backend, matrix selections, enable toggles) persists across restarts via the settings store; **secrets persist only in the keychain**.
24. Corrupt/absent config loads to safe defaults (Claude chat, hardcoded model/effort/mode, suggestion rows = "Default", generators off) without crashing.
25. No telemetry event carries a key or any secret; provider/model *identifiers* may be reported, never key material.
26. Removing/rotating a key or logging out of a vendor CLI is reflected on next probe; key material is never cached outside the keychain.

*(§27–§29 reserved.)*

### Chat reply suggestions (ghost text) — 16e

30. An **enable toggle** ("Suggest a reply after each response") on the Agent page gates this feature; **off by default**.
31. When enabled, after a Claude turn **ends** and the composer is **empty**, twarp generates one suggested next user message and renders it as **dim ghost text** in the composer.
32. The suggestion is produced by the provider/model/effort in the **Chat reply suggestions** matrix row (or the inherited Chat backend if "Default").
33. **Tab or →** accepts the suggestion into the composer as editable text (reusing the editor's accept-autosuggestion binding); the user still presses Enter to send. It **never auto-sends** (§Non-goals).
34. **Any other keystroke** (or starting to type) dismisses the ghost text immediately; it is never sticky.
35. Generation is **off-thread and best-effort**: if the row is unauthenticated, generation fails, or the composer becomes non-empty, **no suggestion appears** — never an error, never a hang (feature 07 §26/§30 rule).
36. Only shown when the pane is idle (not streaming) and focused; a new turn or focus loss clears any pending suggestion.

### Terminal suggestions (AI ghost text) — 16f

40. An **enable toggle** ("AI command suggestions in the terminal") on the Agent page gates this feature; **off by default**.
41. The existing **instant history-based** autosuggestion stays the primary layer and is unchanged; the AI suggestion is a **fallback** shown only when history has no match.
42. When enabled and history is empty, after a **typing-pause debounce**, twarp generates a command suggestion via the **Terminal suggestions** matrix row's provider/model/effort and renders it as ghost text using the terminal's existing autosuggestion surface.
43. The suggestion is accepted with the **existing accept-autosuggestion key** and behaves like any terminal autosuggestion (partial-accept, dismiss-on-type). Generation is **off-thread, debounced, best-effort**: unauth/failure/edit-in-flight → no suggestion, never a block. It **never auto-runs** a command.

## Constraints the user should know (surfaced, not handled)

- **Subscription generation is the heavy path.** A matrix row whose provider is Claude via the *subscription* must generate through the local `claude` CLI (a resident/`-p` invocation), which is slow and quota-bearing — so subscription-backed suggestions are **debounced-on-pause**, not instant, and terminal per-keystroke suggestion via subscription is impractical. An **API-key** provider (fast completion model) is the low-latency path and the recommended default for suggestion rows. This tradeoff is why the matrix allows a different provider per action.
- **Vendor quota/billing** stays the CLI's/API's concern; twarp only surfaces its errors.

## Dependencies & sequencing

- Recommended order within 16: **16a → 16b → 16c** (config + auth land first, immediately useful for chat) → **16d** (provider hardening) → **16e** (reply ghost-text) → **16f** (terminal suggestions). The generators depend on the matrix + auth being in place.
- **Codex/Gemini adapters** remain a Phase-2 follow-on; this feature only requires the schema + disabled selector entries.

## Smoke test

Steps to validate against a built twarp binary. Each sub-phase's PR is gated on its own sub-heading only.

### 16a — Agent page scaffold

1. Launch twarp, open Settings, and confirm an **Agent** entry appears in the settings navigation sidebar.
2. Click it: a dedicated Agent page opens with a **Backend** control listing Claude (enabled, selected by default) and Codex / Gemini (visible but disabled, "coming soon").
3. Click a disabled backend: nothing changes (no selection, no crash), and the "coming soon" affordance is visible.
4. In the **Chat & history** row, change the model and permission mode, quit and relaunch twarp: the choices persist.
5. Open a new Claude Code pane (run `claude` in a terminal tab): its composer pills start at exactly the Chat row's model/effort/permission-mode, regardless of what a previous pane's pills were set to.

### 16b — Auth probe and keychain

1. On the Agent page, the Claude auth-status row resolves (after a brief "checking…") to **Logged in (local CLI)** on a machine with an authenticated `claude` CLI.
2. Save an API key: the row flips to **Using API key**, and the field shows a masked "key set" indicator with Replace/Remove — the key text is never displayed back.
3. Remove the key: the row falls back to **Logged in (local CLI)**. The key never appears in any plaintext settings file (`grep` the settings store for it).

### 16c — Per-action matrix

1. The **Models by action** section shows three rows: Chat & history, Terminal suggestions, Chat reply suggestions — each with its own provider/model (and effort where supported).
2. Set the Terminal suggestions row to an explicit provider and the Reply row to "Default": the Reply row indicates it inherits the Chat backend.
3. Each row shows its auth source; pointing a row at an unauthenticated provider shows an inline warning, not an error dialog.

### 16d — Provider abstraction hardening

1. With Claude selected, all of permission-mode / model / effort controls render; the page renders capability-aware (no control for a capability the backend lacks).
2. Corrupt or delete the agent config from the settings store, relaunch: the page loads safe defaults (Claude, hardcoded model/effort/mode, rows on "Default", generators off) without crashing.

### 16e — Chat reply suggestions

1. The "Suggest a reply after each response" toggle exists on the Agent page and is **off by default**; with it off, no ghost text ever appears in the Claude pane composer.
2. Enable it, complete a short Claude turn, leave the composer empty: dim ghost text with a suggested reply appears; **Tab** accepts it into the composer as editable text (it is not auto-sent); typing any other character dismisses it.

### 16f — Terminal AI command suggestions

1. The "AI command suggestions in the terminal" toggle exists on the Agent page and is **off by default**.
2. Enable it, type a prefix with no history match in a terminal, pause: after the debounce, a ghost-text command suggestion appears and is accepted with the existing accept-autosuggestion key; it never auto-runs.
3. Type a prefix that **does** match history: the instant history suggestion appears as before (the AI layer is only a fallback).
