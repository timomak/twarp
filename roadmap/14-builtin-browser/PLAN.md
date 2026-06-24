# Feature 14 — Built-in browser (Claude-debuggable)

> Pre-spec plan. The full `PRODUCT.md` / `TECH.md` are written via the spec
> workflow before implementation. This file captures the agreed direction so it
> survives across sessions.

## Goal

Add a built-in browser pane to twarp, modeled on [cmux](https://github.com/manaflow-ai/cmux)
(reference clone in gitignored `.external/cmux`). It must be:

1. **Functional for the user** — a real browser pane they can navigate and use.
2. **Scriptable for the Claude CLI session** — Claude can drive the *same live
   tab the user sees* to debug the user's UI (DOM, console, interaction,
   network), exposed as MCP tools so it's effortless.

## Decided direction (owner-confirmed 2026-06-24)

- **Engine: `WKWebView` via `objc2-web-kit`.** Native macOS WebKit, embedded as a
  child `NSView` floated over the Metal host view (`WarpHostView`,
  `crates/warpui/src/platform/mac/objc/host_view.m`). This is the same
  native-layer-over-Metal pattern twarp already uses, and twarp already depends
  on `objc2` / `objc2-app-kit` / `cocoa` / `objc`.
- **Not CEF.** Chromium's multi-process model + per-helper code-signing collides
  with twarp's bundle/signing (`WarpOss.app`), and the binary cost (~150MB+) and
  patch-treadmill aren't worth it. cmux itself uses WKWebView.
- **Agent debugging: injected-JS automation bridge exposed as an MCP server.**
  cmux's proven model (a port of vercel-labs/agent-browser): inject a content
  script, drive via `evaluateJavaScript` / `callAsyncJavaScript`, capture
  console + network. The Claude CLI calls MCP tools natively — the same pattern
  it already uses with the `chrome-devtools` MCP.
- **Shared session.** Claude inspects the exact pixels/tab the user sees, not a
  parallel headless instance. This is the differentiator.
- **mac-first.** Consistent with twarp's native chrome.
- **v1 scope: dev-preview pane** (single tab + navigate + automation). Grow toward
  a full browser later.

### Engine choice rationale (rejected alternatives)

- **CEF / Chromium in-process** — ultimate Chrome-parity + first-class CDP, but
  immature Rust bindings, multi-process Chromium fights twarp's single host view,
  and helper-app code-signing collides with the bundle. Worst Rust fit.
- **Out-of-process Chrome + existing `chrome-devtools` MCP** — effortless for
  Claude *today*, full network fidelity, zero bridge to build — but it's a
  *parallel* Chrome session, not the user's actual tab, and a second engine.
  Kept in back pocket if Chrome-parity debugging ever outranks shared-session.

## Fidelity note

Via injected JS, these are near-CDP quality: DOM/a11y snapshot with element refs,
console logs + uncaught errors, `eval`, click/type/hover, screenshots,
wait-for-condition.

**Network inspection is the weak spot** — WKWebView has no first-class network
observation API. Approach parity by injecting `fetch`/`XHR` hooks or running a
proxy (cmux uses `BrowserSystemProxyMirror`). Budget extra effort here.

"Effortless for Claude" **requires the MCP wrapper** — a raw socket bridge alone
is not enough; the session must auto-discover browser tools.

## Phases (spec-first, each a reviewable PR)

1. **Native embed spike** — `WKWebView` rendering inside a twarp pane,
   frame-synced to the warpui rect every layout pass, with correct clipping,
   z-order, occlusion-on-tab-switch, and focus + keyboard routing between warpui
   and the webview. **Highest risk — gate before anything else.**
2. **Minimal browser UX** — omnibar (URL / navigate), back / forward / reload,
   loading state, single tab, and a trigger to open the pane (mirrors the
   feature-07 `claude` pane wiring: a `LeafContents` + command/trigger).
3. **Automation core** — inject content script: `navigate`, `snapshot`
   (DOM + a11y tree with stable element refs), `click`, `type`, `eval`, plus
   console + network capture. Internal Rust API only.
4. **Claude bridge** — **not "done" until exposed as an MCP server twarp
   registers for the session.** Acceptance: the Claude CLI drives the live pane
   via MCP tools (snapshot / click / console / eval / screenshot) with no manual
   socket poking. Pipe console + network logs back.
5. **Full-browser features (optional / later)** — multiple tabs, history,
   downloads, profiles/cookies, popup handling.

## Key risks

- Native-view compositing/clipping/occlusion over the Metal drawable (phase-1 gate).
- Focus + keyboard routing between warpui and `WKWebView`.
- Network inspection fidelity (the soft spot vs CDP).
- Process/security isolation for the automation surface.

## References

- cmux WKWebView impl: `.external/cmux/Sources/Panels/CmuxWebView.swift`,
  `BrowserPanel*.swift`, `BrowserAutomation.swift`, `BrowserSystemProxyMirror.swift`,
  `AgentSessionWeb*`.
- twarp native host view: `crates/warpui/src/platform/mac/objc/host_view.m`.
- Pane + trigger wiring precedent: feature 07 (Claude Code pane).
