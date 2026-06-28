# 14 — Built-in browser (Claude-debuggable) (PRODUCT)

## Problem

A twarp user debugging a web app they're building lives in two windows: twarp (where
the `claude` session runs) and a separate browser (Chrome + DevTools). When they ask
Claude to "check why the login button does nothing", Claude is blind — it can't see
the page the user sees. The closest tool today is the `chrome-devtools` MCP, but that
drives a *separate* Chrome instance, not the user's actual tab, and it means running a
second browser engine just to debug.

twarp already hosts a local `claude` session and a read-only MCP viewer (feature 13).
What's missing is a surface Claude can *see and drive*: the same live page the user is
looking at.

## Goal

Add a **built-in browser pane** to twarp that is:

1. **A real browser the user can use** — open the pane, type a URL, navigate, go
   back/forward, reload.
2. **Drivable by the local `claude` session** — Claude inspects and controls the
   *same live tab the user sees* (DOM, accessibility tree, console, network,
   click/type/eval/screenshot), surfaced as **MCP tools twarp auto-registers for the
   session** so Claude uses them as naturally as it already uses `chrome-devtools`.

The differentiator over the `chrome-devtools` MCP is **shared session**: Claude debugs
the exact pixels and DOM the user has in front of them, in one window, with no second
engine.

mac-first. v1 is a **dev-preview pane**: a single tab with navigation plus the full
automation surface. It is not a general-purpose web browser yet.

## Non-goals (explicit, v1)

- **No multi-tab / tab strip.** One webview per pane (open more panes for more pages).
- **No history, bookmarks, downloads, autofill, or settings.**
- **No profile / cookie management UI.** The webview uses a single default data store;
  cookies persist for that store, but there is no UI to inspect or clear them.
- **No extensions.**
- **No cross-platform.** macOS / WebKit only this pass (matches twarp's native chrome;
  Windows/Linux would need a different engine).
