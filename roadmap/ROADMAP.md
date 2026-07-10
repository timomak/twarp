# twarp roadmap

Single source of truth for what's being built next. `/twarp-next` reads this file every invocation; the user reads it to see status at a glance.

**Currently active:** `11-git-blame`
**Next up:** `10-file-editor` (resumes) — or owner direction

## Features

| # | Feature | Phase | Spec PR | Impl PR(s) |
|---|---------|-------|---------|-----------|
| 01 | [Tab color shortcuts](01-tab-colors/STATUS.md) | merged | [#2](https://github.com/timomak/twarp/pull/2) | [#3](https://github.com/timomak/twarp/pull/3) |
| 02 | [AI removal](02-ai-removal/STATUS.md) | merged | [#4](https://github.com/timomak/twarp/pull/4) | [#6](https://github.com/timomak/twarp/pull/6), [#7](https://github.com/timomak/twarp/pull/7), [#9](https://github.com/timomak/twarp/pull/9), [#10](https://github.com/timomak/twarp/pull/10), [#11](https://github.com/timomak/twarp/pull/11), [#12](https://github.com/timomak/twarp/pull/12), [#13](https://github.com/timomak/twarp/pull/13), [#14](https://github.com/timomak/twarp/pull/14), [#15](https://github.com/timomak/twarp/pull/15), [#16](https://github.com/timomak/twarp/pull/16), [#17](https://github.com/timomak/twarp/pull/17), [#18](https://github.com/timomak/twarp/pull/18) |
| 03 | [Render markdown by default](03-md-rendered/STATUS.md) | merged | [#49](https://github.com/timomak/twarp/pull/49) | [#50](https://github.com/timomak/twarp/pull/50) |
| 04 | [Custom command shortcuts](04-command-shortcuts/STATUS.md) | merged | [#51](https://github.com/timomak/twarp/pull/51) | 4a [#52](https://github.com/timomak/twarp/pull/52), 4b [#53](https://github.com/timomak/twarp/pull/53), 4c [#54](https://github.com/timomak/twarp/pull/54), 4d [#55](https://github.com/timomak/twarp/pull/55) |
| 05 | [Open Changes panel](05-open-changes/STATUS.md) | merged | [#56](https://github.com/timomak/twarp/pull/56), respec [#58](https://github.com/timomak/twarp/pull/58) | 5a [#59](https://github.com/timomak/twarp/pull/59), 5c+5e [#60](https://github.com/timomak/twarp/pull/60), 5e polish [#61](https://github.com/timomak/twarp/pull/61), 5b [#62](https://github.com/timomak/twarp/pull/62), 5d [#63](https://github.com/timomak/twarp/pull/63) |
| 06 | [Tab rename shortcut](06-tab-rename/STATUS.md) | merged | [#64](https://github.com/timomak/twarp/pull/64) | [#65](https://github.com/timomak/twarp/pull/65) |
| 07 | [Claude Code panel](07-claude-code-panel/STATUS.md) | reopened (7p impl-in-review — attention notifications + tab status dot, owner-directed 2026-07-07) | [#66](https://github.com/timomak/twarp/pull/66), respec [#68](https://github.com/timomak/twarp/pull/68), main-pane respec [#70](https://github.com/timomak/twarp/pull/70), 7i amendment [#77](https://github.com/timomak/twarp/pull/77) (folded into #76), phase-2 specs [#78](https://github.com/timomak/twarp/pull/78) | 7b [#69](https://github.com/timomak/twarp/pull/69) (merged — sidebar host, superseded), 7b [#71](https://github.com/timomak/twarp/pull/71) (merged — main-pane host), 7c [#72](https://github.com/timomak/twarp/pull/72) (merged — live driver), polish [#74](https://github.com/timomak/twarp/pull/74) (merged — shell polish + metadata chips; superseded auto-closed #73), 7d [#75](https://github.com/timomak/twarp/pull/75) (merged — tool cards + review feedback), 7e–7j bundled [#76](https://github.com/timomak/twarp/pull/76) (merged), phase 2 7k–7n bundled [#79](https://github.com/timomak/twarp/pull/79) (merged — streaming, rich input, composer controls, plan rendering) |
| 08 | [macOS-style UI overhaul](08-macos-ui/STATUS.md) | merged | #81 | #81 (spec+impl bundled, owner-directed) |
| 09 | [Rebrand to twarp](09-rebrand/STATUS.md) | merged | — | — |
| 10 | [File editor with go-to-definition](10-file-editor/STATUS.md) | merged | [#155](https://github.com/timomak/twarp/pull/155) | — |
| 11 | [Git blame](11-git-blame/STATUS.md) | not-started | — | — |
| 12 | [Project search & replace](12-project-search-replace/STATUS.md) | not-started | — | — |
| 13 | [MCP viewer in Claude pane](13-mcp-viewer/STATUS.md) | merged | [#91](https://github.com/timomak/twarp/pull/91) | 13a [#95](https://github.com/timomak/twarp/pull/95) |
| 14 | [Built-in browser (Claude-debuggable)](14-builtin-browser/STATUS.md) | merged (phase 3 14g–14l shipped 2026-07-09) | [#96](https://github.com/timomak/twarp/pull/96) | 14a [#111](https://github.com/timomak/twarp/pull/111), 14b [#112](https://github.com/timomak/twarp/pull/112), 14c [#113](https://github.com/timomak/twarp/pull/113), 14d [#114](https://github.com/timomak/twarp/pull/114), 14e [#115](https://github.com/timomak/twarp/pull/115), 14f (fleet), 14g–14l [#171](https://github.com/timomak/twarp/pull/171)–[#178](https://github.com/timomak/twarp/pull/178) |
| 15 | [Computer control overlay (Claude drives the Mac)](15-computer-control/STATUS.md) | merged | — | — |
| 16 | [Agent settings page](16-agent-settings/STATUS.md) | merged | — | — |

## Phases

- `not-started` — no work begun
- `spec-pending` — `/twarp-next` is writing PRODUCT.md / TECH.md
- `spec-in-review` — spec PR open, awaiting user review + merge
- `impl-pending` — specs merged, `/twarp-next` is implementing the next sub-phase
- `impl-in-review` — impl PR open, awaiting user review + merge
- `merged` — feature shipped

## Rules

- Only one feature is active at a time.
- A feature advances from `spec-in-review` → `impl-pending` only after the spec PR is **merged to master**.
- Features 02, 05, 07, 08, 09, 10, 11, and 12 are sub-phased; their STATUS.md tracks individual sub-PRs and the feature only reaches `merged` after every sub-PR ships.
- The next feature only starts after the current one reaches `merged`.
- Git is the source of truth. If STATUS.md and `gh pr view` disagree, trust git and update STATUS.md.

## Order rationale

1. **Tab colors first** — smallest scope, validates the workflow at low risk; upstream has groundwork on `oz-agent/APP-4321-active-tab-color-indication`.
2. **AI removal second** — establishes the fork's identity. Cherry-pick conflicts from upstream become unavoidable from here, so eat the cost after the workflow is proven.
3. **Render markdown by default third** — small default flip on whatever surface(s) twarp uses to display `.md` files. After AI removal so the markdown render path isn't entangled with the deleted assistant transcript renderer.
4. **Command shortcuts fourth** — independent subsystem, no dependency on 01–03.
5. **Open Changes panel fifth** — largest user-facing scope, sub-phased into panel scaffold → diffs → staging → commit/push → file timeline.
6. **Tab rename shortcut sixth** — small, isolated keyboard binding that hooks into the existing rename interaction. Sequenced here only because 03–05 were already queued; nothing about its scope blocks earlier placement, and it stays before rebrand so the rename keybinding lands in `twarp_*` crates rather than churning during 9b.
7. **Claude Code panel seventh** — large user-facing scope, sub-phased. Re-introduces Warp Agent Mode's rendering layer (removed in feature 02) as a host for the local `claude` subprocess running on the user's Claude Max subscription. No LLM client, no billing, no cloud sync — only the renderer comes back. Slotted before the rebrand because cherry-picks from upstream agent crates are much harder once every `warp_*` / `warpui*` crate has been renamed. **Phase 2 (7k–7n: streaming, rich input, composer controls, plan rendering; owner-directed 2026-06-15) stays under 07 and also precedes rebrand** — it churns the same `claude_code` / `claude_code_view` / `terminal` crates the rename touches, so doing it first pays that merge cost once.
8. **macOS-style UI overhaul eighth** — owner-requested visual pass (macOS-style sidebar restyle, Chrome-style tabs + drag-to/between-windows, Claude chat fade-out, sessions search). Slotted *before* the rebrand on the same upstream-sensitivity logic as 07: the heavy sub-phases churn the most upstream-divergent files (`app/src/tab.rs`, `app/src/workspace/view.rs`), and the cross-window-drag sub-phases (8b/8c) **port an upstream feature** (`transfer_view_tree_to_window`; commits `3984e67f`, `d7c45cab`) — doing all this before the crate-rename pass keeps cherry-picks clean. The sidebar work is a **warpui restyle that emulates the macOS look**, not native-AppKit embedding (the whole window is one Metal drawable; embedding a real `NSOutlineView` would fork focus/layout/overlays/theming — see 08's TECH.md).
9. **Rebrand last among the upstream-sensitive features** — file/crate renames are the worst case for git merges, so push them as late as possible to keep upstream cherry-picks clean. By feature 09, AI code is gone and the agent renderer + macOS UI pass are wired up, so the brand surface to rename is settled.
10. **File editor surface tenth** — pivots twarp from "terminal" to "terminal + IDE" by exposing the existing `crates/editor/` + `crates/lsp/` infrastructure as a first-class file-editing workflow. Headline gesture is cmd+click → LSP definition (already callable from `app/src/code/local_code_editor.rs`, just not wired to a workflow where you can open arbitrary files). Placed after rebrand because wiring across `app/src/code/`, `crates/editor/`, and `crates/lsp/` would otherwise be churned during the rename pass.
11. **Git blame eleventh** — depends on 10 (no blame without a file-editing surface). Genuinely net-new code: `git blame --porcelain` parser, gutter rendering, commit-detail popover. No upstream cherry-pick risk because blame is new.
12. **Project search & replace twelfth** — wires the existing `warp_ripgrep` crate into a project-wide search UI plus a replace-all flow. Independent of 10 in principle; sequenced after for result-click → open-file.
13. **MCP viewer thirteenth** — owner-requested follow-on to feature 07. Surfaces the MCP servers configured for the local `claude` CLI as a read-only view inside the Claude pane (a composer pill/popover listing connected servers + their tools). No add/edit/remove this pass — management stays in the `claude` CLI. Pulled to active ahead of 09 by owner direction (2026-06-23) because it's a small, self-contained win on the just-shipped Claude pane and touches only `claude_code*` crates the rebrand hasn't renamed yet.
14. **Built-in browser fourteenth — next up after 13 (owner-directed 2026-06-24, pulled ahead of 09).** Owner-requested. A built-in browser pane modeled on [cmux](https://github.com/manaflow-ai/cmux): `WKWebView` (via `objc2-web-kit`) embedded as a child `NSView` over the Metal host view, with an injected-JS automation bridge **exposed as an MCP server** so the Claude CLI session can debug the *same live tab the user sees* (DOM, console, interaction, network). mac-first; v1 = dev-preview pane. CEF rejected (multi-process Chromium + helper code-signing collide with the bundle). Net-new code, low cherry-pick risk; pulled ahead of the rebrand (09) and the IDE-pivot features (10–12) by owner direction. See [14-builtin-browser/PLAN.md](14-builtin-browser/PLAN.md).
15. **Computer control overlay fifteenth — next up after 14 (owner-directed 2026-06-27, pulled ahead of 09).** Owner-requested. Lets a Claude session see and control the user's Mac like Anthropic's Claude desktop app: an always-on-top corner overlay, full-screen screenshots that **exclude twarp's own chrome**, a glow border signaling capture is live (tinted to the active tab's color via `floating_panel_surface_fill`, not a fixed orange), and mouse/keyboard control in a screenshot → action → screenshot loop. Big head start — Warp's existing cross-platform `crates/computer_use` crate already provides input injection + screen capture (and `app/Cargo.toml` already depends on it), so the work is the overlay UX, self-exclusion from capture, the Claude-session agent loop, and TCC permissions + safety. Overlay chrome is plain AppKit beside warpui (the Metal-only constraint doesn't bite), so low cherry-pick risk; placement vs. the rebrand (09) decided when it goes active. See [15-computer-control/PLAN.md](15-computer-control/PLAN.md).
16. **Agent settings page sixteenth (owner-directed 2026-07-01).** A new **Agent** settings page consolidating agent config that feature 07 left scattered (composer pills, per-session SQLite defaults, hardcoded spawn flags): pick the agent CLI backend (Claude enabled; Codex/Gemini disabled entries), reuse local CLI auth or supply an API key (**net-new OS-keychain storage — twarp has none today**), set new-chat defaults (permission mode / model / effort) that seed panes, and configure a **per-action model matrix** (chat / terminal-suggestions / reply-suggestions, each with an independent provider+model+effort — owner-confirmed). Phase 1 is **Claude-only with a multi-provider, capability-aware schema**; the `CLIAgent` enum already exists as stubs, so the selector is cheap and the driver adapters are the real later work. Config UI is **unified** (the Chat matrix row IS the new-chat model/effort/mode, authoritative over the pills' last-used memory) and **also builds both suggestion consumers** (owner-expanded 2026-07-01): reply ghost-text in the composer (16e) and terminal AI command suggestions (16f), each off-by-default behind its own toggle, reusing the existing editor/terminal ghost-text infra. Sub-phased 16a–16f. Low upstream-cherry-pick risk (net-new settings surface + additive suggestion backends), so placement vs. rebrand (09) is flexible. See [16-agent-settings/PRODUCT.md](16-agent-settings/PRODUCT.md).

## Out of scope for `/twarp-next`

- **Upstream cherry-picks.** Run on a separate cadence — schedule a recurring agent (`/schedule`) to fetch, list new commits, and propose cherry-picks. Not driven by this skill.
- **CI / repo hygiene unrelated to the active feature.**

## Spec storage convention

For twarp roadmap features, specs live alongside `STATUS.md`:

```
roadmap/<NN-feature>/PRODUCT.md
roadmap/<NN-feature>/TECH.md
```

This intentionally overrides the repo's default `specs/<linear-ticket>/...` convention, because twarp roadmap features are not tracked in Linear.
