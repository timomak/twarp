# 14 — Built-in browser (Claude-debuggable) (TECH)

Implements PRODUCT.md. A `WKWebView`-backed browser pane plus an in-process MCP server
that lets the local `claude` session drive the live pane. mac-first. All references
below were verified against the tree at spec time (post-13a master); line numbers are
approximate and will drift — match by symbol name.

## Architecture overview

Three layers, sub-phased so the riskiest one gates the rest:

1. **Native embed (14a, the gate).** A `WKWebView` added as a child `NSView` of the
   `WarpHostView` (`crates/warpui/src/platform/mac/objc/host_view.m`), frame-synced to
   the warpui pane rect every layout pass, clipped, z-ordered, and hidden on tab switch
   / occlusion. This is the highest-risk piece because twarp **does not float any native
   view over its Metal drawable today** — there is no existing precedent to copy
   (verified: no `addSubview` over `WarpHostView`; file pickers use modal panels, not
   embedded subviews). Build this first behind a temporary dev entry point.
2. **Browser pane + UX (14b).** A new `LeafContents::Browser` pane type with omnibar /
   back / forward / reload / loading, opened via a menu/palette action, persisted by
   last URL — mirroring the feature-07 Claude pane's pane/BackingView/persistence
   wiring.
3. **Automation + Claude bridge (14c → 14d).** An injected-JS automation core
   (navigate/snapshot/click/type/eval/screenshot/console/network/wait) exposed first as
   an internal Rust API (14c), then as an **in-process MCP server** twarp registers with
   every `claude` spawn via `--mcp-config` (14d).

New crate: **`crates/browser`** (mac-only) holds the engine binding, the automation
core, and the MCP server. The pane view (`BrowserView`) lives in `app/` like the other
pane views, depending on `crates/browser`.

## Dependencies

Add (macOS target only):

- **`objc2-web-kit`** for `WKWebView` / `WKWebViewConfiguration` /
  `WKUserContentController` / `WKScriptMessageHandler` / `WKNavigationDelegate`. Use the
  `0.3.x` line that pairs with the workspace's existing `objc2` `0.6.3` +
  `objc2-app-kit` `0.3.2` + `objc2-foundation` `0.3` (the same framework-crate
  generation). **Verify the exact compatible version at spike time** and pin it in the
  workspace `[workspace.dependencies]`.
- No other new third-party deps. `objc2`, `objc2-app-kit`, `objc2-foundation`,
  `objc2-core-foundation`, `cocoa`, `objc` are already present
  (workspace `Cargo.toml:195-200`, `crates/warpui/Cargo.toml:65-77`).

CEF/Chromium is rejected (PLAN.md §"Engine choice rationale"): multi-process Chromium
fights twarp's single host view, helper-app code-signing collides with the bundle, and
the Rust bindings are immature.

## 14a — Native embed (the gate)

### Host-view subview

The browser pane needs a `WKWebView` positioned over the pane's rectangle. The host
view is a single full-window `WarpHostView` (an `NSView` backed by a `CAMetalLayer`,
`opaque = NO`) created in `window.m` and kept as first responder
(`host_view.m:284-305`, `window.m:762-774`). Approach:

- Add an Objective-C surface to `host_view.m` to **create / position / show-hide /
  destroy a `WKWebView` subview**, keyed by an opaque id so multiple browser panes can
  each own one:
  - `warp_host_create_webview(host) -> webview_id`
  - `warp_host_set_webview_frame(host, webview_id, NSRect)` — `setFrame:` + ensure
    `clipsToBounds = YES` on a container so the page never paints outside the pane.
  - `warp_host_set_webview_hidden(host, webview_id, BOOL)` — `setHidden:` for tab
    switch / occlusion.
  - `warp_host_load_url(host, webview_id, NSString*)` / `goBack` / `goForward` /
    `reload` / `stopLoading`.
  - `warp_host_destroy_webview(host, webview_id)` — `removeFromSuperview` + release.
