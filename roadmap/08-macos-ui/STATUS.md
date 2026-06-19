# 08 — macOS-style UI overhaul

**Phase:** impl-in-review
**Spec PR:** [#81](https://github.com/timomak/twarp/pull/81) — **owner-directed bundle: all of 8a–8f implemented in the same PR as the spec** (override of the spec-merge gate and one-sub-phase-per-PR rule, 2026-06-18; same call as the feature-07 bundling override)
**Impl PRs:** folded into #81

## Sub-phase status (all in #81)

All six implemented and integrated on `twarp-08-specs`. Combined `cargo check -p warp` is clean (0 errors/warnings); 8e's 3 unit tests pass. Drag gestures, the gradient, and the macOS look are **manual-only** surfaces — they need a launched binary to validate (this Mac can't fully run presubmit; see smoke test below).

- [x] **8a — Chrome-style tabs (look).** `tab.rs::render_tab_container_internal`: `CornerRadius::with_top(7.0)` top-rounded shape, active tab drops its bottom border to seat onto the pane; inactive tabs recessed. Feature 01 colors (`styles.background`) + feature 06 rename untouched. (commit `c13e580b`)
- [x] **8b — Drag tab → new window.** `DragTabsToWindows` added to `DOGFOOD_FLAGS`; `try_detach_tab_on_drag` detaches a multi-tab past `DETACH_SENSITIVITY` via the live view-tree transfer; origin reflow + last-tab window-close; `TabData::detached` set; snap-back below threshold. (commits `c13e580b`, superseded/extended by `64d8b90b`)
- [x] **8c — Drag tab between windows.** Full port of upstream `3984e67f` / `d7c45cab`: 1849-line `workspace::cross_window_tab_drag.rs` state machine, insertion ghost (slot + floating chip), cross-window screen-space attach targeting, and the warpui `set_window_alpha` platform chain (down to Obj-C `[window setAlphaValue:]`) for preview-hide. macOS has full `ordered_window_ids` so z-order attach works; Linux/test window-ordering bits intentionally not ported. (commit `64d8b90b`)
- [x] **8d — Claude chat fade-out.** `claude_code_view.rs`: a `COMPOSER_CLEARANCE`-tall vertical `Fill::Gradient` band (transparent → `theme.background().into_solid()`), painted between transcript and the opaque composer, no hit-testing. (commit `7c37fb59`)
- [x] **8e — Sessions search.** `claude_sessions_search: ViewHandle<EditorView>` single-line field; `filter_session_indices` case-insensitive substring on `title`, preserving original indices for resume; distinct "No matching sessions" empty state. 3 unit tests. (commit `c436143c`)
- [x] **8f — macOS sidebar restyle.** Five pinned `ColorU` consts; root `Container` fixed light fill (ignores theme); pill segmented switcher (`render_pill_segment`, same routing as old `render_button`); muted headers + soft light row hover; restyled sessions rows/footer. (commit `c436143c`)

## Known follow-ups / caveats (for the smoke pass)

- **§25 theme-leakage:** the inherited non-sessions panels (Project Explorer / Shortcuts / Timeline) still read theme-derived text/hover colors; under a **dark** terminal theme they may have low contrast against the pinned-light sidebar. Left per §25 (no bespoke re-layout this pass) — verify legibility and decide if a follow-up pins them.
- **8b single-tab detach:** dragging the *only* tab of a window off the strip uses the upstream single-tab "source-follows-cursor" path (8c brought this in); verify it doesn't flash a blank window.
- **Dead code:** old `render_button` (8f) is unreferenced but compiles clean; remove in a later cleanup if desired.
- **warpui platform changes (riskiest):** `set_window_alpha` added across `warpui_core` + `warpui/mac` + Obj-C. Verify window-alpha preview-hide looks clean and nothing else regressed window behavior.

## Scope

Make twarp look and feel like a first-class macOS app, in four owner-requested areas:

1. **macOS-style sidebar** — restyle the shared left-panel chrome (background, section headers, row style, top switcher, footer) and rework the Claude **sessions** panel to mirror the Claude macOS app. Other tool panels inherit the new chrome but aren't individually reworked this pass.
2. **Chrome-style tabs** — tabs shaped like Chrome (top-rounded), plus drag a tab **out** to a new window and **between** windows.
3. **Claude chat fade-out** — a bottom gradient mask on the Claude chat scroll area so messages fade under the floating composer.
4. **Sessions search** — a search bar over the Claude sessions history list (filter by text, like the Claude macOS app).

