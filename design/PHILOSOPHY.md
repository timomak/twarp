# twarp design philosophy

This document is the law for how twarp looks and feels. It exists because the audit of 2026-07-16 found the opposite of a system: ~11 distinct text sizes and 16 distinct padding values in one pane, two competing default radii app-wide (4px ×124, 8px ×87), three different colors doing the same 1px border job, and chat prose rendered at terminal line-height. None of those were wrong alone; together they are why the app reads as noisy next to the apps we admire.

It binds through three mechanisms, not good intentions:

1. **Tokens** — `crates/twarp_core/src/ui/tokens.rs` names every legal spacing, radius, type, and elevation value. Views use tokens, not literals.
2. **The UI skill** — `.claude/skills/warp-ui-guidelines/SKILL.md` points here; every agent doing UI work reads this file first.
3. **The UX gate** — fleet screenshot review checks conformance (see "Enforcement" below). New violations are review blockers.

The calibration reference is the OpenAI Codex desktop app: calm neutrals, hairline borders, whitespace as the grouping tool, color only as meaning. We are not copying it — we are matching its level of restraint while keeping what is ours (per-tab colors, the block-based terminal).

## North star: a calm instrument

twarp is a terminal that reads like a document.

- **The terminal grid is sacred.** Cell metrics, mono font, full-width content, user-chosen terminal font size — none of this document's rules apply inside the grid. It is dense because terminals are dense.
- **Everything around the grid is calm.** Chrome, panels, and agent surfaces group with whitespace first, subtle fills second, borders last. Prose breathes.
- **One decorative hue per screen: the tab color.** The user's per-tab color is twarp's identity system and it is sacred. It only reads as identity if nothing else competes — every other hue on screen must be *information* (success, warning, error, diff) or neutral.
- **A border means "you can act on this."** Cards with hairlines are actionable artifacts (an edit to review, a command to approve). Passive content is never boxed.

## Surface classes

Every view belongs to exactly one class, and the class decides its density:

| Class | Examples | Rules |
|---|---|---|
| **terminal** | the grid, PTY output | Exempt. User font settings govern. |
| **conversation** | agent pane transcript, empty states | `PROSE` type, max measure 720 centered, `XL` (24) rhythm between turns, no boxes around plain text. |
| **chrome** | Projects sidebar, tool rails, composer, settings, pills, meta rows | `UI`/`LABEL`/`CAPTION` type, `SM`/`MD` gaps, hairlines only where actionable. |

## Tokens

Values live in `tokens.rs`; this table is the human-readable mirror. **No raw numeric literals for spacing, radius, font size, or line height in view code.** If no token fits, the fix is a discussion here, not a one-off.

- **Spacing** (pt): `XXS 2` (icon↔text only) · `XS 4` · `SM 8` · `MD 12` · `LG 16` · `XL 24` · `XXL 32`. No 3/5/6/7/9/10/14/20 — move a whole step, don't split the difference.
- **Radius**: `CHIP 6` (pills, badges, small buttons) · `CARD 10` (cards, inputs, selection fills) · `PANEL 14` (composer, floating panels, menus). Never nest a larger radius inside a smaller one.
- **Type** (size / line-height): `PROSE 14/1.55` · `UI 13/1.4` · `LABEL 12/1.35` · `CAPTION 11/1.3` · `CODE 12.5/1.45 mono` · `HEADING 16/1.3 semibold`. Six styles. Mono is for code, paths, and commands — never for UI labels. Prose never renders at the 1.2 default.
- **Elevation**: `POPOVER` (y2 blur10 α.09) for menus/popovers/tooltips · `PANEL` (y4 blur24 α.13) for detached surfaces. Exactly two shadows; fixed chrome gets none.
- **Border**: 1px `theme().outline()` (the alpha hairline) is the *only* border. No full-opacity strokes, no 2px borders, no `surface_2()`-as-border, no `background()`-as-border.

## Color

- **Roles, never values.** Colors come from `appearance.theme()` accessors (`background`, `surface_1/2/3`, `outline`, `main/sub/hint_text_color`, `accent`, semantic `ui_*`). Raw `ColorU::new(...)` in a view is a violation.
- **The budget.** A screen may contain: the neutral ramp, text colors, semantic colors carrying meaning, and the tab accent. Any additional hue needs a stated reason in the PR description.
- **Grouping order: whitespace → fill → border.** Prefer a gap; if structure needs more, a 2–4% alpha fill (`surface_overlay_1`); a hairline only for actionable surfaces.
- **Full-bleed saturated tints are banned.** State color on a large region (error block, selected row) renders as: 2px left accent bar + 3–4% alpha wash + normal text — never a saturated background band.
- **`PhenomenonStyle` is deprecated.** Its hardcoded hex palette ignores the active theme. No new uses; existing uses migrate to theme roles during sweeps.
- **Per-project color, precisely scoped.** The tab-backed project's color paints its identity dot and designated text accents (pane-header title, agent-pane accent). It does not paint surfaces. Panels and the sidebar are neutral so color reads as identity, not wallpaper.

