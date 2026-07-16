# 19 — Design system & visual overhaul

**Phase:** impl-pending (spec merged 2026-07-16)
**Spec PR:** [#215](https://github.com/timomak/twarp/pull/215) (also ships 19a: `design/PHILOSOPHY.md` + `twarp_core::ui::tokens` + UI-skill wiring)
**Impl PRs:** —

## Scope

Owner-directed 2026-07-16. Bring the non-terminal UI to the Codex-desktop-app level of restraint, and make it enforceable: philosophy + tokens (19a), the Codex shell — full-height macOS left sidebar with the traffic lights inside it, tab strip starting to its right, right panel as a floating inspector (19b), document-calm agent pane (19c), tab/block chrome refinement — per-tab colors and flares stay (19d), app-wide sweep (19e). Pure restyle; zero behavior change. Ships **before** feature 18 (Codex backend).

## Sub-phases

- [x] **19a — philosophy + tokens + enforcement.** `design/PHILOSOPHY.md`, `crates/twarp_core/src/ui/tokens.rs`, warp-ui-guidelines skill pointer. Bundled into the spec PR (owner bundling rule — nothing smoke-testable alone).
- [x] **19b — the Codex shell.** Full-height left sidebar (layout inversion behind `DesignShellV1` flag, traffic-light zone, tab-strip origin), right-panel inspector restyle.
- [x] **19c — agent pane as a document.** Type ramp + prose measure, turn rhythm, tool-run collapse ("Worked for Ns"), one card anatomy, composer restyle.
- [x] **19d — tabs & blocks.** Active-tab contrast guarantee, single indicator slot, deduped status, block accent+wash treatment.
- [x] **19e — sweep.** Settings pages, raw-literal retirement, PhenomenonStyle migration, shadow/icon consolidation.

## Smoke test

Per sub-phase in TECH.md "Validation". Headline (19b): sidebar open → full-height column with traffic lights inside it and tabs starting at its right edge; drag the window from the sidebar's top band; fullscreen round-trip; sidebar closed → today's chrome exactly.
