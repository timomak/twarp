# Design system & visual overhaul — TECH

Companion to [PRODUCT.md](PRODUCT.md); §N below references its invariants. Layout facts verified against the tree at spec time (2026-07-16).

## Sub-phases

| Sub-phase | Scope | PRODUCT § |
|---|---|---|
| **19a** | `design/PHILOSOPHY.md` + `twarp_core::ui::tokens` + UI-skill/UX-gate enforcement wiring — ships **with this spec PR** | §26 (partially), decision 5 |
| **19b** | Shell: full-height left sidebar (Codex layout), right-panel inspector restyle | §1–§13 |
| **19c** | Agent pane restyle (conversation surface) | §14–§20 |
| **19d** | Tab strip refinement + block state chrome | §21–§25 |
| **19e** | Settings + app-wide sweep (literals, PhenomenonStyle, shadows, icons) | §26–§32 |

Recommended order: 19b → 19c → 19d → 19e (19b is the owner's headline; 19c is the biggest visual win; 19d/19e are mechanical and fleet-parallelizable).

## 19a — tokens & enforcement (in this PR)

- `crates/twarp_core/src/ui/tokens.rs`: `spacing` / `radius` / `type_ramp` (`TypeStyle {size, line_height}`) / `elevation` / `border` / `measure`. Pure constants, no deps; wired via `ui/mod.rs`.
- `.claude/skills/warp-ui-guidelines/SKILL.md` gains a leading guideline pointing at PHILOSOPHY.md + tokens (the existing button-theme rule stays).
- Fleet UX-gate rubric (fleet prompt assets) gains the four checks from PHILOSOPHY "Enforcement". Follow-up in 19b's PR if the fleet files are settled then.
- Existing constants (`DEFAULT_UI_FONT_SIZE=12` `appearance.rs:12`, `HEADER_FONT_SIZE=18`, local `BODY_FONT_SIZE`/`CODE_FONT_SIZE`/`PILL_CORNER_RADIUS` in `claude_code_view.rs:160-214`) stay until their surfaces are swept; do **not** re-point them at tokens globally in 19a (that would restyle everything at once, unreviewable).

## 19b — the Codex shell

### Current layout (why this is a restructure, not a restyle)

`Workspace::render` builds `Flex::column[ tab_bar(full width), main_content ]` (`app/src/workspace/view.rs:20394-20412`); the sidebar and right panel are children of the `main_content = Flex::row` **below** the strip (`view.rs:17051-17075`, insertion at `:17653-17666`), each wrapped as a floating card via the shared `floating_panel_*` helpers (`view.rs:584-635`: margin 8, radius 10, outline border, shadow a24/blur10).

Target: `Flex::row[ sidebar(full height), Flex::column[ tab_bar, content_row ] ]` — the sidebar leaves `main_content` and becomes a top-level left column; the strip lives only in the right column.

### Seams (all verified path:line)

1. **Layout inversion** — `render` (`view.rs:20375`), `render_banner_and_active_tab` (`:17028`), panel insertion (`:17653-17657`). Lift `ChildView(left_panel_view)` (gated by `pane_group.left_panel_open`) out of the row into the new top-level column. The right panel **stays** in `main_content` (decision 3: floating inspector).
2. **Tab-strip origin** — `compute_tab_bar_left_padding` (`view.rs:16565-16586`) currently returns ~`traffic_light_data.width()/zoom + 16` ≈ 80 on mac. New rule: sidebar **open** → plain `TAB_BAR_PADDING_LEFT` (the sidebar owns the clearance); sidebar **closed** → today's value. ⚠ **Trap:** the existing `current_workspace_state.is_left_panel_open()` consulted there is the **theme chooser** (`workspace/util.rs:206`), *not* the sidebar. The sidebar flag is `pane_group.left_panel_open`. Both must be handled; do not conflate.
3. **Traffic-light zone inside the sidebar** — the buttons are real `standardWindowButton`s pinned by Auto Layout at x≈12/32/52, size 14, centerY+1 (`twarpui/src/platform/mac/objc/window.m:280-305`), compositing **over** the full-size Metal drawable (`NSFullSizeContentView`, `window.m:646`; transparent titlebar `:327`). No ObjC changes needed: the sidebar simply reserves `traffic_light_data.width()/zoom` (64, non-scaling — `traffic_lights.rs:142-148`) × the titlebar band height at its top-left (§2). Fullscreen: mirror the existing fullscreen branch (lights hidden → no reservation) (§8).
4. **Titlebar band & window drag** — band height = `TOTAL_TAB_BAR_HEIGHT`(35)·zoom pushed from `update_titlebar_height` (`view.rs:11513-11521` → `window.m:378-386`); `mouseInTitleBar:` (`host_view.m:423-430`) makes the top band a window-drag region wherever twarp UI doesn't consume the event (`host_view.m:939-947`) — §7 comes for free, but sidebar header controls must consume their clicks (the existing pill switcher already does).
5. **De-carding the sidebar** — `left_panel.rs:2746-2817`: drop the `FLOATING_PANEL_MARGIN`(8) insets (`:2810-2814`), radius/border/shadow (`:2767-2777`); keep `Resizable`/`DragBarSide::Right` (`:2786-2798`, min 250 via `drive/panel.rs:46`, max 0.75·window). ⚠ Do **not** edit the shared `floating_panel_*` helpers — they're also used by the right panel and `claude_code_view.rs:4304`; fork per-surface styling instead. The full-height hairline seam replaces the card border; kill the strip's bottom border **within the sidebar column** only (`view.rs:16609-16610` stays for the right column) (§4).
6. **Resize handle vs drag band** — with the sidebar full-height, the drag bar's top ~35px overlaps the window-drag band; the `Resizable` hit area must win there (it consumes events, so it should — verify explicitly, it's §6/§7's collision point).
7. **`WORKSPACE_PADDING`(1) (`view.rs:452`)** wraps the workspace; edge-to-edge sidebar needs it excluded on the left (§1).
8. **Adjacent systems that must not regress** (§6, §9, §13): theme-chooser panel & vertical-tabs config-panel paths reusing the panel machinery (`view.rs:17431-17481`), right-panel maximize (`:17663-17685`), WASM-mobile overlay sidebar (`:20422-20461`), cross-window tab drag ghost slots injected into the strip (`:16382-16413`).

**Feature flag**: the inversion ships behind `FeatureFlag` (e.g. `DesignShellV1`), default-on in dogfood, so the strip/sidebar can be A/B'd against the old layout during smoke; flag removal after owner sign-off (decision 6). The right-panel restyle (§11–§12: radius 10→14 via `radius::PANEL`, shadow → `elevation::PANEL`, margin 8→`LG` 16 on its three window edges) is unflagged.

### Right panel (§11–§13)

`right_panel.rs:1949-2000` — token swap on the same card helpers' *call sites* (not the shared fns), keep `Resizable`/`DragBarSide::Left`, maximize path untouched (`right_panel.rs:1952-1953`, `view.rs:17663-17685`).

## 19c — agent pane

All in `app/src/claude_code_view.rs` (10.2k lines) + `claude_code_view/{tool_cards,diff_cards,inline_action,composer,todos,thinking}.rs`.

- **Type**: replace the 8 raw sizes (audit: 46 literals; de-facto body 11.5×16) with `type_ramp`; prose gets `with_line_height_ratio(PROSE.line_height)` — the API exists (`twarpui_core/src/elements/text.rs:315`) and is simply never called in this cluster today (§14).
- **Measure & rhythm**: `measure::PROSE_MAX_WIDTH` centered column; turn spacing `spacing::XL` (§14–§15). Respect the existing `SizeConstraintSwitch` responsive tiers (PR #189).
- **Tool-run collapse** (§16): group consecutive completed `TranscriptItem` tool cards under a disclosure row (reuse `inline_action.rs` `Disclosure`); duration from existing `TurnMetrics`. Pure view-model grouping — no driver changes (keeps 18's refactor surface clean).
- **Card anatomy** (§17): normalize on `inline_action.rs` constants re-pointed at tokens; fix the header-16 / content-14 inset mismatch (`inline_action.rs:63` vs `:75`); pills → `radius::CHIP`; the radius-7-in-radius-8 badge (`claude_code_view.rs:5101-5123`) → CHIP-in-CARD.
- **Regression tripwires** (§19, all have history): SelectableArea forwarding through every scroll wrapper; `with_propagate_drag` on the Fork/copy hoverables (#214); `with_propagate_mousewheel_if_not_handled` on nested diffs; FocusSelf discipline for side-by-side panes; the elapsed-label self-rearming notify timer. A restyle touching these files must re-verify each.

## 19d — tabs & blocks

- Tabs (`app/src/tab.rs`): keep `TabStyles::default` swatch mapping (`:830-866`) + SDF flare (`:1679-1710`, shader `shaders.metal:90-108`). Active-contrast guarantee: derive active/inactive treatment from state opacities (`:124-126,146`) with a computed floor so same-color neighbors separate (§21). Indicator priority: collapse `enum Indicator` (`:746-762`) rendering to one slot (§22). Find and dedupe the double diff-counter (strip-right vs prompt pills) (§22).
- Blocks (§24–§25): locate the block state fills (bookmark/error tint paths in the block renderer) and re-anatomize: 2px accent + ≤4% wash. Terminal *content* untouched.

## 19e — sweep

Grep-driven, fleet-parallelizable, per-surface PRs: `font_size: Some(` (175 app-wide) / paddings off-scale / `CornerRadius::with_all` literals / `PhenomenonStyle` uses / `DropShadow{` definitions (5+ ad-hoc → 2 tokens) / icon dupes (`Globe/Globe4`, `Settings/Gear`, `Share/Share3`, …). Settings pages (`settings_view/*.rs`: ~18 paddings incl. `8.5`, radius-4 default) migrate to tokens + list-row anatomy (§27). Each sweep PR: mechanical, screenshots both themes, no layout moves.

## 19f — owner feedback round (§33–§38)

Owner smoke-tested the shipped shell 2026-07-16; four corrections, all within the `DesignShellV1` flag's surfaces:

- **Revert tabs-as-sidebar** (§33): 19b's shipped interpretation renders tabs as a vertical list in the left column (the UX-gate transcripts describe a "tab-list panel"). Restore the pre-19b horizontal top strip wholesale — placement, activation, reorder, cross-window drag ghost slots, overflow — while keeping the 19b strip-origin rule (starts right of the open Tools panel, traffic-light clearance when closed). The vertical-tabs *config-panel* machinery that predates 19 must be left as it was.
- **Tools panel gets the shell treatment** (§35): the full-height flush column (traffic-light zone, hairline seam, no card chrome) hosts the existing `LeftPanelView` tools (project explorer / global search / drive / agent sessions) — largely re-pointing what 19b built onto the Tools panel host and re-verifying the toggle-close path (the round-1 UX regression: a stranded content panel after close).
- **Rectangle tabs** (§34): stop applying the SDF tab-flare and top-radius on tab chips (`with_tab_flare` / `CornerRadius::with_top` call sites in `app/src/tab.rs`); the Metal shader itself stays (harmless if unused). Keep `TabStyles` color mapping, state opacities, indicator slot, contrast floor from 19d.
- **Search icon** (§36): replace the strip's persistent search input with an `Icon` button pinned at the strip's right end that opens the same search flow; reclaim the width.
- **Gear, not avatar** (§37): the top-right avatar control and its dropdown menu are removed; a NakedTheme-style gear icon button (no fill) dispatches the existing open-settings action directly. Audit the deleted menu's entries and confirm each has an existing alternate home before deletion; list them in the PR description.

Validation: PRODUCT `### 19f` smoke steps via the UX gate; the §33 revert must re-verify the 19b matrix (open/closed/fullscreen, window drag, resize) since it reshapes the same layout code.

## Risks

1. **19b blast radius** — the layout inversion touches the most upstream-divergent file (`workspace/view.rs`). Mitigations: feature flag, no shared-helper mutations, explicit checks on the four adjacent systems (seam 8), UX-gate rounds with the sidebar open/closed/fullscreen.
2. **The two "left panel" flags** (seam 2) — highest-probability logic bug; unit-test `compute_tab_bar_left_padding` for the 2×2 (sidebar × theme-chooser) matrix.
3. **Flex infinite-constraint SIGABRT** (dogfood-only debug_assert) — layout restructures have tripped it before (pane-header/restore history); run dogfood builds during 19b.
4. **Type-size jump readability** (12→13/14) at non-default zoom — verify at zoom extremes (§31).
5. **Warp upstream cherry-picks** — view.rs churn makes future ports harder; keep 19b's diff as move-not-rewrite where possible.

## Validation

- Every sub-phase PR: light+dark screenshots of each touched surface, at default and one non-default zoom (§31).
- 19b: manual matrix — sidebar open/closed × fullscreen × theme-chooser open × right panel open/maximized; window drag from sidebar top band; traffic-light clicks; resize handles; persisted-width migration (§1–§13, §32).
- 19c: golden-transcript rendering unchanged in *content* (same items, same order, same affordances) — only presentation differs; tripwire list walked one by one (§19).
- Fleet UX gate runs on every impl PR (live screenshots on other-mac), with the new rubric items active from 19b onward.
- `cargo check` + targeted `cargo test -p` per touched crate; full presubmit on other-mac (this Mac can't run it fully).