## Typography rules

- Six styles, applied via the token ramp. A new size is a philosophy change, not a local decision.
- Conversation bodies are `PROSE` with `line_height 1.55` — this single rule is most of the difference between "log output" and "designed."
- `CAPTION` section headers in chrome (sidebar sections, settings groups) may letter-space slightly and use `sub_text_color`; they are the only all-caps text allowed.
- Never mix mono and UI fonts within one label.

## Component anatomy

One skeleton per component class. Deviating from an anatomy is a violation even if every individual value is a token.

- **Card** (tool run, diff, approval, artifact): radius `CARD`, hairline border, header row (`LG` horizontal / `MD`-ish vertical padding via tokens), *identical* horizontal padding for header and body (the 16-vs-14 misalignment class of bug), icon in a fixed slot, meta right-aligned in `LABEL`, optional action cluster right.
- **Pill / chip**: radius `CHIP`, `XS`/`SM` internal padding, `LABEL` text, hairline only when interactive; static status chips use a fill, no border.
- **Approval prompt**: it is a Card with a verb-first `UI` title ("Claude wants to run …"), `CODE` detail, and the action cluster (Allow once / Always / Deny). One anatomy for every provider and every tool.
- **List row** (sidebar items, session lists, settings nav): `UI` text, `CARD`-radius selection fill (`surface_overlay_2` hover, `surface_3` selected), no per-row borders, no dividers between rows.
- **Empty state**: `PROSE` copy, one clear next action, centered in the conversation measure. Never a bare void.
- **Composer**: radius `PANEL`, hairline border, internal pills follow pill anatomy, one primary send affordance.

## Shell rules

- **Projects owns the left source list** on supported macOS builds: full window height including the titlebar corner, with no logo. The traffic lights float over its reserved top area (they are real AppKit buttons compositing above the Metal layer; nothing interactive may sit beneath them). The list is neutral, edge-to-edge, separated from content by one `outline()` hairline, and uses `CAPTION` headers plus source-list rows.
- **Folder-backed projects form an app-wide library; open tabs are direct chat children.** Tabs sharing an assigned folder render one level beneath that project, while unopened saved projects remain visible and scratch projects stay window-local. A tab with several panes is still one chat row—panes never add another sidebar level. There is no global horizontal tab strip and no replacement toolbar consuming its height. Project colors remain identity dots and flow to newly created/promoted chats.
- **Files, Search, and Code Review share one right-side tool host.** A stable narrow activity strip selects one tool at a time. The active rail is full height, neutral `surface_1`, edge-to-edge, and separated by one left `outline()` hairline. Search is a peer destination rather than content layered over Files; Files and Search share the utility-rail width while Code Review retains its independent width.
- **Legacy and unsupported shells remain valid fallback layouts.** When the Projects shell feature is disabled, the existing horizontal-tab and panel rules continue to apply without erasing either shell's persisted state.
- **Window dragging** must keep working from any empty spot in the top band, including over the sidebar's header.

## Motion

warpui has no transition framework (`render()` does not re-run without a `notify()` timer), so motion is deliberate and rare: opacity/position eases of ~150ms driven by the self-rearming-timer pattern, only where they explain a state change (panel open, approval resolve). No shimmer except active streaming. When in doubt: no animation, correct layout.

## Enforcement

- **PR rule**: UI PRs state which surface class they touch and use tokens for every new value. Screenshots (light + dark) in the PR description.
- **UX-gate rubric additions**: raw literal spot-check (grep for `font_size: Some(` and off-scale paddings in the diff), border-color check (only `outline()`), color-budget check (any new hue justified?), anatomy check (does this card/pill match the skeleton?).
- **Migration stance**: new code conforms from day one; touched code gets boy-scouted to tokens; dedicated sweeps (feature 19) retire whole violation classes. No "match the surrounding legacy values" — match the philosophy, note the neighbors for the sweep.
- This document changes by PR with the owner's review, same as code.

## Do / Don't

- **Do** group with a 24px gap. **Don't** draw a box.
- **Do** render prose at 14/1.55 in a 720 measure. **Don't** ship chat text at the 1.2 UI default.
- **Do** use `outline()` for every hairline. **Don't** improvise borders from `surface_2()` or `background()`.
- **Do** put state color in a 2px accent + 4% wash. **Don't** paint a full-bleed tinted band.
- **Do** reuse the Card anatomy for anything actionable. **Don't** invent a fourth button system — there are already three (that's the point).
- **Do** let the tab color be the loudest thing on screen. **Don't** give it competition.