**Not native-component embedding.** twarp renders the whole window into a single Metal drawable via the in-house **warpui** framework (`crates/warpui`, `crates/warpui_core`) — it does **not** use GPUI, and there are no native `NSSplitView`/`NSOutlineView`/`NSTableView` in the app. Splicing a real AppKit source list into the Metal surface would fork focus, layout, overlays, and theming across two UI systems and permanently hybridize the codebase (full analysis in TECH.md). So #1 is a **warpui restyle that emulates the macOS look**, not an embedded native control.

## Owner-confirmed decisions (2026-06-18)

- **Sidebar restyle scope:** shared sidebar **chrome + the Claude sessions panel**. Other tool panels inherit the chrome only.
- **"Section toggle":** the **top tool switcher** (Project Explorer / Global Search / Warp Drive / Shortcuts / Claude Sessions) becomes a macOS **pill segmented control** (à la the Claude app's `Chat | Cowork | Code`).
- **Background:** **flat macOS-style light background** — no native vibrancy/translucency this pass.
- **Theming:** **pinned to the macOS light look regardless of the active twarp theme.** A light sidebar beside a dark terminal theme is intentional two-tone (matches the Claude-app reference), not a bug.
- **No native-embedding spike** — the restyle is the chosen path, not a fallback.

## Sub-phases

Tab work (8a–8c, the #2 cluster) runs as one consecutive block per owner direction, then the lighter Claude-pane wins, then the sidebar restyle.

- [ ] **8a — Chrome-style tabs (look).** Top-rounded tab shape + fill/border tuning in `app/src/tab.rs` (`render_tab_container_internal`). Keep existing per-tab colors (feature 01) and rename (feature 06). Low.
- [ ] **8b — Drag tab → new window.** Re-enable the `DragTabsToWindows` feature flag (`crates/warp_features`) and detach-to-new-window via `create_transferred_window` + `transfer_view_tree_to_window` (`app/src/root_view.rs`); `TabData::detached` already reserved. Medium.
- [ ] **8c — Drag tab between windows.** Cross-window drop hit-testing + insertion ghosts + drag-state machine, porting the upstream implementation (`transfer_view_tree_to_window`; upstream commits `3984e67f`, `d7c45cab`). Medium-high.
- [ ] **8d — Claude chat fade-out.** Bottom gradient overlay above the floating composer in `app/src/claude_code_view.rs` (`ClippedScrollable` + `Fill::Gradient` / `Container::with_foreground_overlay`; pattern in `code_review_view.rs`). Low.
- [ ] **8e — Sessions search bar.** Search field over `claude_sessions` in `left_panel.rs` (`render_claude_sessions_panel`); reuse `EditorView::single_line()` + substring filter on `session.title`. Low.
- [ ] **8f — macOS sidebar restyle.** Flat light chrome, pill segmented switcher, muted section headers + macOS disclosure styling, restyled sessions panel + footer, pinned-light regardless of theme. Touches `app/src/workspace/view/left_panel.rs` (`LeftPanelView`). Medium.

## What's already built (audited 2026-06-18)

- **Tabs:** warpui `Container`-based tabs with configurable corner radius/border/fill (`app/src/tab.rs`); within-window drag-reorder already works (`app/src/workspace/view.rs`).
- **Cross-window drag scaffolding:** `DragTabsToWindows` flag, `create_transferred_window` + `transfer_view_tree_to_window` (`app/src/root_view.rs`), reserved `TabData::detached`; full feature exists in upstream history to port.
- **Chat surface:** `ClippedScrollable` + floating composer in `app/src/claude_code_view.rs`; warpui supports linear-gradient fills + foreground overlays (used in `code_review_view.rs`).
- **Sessions list:** `claude_sessions: Vec<claude_code::sessions::StoredSession>` rendered in `left_panel.rs`; no filtering yet.
- **Search input:** reusable `EditorView::single_line()` (patterns in `app/src/search/search_bar.rs`, global search).
- **Native vibrancy:** available (`crates/warpui/src/platform/mac/objc/window_blur.m`) — intentionally **not** used this pass (flat-background decision), but on hand if the look changes later.

## Why this slot (before the rebrand)

Sequenced as 08, ahead of 09-rebrand, on the roadmap's own logic: the heavy items churn the most upstream-divergent files (`app/src/tab.rs`, `app/src/workspace/view.rs`), and 8b/8c **port an upstream feature** — doing all this before the crate-rename pass keeps upstream cherry-picks clean. The rebrand stays last among the upstream-sensitive features.

## Notes

- Each sub-phase still follows the spec-first rule: PRODUCT.md / TECH.md before implementation.
- 8c is the largest lift; if review size demands it, 8b and 8c may bundle or split per the usual sub-phasing judgment.
- Out of scope this pass: native AppKit embedding, vibrancy/translucency, restyling the non-sessions tool panels beyond inherited chrome.
