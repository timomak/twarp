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
- Tab strip refinement that **keeps per-tab colors and flares** — they are twarp's identity.
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

21. Per-tab colors and the Chrome-style flare shape are **retained**. The active tab is unambiguously distinguishable from every neighboring tab in both light and dark themes, including when adjacent tabs share the same custom color (contrast comes from state treatment, not hue alone).
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

## Open questions

- Invariant 4 note: should the neutral sidebar carry an optional ~3% active-tab tint wash as a twarp signature? (Recommendation: ship neutral, evaluate after 19b.)
- Invariant 24: exact accent/wash treatment for *bookmarked* (vs error) blocks — same anatomy with a different accent color, or no wash at all?
