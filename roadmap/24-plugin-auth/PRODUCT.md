# 24 — Plugin auth: remote-first add flow + MCP OAuth (PRODUCT)

## Problem

The Plugins add flow (23) presents one generic form for every integration:
transport dropdown, command, args, env vars. That shape is right for local
stdio servers but wrong for the case users actually hit — hosted remote
servers like Composio, Notion, or Linear, where the entire configuration is
one URL and an identity. Worse, twarp has **no way to authenticate** a remote
server: no OAuth handshake, no headers field. The MCP spec defines a standard
browser-consent OAuth flow that every major hosted server (including Composio)
supports; competing clients offer "paste URL → browser opens → click Allow →
connected". In twarp today the only workaround is smuggling credentials
through query strings minted on the provider's dashboard.

## Goal

Make adding a hosted integration feel like installing an app: pick it from the
gallery (or paste a URL), press **Connect**, approve in the browser, done.
Local stdio servers remain fully supported as the explicit advanced path.

## Non-goals

- No remote/marketplace gallery fetch — presets stay compiled-in (unchanged
  from 23).
- No support for provider-proprietary auth schemes beyond standard MCP OAuth
  and static bearer/header auth.
- No re-plumbing of stdio env-var handling; that form is merely relocated
  behind "Advanced".

Figma: none provided (no mock; match existing Plugins page chrome and the
Codex-app-style design language from feature 19).

## Behavior

### Remote-first add form

1. "Add plugin" and gallery quick-add open the same inline editor as today,
   but the server sub-form defaults to **remote**: visible fields are Name,
   **Server URL**, and a **Connect** button. No transport dropdown, no
   command/args/env on first paint.
2. An **Advanced** disclosure (collapsed by default) reveals: transport
   switch to `Command (stdio)` (which swaps in the existing command/args/env
   fields), an optional **Headers** multiline field (`Header-Name: value`,
   one per line), and env vars. Opening Advanced never loses entered data;
   switching transport back and forth preserves both field sets until save.
3. Gallery presets whose target is a remote server (Composio, Notion,
   Linear, Slack, …) prefill Name + URL so the user's only action is
   **Connect**. Stdio presets (if any) open with Advanced expanded.
4. Validation: remote requires a syntactically valid `http(s)://` URL; stdio
   requires a command (unchanged). Header lines that don't parse as
   `Name: value` block save with an inline error.

### Connect & the OAuth handshake

5. Pressing **Connect** (or Save on a remote server that has never
   connected) probes the URL:
   - If the server responds without demanding auth → state becomes
     **Connected**, showing the tool count discovered.
   - If the server demands OAuth (standard MCP 401 + OAuth metadata) →
     twarp starts the browser-consent flow: the default browser opens the
     provider's authorization page; the editor shows **Waiting for
     browser…** with a Cancel affordance.
   - If the server demands a static credential (401 without OAuth
     metadata) → the editor opens Advanced with the Headers field focused
     and an inline hint ("This server expects an Authorization header").
6. Completing consent in the browser returns the user to twarp
   automatically; the pane transitions to **Connected** without further
   input. Denying consent, closing the browser tab, or pressing Cancel
   returns the editor to **Not connected** with a retriable error — never a
   wedged spinner. The wait times out (~2 min) into the same retriable
   state.
7. OAuth credentials are stored in the macOS keychain, never in plaintext
   config or the DB. Tokens auto-refresh silently; the user is only
   re-prompted for browser consent when the provider revokes/expires the
   grant beyond refresh.
8. Auth is per-server, not per-plugin: a plugin with two remote servers can
   have one connected and one pending, each with its own Connect state.

### Status, everywhere the server appears

9. Each remote server row (editor and plugin card) shows a status chip:
   **Connected** (with tool count), **Needs authorization**, **Error**
   (with a hover/expand for the message), or **Local** for stdio servers
   (no probe is attempted for stdio).
10. A connected server row offers **Disconnect** (revokes locally: deletes
    stored tokens, returns to Needs authorization). A needs-auth row offers
    **Connect**. Status chips update live when a background refresh fails.
11. Agents (Claude and Codex sessions) using a connected server get working
    auth transparently — a session started after Connect succeeds must be
    able to call the server's tools with no extra user step. If a server the
    agent needs is in **Needs authorization**, the Plugins page is the fix;
    the agent session surfaces the failure as the provider's error, not a
    twarp crash.
12. Existing configured servers migrate untouched: current URL-with-embedded-
    key entries keep working exactly as before (they probe as "no auth
    demanded" → Connected). Nothing forces re-auth on upgrade.

### Edge cases

13. Two OAuth flows can't run concurrently for the same server; starting a
    second Connect cancels the first. Flows for *different* servers may
    overlap safely.
14. Quitting twarp mid-consent abandons the flow cleanly; relaunching shows
    Needs authorization, and Connect works again.
15. Offline / unreachable URL → **Error** state with the network error,
    retriable; never blocks saving the plugin (a plugin may be saved with a
    server in any auth state).
16. Deleting a server or plugin deletes its keychain material.