- These mirror the existing FFI idiom: Rust declares `extern "C"`/`extern "C-unwind"`
  functions and the objc layer calls back for layout, exactly as
  `warp_ime_position(object, content_rect) -> NSRect` does
  (`window.rs:1257-1283`), using `RectFExt::to_ns_rect` /
  `Vector2FExt::to_ns_point` (`geometry.rs:1-27`) to convert warpui `RectF` → `NSRect`.

### Frame sync, z-order, clipping, occlusion

- **Frame sync:** the `BrowserView` reports its laid-out content rect each render; a
  warpui→objc call sets the webview frame. Hook the same resize/layout path the window
  already uses (`window.rs` view-size callback near `warp_view_set_frame_size`,
  ~`1286`). Convert warpui's top-left coords to AppKit's bottom-left as
  `warp_ime_position` does.
- **Z-order:** `addSubview:` places the webview above the `CAMetalLayer`. Because the
  Metal layer is `opaque = NO`, twarp chrome that must sit *above* the page (none in v1
  — the omnibar is drawn in warpui *outside* the page rect) stays correct as long as the
  webview frame is the page sub-rect only, never the whole pane.
- **Clipping:** wrap the `WKWebView` in a clipping container `NSView`
  (`clipsToBounds = YES`) sized to the page rect; rounded-corner/border chrome stays in
  warpui.
- **Occlusion:** hide the webview when (a) the pane's tab is not active, (b) the pane is
  not visible in its split, (c) the window `occlusionState` drops
  `NSWindowOcclusionStateVisible`, or (d) the window miniaturizes. The `BrowserView`
  already knows its own visibility from the render pass (it stops being rendered); drive
  `set_webview_hidden(true)` when render stops and `false` when it resumes. Add a
  window-occlusion observer if the render-pass signal proves insufficient.

### Focus & keyboard routing

`WarpHostView` is the permanent first responder and routes all key/mouse events to Rust
(`host_view.m:157-262`, `NSTextInputClient` at `402-519`). A child `WKWebView` manages
its own first-responder/IME. Plan:

- On mouse-down inside the page rect, let the `WKWebView` become first responder (native
  default) so the page gets keys.
- twarp global shortcuts (switch pane/tab) must still fire: keep them on a key-equivalent
  / menu path that works regardless of first responder, or intercept at the
  `NSWindow`/app level before the webview consumes them.
- On pane/tab switch away, resign the webview's first responder back to `WarpHostView`.
- **This is the second-highest risk after compositing — validate in the 14a smoke
  test** (click into page → type; then Cmd-switch panes → shortcut still works).

## 14b — Browser pane + UX

### New pane type (mirror `ClaudeCode`)

Add `LeafContents::Browser(BrowserPaneSnapshot)` (`app/src/app_state.rs:721-750`,
alongside `ClaudeCode(ClaudeCodePaneSnapshot)` ~742) where
`BrowserPaneSnapshot { url: Option<String> }`. Every match site the Claude pane touches
gains a `Browser` arm (verified list):

