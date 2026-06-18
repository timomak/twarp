# macOS-style UI overhaul — TECH

Companion to [PRODUCT.md](PRODUCT.md). PRODUCT-invariant numbers (§1–§28) are referenced throughout; the Testing table at the end maps sub-phases to them.

> **File:line references are point-in-time** (gathered 2026-06-18, verified against the working tree). The `warp` app crate churns; re-grep symbols before editing rather than trusting a line number.

## Context

twarp renders its **entire window into a single Metal drawable** via the in-house **warpui** framework (`crates/warpui`, `crates/warpui_core`). It does **not** use GPUI, and there are no native `NSSplitView` / `NSOutlineView` / `NSTableView` anywhere in the app. Every "macOS-style" surface in this feature is therefore an **emulation in warpui primitives** (`Container`, `Flex`, `Fill`, `Border`, `TextInput`, `Draggable`), not a real AppKit control.

This feature is four loosely-coupled visual/interaction changes (PRODUCT's four areas → sub-phases 8a–8f). They share almost no code, which is why they sub-phase cleanly. The two functional items (8b/8c, cross-window tab drag) are a **port of an existing upstream feature whose scaffolding already lives in the tree**, not net-new architecture.

### Why warpui, not AppKit (load-bearing — PRODUCT §1, non-goals)

The sidebar restyle (8f) is the place someone would be tempted to embed a real `NSOutlineView`. We do not, because:

- The window is one Metal drawable. An embedded `NSView` would be a **second, overlapping UI system** with its own focus ring, hit-testing, z-order, and event loop — twarp's overlays (command palette, popovers, drag ghosts) draw in warpui and would render *under or over* an AppKit subview unpredictably.
- Theming would fork: warpui themes are applied per-`Fill`; an `NSOutlineView` themes via `NSAppearance` + cell views. Keeping them in sync (and the pinned-light decision, §21) would mean maintaining two theme paths forever.
- Layout/resize (the existing `ResizableStateHandle` on `LeftPanelView`) is warpui-driven; an embedded native view would need a parallel frame-sync bridge.

So 8f is a **restyle of the existing `LeftPanelView` warpui tree** — new fills, paddings, a pill switcher element, muted header spans — emulating the macOS look. Native vibrancy (`crates/warpui/src/platform/mac/objc/window_blur.m`) exists and is deliberately **not** used this pass (flat-light decision, §2).

---

## 8a — Chrome-style tab shape (PRODUCT §1–§5)

**Anchor:** `app/src/tab.rs`, `Tab::render_tab_container_internal(&self, is_hovered, is_tab_dragging) -> Box<dyn Element>` (~`tab.rs:1365–1632`).

The tab is already a warpui `Container` with configurable corner radius, border, and fill — this is purely a styling change, no structural rework.

- **Corner radius (the headline change).** The container's corner radius is currently uniform. Chrome-style = **top-rounded only**. warpui `CornerRadius` supports per-corner radii (`CornerRadius::with_all(..)` is used at `tab.rs:1118`; use the per-side constructor instead) — set top-left/top-right to the tab radius and bottom corners to `0.` so the active tab seats flush onto the pane area below (§1). Re-check the exact `CornerRadius` builder name at edit time.
- **Fill / active vs inactive (§2).** Fill is assembled at `tab.rs:1382–1439` from `self.styles.background` (a `ThemeFill`) blended with opacity. Tune so the **active** tab fill matches/continues the pane background beneath it and inactive tabs are recessed (lower opacity or a muted theme fill). Hover already flows through `is_hovered` — give it a subtle step.
- **Border (§1, §2).** Border is `Border::all(1.)` with `.with_sides(false, is_first_tab, false, true)` (~`tab.rs:1599–1611`). For a seated tab, drop the **bottom** border on the active tab (so it merges with the content) while keeping side/top borders. Keep the existing first-tab left-border logic.
- **Feature 01 colors (§3) — must survive.** The per-tab color is `self.styles.background` (`ThemeFill::Solid` or gradient, extracted ~`tab.rs:1373`). Do **not** replace this with a hard-coded fill; the color indicator must keep flowing from `styles.background`. If the color currently *is* the whole fill, ensure the new shape still renders it (e.g. as the tab fill or a top color strip) so ⌘⌥1–9 stays visible.
- **Feature 06 rename (§4) — must survive.** The rename editor swaps in at `tab.rs:1109–1151` when `is_tab_being_renamed()`, dispatched via `RenameTab(tab_index)` on double-click (`tab.rs:1806`). The `TextInput` lives inside the same container — verify the new padding/radius doesn't clip the editor or shrink its hit target.
- **Uniformity (§5).** `render_tab_container_internal` renders *all* tab types, so the shape change is automatically uniform; just confirm the new-tab "+" affordance (rendered nearby in `tab.rs`) gets a consistent treatment.

**Risk:** low. Single function, no new state, no flag. Main pitfall is regressing feature 01/06 hit-testing — the smoke test (§28 regression gate) covers it.

---

## 8b — Drag tab → new window (PRODUCT §6–§9)

**Most of this already exists and is dormant.** The pieces:

- **Within-window drag** (`tab.rs:1837–1850`): a `Draggable` with `on_drag_start → StartTabDrag`, `on_drag → DragTab { tab_index, tab_position }`, `on_drop → DropTab`. Crucially, the drag axis is **conditionally locked**:
  ```rust
  let draggable = if FeatureFlag::DragTabsToWindows.is_enabled() {
      draggable                                   // free 2-D drag (can leave the strip)
  } else {
      draggable.with_drag_axis(DragAxis::HorizontalOnly)   // locked to reorder
  };
  ```
  So **enabling the flag is what unlocks vertical/out-of-strip dragging.**
- **The transfer primitive** (`app/src/root_view.rs:636–699`): `create_transferred_window(info: TabTransferInfo, for_drag: bool, ctx) -> (WindowId, Vec<EntityId>)` adds a window and calls `ctx.transfer_view_tree_to_window(pane_group_id, source_window_id, new_window_id)` (~`root_view.rs:689`), then adopts the transferred pane group into the new workspace. **This moves the live view tree — processes (terminals, `claude` sessions) keep running** (§8), which is exactly the §8/§12 "no restart" guarantee.
- **The flag** (`crates/warp_features/src/lib.rs:189`): `FeatureFlag::DragTabsToWindows` exists but is in **no** rollout list (not DOGFOOD/PREVIEW/RELEASE, lines ~842–921) → disabled everywhere.
- **`TabData::detached: bool`** (`tab.rs:212`, init `false` ~`tab.rs:229`): reserved field, "used by a later drag-tab branch to distinguish tabs that moved into detached windows" — currently unused. 8b is that branch.

**Work for 8b:**

1. **Enable the flag** for twarp. Add `DragTabsToWindows` to the appropriate rollout list (DOGFOOD per twarp's local-binary build) via the `add-feature-flag`/`promote-feature` skills, or — given twarp is a personal fork that ships always-on features (cf. feature 07's "no flag") — consider unconditionally treating it enabled. Decide in implementation; the spec requires only that the gesture is live. (PRODUCT §26 allows exactly this one flag change.)
2. **Detach gesture & threshold (§6, §9).** With the axis unlocked, `DragTab` now reports 2-D `tab_position`. In the workspace drag handler (`WorkspaceAction::DragTab` / `DropTab` — `app/src/workspace/view.rs`, the within-window reorder logic), add a branch: when the drop position is **clearly off the strip** (past a vertical threshold), route to `create_transferred_window(.., for_drag=true, ..)` instead of reordering. Below threshold → existing reorder (unchanged, §9). Releasing before threshold cancels (snap back).
3. **Origin cleanup (§7).** After a successful detach, remove the tab from the origin workspace and reflow; if it was the last tab, close the origin window. Re-use whatever `transfer_view_tree_to_window` leaves behind (the source pane group is moved, not copied) — verify the origin doesn't keep a dangling reference.
4. **Set `TabData::detached`** on the moved tab as the field's doc-comment intends, so 8c can tell transferred tabs apart.

**Risk:** medium. The primitive exists and is tested upstream, but the **gesture→transfer wiring and origin cleanup** are the new code. The dangerous edge is window/workspace lifecycle (empty-window close, focus handoff).

---

## 8c — Drag tab between windows (PRODUCT §10–§12) — port upstream

**This is a port, not a rebuild** (PRODUCT §5). Upstream `warpdotdev/warp` implemented cross-window tab drag; the relevant commits are **`3984e67f`** and **`d7c45cab`** (cited in STATUS.md/ROADMAP). Recover the cross-window drop hit-testing, insertion-ghost rendering, and drag-state machine from those commits and reapply onto twarp's (now-unlocked, 8b) drag path. `transfer_view_tree_to_window` already supports an arbitrary destination `WindowId` (`root_view.rs:689` takes a `new_window_id`), so the same primitive moves a tab into an **existing** window — 8b proves the transfer; 8c adds *which* window and *where in its strip*.

**Work for 8c:**

1. **Cross-window hit-testing (§10, §11).** During a `DragTabsToWindows` drag, determine whether the cursor is over **another** twarp window's tab strip. This needs global (screen-space) drag tracking; port upstream's approach rather than inventing one (the warpui `Draggable` reports rects in window space — upstream's commits show how they reconcile across windows).
2. **Insertion indicator (§11).** Render a drop ghost / insertion marker in the **target** window's strip at the computed index, tracking the cursor. This is a warpui overlay element in the target window's tab-strip render (`tab.rs` strip layout / `workspace/view.rs`).
3. **Commit (§10).** On drop over a target strip, call `create_transferred_window`'s sibling path that targets the **existing** window id (or factor a `transfer_tab_to_window(target_window_id, index, ..)` that wraps `ctx.transfer_view_tree_to_window(.., dest)`), insert at the indicated index, then run the same origin cleanup as 8b (§7).
4. **Fallbacks (§11).** Drop **outside** any strip → detach-into-new-window (8b path) or cancel, per the ported state machine. Identity preserved end-to-end (§12): color (`styles.background`), name (`TabData` name), live processes (carried by the view-tree transfer).

**Risk:** medium-high — the largest lift in the feature. **8b and 8c may bundle into one PR** (PRODUCT §load-bearing-6): 8b alone (detach-to-new-window) is smoke-testable end-to-end, but if review judgment prefers, ship them together since they share the drag-state machine and origin-cleanup code. Follow the twarp "bundle when the first sub-phase can't be validated alone" rule.

---

## 8d — Claude chat fade-out (PRODUCT §13–§16)

**Anchor:** `app/src/claude_code_view.rs`. The transcript scrolls in a `ClippedScrollable::vertical(self.scroll_state, content, ScrollbarWidth::Auto, .., Fill::None)` (~`claude_code_view.rs:1698–1708`), and the composer floats above it with `COMPOSER_CLEARANCE: f32 = 140.` / `COMPOSER_MAX_HEIGHT: 184.` / `COMPOSER_CORNER_RADIUS: 14.` (~`claude_code_view.rs:115–123`). Today the transcript just ends under the composer at a hard edge.

**Mechanism.** warpui supports gradient fills and foreground overlays on `Container` (`crates/warpui_core/src/elements/container.rs`):

- `Container::with_foreground_overlay<F: Into<Fill>>(overlay)` (~`container.rs:60–66`) — draws a fill *on top of* the container's children. This is the dimming primitive used for inactive panes (`app/src/pane_group/pane/view/mod.rs:409–418`, `theme().inactive_pane_overlay()`).
- `Fill::Gradient { start, end, start_color, end_color }`, via `with_background_gradient(start, end, gradient)` / `with_horizontal_background_gradient(..)` (~`container.rs:174–200`). A **vertical** gradient is `start=(0,0)`, `end=(0,1)`.

**Approach (§13–§16).** Wrap the scroll area (or a thin region pinned to its bottom) in a `Container` with a **foreground overlay** that is a vertical `Fill::Gradient` from **transparent (top) → the pane background color (bottom)**, occupying roughly the composer-clearance band at the bottom of the scroll viewport. Because the overlay is foreground, transcript content scrolling underneath fades into the background (§13); the **floating composer is drawn after / above this overlay** so it stays fully opaque (§14). The gradient's bottom color is read from the **theme's pane background** (not hard-coded), so it disappears-by-design in both light and dark (§16). The overlay is purely cosmetic — it does not touch `scroll_state` extent or hit-testing (§15), and it is anchored to the composer band so resize keeps it in place (§15).

**Reference pattern:** the inactive-pane dim overlay (`pane/view/mod.rs:409`) shows the foreground-overlay call shape; the gradient builders show the `Fill::Gradient` construction. (Note: `code_review_view.rs` was the originally-cited exemplar; the live overlay+gradient precedents above are the concrete ones to copy.)

**Risk:** low. One overlay element; main care is z-order (overlay below composer, above content) and reading the theme background dynamically.

---

## 8e — Sessions search (PRODUCT §17–§20)

**Anchors:**
- Sessions live on `LeftPanelView` as `claude_sessions: Vec<claude_code::sessions::StoredSession>`, rendered by `render_claude_sessions_panel(&self, app) -> Box<dyn Element>` (`app/src/workspace/view/left_panel.rs:2689–2744`), with each row from `render_claude_session_row(idx, session, app)` (~`left_panel.rs:2763` reads `session.title`).
- `StoredSession` (`crates/claude_code/src/sessions.rs:20–32`) has `id`, **`title: String`** (first user message, truncated), `timestamp: SystemTime`, `jsonl_path: PathBuf`. **`title` is the filter key** (§17).
- Single-line input: `EditorView::single_line(options, ctx)` is the standard reusable text input (20+ call sites, e.g. `app/src/resource_center/keybindings_page.rs:94`; the tab-rename `TextInput` at `tab.rs:1109` is the inline variant). Use `EditorView::single_line` for a real search field with focus/caret/select-all.

**Work for 8e:**

1. **Add a search-field view + filter state to `LeftPanelView`** (struct ~`left_panel.rs:282–310`): an `EditorView` handle for the query (created via `ctx.add_typed_action_view(|ctx| EditorView::single_line(options, ctx))`, cf. keybindings_page.rs:94) and the panel reads its current text each render.
2. **Render the field above the list** in `render_claude_sessions_panel` (§17): put the `EditorView` element above the `heading`/rows `Flex::column`. Read the editor's text (the editor exposes its buffer text — re-grep the accessor used elsewhere, e.g. `.as_ref(ctx)` + a text getter) and **substring-filter** `self.claude_sessions` by `session.title` case-insensitively before building rows.
3. **Empty / cleared states (§18).** Query matches nothing → render a "No matching sessions" span in place of the rows (the panel already has an empty-state branch for *zero stored sessions* — add a distinct *no-match* branch). Empty query → no filter (full list).
4. **Resume unchanged (§19, §20).** Row selection already resumes via the existing `render_claude_session_row` click handler (feature 07); filtering only changes *which rows render*, never the session data or the resume path. The filter is transient view state — not persisted.

**Risk:** low. Self-contained in one panel; the input primitive and the data model already exist. Sequenced **before 8f** so the search field exists when 8f restyles the panel chrome around it.

---

## 8f — macOS sidebar restyle (PRODUCT §21–§25)

**Anchor:** `app/src/workspace/view/left_panel.rs`. `LeftPanelView` (struct ~`left_panel.rs:282–310`) drives the panel; the tool switcher is the `ToolPanelView` enum (`left_panel.rs:211–228`: `ProjectExplorer`, `GlobalSearch { .. }`, `WarpDrive`, `Shortcuts`, `ClaudeSessions`, legacy `ConversationListView`), routed in the render fn (`ToolPanelView::ClaudeSessions => self.render_claude_sessions_panel(app)`, ~`left_panel.rs:3964`). `MouseStateHandles` holds the per-button hit state (e.g. `claude_sessions_button`, ~`left_panel.rs:75`).

This is a **warpui restyle of the existing tree** (no native control, §1, §Why warpui). Sub-parts:

1. **Flat light background, pinned (§21, §2, §3).** Replace the panel's root `Container` fill with a **flat macOS-light fill that does not read from the active theme** — a fixed light `ColorU`, so a dark terminal theme leaves the sidebar light (intended two-tone). Audit the panel tree for theme-derived fills that would otherwise re-darken it. *Scope note:* the Claude chat pane (`claude_code_view.rs`) is **not** part of this — it keeps following the theme, which is why 8d's fade reads the theme background, not the pinned light (§16 vs §21).
2. **Pill segmented switcher (§22, §4).** Replace the current tool-switcher affordance with a **macOS pill segmented control**: a rounded `Container` bar holding one segment per `ToolPanelView` destination; the active segment is a filled pill, the rest quiet. Routing is unchanged — each segment dispatches the same panel-switch action the current switcher does; only the rendered shape changes. Build from warpui `Flex::row` + per-segment `Container` (rounded `CornerRadius`, active vs quiet `Fill`), reusing the existing `MouseStateHandles` hit-state pattern.
3. **Muted headers + macOS disclosure + soft rows (§23).** Restyle section-heading spans (smaller, lighter, quiet weight — currently plain `appearance.ui_builder().span(..)`, e.g. `left_panel.rs:2717`) and give expand/disclosure affordances macOS styling. Replace dense dark row hover/selection fills with a **soft light highlight**.
4. **Sessions panel + footer (§24).** Restyle `render_claude_sessions_panel` rows (the 8e list) and the sidebar footer to match the Claude macOS app — row spacing, typography, secondary text for `session.timestamp`/snippet. The 8e search field sits inside this restyled chrome.
5. **Inherited chrome for other panels (§25).** Project Explorer / Global Search / Warp Drive / Shortcuts render inside the **same restyled shell** (background, switcher, header style) with **no bespoke internal re-layout** — verify their existing content doesn't break against the new light background (e.g. dark-on-dark text that's now dark-on-light, or vice-versa).

**Risk:** medium. Broad surface (touches the whole panel tree) but no new architecture. The trap is **theme leakage** — any child fill still reading the active theme will fight the pinned-light decision; audit thoroughly. Sequenced **last** so it restyles the finished 8e search field rather than churning it twice.

---

## Feature flags

Only one flag is involved: **`FeatureFlag::DragTabsToWindows`** (`crates/warp_features/src/lib.rs:189`), currently disabled in all rollout lists. 8b enables it for twarp (PRODUCT §26). No new flags are introduced for 8a/8d/8e/8f — consistent with twarp's always-on posture for personal-fork features (cf. feature 07). The flag-enable is the one place to use the `add-feature-flag` / `promote-feature` skills if a clean rollout-list edit is preferred over treating it unconditionally-on.

## Theming note (cross-cutting)

Two intentionally-different theme behaviors must not be conflated:

- **Sidebar (8f): pinned macOS-light**, ignores the active theme (§21).
- **Claude pane fade (8d): follows the active theme** — fades to the pane background in light *and* dark (§16).

A reviewer seeing "light sidebar, dark chat" should read it as the **intended** two-tone (PRODUCT §3, §21), not a theme bug.

## Out of scope (this feature)

- Native AppKit embedding; vibrancy/translucency (`window_blur.m` stays unused).
- Bespoke re-layout of the non-sessions tool panels (they inherit chrome only, §25).
- Any change to session persistence, the `claude` driver, or feature 07's resume path (8e filters a `Vec`, nothing more).
- Theme-system changes beyond the localized pinned-light sidebar fill.

## Testing

Maps sub-phases → PRODUCT invariants → smoke-test steps. Presubmit (`./script/presubmit`) must be green before "ready for review"; per twarp's tooling notes, prefer `nextest` where this Mac's presubmit is unreliable.

| Sub-phase | PRODUCT §§ | Smoke steps | Primary files |
|-----------|-----------|-------------|---------------|
| 8a tab shape | §1–§5, §28 | 1–3, 16 | `app/src/tab.rs` |
| 8b detach→new window | §6–§9, §26, §28 | 4–6, 16 | `app/src/workspace/view.rs`, `app/src/root_view.rs`, `crates/warp_features/src/lib.rs`, `app/src/tab.rs` |
| 8c drag between windows | §10–§12, §28 | 7–8, 16 | `app/src/tab.rs`, `app/src/workspace/view.rs`, `app/src/root_view.rs` (+ upstream `3984e67f`, `d7c45cab`) |
| 8d chat fade-out | §13–§16 | 9–10 | `app/src/claude_code_view.rs`, `crates/warpui_core/src/elements/container.rs` |
| 8e sessions search | §17–§20 | 11–12 | `app/src/workspace/view/left_panel.rs`, `crates/claude_code/src/sessions.rs` |
| 8f sidebar restyle | §21–§25, §27 | 13–15, 18 | `app/src/workspace/view/left_panel.rs` |
| cross-cutting | §26–§28 | 16–18 | — |

**Manual-only surfaces.** Drag gestures (8b/8c), gradient appearance (8d), and the pinned-light/two-tone look (8f, §27 acceptance gate) are validated by the smoke test against a built binary, not unit tests — per twarp's keybinding/UI rule, **launch the app to verify**. 8e's substring filter is the one piece with clean unit-test surface (filter fn over a `Vec<StoredSession>`); add a `rust-unit-tests` case for it.