- **No full CDP-fidelity network inspection.** Network capture is best-effort via
  in-page `fetch`/`XHR` instrumentation (see "Known limitations"); a system-proxy
  mirror (cmux's `BrowserSystemProxyMirror`) is out of scope for v1.
- **No popup / `window.open` / new-window handling** beyond loading into the same
  webview (or a clear no-op); multi-window comes with multi-tab later.
- **Not CEF/Chromium.** Engine is `WKWebView` (see TECH.md for the rejected
  alternatives).

## Users & value

- **The user, debugging their own app.** Opens the app in the browser pane, hits a bug,
  asks Claude. Claude snapshots the DOM, reads the console error, clicks the broken
  button, and reports back — all against the user's live tab, in twarp.
- **Claude, as the agent.** Gains a first-class "look at and drive the page" capability
  scoped to twarp, discovered automatically through MCP (no manual socket wiring, no
  second browser to launch).
- **The user, just browsing.** A lightweight in-app browser for quick lookups without
  leaving twarp.

## Behavior

### 1. Opening the pane

- A new pane type, **Browser**, opens in the main content area like any other pane
  (Terminal / Claude Code / Network Log).
- Trigger: a command-palette / menu action **"New Browser Pane"** (with a default
  keybinding), opening the pane in place of — or split alongside — the focused pane,
  exactly as other panes are created.
  - **Deliberate divergence from feature 07:** the Claude pane is opened by intercepting
    the real `claude` terminal command. The browser has no natural CLI command to
    intercept, and inventing one risks shadowing a real binary on the user's `PATH`.
    So v1 opens via the menu/palette/keybinding, not a terminal-command trigger.
- The pane opens to a **blank state** with a focused omnibar (no default home page).

### 2. Browser chrome (the UX)

- A slim top bar with:
  - **Back**, **Forward**, **Reload** controls. Back/Forward are disabled (muted) when
    there's nowhere to go; Reload becomes a **Stop** affordance while a load is in
    flight.
  - An **omnibar** (URL field). Submitting navigates. Input without a scheme is treated
    as a URL when it looks like one (has a dot / is `localhost[:port]`), otherwise it is
    a no-op in v1 (no search-engine fallback — non-goal).
  - A **loading indicator** (the Reload→Stop swap plus a subtle progress treatment)
    while a page loads; it clears on load finish or failure.
- The page content fills the rest of the pane. It is a **live, interactive `WKWebView`**:
  the user can scroll, click, type, select, and use the page normally.
- The omnibar reflects the **current committed URL** as the user navigates (clicks
  in-page, redirects, Claude-driven navigation all update it).
- The pane's **header/tab title** reflects the page `<title>` (falling back to the host)
  so the tab is identifiable.

### 3. Coexisting with the rest of twarp

- The webview is **clipped to the pane's rectangle** — it never paints over the tab bar,
  sidebar, omnibar of other panes, or pane borders.
- When the pane's **tab is switched away, the pane is hidden, or the window is
  occluded/minimized, the webview is hidden** (not left floating over whatever is now
  on screen). Switching back restores it at the correct position.
- **Resizing** the pane / window keeps the webview frame-synced with no visible lag or
  tearing at the pane edges.
- **Keyboard focus**: clicking into the page gives the webview focus and keystrokes go
  to the page (typing in a form, etc.). Clicking the omnibar focuses the omnibar.
  twarp's global shortcuts (switch pane / tab) still work; focus returns sanely to
  twarp chrome when the user leaves the page.
- **Closing** the pane tears down the webview cleanly (no leaked process/host view, no
  lingering audio from a page).

### 4. Persistence

- A browser pane **persists across restart** like the Claude pane: on relaunch it
  reopens and restores its **last committed URL** (re-navigating to it). In-page state
  (form contents, scroll, JS heap) is not restored — only the URL.
- A pane that was never navigated (blank) restores as a blank browser pane (or is not
  persisted), consistent with how the zero-state Claude pane is handled.

### 5. Claude automation surface (the headline)

When a browser pane exists in the workspace, the local `claude` session gains a set of
**MCP tools** for driving it, registered automatically (the user does nothing). The
tools mirror the shape Claude already knows from the `chrome-devtools` MCP:

| Tool | Behavior |
|------|----------|
| `browser_navigate(url)` | Navigate the pane to `url`. **If no browser pane is open, one is opened** (and twarp comes forward), then navigates. |
| `browser_snapshot()` | Return a structured DOM + accessibility-tree snapshot with **stable element refs** Claude can target in subsequent calls (the agent-browser model). |
| `browser_click(ref)` | Click the element identified by a snapshot `ref`. |
| `browser_type(ref, text)` | Focus the element and type `text`. |
| `browser_eval(js)` | Evaluate JavaScript in the page and return the JSON-serializable result (or an error). |
| `browser_screenshot()` | Return a PNG screenshot of the current viewport. |
| `browser_console()` | Return recent console messages and uncaught errors captured since the page loaded. |
| `browser_network()` | Return recent network activity captured via in-page instrumentation (see limitations). |
| `browser_wait(...)` | Wait for a condition (selector appears / navigation settles / timeout) before returning, so Claude can sequence actions reliably. |

Behavioral guarantees:

- **Same live tab.** Every tool acts on the exact webview the user sees — a
  `browser_click` visibly clicks; a `browser_navigate` visibly navigates; the omnibar
  and content update in front of the user.
- **Auto-discovery.** The tools appear in the session's MCP server list (visible in the
  feature-13 MCP viewer as a connected server, e.g. `twarp-browser`) without the user
  configuring anything.
- **One target.** If multiple browser panes are open, tools target the **most recently
  focused** browser pane; the resolved target is stated in tool results so Claude isn't
  guessing. (Multi-pane targeting/selection is a later enhancement.)
- **Safety / honesty.** A tool that can't act (no page loaded, ref stale, eval threw)
  returns a clear error, never a hang and never a faked success.

### 6. Relationship to feature 13 (MCP viewer)

The browser automation server is a normal MCP server from the session's point of view,
so it shows up in the feature-13 viewer like any other: `twarp-browser · connected`,
with its tools listed as Claude uses them. No special-casing in the viewer.