| File | Symbol | Arm |
|------|--------|-----|
| `app/src/app_state.rs` | `LeafContents` enum (~721) | `Browser(BrowserPaneSnapshot)` |
| `app/src/app_state.rs` | `is_persisted()` (~763) | persist iff `snapshot.url.is_some()` (mirror Claude's `session_id.is_some()`) |
| `app/src/pane_group/mod.rs` | `restore_pane_leaf()` (~1550) | rebuild `BrowserPane::new_restore(url)` (template ~1796-1826) |
| `app/src/persistence/sqlite.rs` | `save_pane_state()` kind match (~1030) | kind string `BROWSER_PANE_KIND` |
| `app/src/persistence/sqlite.rs` | `save_pane_state()` content match (~1075) | insert `browser_panes(url)` row (template ~1289-1298) |
| `app/src/persistence/sqlite.rs` | `read_node()` (~2551) | reconstruct `LeafContents::Browser` from row |
| `crates/persistence/src/model.rs` | constants (~531-574) | `pub const BROWSER_PANE_KIND: &str = "browser"` |
| `app/src/launch_configs/launch_config.rs` | `pane_kind()` (~140) | `Browser` arm |
| `app/src/tab_configs/session_config.rs` | `tab_config_pane_type()` (~284) | `Browser` arm |
| `app/src/pane_group/pane/mod.rs` | `IPaneType` (~131) + `Display` (~154) + `PaneId` ctors (~179) | `Browser` variant + `from_browser_pane_{ctx,view}` |
| `app/src/pane_group/pane/mod.rs` | module decl | `pub(super) mod browser_pane;` |

Add a new DB table `browser_panes(node_id, url)` (migration), parallel to
`claude_code_panes`.

### Views

- **`crates/browser`**: `BrowserEngine` — owns a webview id, exposes
  `load(url)`, `back()`, `forward()`, `reload()`, `stop()`, `current_url()`,
  `title()`, `can_go_back/forward()`, plus the automation API (14c). It talks to the
  host-view FFI (14a) and receives navigation/title/loading callbacks via a
  `WKNavigationDelegate` bridged back to Rust (an `extern "C-unwind"` callback per
  event, same pattern as the input callbacks).
- **`app/src/browser_view.rs`**: `BrowserView : BackingView` — renders the warpui chrome
  (omnibar + back/forward/reload/loading) above the page rect, owns the
  `BrowserEngine`, and reports the page rect for frame-sync. Implement the `BackingView`
  trait (`app/src/pane_group/pane/mod.rs:940-1034`): `render_header_content`,
  `focus_contents`, `close` (destroy webview), `set_focus_handle`, custom-action enum
  for toolbar buttons. Model it on `ClaudeCodeView`
  (`app/src/claude_code_view.rs:5573+`).
- **`app/src/pane_group/pane/browser_pane.rs`**: `BrowserPane : PaneContent`
  (`mod.rs:521-593`) wrapping `ViewHandle<PaneView<BrowserView>>` — mirror
  `claude_code_pane.rs:25-257` (`new`, `new_restore`, `from_view`, `attach`/`detach`,
  `snapshot`, `cwd` if relevant).

### Opening the pane

Add a `WorkspaceView::open_browser_pane(url: Option<String>, ctx)` mirroring
`open_claude_code_pane` (`app/src/workspace/view.rs:12801-12820`): create
`BrowserPane::new(url, ctx)` and `replace_pane`/split. Wire a command-palette / menu
action **"New Browser Pane"** + default keybinding to dispatch it. No terminal-command
trigger in v1 (PRODUCT §1 rationale), so `terminal/input.rs` is **not** touched —
unlike feature 07.

## 14c — Automation core (injected JS)

Port the cmux / vercel-labs `agent-browser` model (`.external/cmux`
`Sources/Panels/BrowserAutomation.swift`):

- Install a **content script** via `WKUserContentController.addUserScript` at document
  start, plus a `WKScriptMessageHandler` for page→host messages
  (console/network/errors).
- Drive actions with `evaluateJavaScript:` / `callAsyncJavaScript:completionHandler:`.
  **All WKWebView calls run on the main thread** (see Threading); `callAsyncJavaScript`'s
  completion resolves the awaiting Rust future so the background caller never blocks the
  main thread (cmux's "webkit waits off main" rule —
  `.external/cmux/.github/review-bot-rules/browser-automation-webkit-waits-off-main.md`).

Internal Rust API on `BrowserEngine` (all `async`, returning structured results):

- `navigate(url)` — load + await navigation settle.
- `snapshot() -> Snapshot` — DOM + a11y tree with **stable element refs** (assign each
  candidate element a `data-twarp-ref` / id map the script maintains; refs invalidate on
  next snapshot or navigation).
- `click(ref)`, `type(ref, text)` — resolve ref → element, dispatch realistic events.
- `eval(js) -> Value` — return JSON-serializable result or a structured error.
- `screenshot() -> Png` — `WKWebView.takeSnapshot(with:completionHandler:)`.
- `console() -> Vec<ConsoleEntry>` — buffered from the injected `console`/`onerror`
  overrides (ring buffer, capped).
- `network() -> Vec<NetEntry>` — buffered from injected `fetch`/`XHR` wrappers (best
  effort; see PRODUCT "Known limitations" — document the gaps honestly in the tool
  output).
- `wait(WaitSpec) -> ()` — selector-appears / nav-settles / timeout.

14c ships with `crates/browser` unit/integration coverage where feasible (snapshot ref
assignment, console/network buffering parsing) and is exercised via a dev harness; it is
**not** user-visible alone, so per `twarp_bundle_when_not_testable` it may be bundled
with 14b or 14d if the standalone smoke test is thin.

## 14d — Claude bridge (in-process MCP server)

### Server

Run a **workspace-level singleton MCP server** in-process (mac-only), started with the
workspace, bound to **loopback** (`127.0.0.1:<ephemeral port>`). It exposes the
`browser_*` tools from §5 of PRODUCT.

- **Transport:** local **HTTP + SSE** MCP server (the shape `claude --mcp-config`
  accepts for non-stdio servers: `{ "type": "sse"|"http", "url": "http://127.0.0.1:<port>/sse" }`).
  Reuse `crates/jsonrpc` (`service.rs`, `transport.rs`) for the JSON-RPC core; the new
  work is the MCP method set (`initialize`, `tools/list`, `tools/call`) and an
  SSE/HTTP `Transport` impl. The existing `jsonrpc` crate already powers LSP
  (`crates/lsp/src/service.rs`), so the request/response plumbing is proven.
  - **Fallback** if loopback HTTP/SSE proves fiddly with the CLI: a tiny **stdio helper
    binary** twarp spawns (`{ "command": ... }` form) that proxies to the in-process
    server over a unix socket. Costs an extra signed binary in the bundle — prefer
    in-process HTTP/SSE.
- **Why in-process, not a separate engine:** the server must drive the *same* webview
  the user sees. Living in twarp's process, each `tools/call` marshals to the main
  thread and calls `BrowserEngine` directly on the resolved target pane.
- **Target resolution:** the server holds a weak registry of open `BrowserView`s
  (registered on pane attach, deregistered on close) ordered by last-focus. Tools act on
  the most-recently-focused one; `browser_navigate` with an empty registry dispatches
  `open_browser_pane` (bringing twarp forward) and then navigates. Each tool result
  states the resolved target.

### Wiring it into `claude`

Thread an MCP config through the spawn path so **every** `claude` session gets the
`twarp-browser` server (auto-discovery, PRODUCT §5):

- `SpawnOptions` (`crates/claude_code/src/driver.rs:84-107`): add
  `mcp_config: Option<String>` (a file path or inline JSON).
- `spawn_session` (`driver.rs:132-215`): if set, `cmd.arg("--mcp-config").arg(config)`.
- `LaunchOptions` (`crates/claude_code/src/launch.rs:22-34`) + `parse_launch_args`
  (`launch.rs:63-114`): no user flag needed — twarp injects the config itself.
- `ClaudeCodeView::spawn_session` (`app/src/claude_code_view.rs:2395-2412`): build the
  `--mcp-config` JSON from the running server's URL and set `opts.mcp_config`. The
  server must be up *before* the first claude spawn — hence the workspace singleton.
- The server appears in the session init's `mcp_servers` list, so the **feature-13 MCP
  viewer shows `twarp-browser` with no special-casing** (PRODUCT §6).

### Tool schema

Tools mirror the `chrome-devtools` MCP names/shapes Claude already knows
(`browser_navigate`, `browser_snapshot`, `browser_click`, `browser_type`,
`browser_eval`, `browser_screenshot`, `browser_console`, `browser_network`,
`browser_wait`). Inputs/outputs are JSON; `browser_screenshot` returns an image content
block; errors are MCP tool errors with actionable messages (stale ref → "re-snapshot").

## Threading & safety

- **Main-thread WKWebView.** All `WKWebView` construction and API calls (load, eval,
  snapshot, takeSnapshot) must run on the AppKit main thread. The MCP server runs on a
  background async task; each operation marshals to the main thread via the app's
  main-thread executor / `callback_dispatcher` and awaits the async-JS completion — never
  blocking the main thread waiting on JS (the cmux off-main rule).
- **Loopback only.** The MCP server binds `127.0.0.1`; it is not reachable off-box. It
  drives a browser and evaluates arbitrary JS in the user's pages, so it must never
  listen on a routable interface.
- **Feature flag.** Gate the whole feature (pane type + auto-injected MCP server) behind
  a `FeatureFlag` during rollout (`/add-feature-flag`), since 14d changes *every* claude
  session by adding a server. Promote per `/promote-feature` once stable.

## Per-crate change matrix

| Crate / file | Change | Risk |
|--------------|--------|------|
| `crates/browser` (new, mac-only) | `WKWebView` binding, `BrowserEngine`, automation core, MCP server | High (new native surface) |
| `crates/warpui/.../host_view.{h,m}` | webview create/frame/hide/destroy + nav delegate callbacks | High (no existing over-Metal subview) |
| `crates/warpui/.../window.rs` | FFI decls + layout/occlusion hooks | Medium |
| workspace `Cargo.toml` | add `objc2-web-kit` | Low |
| `app/src/browser_view.rs` (new) | `BrowserView : BackingView` + omnibar/nav chrome | Medium |
| `app/src/pane_group/pane/browser_pane.rs` (new) | `BrowserPane : PaneContent` | Medium |
| `app/src/app_state.rs`, `pane_group/mod.rs`, `persistence/sqlite.rs`, `crates/persistence/src/model.rs`, `launch_config.rs`, `session_config.rs`, `pane_group/pane/mod.rs` | `Browser` variant + match arms + persistence table | Medium (many sites, each mechanical) |
| `app/src/workspace/view.rs`, app menus / keybindings | `open_browser_pane` + "New Browser Pane" action | Low |
| `crates/claude_code/src/driver.rs`, `launch.rs`; `app/src/claude_code_view.rs` | `mcp_config` on `SpawnOptions`, `--mcp-config` injection | Low |
| `crates/jsonrpc` | reused; possibly an SSE/HTTP `Transport` impl | Medium |

## Sub-phasing

Each sub-phase is a reviewable PR; **14a is a hard gate** (if native compositing/focus
can't be made correct, the whole approach is reconsidered before investing in 14b+).

- **14a** — native embed spike (compositing + focus). Ships behind a dev entry point.
- **14b** — browser pane + UX + persistence.
- **14c** — automation core (internal Rust API). May bundle with 14b or 14d if its
  standalone smoke test is thin (`twarp_bundle_when_not_testable`).
- **14d** — MCP server + `--mcp-config` injection (the acceptance bar: Claude drives the
  live pane via MCP). Feature-flag-gated.
- **14e** — full-browser features (tabs/history/downloads/profiles/popups). Optional /
  later; **not required for feature 14 to reach `merged`.**

## Risks / caveats

- **Native view over the Metal drawable (14a).** No existing precedent in twarp; the
  whole window is one `CAMetalLayer`-backed `WarpHostView`. Compositing, clipping,
  per-tab occlusion, and resize sync are unproven here — the gate.
- **Focus/keyboard routing** between `WarpHostView` (permanent first responder) and a
  child `WKWebView` (own IME/first responder) — global shortcuts must survive page focus.
- **Network fidelity** is the known soft spot (WKWebView has no network API); v1 is
  best-effort in-page hooks and must say so in tool output.
- **Always-injected MCP server** changes every claude session — feature-flag it, and
  ensure a dead/absent server degrades gracefully (claude shows it `failed` in the
  viewer, other tools unaffected) rather than wedging the session.
- **Main-thread discipline** — a blocking wait on JS from the main thread will beachball
  twarp (cf. `twarp_07_focus_loop_beachball`); the async-JS marshaling must be strict.
- **Presubmit is manual-verify** for the native/UI surface on this Mac
  (`twarp_presubmit_tooling`); the PRODUCT smoke test per sub-phase is the gate.
