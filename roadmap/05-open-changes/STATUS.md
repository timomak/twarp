# 05 — Open Changes panel

**Phase:** impl-in-review (5a)
**Spec PRs:** [#56](https://github.com/timomak/twarp/pull/56) (initial), [#58](https://github.com/timomak/twarp/pull/58) (respec — supersedes #56), both merged
**Impl PRs:** 5a in review — (PR [#57](https://github.com/timomak/twarp/pull/57) closed; left-panel approach scrapped)

## Scope

Rework the existing right-side **Code Review panel** (top-right Diff icon → `WorkspaceAction::ToggleRightPanel`) into a VS Code-style Source Control view. The panel surface, toggle, refresh hooks, inline diff rendering, and commit/push UI are inherited from the existing `app/src/code_review/`. New work: sidebar split (Staged Changes / Changes), hunk-level staging, in-progress-op banner, Timeline.

See README §5 and [PRODUCT.md](PRODUCT.md) / [TECH.md](TECH.md) for the full rework framing.

## Sub-phases (revised)

The original 5a–5e split assumed a from-scratch build. With the rework framing, the gap collapses to four sub-PRs:

- [x] **5a — Sidebar split.** Two sections (Staged Changes / Changes) with status glyphs, populated from `git status --porcelain=v2`. Click → existing inline diff expansion (unchanged). Covers PRODUCT §§1–9, §§24–25, §§27, §§29–30 (verify).
- [ ] **5b — Hunk-level staging affordances.** Hover-revealed `[+]` / `[−]` / `[↺]` on hunk headers inside the inline diff. Patch synthesis + `git apply --cached` / `--reverse`. Covers PRODUCT §12.
- [ ] **5c — In-progress op banner + file-level discard/unstage polish.** `InProgressOp` detection (`MERGE_HEAD` / `rebase-merge` / `CHERRY_PICK_HEAD` / `BISECT_LOG`), banner, conflict-row `[Resolve…]`, commit-button label gating, file-level hover affordances. Covers PRODUCT §§10–11, §§13–14.
- [ ] **5d — File Timeline.** New per-file commit-history section below the inline diff. Paged `git log --follow`. Click → commit-diff replaces inline diff; `[Back to working diff]` restores. Rename badge + `↑` local-only marker. Covers PRODUCT §§18–23.

Commit / push / pull / fetch (former 5e in the prior split) is **dropped** as a dedicated sub-phase: PRODUCT §§15–17 inherit the existing implementation behind `FeatureFlag::GitOperationsInCodeReview`. Gaps surfaced during verification become follow-ups, not impl PRs.

## Notes

- This is the **respec**. The originally-merged spec PR (#56) defined the feature as a new left-panel tab and proposed a brand-new `app/src/open_changes/` module. That was the wrong host — the existing right-side Code Review panel already provides most of what was needed. Owner feedback redirected the work to a rework of `app/src/code_review/`.
- 5a impl PR [#57](https://github.com/timomak/twarp/pull/57) was closed without merging. The parser logic (`parse_porcelain_v2` + 27 unit tests) carries over from `app/src/open_changes/repo.rs` into `app/src/code_review/porcelain_v2.rs` in the 5a impl PR.
- 5d (Timeline) is the most likely scope-cut candidate if the surface gets unwieldy — feature can ship as `merged` after 5c if 5a–5c are solid.
- Backed by the existing `Repository` watcher + `DiffStateModel` invalidation channel. No new file watcher.

## Why this is feature 05 (last user-facing scope)

Largest surface area; most UI; benefits the most from a stable foundation (post-AI-removal). Slotted just before the rebrand so it ships onto a tree that already reflects the fork's identity. The small markdown-render-default change at 03 is a default flip, not a structural feature, so it doesn't displace this one as the last big user-visible build.