## Known limitations (by design, v1)

- **Network inspection is best-effort.** `WKWebView` has no first-class network
  observation API, so `browser_network()` reports what in-page `fetch`/`XHR` hooks can
  see — it will miss requests made before the hook installs, non-`fetch`/`XHR` loads
  (img/script/CSS subresources, WebSocket frames), and exact timing/headers fidelity.
  This is enough to debug most app-level API calls; a proxy-based mirror for full
  fidelity is a later enhancement. The tool's output must say it's best-effort, not
  imply CDP-grade completeness.
- **Snapshot refs are per-snapshot.** Element refs are valid until the next snapshot or
  a navigation; a stale ref returns a clear "re-snapshot" error rather than clicking the
  wrong thing.
- **Single data store.** All browser panes share one cookie/storage store; there's no
  isolation or private mode in v1.

## Smoke test

The feature is sub-phased (14a–14e); validate each as it ships. Build twarp with
`./script/run`.

### 14a — Native embed spike

1. Open twarp. Open a Browser pane (temporary dev entry point is fine for this phase).
   A `WKWebView` renders inside the pane, navigated to a test URL (e.g.
   `https://example.com`).
2. The webview is **clipped to the pane** — it does not paint over the tab bar, the
   sidebar, or adjacent panes.
3. Split the pane / resize the window: the webview stays frame-synced to the pane rect
   with no tearing or lag at the edges.
4. Switch to another tab: the webview **disappears** (does not float over the other
   tab's content). Switch back: it reappears in the right place.
5. Minimize/restore and occlude/reveal the window: no stray webview painting.
6. Click into the page: the page takes keyboard focus and accepts typing; twarp's
   pane/tab-switch shortcuts still work.
7. Close the pane: the webview is gone, no leaked view, no lingering page audio.

### 14b — Minimal browser UX

1. Open a Browser pane via the **"New Browser Pane"** menu/palette action (and its
   keybinding). It opens focused on the omnibar.
2. Type `example.com` and submit: it navigates to `https://example.com`; the omnibar
   shows the committed URL; the tab title reflects the page title.
3. Click a link on the page: the omnibar updates to the new URL; **Back** becomes
   enabled.
4. Back, then Forward: navigation works and the buttons enable/disable correctly.
5. Reload shows a loading state and swaps to a **Stop** affordance mid-load; Stop
   cancels a slow load.
6. Restart twarp: the Browser pane reopens and re-navigates to its last URL.

### 14c — Automation core (internal API)

1. With a page loaded, exercise the internal automation API (via a dev harness/test):
   `navigate`, `snapshot`, `click`, `type`, `eval`, `screenshot`, `console`, `network`,
   `wait` each return sane results against the live page.
2. `snapshot` returns element refs; a `click(ref)` on a button visibly activates it in
   the pane.
3. `eval("document.title")` returns the page title; `console()` includes a
   `console.log` the page emitted and any uncaught error.

### 14d — Claude bridge (MCP)

1. Open a Browser pane and navigate to a small test page (e.g. a local app). Run
   `claude` in a tab to open the Claude pane.
2. Open the feature-13 **MCP viewer**: a `twarp-browser` server appears, `connected`.
3. Ask Claude: "take a snapshot of the open browser page and click the Submit button."
   Claude calls `browser_snapshot` then `browser_click`; the click is **visible in the
   pane** on the user's live tab.
4. Ask Claude to read the console: `browser_console` returns the page's logs/errors.
5. Ask Claude to navigate with **no browser pane open**: `browser_navigate` opens a
   browser pane, brings twarp forward, and loads the URL.
6. Ask Claude for a screenshot: `browser_screenshot` returns an image of the current
   viewport.
7. With two browser panes open, focus one, then ask Claude to act: the tool result
   names which pane it targeted (the most-recently-focused one).

### 14e — Full-browser features

Out of scope for this feature's `merged` gate; tracked as optional/later (tabs,
history, downloads, profiles, popups). No smoke test required for 14 to ship.
