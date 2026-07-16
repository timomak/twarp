# 19 — Design system & visual overhaul

**Phase:** impl-pending (re-opened 2026-07-16 for the owner smoke-test feedback round, 19f)
**Spec PR:** [#215](https://github.com/timomak/twarp/pull/215) (also ships 19a: `design/PHILOSOPHY.md` + `twarp_core::ui::tokens` + UI-skill wiring)
**Impl PRs:** 19b [#219](https://github.com/timomak/twarp/pull/219), 19c [#221](https://github.com/timomak/twarp/pull/221), 19d [#222](https://github.com/timomak/twarp/pull/222), 19e (fleet)

## Scope

Owner-directed 2026-07-16. Bring the non-terminal UI to the Codex-desktop-app level of restraint, and make it enforceable: philosophy + tokens (19a), the Codex shell (19b), document-calm agent pane (19c), tab/block chrome refinement (19d), app-wide sweep (19e), and the owner's post-smoke feedback round (19f: horizontal tabs restored as plain rectangles, the full-height shell treatment on the **Tools panel**, search collapsed to an icon, gear instead of avatar).

## Sub-phases

- [x] **19a — philosophy + tokens + enforcement.** `design/PHILOSOPHY.md`, `crates/twarp_core/src/ui/tokens.rs`, warp-ui-guidelines skill pointer. Bundled into the spec PR (owner bundling rule — nothing smoke-testable alone).
- [x] **19b — the Codex shell.** Full-height left sidebar (layout inversion behind `DesignShellV1` flag, traffic-light zone, tab-strip origin), right-panel inspector restyle.
- [x] **19c — agent pane as a document.** Type ramp + prose measure, turn rhythm, tool-run collapse ("Worked for Ns"), one card anatomy, composer restyle.
- [x] **19d — tabs & blocks.** Active-tab contrast guarantee, single indicator slot, deduped status, block accent+wash treatment.
- [x] **19e — sweep.** Settings pages, raw-literal retirement, PhenomenonStyle migration, shadow/icon consolidation.
- [ ] **19f — owner feedback round.** Restore the horizontal top tab strip (revert any tabs-as-sidebar-list presentation from 19b); the full-height Codex-shell column treatment (traffic lights inside, flush edge-to-edge, no card chrome) applies to the **Tools panel** (files / search / drive / agent sessions) instead; tab chips become plain rectangles — no flare, no rounded-top silhouette — keeping per-tab colors, single indicator slot, and the active-contrast floor; the chrome's persistent search input collapses to a search icon at the far right of the strip; the top-right avatar/profile control becomes a plain gear glyph (no background fill) that opens Settings directly, and its dropdown menu is deleted. PRODUCT §33–§38.

## Smoke test

Per sub-phase in TECH.md "Validation" and PRODUCT.md `## Smoke test` (the UX gate reads PRODUCT's per-sub-phase sections; 19f's is `### 19f — owner feedback round`).
