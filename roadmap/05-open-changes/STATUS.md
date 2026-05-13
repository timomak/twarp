# 05 — Open Changes panel

**Phase:** impl-in-review (5a)
**Spec PRs:** [#56](https://github.com/timomak/twarp/pull/56) (initial), [#58](https://github.com/timomak/twarp/pull/58) (respec — supersedes #56), both merged
**Impl PRs:** 5a [#59](https://github.com/timomak/twarp/pull/59) — (PR [#57](https://github.com/timomak/twarp/pull/57) closed; left-panel approach scrapped)

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

## Spec deviations adopted during 5a impl review (PR #59)

Owner feedback during 5a review reframed the panel's interaction model:

- **Click → new tab, not inline expansion.** PRODUCT §8 (`[inherits]`: click → expand inline below the row) is **superseded**. Clicking a sidebar row now dispatches `CodeReviewAction::OpenInNewTab`, which opens the file in a tab in the main editor area. The legacy inline-diff list inside the Code Review panel is removed; the panel becomes sidebar-only.
- **Sidebar always visible.** `file_sidebar_expanded` defaults to `true`; the file-nav toggle button is removed from the panel header. Section-level collapse/expand on the `Staged Changes` / `Changes` headers replaces the per-sidebar toggle.
- **`warp-oss` enables `GitOperationsInCodeReview`.** Upstream gates the right-side panel layout behind a Preview flag, but the rework is twarp's canonical Code Review surface. Enabled unconditionally in `app/src/bin/oss.rs` so `cargo run` shows it.

## Follow-up: dedicated diff viewer (post-5a)

The owner's review also asked for a VS Code-style diff viewer with width-aware side-by-side / inline-on-narrow switching. The 5a PR routes clicks to `WorkspaceAction::OpenFileInNewTab` via the existing `open_code_review_file` helper, which opens the file with whatever target the workspace chooses — typically the code editor with gutter diff markers. That's the minimum-viable diff destination; a bespoke side-by-side viewer is **not** part of 5a. Treat as its own scoped work item once 5b/5c/5d are stable.

## Why this is feature 05 (last user-facing scope)

Largest surface area; most UI; benefits the most from a stable foundation (post-AI-removal). Slotted just before the rebrand so it ships onto a tree that already reflects the fork's identity. The small markdown-render-default change at 03 is a default flip, not a structural feature, so it doesn't displace this one as the last big user-visible build.
