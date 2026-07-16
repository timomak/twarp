# Design system & visual overhaul — PRODUCT

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants; TECH.md references these numbers. Sub-phase tags (19a–19e) are defined in TECH.md.

Figma: none provided — the visual reference is the **OpenAI Codex desktop app** (owner-supplied screenshot, 2026-07-16) and the rules codified in [`design/PHILOSOPHY.md`](../../design/PHILOSOPHY.md).

## Summary

Bring twarp's whole non-terminal UI up to the restraint level of the Codex desktop app: a written design philosophy with an enforced token system (19a), a **full-height, macOS-native-feeling left sidebar** with the traffic lights inside it and the tab strip beginning to its right (19b), the same principles applied to the right code-review panel (19b), a document-calm agent pane (19c), refined tabs and block chrome (19d), and an app-wide sweep that retires the legacy inconsistencies (19e). Pure restyle: no feature behavior, keybinding, or persistence semantics change.

## Problem

Seventeen features shipped without a design system. The audit (2026-07-16) found ~11 text sizes and 16 padding values in the agent pane alone, two competing default radii app-wide, three different hairline border colors, prose at terminal line-height, and two disconnected color systems. Each PR was locally reasonable; the sum reads as noise. Separately, the sidebar and right panel float as inset cards below the tab strip — nothing like the anchored, full-height source list that makes Codex/Finder/Linear feel native.

## Goals / Non-goals

**Goals**

- One binding philosophy + token set (`design/PHILOSOPHY.md`, `twarp_core::ui::tokens`), enforced via the UI skill and the fleet UX gate.
- The Codex shell: full-height left sidebar owning the window's left edge (traffic lights composite over its top), tab strip starting to its right; right panel as a polished floating inspector.
- The agent pane as a calm document (prose measure, turn rhythm, one card anatomy, collapsed tool-run groups).
- Tab strip refinement that **keeps per-tab colors** — they are twarp's identity. *(Flares were retained through 19d, then dropped in 19f by owner direction: plain rectangles.)*
- Sweeps that retire whole violation classes (raw literals, PhenomenonStyle, ad-hoc shadows, duplicate icons).

**Non-goals**

