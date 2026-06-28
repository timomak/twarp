# 14 — Built-in browser (Claude-debuggable)

**Phase:** spec-in-review
**Spec PR:** [#96](https://github.com/timomak/twarp/pull/96)
**Impl PRs:** —

Specs: [PRODUCT.md](PRODUCT.md) · [TECH.md](TECH.md) (pre-spec direction in [PLAN.md](PLAN.md)).

## Scope

A built-in browser pane modeled on [cmux](https://github.com/manaflow-ai/cmux): functional for the user, and scriptable so the Claude CLI session can drive the *same live tab the user sees* to debug the user's UI. Engine is `WKWebView` (via `objc2-web-kit`) embedded as a child `NSView` over the Metal host view; agent control is an injected-JS automation bridge exposed as an MCP server. mac-first. v1 = dev-preview pane (single tab + navigate + automation).

See [PLAN.md](PLAN.md) for the full pre-spec plan, engine rationale, and risks.

## Why this slot

Net-new feature, owner-requested. Independent of the IDE-pivot features (10–12) and the MCP viewer (13). Sequenced after the active queue; placement vs. the rebrand (09) to be decided when it goes active — the browser is a new crate/pane and touches little upstream-divergent code, so it carries low cherry-pick risk either way.

## Sub-phases (from PLAN.md)

- [ ] **14a — Native embed spike.** `WKWebView` rendering in a twarp pane, frame-synced to the warpui rect; correct clipping, z-order, occlusion-on-tab-switch, focus + keyboard routing. **Highest-risk gate.**
- [ ] **14b — Minimal browser UX.** Omnibar (navigate), back/forward/reload, loading state, single tab, open trigger (mirrors feature-07 pane wiring).
- [ ] **14c — Automation core.** Injected content script: navigate/snapshot/click/type/eval + console + network capture. Internal Rust API.
- [ ] **14d — Claude bridge.** Expose the automation core as an MCP server twarp registers for the session. Acceptance: Claude CLI drives the live pane via MCP tools. Not "done" until MCP-exposed.
- [ ] **14e — Full-browser features (optional/later).** Tabs, history, downloads, profiles/cookies, popups.

## What's already built

- Native host view + Metal-overlay pattern: `crates/warpui/src/platform/mac/objc/host_view.m`
- `objc2` / `objc2-app-kit` / `cocoa` / `objc` deps already in the workspace
- Pane + terminal-trigger precedent: feature 07 (Claude Code pane)
- cmux reference clone in gitignored `.external/cmux`

## Notes

- **Network inspection is the weak spot** (WKWebView has no first-class network API) — inject `fetch`/`XHR` hooks or a proxy (cmux uses `BrowserSystemProxyMirror`). Budget extra.
- CEF rejected: multi-process Chromium + helper-app code-signing collide with twarp's bundle/signing; immature Rust bindings.
- Back-pocket alternative if Chrome-parity debugging ever outranks shared-session: out-of-process Chrome driven by the existing `chrome-devtools` MCP.
