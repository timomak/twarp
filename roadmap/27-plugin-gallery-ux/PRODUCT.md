# 27 — Plugin gallery UX: connector-style add flow (PRODUCT)

## Summary

Rework the Plugins page add flow so established integrations feel like the
connector directories in the Claude and Codex apps: gallery cards with a
single primary action, one-click browser sign-in where the server supports
it, and short guided dialogs where it doesn't. The generic server form
(transport, command, args, env, headers) is demoted to an "Add custom
plugin" escape hatch — it never appears on the happy path for a gallery
connector.

## Problem

Feature 24 made remote OAuth servers *work*, but the add UX is still
form-first: every gallery card, including Slack and Composio, opens the same
generic editor. Composio's card prefills a placeholder URL
(`…/YOUR_SERVER_ID`) the user must know to replace; Slack's card drops the
user into a stdio form with raw env-var fields. On Claude/Codex the same
integrations are one click plus a browser consent. Twarp cannot fully match
that (their smoothness comes from vendor-run backends holding pre-registered
OAuth clients; twarp is account-less and client-side only), but it can match
the *shape*: one card, one action, no generic form unless you asked for it.

## Goals / Non-goals

- **Goal:** gallery connectors present exactly one primary action, and the
  user never sees transport/command/env/header fields unless they open
  "Add custom plugin" or explicitly edit an installed entry.
- **Goal:** connectors that need user input before connecting (Composio's
  per-user server URL, Slack's tokens) get a focused dialog asking only for
  that input, with copy explaining where to get it.
- **Non-goal:** no remote/marketplace gallery fetch — the connector list
  stays compiled-in (feature 23 decision, unchanged).
- **Non-goal:** no vendor backend, no shipped pre-registered OAuth clients
  (blocked anyway; see 24 TECH follow-ups).
- **Non-goal:** no change to the OAuth handshake, token storage, or the
  Claude/Codex config emission from feature 24.

## Figma

Figma: none provided. Visual reference is the Claude desktop connector
directory and the Codex app's integrations page; layout follows the
feature-19 tokens and the existing 720px settings column.

## Behavior

### Gallery cards

1. The Plugins page leads with a connector gallery: one card per compiled-in
   connector showing its product icon, name, one-line description, and a
   single primary action button. No transport, command, URL, or env detail
   is visible on a card.
2. Each connector is one of three kinds, which determines its primary
   action:
   - **One-click** (server has a fixed public URL and supports browser
     sign-in — Notion, Linear, GitHub, Cloudflare): button reads
     **Connect**.
   - **Setup-required remote** (server URL is per-user — Composio): button
     reads **Set up…**.
   - **Credential-based** (no hosted server; runs locally with pasted
     tokens — Slack): button reads **Set up…**.
3. **One-click flow:** pressing Connect immediately saves the plugin under
   its canonical name and URL (both providers enabled), opens the browser
   consent, and puts the card into a connecting state with a **Cancel**
   action. On grant the card shows a **Connected** chip. On deny, timeout,
   or cancel the card shows the failure inline with a **Retry** action; the
   saved entry remains (it is the same entry, not a duplicate, on retry).
   No form or dialog appears at any point in this flow.
4. **Setup-required remote flow (Composio):** Set up… opens a small dialog
   containing only the inputs the connector actually needs — for Composio, a
   single "Server URL" field — plus one sentence of guidance and a link to
   the page where the user obtains the value (the Composio dashboard). The
   primary button is Connect, disabled until the field parses as an http(s)
   URL. On Connect the dialog closes and the flow continues exactly as
   invariant 3. The placeholder `YOUR_SERVER_ID` URL is never shown in an
   editable URL field as if it were valid, and can never be persisted.
5. **Credential-based flow (Slack):** Set up… opens a dialog with one
   labeled field per required credential (e.g. "Bot token", "Team ID") —
   never raw `KEY=` env lines — each with a one-line description and a link
   to where the credential is created. The primary button is Add, disabled
   while any required field is empty. On Add the plugin is saved with both
   providers enabled and appears in Installed; no browser consent is
   involved and the card does not claim one will happen.
6. Dialogs from invariants 4–5 are dismissable (Esc, click-outside, or a
   Cancel button) without saving anything.
7. A connector that already has an installed entry shows that state on its
   gallery card (e.g. a checkmark or **Added**), and its action changes to
   **Manage**, which scrolls/jumps to the installed entry. Pressing the
   card's action repeatedly can never create a second entry for the same
   connector; a second distinct instance of the same service is still
   possible via Add custom plugin.
8. **Add custom plugin** remains available as a visually secondary
   affordance after the gallery, and opens the feature-24c remote-first
   editor (Name + Server URL + Connect; Advanced disclosure for transport,
   stdio fields, env, headers). Its behavior is unchanged by this feature.

### Installed entries

9. Installed plugin cards keep the feature 23/24 surface: product icon,
   name, status chip (Connected / Connecting / Needs authentication / Not
   connected / failure), per-provider Claude/Codex toggles, and
   state-dependent Connect / Cancel / Disconnect actions. An explicit Edit
   affordance opens the full editor; editing is never required to read
   status or reconnect.
10. When a remote server rejects browser sign-in (the 24 "needs static
    auth" classification), the installed card says in plain language that
    this server doesn't support browser sign-in and offers the editor's
    headers field as the path — it does not show a dead Connect button.
11. The existing Codex warning for non-bearer headers (Claude-only servers)
    is preserved wherever headers can be entered.
12. Quitting mid-consent and relaunching lands the entry in Needs
    authentication with a working Connect action (feature 24 invariant,
    must not regress).

### Cross-cutting

13. Pre-existing plugin rows are untouched by this feature: no migration,
    no renaming, no state change. The Skills and Import sections of the
    page are unchanged.
14. The gallery requires no network access to render; connector metadata
    (names, descriptions, kinds, field lists, doc links) is compiled in.
    Doc links open in the default browser.
15. All new chrome uses the feature-19 tokens and the active-tab-following
    surface colors; product icons come from the existing external product
    icon set, with a generic fallback for connectors without one.

### Open question

16. **Open question:** the Slack preset currently wraps the archived
    community `@modelcontextprotocol/server-slack` stdio server.
    Recommended: keep it as the credential-based exemplar with a caveat
    line in its dialog ("community server, no longer maintained"), and
    swap the backing server without UX change if an official hosted Slack
    MCP appears. Alternative: drop Slack from the gallery until then.