- **No functional changes.** Every action, keybinding, toggle, persistence behavior, and feature works exactly as before; this feature only changes how things look and are laid out.
- **No terminal-grid changes.** Cell metrics, terminal fonts, PTY rendering are untouched.
- **No provider work.** The multi-provider agent pane is feature 18 and lands after 19.
- **No new components beyond the philosophy's anatomies** — this is consolidation, not invention.
- Windows/Linux shell parity is best-effort; the traffic-light/full-height work is macOS-specific by nature (those platforms keep the current layout where the native pieces don't exist).

## Load-bearing decisions (surfaced for review)

Owner-approved 2026-07-16 unless marked **(provisional)**.

1. **19 ships before 18** (visual overhaul before the Codex backend).
2. **True Codex layout for the left sidebar** — full height including the titlebar corner, traffic lights over the sidebar, tab strip shifted right (not the cheaper "edge-to-edge below the strip" variant). Feasibility confirmed: the titlebar is already transparent full-size-content-view with real repositioned buttons compositing over the Metal layer.
3. **(provisional) Right panel keeps the floating-inspector form**, restyled to tokens (radius 14, panel shadow, 16px margins) — this matches the Codex app itself, whose right rail floats while its sidebar is anchored. Its maximize mode is unchanged. If the owner prefers a full-height anchored right panel for symmetry, that swaps in at spec review, not later.
4. **(provisional) Sidebar and panel surfaces are neutral** (macOS source-list style). The per-tab color stays on tab chips and text accents — as it actually behaves today. **Open question:** whether the sidebar gets an *optional, very subtle* active-tab tint wash (~3%) as a twarp signature; recommendation is to ship neutral first and evaluate.
5. **19a (philosophy + tokens) is bundled into this spec PR** — docs plus a constants module with no consumers; nothing smoke-testable ships without it (the owner's bundling rule).
6. **Restyle behind a feature flag only where layout moves** (the 19b shell inversion); token/type/color swaps within existing layouts ship unflagged.

## Behavior

### The shell: left sidebar — 19b

1. When the sidebar is open, it occupies the window's **full height** — from the very top edge (the titlebar region) to the bottom edge — anchored flush to the left, with **no floating-card margins, corner radius, border ring, or drop shadow**.
2. The macOS traffic lights render **inside the sidebar's top area**, in their standard position; they remain fully clickable and are never overlapped by sidebar content. The sidebar's top area reserves that zone (nothing interactive is placed under or within it).
3. When the sidebar is open, the **tab strip begins at the sidebar's right edge** — tabs no longer reserve the traffic-light clearance on the left. When the sidebar is closed, the strip returns to today's layout (content starting after the traffic-light clearance). Opening/closing the sidebar never changes tab order, activation, or scroll position within the strip.
4. The sidebar's surface is a neutral tone visually distinct from the content area, separated from it by a single 1px hairline running the full window height. There is no horizontal rule crossing the sidebar at tab-strip height — the sidebar column reads as one uninterrupted surface.
5. Sidebar content follows the philosophy's list-row anatomy: caption-style section headers; rows with an icon slot, UI-style text, rounded selection fill for hover/selected; **no per-row borders and no divider lines**.
6. All existing sidebar functionality is unchanged and reachable: the tool switcher (files / search / drive / agent sessions), the project explorer with its git timeline sub-panel, global search, the agent past-sessions list with its search field, resize by dragging the right edge (existing min/max), and the existing toggle keybinding/chord. A persisted width set before this feature still applies after it.
7. **Dragging the window from empty space in the sidebar's top band still moves the window**, exactly like dragging the titlebar; double-click-to-zoom in that band keeps working. Interactive sidebar controls win over window-drag where they overlap.
8. In **macOS fullscreen** (traffic lights hidden/auto-hidden), the sidebar's reserved top zone collapses so content uses the space; leaving fullscreen restores the reservation. No layout jump other than that reservation.
9. With the sidebar **closed**, nothing about today's window chrome regresses: traffic lights sit over the (transparent) titlebar band, tabs start after the clearance, window drag works.
10. On Windows/Linux, the sidebar keeps its current (pre-19b) presentation; no full-height/titlebar work is attempted where the native affordances don't exist.

### The shell: right panel (code review / Open Changes) — 19b

11. The right panel renders as a **floating inspector card**: panel-radius corners, the panel elevation shadow, a 1px hairline, and consistent margins from the window's top/right/bottom edges — visually calm, matching the Codex right rail.
12. Its contents (repo dropdown, code review view, maximize button) and behaviors (toggle keybinding, resize from the left edge, maximize-to-fill) are unchanged. Maximized mode fills the content area edge-to-edge as today.
13. Left sidebar and right panel can be open simultaneously without visual collision; the terminal content area shrinks between them as today.

### Agent pane as a document — 19c

14. Conversation prose renders at the philosophy's PROSE style (14pt, 1.55 line-height) within a centered measure of at most 720px; at narrow pane widths the measure yields gracefully (existing responsive tiers keep working).
15. Successive turns are separated by clear vertical rhythm (24px class spacing); plain assistant text is **never boxed or bordered**.
16. After a turn completes, its consecutive tool-run cards **collapse into a single summary row** ("Worked for Ns · M actions" class) that expands on click to reveal the full cards; while a turn is streaming, tool cards remain live and visible as today. Expanding/collapsing loses no information and no interactive affordance (review, undo, copy).
17. Every tool card, diff card, and approval prompt uses the **one card anatomy** (same radius, same hairline, header and body left-aligned to the same inset, meta right-aligned); the approval prompt keeps its Allow-once/Always/Deny actions and keyboard handling unchanged.
18. The composer follows the composer anatomy (panel radius, hairline, pill-styled controls, one primary send affordance); every existing control (model/effort/permission pills, context pills, worktree toggle, mic, attach, Stop, raw-CLI toggle) remains present and functional, restyled only.
19. Text selection, copy, fork, scroll behavior, streaming, notifications/tab-dot signals — all preserved exactly (these have regression history; see TECH).
20. The pane's empty state presents prose-styled guidance with one clear next action, centered in the measure — not a bare void.

### Tabs & blocks — 19d

21. Per-tab colors are **retained**. The active tab is unambiguously distinguishable from every neighboring tab in both light and dark themes, including when adjacent tabs share the same custom color (contrast comes from state treatment, not hue alone). *(Flare retention originally specced here was superseded by the owner feedback round — see §34: tabs are plain rectangles.)*
22. Each tab shows **at most one indicator glyph** in a fixed slot (agent attention, error, sync, etc. by priority); a given piece of status never renders twice in the chrome (e.g., the working-tree diff count appears in exactly one place).
23. Tab titles truncate with an ellipsis and never overlap their indicator or close affordances.
24. Terminal **block state emphasis** (error exit, bookmark/highlight) renders as a 2px left accent bar plus a subtle (≤4% alpha) background wash — replacing today's full-bleed saturated band. Exit-status legibility does not regress: an error block remains identifiable at a glance in light and dark themes.
25. Block affordances (filter, kebab menu, pills row under the input) restyle to pill/caption anatomy without losing any action.

### App-wide conformance & sweep — 19a / 19e

26. All values named by the philosophy (spacing, radius, type, elevation, borders) come from the token module in **new and touched** UI code; the sweep retires raw literals from the agent pane cluster, sidebar, right panel, tab strip, and settings pages.
27. Settings pages adopt the token scale and list-row anatomy; every settings control keeps its function and its section placement (no IA changes).
28. The static hardcoded palette (`PhenomenonStyle` uses) is migrated to adaptive theme roles in swept surfaces — swept UI responds correctly to theme changes, light and dark.
29. Floating surfaces app-wide use one of the two elevation tokens; menus/popovers look consistent wherever they appear.
30. Duplicate icon variants are consolidated in swept surfaces so the same concept uses the same glyph everywhere.
31. After each sub-phase, both **light and dark** themes render correctly on every touched surface (no hardcoded-for-one-theme colors), at 100% and at non-default zoom.
32. Nothing in this feature changes what is persisted or how: panel widths, tab colors, session lists, settings values all read/write exactly as before (a downgrade/upgrade across this feature loses nothing).

### Owner feedback round (post-smoke, 2026-07-16) — 19f

33. **Tabs are a horizontal strip at the top of the window**, exactly as pre-19b in placement and behavior (creation, activation, close, reorder, cross-window drag, overflow). No tabs-as-a-sidebar-list presentation remains in any flag state — the 19b interpretation that rendered tabs as a vertical left-hand list is reverted.
34. **Tab chips are plain rectangles** — no flare and no rounded-top silhouette. Per-tab colors, the single indicator slot (§22), title truncation (§23), and the active-contrast guarantee (§21) all still hold on the rectangular chips.
35. **The full-height shell treatment (§1–§10) applies to the Tools panel** — the files / search / drive / agent-sessions panel — not to tabs: it is the flush, edge-to-edge, full-window-height column with the traffic lights inside its top area and no floating-card chrome. With the Tools panel open, the tab strip begins at its right edge (§3); with it closed, the strip spans the window as before. Its existing toggle, tool switcher, resize, and persisted width keep working (§6).
36. **The chrome's persistent search input becomes an icon**: a search glyph sits at the far right of the tab strip with no always-visible text field; clicking it opens the same search experience the input provided. Nothing else about search behavior changes.
37. **The top-right avatar/profile control is replaced by a plain gear glyph** — no background fill, no avatar imagery. Clicking it navigates directly to Settings. Its previous dropdown menu is **deleted**; anything that menu offered remains reachable through an existing home (Settings, the command palette, or the menu bar) — no capability is lost without an alternate path.
38. §33–§37 render correctly in light and dark themes and at non-default zoom, and none of them change persisted state shape (§32 holds across this round).

## Open questions

- Invariant 4 note: should the neutral sidebar carry an optional ~3% active-tab tint wash as a twarp signature? (Recommendation: ship neutral, evaluate after 19b.)
- Invariant 24: exact accent/wash treatment for *bookmarked* (vs error) blocks — same anatomy with a different accent color, or no wash at all?

## Smoke test

Steps assume a built `twarp-oss` launched fresh. Each sub-phase's steps validate its invariants (§ refs above).

### 19b — the Codex shell

1. Toggle the left sidebar open (its existing keybinding or the toolbar toggle). The sidebar renders as one uninterrupted column from the very top of the window to the bottom — no gap above it, no margins around it, no rounded card corners or shadow, and no horizontal line crossing it at tab-strip height (§1, §4).
2. The traffic lights sit inside the sidebar's top area and all three respond to hover (highlight) and click — close/minimize/zoom still work (§2).
3. With the sidebar open, the first tab starts at the sidebar's right edge (no dead traffic-light gap between sidebar and first tab). Close the sidebar: tabs return to starting after the traffic-light clearance; tab order and active tab unchanged (§3).
4. Drag from an empty spot in the sidebar's top band: the window moves. Double-click the same area: the window zooms (§7).
5. Drag the sidebar's right edge: it resizes within its existing min/max; quit and relaunch: the width persists (§6, §32).
6. Switch between the sidebar tools (files / search / drive / agent sessions): each renders with section headers and hover/selection fills, no divider lines between rows (§5, §6).
7. Enter macOS fullscreen: the sidebar's top reservation collapses (no dead zone); exit fullscreen: traffic lights return over the sidebar (§8).
8. Open the right panel (code review): it floats as a rounded card with a shadow, inset from the window's top/right/bottom edges; its maximize button still fills the content area; toggling it closed/open works (§11–§12).
9. Open both panels at once with a terminal in the middle: no visual overlap or clipping (§13).
10. Repeat steps 1–3 in a light theme and a dark theme: the sidebar surface is distinct from the content in both, separated by a 1px hairline (§4, §31).

### 19c — agent pane as a document

1. Run `claude` in a terminal at a repo cwd: the agent pane opens. Send a short prompt; the reply prose is noticeably larger-with-more-leading than UI labels (14pt-class with relaxed line-height), centered in a column that doesn't span edge-to-edge on a wide pane (§14).
2. Plain assistant text has no box or border around it; consecutive turns have clear vertical breathing room (§15).
3. Send a prompt that triggers tool use (e.g. "list the files in this directory and read one"). While streaming, tool cards are visible live; after the turn completes, the tool runs collapse into a single "Worked for …" summary row; clicking it expands the full cards with all their actions intact (§16).
4. Trigger an edit (e.g. "add a comment to README"): the diff card and any approval prompt share the same corner radius and border treatment, and their header/body text left-align to the same inset; Allow/Deny buttons work (§17).
5. The composer is a rounded panel with visible but subtle border; every pill (model/effort/permission, context pills, worktree) is present and clickable; Stop works mid-turn (§18).
6. Select and copy a sentence from a completed reply (drag + Cmd+C): it copies. Fork and copy-response buttons still work (§19).
7. Open a brand-new pane: the empty state shows styled guidance text, not a blank void (§20).

### 19d — tabs & blocks

1. Create 4+ tabs, give two adjacent tabs the same custom color: the active tab is still unmistakably distinguishable from its same-color neighbor in light and dark themes (§21).
2. A tab with agent attention (or an error) shows exactly one indicator glyph; nowhere in the chrome does the same status (e.g. the working-tree diff count) appear twice (§22).
3. Narrow the window until tabs truncate: titles ellipsize without overlapping indicators (§23).
4. Run a failing command (e.g. `false` or a bad `git checkout`): the block shows a thin left accent bar and a subtle wash — not a full-width saturated color band — and is still identifiable as an error at a glance in both themes (§24).
5. Block affordances (filter/kebab on hover, the context pills under the input) are present and functional (§25).

### 19e — sweep

1. Open Settings and visit every page: consistent paddings and radii, no obviously misaligned rows; all controls still function (§27).
2. Switch theme light↔dark with Settings open and with a menu/popover open: every swept surface follows the theme (nothing stays light-pinned), and floating surfaces carry a consistent shadow (§28, §29, §31).
3. Spot-check swept surfaces at a non-default zoom level: layout holds (§31).

### 19f — owner feedback round

1. The window shows a **horizontal tab strip at the top** with 3+ tabs; tabs are colored **plain rectangles** (no curved flare silhouette); creating, switching, and closing tabs works (§33–§34).
2. There is **no vertical list of tabs in the left sidebar** in any state (§33).
3. Toggle the Tools panel open: it renders as the full-height flush column — traffic lights inside its top area, no gap/margins/rounded corners/shadow — and the tab strip starts at its right edge; switch between its tools (files / search / agent sessions): all reachable; toggle it closed: the strip spans as before and no leftover panel remains (§35).
4. The far right of the tab strip shows a **search icon** with no persistent text input; clicking it opens the search UI (§36).
5. The top-right corner shows a **plain gear glyph with no background or avatar**; clicking it opens Settings directly — no dropdown menu appears (§37).
6. Repeat steps 1–5 in a light theme and a dark theme (§38).
