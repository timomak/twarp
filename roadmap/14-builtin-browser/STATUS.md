# 14 — Built-in browser (Claude-debuggable)

**Phase:** merged — 14f fixed + merged 2026-07-01 (fleet/14f); phase 3 (14g–14l) merged 2026-07-09
**Spec PR:** [#96](https://github.com/timomak/twarp/pull/96) (merged)
**Impl PRs:** 14a [#111](https://github.com/timomak/twarp/pull/111), 14b [#112](https://github.com/timomak/twarp/pull/112), 14c [#113](https://github.com/timomak/twarp/pull/113), 14d [#114](https://github.com/timomak/twarp/pull/114), 14e [#115](https://github.com/timomak/twarp/pull/115)

## ⚠️ Reopened 2026-07-01 — feature is NOT functional

A live computer-use UX drive on other-mac (evidence: `fleet/runs/ux_14-ux-verify/`, VERDICT
**regression**) found the browser pane **loads pages but never renders them**: submitting a URL
navigates at the state layer (tab/header/omnibar update to the fetched page's `<title>`, proving the
WKWebView fetches + parses), but the **content area stays solid black** through load, Reload, and full
re-layout — the webview never composites/paints into the pane. The omnibar chrome and clipping (14a)
are correct; the compositing/paint path is broken.

This slipped through because all of 14a–14e were queued with `ux: False` and only ever saw the
bootstrap-screenshot gate, never a live drive.

- [ ] **14f — Fix WKWebView paint.** The child-`NSView`/Metal-overlay webview must actually composite
  its page content into the pane rect (not just navigate). Re-verify with a `ux:true` gate that
  `example.com` renders visibly.

Specs: [PRODUCT.md](PRODUCT.md) · [TECH.md](TECH.md) (pre-spec direction in [PLAN.md](PLAN.md)).

## Scope

A built-in browser pane modeled on [cmux](https://github.com/manaflow-ai/cmux): functional for the user, and scriptable so the Claude CLI session can drive the *same live tab the user sees* to debug the user's UI. Engine is `WKWebView` (via `objc2-web-kit`) embedded as a child `NSView` over the Metal host view; agent control is an injected-JS automation bridge exposed as an MCP server. mac-first. v1 = dev-preview pane (single tab + navigate + automation).

See [PLAN.md](PLAN.md) for the full pre-spec plan, engine rationale, and risks.

## Why this slot

Net-new feature, owner-requested. Independent of the IDE-pivot features (10–12) and the MCP viewer (13). Sequenced after the active queue; placement vs. the rebrand (09) to be decided when it goes active — the browser is a new crate/pane and touches little upstream-divergent code, so it carries low cherry-pick risk either way.

## Sub-phases (from PLAN.md)

- [x] **14a — Native embed spike.** `WKWebView` rendering in a twarp pane, frame-synced to the warpui rect; correct clipping, z-order, occlusion-on-tab-switch, focus + keyboard routing. **Highest-risk gate.**
- [x] **14b — Minimal browser UX.** Omnibar (navigate), back/forward/reload, loading state, single tab, open trigger (mirrors feature-07 pane wiring).
- [x] **14c — Automation core.** Injected content script: navigate/snapshot/click/type/eval + console + network capture. Internal Rust API.
- [x] **14d — Claude bridge.** Expose the automation core as an MCP server twarp registers for the session. Acceptance: Claude CLI drives the live pane via MCP tools. Not "done" until MCP-exposed.
- [x] **14e — Full-browser features (optional/later).** Tabs, history, downloads, profiles/cookies, popups.

## Phase 3 (owner-directed 2026-07-09) — polish, reliability, shared control

Driven end-to-end in one session from owner notes + a live MCP test drive. All merged:

- [x] **14g — Tab/omnibar polish** ([#171](https://github.com/timomak/twarp/pull/171)): tab close X right-aligned (SpaceBetween); non-URL omnibar input falls back to a Google search.
- [x] **14h — Pane lifecycle** ([#172](https://github.com/timomak/twarp/pull/172)): `PendingTabRestore` retry loop — the pane survives session restore and cross-window transfer instead of dying when `BrowserEngine::new` returns `None` (window not registered yet); all tabs preserved on transfer; explicit "Reconnecting…" body. Fixes the PR #144 restore-blank bug too.
- [x] **14i — Focus isolation** ([#173](https://github.com/timomak/twarp/pull/173)): automation never steals the keyboard (native first-responder guard around evals; MCP opens panes unfocused; no `show_window_and_focus_app`). Companion fix: `DetachType::Moved` no longer destroys webviews (same-window pane moves).
- [x] **14j — Per-session scoping** ([#174](https://github.com/timomak/twarp/pull/174)): one SSE endpoint per Claude session; tools bind per connection to a `BrowserView` `EntityId` (survives moves, ignores focus); `browser_navigate` opens panes in the invoking session's tab.
- [x] **14k — Automation robustness** ([#175](https://github.com/timomak/twarp/pull/175)): CSP-proof `browser_eval` (native expression eval, not page-world string-eval); `browser_snapshot` selector/max_elements filters; new verbs hover/select/scroll/back/forward/close. (Isolated-`WKContentWorld` split deferred — no driving bug once eval went native.)
- [x] **14l — Shared control** ([#176](https://github.com/timomak/twarp/pull/176) + annotations PR): agent input lease — while Claude drives, a native shield locks the *page* (chrome stays interactive), banner + "Take control"; lease renews per tool call, expires 4s idle. Annotation mode: toolbar button arms one click → element under it (selector/name via `describePoint`) pre-fills the bound Claude session's composer for the user's note. Idle panes browse normally; annotate works in both modes.

Known follow-ups (not scheduled): per-session server teardown on session end (tiny leak: one localhost listener per session per app run); isolated content world for tamper-proofing; annotation screenshot crops; richer network capture (subresources/WebSocket).

## What's already built

- Native host view + Metal-overlay pattern: `crates/warpui/src/platform/mac/objc/host_view.m`
- `objc2` / `objc2-app-kit` / `cocoa` / `objc` deps already in the workspace
- Pane + terminal-trigger precedent: feature 07 (Claude Code pane)
- cmux reference clone in gitignored `.external/cmux`

## Notes

- **Network inspection is the weak spot** (WKWebView has no first-class network API) — inject `fetch`/`XHR` hooks or a proxy (cmux uses `BrowserSystemProxyMirror`). Budget extra.
- CEF rejected: multi-process Chromium + helper-app code-signing collide with twarp's bundle/signing; immature Rust bindings.
- Back-pocket alternative if Chrome-parity debugging ever outranks shared-session: out-of-process Chrome driven by the existing `chrome-devtools` MCP.
