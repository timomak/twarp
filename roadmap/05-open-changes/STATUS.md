# 05 — Open Changes panel

**Phase:** impl-in-review (5c)
**Spec PRs:** [#56](https://github.com/timomak/twarp/pull/56) (initial), [#58](https://github.com/timomak/twarp/pull/58) (respec — supersedes #56), both merged
**Impl PRs:** 5a [#59](https://github.com/timomak/twarp/pull/59) merged; 5c [#60](https://github.com/timomak/twarp/pull/60) — (PR [#57](https://github.com/timomak/twarp/pull/57) closed; left-panel approach scrapped)

## Scope

Rework the existing right-side **Code Review panel** (top-right Diff icon → `WorkspaceAction::ToggleRightPanel`) into a VS Code-style Source Control view. The panel surface, toggle, refresh hooks, inline diff rendering, and commit/push UI are inherited from the existing `app/src/code_review/`. New work: sidebar split (Staged Changes / Changes), hunk-level staging, in-progress-op banner, Timeline.

See README §5 and [PRODUCT.md](PRODUCT.md) / [TECH.md](TECH.md) for the full rework framing.

## Sub-phases (revised)

The original 5a–5e split assumed a from-scratch build. With the rework framing — and after the click→tab pivot during 5a review — the work now lays out as five sub-PRs:

- [x] **5a — Sidebar split.** Two sections (Staged Changes / Changes) with status glyphs, populated from `git status --porcelain=v2`. Click → opens the file in a new tab (placeholder; real diff view ships in 5e). Covers PRODUCT §§1–9, §§24–25, §§27, §§29–30 (verify).
- [ ] **5b — Hunk-level staging affordances.** Hover-revealed `[+]` / `[−]` / `[↺]` on hunk headers inside the diff. Patch synthesis + `git apply --cached` / `--reverse`. Covers PRODUCT §12. **Dependency:** lands after 5e since the hunk overlays live inside the diff pane.
- [x] **5c — In-progress op banner + file-level discard/unstage polish.** `InProgressOp` detection (`MERGE_HEAD` / `rebase-merge` / `CHERRY_PICK_HEAD` / `BISECT_LOG`), banner, conflict-row `[Resolve…]`, commit-button label gating, file-level hover affordances. Covers PRODUCT §§10–11, §§13–14. Independent of 5e. **5c polish carve-out:** the PRODUCT §11 inline `Discard changes to <basename>?` confirmation and the untracked-file 10-second undo toast are deferred to a `5c.2` follow-up — 5c ships the banner, commit-label gating, stage/unstage/resolve hover cluster, and reuses the existing modal discard dialog.
- [ ] **5d — File Timeline.** Per-file commit-history section inside the diff pane. Paged `git log --follow`. Click → commit-diff replaces working diff; `[Back to working diff]` restores. Rename badge + `↑` local-only marker. Covers PRODUCT §§18–23. **Dependency:** lands after 5e (Timeline lives inside the diff pane).
- [ ] **5e — Diff viewer pane (new).** Dedicated diff view that clicking a sidebar row opens in a new tab in the main editor area. Initial scope: unified inline diff (red/green decorations in a single editor) — `LocalCodeEditorView` with `set_base()` from the file's HEAD content, hosted in a fresh `CodePane`. Carries `content_at_head` from the panel through a new action/event chain (`CodeReviewAction::OpenFileDiffInNewTab` → `CodeReviewViewEvent` → `RightPanelEvent` → workspace handler that calls `set_base` on the new editor). Width-aware side-by-side / inline-on-narrow is **explicitly out of scope** for 5e — it's a separate diff-editor component (the upstream `CodeDiffView` was removed in feature 02 per `workspace/view.rs:6877`; rebuilding it cleanly is a multi-day chunk). 5e respec PRODUCT §8 — current `[inherits]` (inline expansion in panel) is dead. Replaces it with `[new]` (open-in-new-tab via dedicated action).

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

## Follow-up: dedicated diff viewer (5e)

The owner's review asked for a VS Code-style diff viewer that opens in a new tab on click. In 5a the row click dispatches `CodeReviewAction::OpenInNewTab` via the existing `open_code_review_file` helper, which opens the file as a regular code-editor tab — **no diff base is set on that editor**, so the user sees the file contents without red/green decorations. That's the documented gap; 5e closes it.

5e split into two pieces if appetite dictates:
- **5e.1 (unified)** — port `content_at_head` through a dedicated action/event chain and call `set_base()` on the new tab's editor. Inline unified diff only.
- **5e.2 (side-by-side)** — new two-editor `DiffPane` with synced scroll + width-aware switching to unified on narrow widths. The upstream `CodeDiffView` was removed in feature 02; rebuilding it cleanly is its own design surface.

## Why this is feature 05 (last user-facing scope)

Largest surface area; most UI; benefits the most from a stable foundation (post-AI-removal). Slotted just before the rebrand so it ships onto a tree that already reflects the fork's identity. The small markdown-render-default change at 03 is a default flip, not a structural feature, so it doesn't displace this one as the last big user-visible build.
