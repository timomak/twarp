# macOS-style UI overhaul — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers. Chord names are macOS.

## Summary

A visual + interaction pass that makes twarp read as a first-class macOS app across four owner-requested areas:

1. **Chrome-style tabs** — tabs shaped like Chrome/Safari (top-rounded, seated on the tab strip), plus the ability to drag a tab **out** to a new window and **between** existing windows.
2. **macOS-style sidebar** — the shared left-panel chrome (background, top tool switcher, section headers, row style, footer) restyled to a flat macOS light look, with the Claude **sessions** panel reworked to mirror the Claude macOS app. Other tool panels inherit the new chrome but aren't individually reworked.
3. **Claude chat fade-out** — a bottom gradient mask over the Claude chat scroll area so messages fade under the floating composer instead of ending in a hard cut.
4. **Sessions search** — a search field over the Claude sessions history list that filters by text, like the Claude macOS app.

This feature changes **look and direct-manipulation behavior only**. It introduces no new services, no new persisted data, and no AI/account surface (feature 02's removals stay removed).

## Problem

twarp is a Warp fork that has, feature by feature, diverged toward a focused personal IDE. Its chrome still reads as "a terminal with panels": tabs are flat rectangles, the left panel uses Warp's stock dark dense styling, the Claude chat scroll area ends in a hard edge under the composer, and the sessions list has no way to find an old conversation except scrolling. None of these are broken, but together they make twarp feel like a power-tool rather than a native macOS app.

Two of the gaps are also **functional**, not only cosmetic: you cannot pull a tab out into its own window or move it between windows (a baseline macOS/Chrome expectation that upstream Warp supports but twarp has dormant), and you cannot search session history. This pass closes the cosmetic gap and re-enables the two functional ones.

## Goals / Non-goals

**Goals**

- **Chrome-style tab shape** — tabs are top-rounded and visually seated on the strip, while keeping feature 01 per-tab colors and feature 06 rename intact.
- **Tab → new window** — drag a tab out of the strip to detach it into a new window carrying its full pane tree.
- **Tab → between windows** — drag a tab from one window's strip and drop it into another window's strip at a chosen position, with clear drop feedback.
- **macOS sidebar look** — flat light background, a pill segmented tool switcher, muted section headers with macOS-style disclosure, restyled sessions panel + footer — pinned to the macOS light look regardless of the active twarp theme.
- **Claude chat fade-out** — the scroll area fades to the panel background at the bottom so content slides under the floating composer.
- **Sessions search** — a search bar filters the sessions list by substring match on the session title, live as you type.

**Non-goals**

- **No native AppKit embedding.** twarp renders the whole window into a single Metal drawable via the in-house **warpui** framework; it does not use GPUI and embeds no native `NSSplitView` / `NSOutlineView` / `NSTableView`. The sidebar is a **warpui restyle that emulates the macOS look**, not an embedded native control (full rationale in TECH.md).
- **No vibrancy / translucency this pass.** Native window blur exists in the codebase but is intentionally not used; the decision is a flat light background.
- **No restyle of the non-sessions tool panels** beyond the inherited chrome (Project Explorer / Global Search / Warp Drive / Shortcuts get the new shell, not bespoke layouts).
- **No new feature flags persisted to the user** beyond re-enabling the dormant cross-window-drag flag (`DragTabsToWindows`); no new settings UI.
- **No AI/account/billing surface** of any kind — this feature touches chrome, not services.
- **No theme system rework.** The sidebar is pinned-light by deliberate two-tone choice; the rest of twarp's theming is untouched.

## Figma

Figma: none provided. The **visual targets are the Claude macOS app** (sidebar chrome, pill switcher, sessions list + search) and **Chrome/Safari** (top-rounded tabs seated on the strip, drag-out / drag-between behavior). Net-new chrome follows existing twarp/warpui pane conventions. A light sidebar beside a dark terminal theme is the **intended** two-tone look (it matches the Claude-app reference), not a regression.

**Visual consistency with these references is the acceptance gate** for the cosmetic sub-phases (8a, 8d, 8f) — a surface that renders the right structure in twarp's old flat/dark shape is not done.

## Load-bearing decisions (surfaced for review)

These shape everything below; flagged here rather than buried so they're easy to veto. Owner-confirmed decisions are carried from STATUS.md (2026-06-18).

1. **warpui restyle, not native embedding.** The sidebar's macOS look is emulated in warpui. Splicing a real AppKit source list into the Metal surface would fork focus, layout, overlays, and theming across two UI systems and permanently hybridize the codebase. (TECH.md §Why warpui, not AppKit.)
2. **Flat light background, no vibrancy.** The sidebar uses a flat macOS-style light fill this pass; vibrancy/translucency is deferred even though `window_blur.m` exists.
3. **Sidebar pinned to macOS light** regardless of the active twarp theme. Intentional two-tone with the (possibly dark) terminal. The *Claude chat pane* itself continues to follow the twarp theme (it is not part of the sidebar restyle).
4. **Top tool switcher → pill segmented control** (à la the Claude app's `Chat | Cowork | Code`), replacing the current switcher affordance. Same destinations, new shape.
5. **Cross-window drag is a port, not a rebuild.** 8b/8c re-enable the dormant `DragTabsToWindows` flag and port upstream's `transfer_view_tree_to_window` implementation (upstream commits `3984e67f`, `d7c45cab`) rather than writing detach/transfer from scratch. (STATUS.md, TECH.md §Tab transfer.)
6. **Tab cluster runs as one consecutive block (8a→8b→8c), then the lighter Claude-pane wins (8d, 8e), then the sidebar restyle (8f)** — per owner direction. 8b and 8c may bundle into one PR if 8b alone has too little for the smoke test to validate end-to-end (the established twarp sub-phasing judgment).

## Behavior

Invariants are grouped by area and annotated with the sub-phase that delivers them. They are numbered for TECH.md to reference.

### Chrome-style tab shape — 8a

1. Each tab in the strip is rendered with a **top-rounded** shape (rounded top corners, square or seated bottom) so the active tab reads as a Chrome/Safari-style tab connected to the content area below it, not a free-floating rectangle.

2. The **active** tab is visually distinguished from inactive tabs (fill/elevation) and reads as continuous with the pane area beneath it. Inactive tabs are recessed/muted by comparison. Hover gives a subtle affordance.

3. **Feature 01 per-tab colors survive.** A tab assigned a color (⌘⌥1–9) still shows that color indicator in the new shape; the shape change does not remove or relocate the color affordance.

4. **Feature 06 rename survives.** Double-click / the rename chord still renames a tab in place; the new shape does not break the inline rename editor's hit target or layout.

5. The new tab shape applies uniformly to all tabs (terminal, editor, Claude Code, etc.) and to the new-tab affordance; no tab type keeps the old rectangle.

### Drag a tab out to a new window — 8b

6. Dragging a tab off the tab strip far enough to leave the strip (a detach gesture) and releasing **detaches** it into a brand-new window. The new window carries that tab's **entire pane tree** (splits, terminals, editors, Claude panes) intact and focused.

7. After a detach, the **origin** window no longer contains that tab; its remaining tabs reflow. If the detached tab was the origin window's last tab, the origin window closes (no empty windows left behind).

8. A detached tab in its new window behaves like any other tab — it can be renamed, colored, reordered, split, and dragged again. Its running processes (terminals, `claude` sessions) **continue uninterrupted** across the move (the view tree is transferred, not recreated).

9. The detach gesture is **distinguishable from a within-window reorder**: a small drag along the strip still reorders (feature unchanged); only a drag that clearly leaves the strip detaches. Releasing before the detach threshold cancels cleanly (tab returns to its slot).

### Drag a tab between windows — 8c

10. With two twarp windows open, dragging a tab from window A's strip and dropping it onto window B's strip **moves** the tab (with its full pane tree, processes intact) into window B at the drop position. Window A loses it (and closes if it becomes empty, per §7).

11. During a cross-window drag, the target window's strip shows **drop feedback**: an insertion indicator at the position where the tab will land as the cursor moves across the strip. Dropping commits to the indicated position; dropping outside any strip falls back to detach-into-new-window (§6) or cancels, per TECH.md's chosen drag-state model.

12. A cross-window move preserves tab identity end to end: per-tab color (§3), custom name (§4), and the live processes inside continue without restart, exactly as for a same-window detach (§8).

### Claude chat fade-out — 8d

13. The Claude Code chat scroll area has a **bottom gradient mask**: content near the bottom edge fades from full opacity to the pane/composer background, so messages appear to slide **under** the floating composer rather than ending at a hard horizontal line.

14. The fade sits **above the scrolled content but below the floating composer** — the composer itself stays fully opaque and legible; only transcript content passing behind/under it fades.

15. The fade is purely visual: it does **not** change scroll extent, hit-testing, or how far the user can scroll. The last message remains fully readable when scrolled to the bottom (the fade region clears it). Resizing the pane keeps the fade anchored to the composer.

16. The fade color tracks the **Claude pane's** background (which follows the twarp theme — the pane is not part of the pinned-light sidebar restyle, §3), so the gradient is invisible-by-design in both light and dark themes (it fades to whatever the background is, never a hard-coded color).

### Sessions search — 8e

17. The Claude **sessions** history list (in the left panel) has a **search field** above it. Typing filters the list **live** to sessions whose title contains the query as a substring (case-insensitive). Clearing the field restores the full list.

18. When a query matches nothing, the list shows a clear **empty state** ("No matching sessions" or equivalent) rather than a blank area. An empty query is not a filter (shows everything).

19. The search field follows existing twarp single-line input conventions (focus, caret, select-all, clearing); it filters only — it does not create, rename, or delete sessions. Filtering does not alter persisted session data in any way.

20. Selecting a session from the **filtered** list resumes it exactly as selecting from the unfiltered list does (feature 07 behavior unchanged). The filter state is a transient view concern.

### macOS sidebar restyle — 8f

21. The left panel uses a **flat macOS-style light background** (no vibrancy), **pinned to the light look regardless of the active twarp theme** (§2, §3). A dark terminal theme beside the light sidebar is the intended two-tone.

22. The **top tool switcher** (Project Explorer / Global Search / Warp Drive / Shortcuts / Claude Sessions) renders as a **macOS pill segmented control**: a rounded segmented bar where the active destination is a filled pill, the rest are quiet. Clicking a segment switches the panel as before — destinations and routing are unchanged, only the affordance is new.

23. **Section headers** are restyled to a muted macOS look (smaller, lighter, uppercase-or-quiet weight), and any disclosure/expand affordances use macOS-style disclosure styling. Row hover/selection use a soft macOS highlight rather than the old dense dark rows.

24. The **sessions panel** (the 8e list + search) and the **sidebar footer** are restyled to match the Claude macOS app sidebar (row spacing, typography, secondary text for timestamps/snippets). The search field (8e) sits within this restyled chrome.

25. The non-sessions tool panels (Project Explorer, Global Search, Warp Drive, Shortcuts) **inherit the new chrome** (background, switcher, header style) but are **not** individually re-laid-out this pass — their existing internal content renders inside the new shell without bespoke rework.

### Cross-cutting

26. No sub-phase introduces any Warp/Anthropic sign-in, API-key field, usage meter, billing UI, or new persisted user data. The only flag change is re-enabling `DragTabsToWindows`.

27. **(Acceptance gate, cosmetic sub-phases)** Side-by-side with the references — Chrome/Safari for tabs (8a), the Claude macOS app for the sidebar (8f) and chat fade (8d) — twarp is *visually consistent*: top-rounded seated tabs, a flat light sidebar with a pill switcher and muted headers, and a chat area that fades under the composer. A surface that carries the right data in twarp's old flat/dark rectangle shape fails this gate even if every functional step passes.

28. **(Regression gate)** After every sub-phase, pre-existing tab behavior holds: within-window drag-reorder (unchanged), per-tab colors (feature 01), tab rename (feature 06), and Claude session resume (feature 07) all still work.

## Smoke test

Run against a freshly built twarp binary (`./script/run`, or the release bundle per the team's build notes). The Claude-related steps need the `claude` CLI installed and logged in. Chord names are macOS.

### 8a — Chrome-style tabs (look)

1. Open twarp with several tabs. Each tab is **top-rounded** and seated on the strip (Chrome/Safari-style), not a flat rectangle. The **active** tab reads as continuous with the pane below; inactive tabs are recessed.
2. Assign a color to a tab (⌘⌥3). The color indicator still shows in the new shape. Rename a tab (feature 06 gesture) — the inline editor still works and is hit-testable.
3. Open a terminal tab, an editor tab, and a Claude Code tab — all three use the new shape; none keeps the old rectangle.

### 8b — Drag tab → new window

4. Drag a tab a short distance along the strip → it **reorders** (unchanged). Drag a tab clearly **off** the strip and release → it **detaches into a new window** carrying its full pane tree, focused.
5. Detach a tab that had a running terminal and (if available) a live `claude` session → in the new window the terminal scrollback and the session are **intact and still running** (not restarted).
6. Detach the **last** tab of a window → the now-empty origin window closes. Begin a detach drag and release **before** leaving the strip → the tab snaps back, nothing detaches.

### 8c — Drag tab between windows

7. With two windows open, drag a tab from window A's strip over window B's strip → an **insertion indicator** appears and tracks the cursor across B's strip. Drop → the tab lands at the indicated position in B; A loses it (and closes if it was A's last tab).
8. The moved tab keeps its **color** (feature 01) and **custom name** (feature 06), and its inner processes keep running. Drag a tab and drop it **outside** any strip → it detaches into a new window (or cancels) per the spec, never vanishing.

### 8d — Claude chat fade-out

9. Open a Claude Code pane with enough messages to scroll. The bottom of the transcript **fades** into the composer background — messages slide under the floating composer, no hard horizontal cut.
10. Scroll to the very bottom → the last message is **fully readable** (the fade clears it). The composer stays fully opaque. Resize the pane → the fade stays anchored to the composer. Toggle the twarp theme between light and dark → the fade tracks the background in both (no hard-coded band).

### 8e — Sessions search

11. With several past Claude sessions in this cwd, open the sessions list. A **search field** sits above it. Type part of a session's title → the list filters **live** to matching sessions (case-insensitive substring).
12. Type a query that matches nothing → an **empty state** message shows (not a blank panel). Clear the field → the full list returns. Select a session from a **filtered** list → it resumes normally (feature 07 unchanged).

### 8f — macOS sidebar restyle

13. The left panel has a **flat light** background. Switch the twarp theme to a dark terminal theme → the sidebar **stays light** (intended two-tone); the terminal/Claude pane follows the theme.
14. The top tool switcher is a **pill segmented control**; the active destination is a filled pill, the rest quiet. Click each segment → it routes to the same panels as before. Section headers are **muted/macOS-styled**; row hover/selection use a soft highlight.
15. The sessions panel + footer match the Claude macOS app (spacing, secondary text for timestamps/snippets), with the 8e search field inside the restyled chrome. The non-sessions panels (Explorer, Global Search, Warp Drive, Shortcuts) render inside the **new shell** without broken layout.

### Cross-cutting

16. After each sub-phase: within-window drag-reorder, per-tab colors (feature 01), tab rename (feature 06), and Claude session resume (feature 07) all still work — no regression.
17. **(Acceptance gate)** Side-by-side with Chrome/Safari (tabs) and the Claude macOS app (sidebar, chat fade), twarp is visually consistent — seated top-rounded tabs, flat light sidebar with a pill switcher and muted headers, chat fading under the composer. Old flat/dark rectangle shapes fail this step even if every functional step passes.
18. Throughout, no Warp/Anthropic sign-in, API-key field, usage meter, or billing UI appears anywhere in the restyled chrome.
