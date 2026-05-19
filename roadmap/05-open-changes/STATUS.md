# 05 — Open Changes panel

**Phase:** impl-in-review (5d [#63](https://github.com/timomak/twarp/pull/63) open)
**Spec PRs:** [#56](https://github.com/timomak/twarp/pull/56) (initial), [#58](https://github.com/timomak/twarp/pull/58) (respec — supersedes #56), both merged
**Impl PRs:** 5a [#59](https://github.com/timomak/twarp/pull/59) merged; 5c + 5e bundled [#60](https://github.com/timomak/twarp/pull/60) merged; 5e polish [#61](https://github.com/timomak/twarp/pull/61) merged (deleted-line click/copy); 5b [#62](https://github.com/timomak/twarp/pull/62) merged; 5d [#63](https://github.com/timomak/twarp/pull/63) open. PR [#57](https://github.com/timomak/twarp/pull/57) closed; left-panel approach scrapped.

## Scope

Rework the existing right-side **Code Review panel** (top-right Diff icon → `WorkspaceAction::ToggleRightPanel`) into a VS Code-style Source Control view. The panel surface, toggle, refresh hooks, inline diff rendering, and commit/push UI are inherited from the existing `app/src/code_review/`. New work: sidebar split (Staged Changes / Changes), hunk-level staging, in-progress-op banner, Timeline.

See README §5 and [PRODUCT.md](PRODUCT.md) / [TECH.md](TECH.md) for the full rework framing.

## Sub-phases (revised)

The original 5a–5e split assumed a from-scratch build. With the rework framing — and after the click→tab pivot during 5a review — the work now lays out as five sub-PRs:

- [x] **5a — Sidebar split.** Two sections (Staged Changes / Changes) with status glyphs, populated from `git status --porcelain=v2`. Click → opens the file in a new tab (placeholder; real diff view ships in 5e). Covers PRODUCT §§1–9, §§24–25, §§27, §§29–30 (verify).
- [x] **5b — Hunk-level staging affordances.** Hover-revealed `[+]` / `[−]` / `[↺]` on hunk headers inside the diff. Patch synthesis + `git apply --cached` / `--reverse`. Covers PRODUCT §12. New `app/src/code_review/hunk_patch.rs` synthesizes single-hunk unified diffs and pipes them to `git apply` via a new stdin-capable helper. New `StageHunkButton` / `UnstageHunkButton` gutter buttons sit next to the existing `RevertHunkButton`; clicks emit `CodeEditorEvent::{Stage,Unstage}HunkRequested` which flow through `LocalCodeEditorView` to `WorkspaceView::dispatch_hunk_stage_op`, which resolves the repo and calls new `DiffStateModel::stage_hunk` / `unstage_hunk`.
- [x] **5c — In-progress op banner + file-level discard/unstage polish.** `InProgressOp` detection (`MERGE_HEAD` / `rebase-merge` / `CHERRY_PICK_HEAD` / `BISECT_LOG`), banner, conflict-row `[Resolve…]`, commit-button label gating, file-level hover affordances. Covers PRODUCT §§10–11, §§13–14. Independent of 5e. **5c polish carve-out:** the PRODUCT §11 inline `Discard changes to <basename>?` confirmation and the untracked-file 10-second undo toast are deferred to a `5c.2` follow-up — 5c ships the banner, commit-label gating, stage/unstage/resolve hover cluster, and reuses the existing modal discard dialog.
- [x] **5d — File Timeline.** Per-file commit-history section. Paged `git log --follow --name-status --format=...`. Rename badge + `↑` local-only marker. Covers PRODUCT §§18–20, §22–§23. PRODUCT §21 (click → commit-diff swap) is carved out to a `5d.2` follow-up — the diff swap needs synthetic content support in `LocalCodeEditorView`, which is its own plumbing surface.
- [x] **5e — Diff viewer pane.** Clicking a sidebar row switches the panel from sidebar view to a single-file diff view. A `← Files` header at the top returns to the sidebar. The diff reuses the per-file `LocalCodeEditorView` that the panel already constructs via `apply_diff_to_code_editor` (HEAD content already set as the base), so unified inline diff (red/green decorations) appears out of the box. **Pragmatic pivot from the planned new-tab design:** opening a real tab in the workspace would have required new public APIs on `TabData`, a `pending_diff_base` plumb-through to `LocalCodeEditorView`, and a workspace handler with async-timing for `BufferLoaded` — substantial cross-component churn for a result the user already has via the in-panel view. Width-aware side-by-side / inline-on-narrow remains out of scope (the upstream `CodeDiffView` was removed in feature 02; rebuilding side-by-side is its own multi-day chunk). 5e supersedes PRODUCT §8 `[inherits]` (inline-expansion under the row) with the panel-switching model documented here.

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

## Spec deviations adopted during 5c+5e impl review (PR #60)

Owner feedback during the first 5e review redirected the row-click target surface:

- **Click → split pane, not new tab.** The earlier 5e cut opened a new workspace tab via `WorkspaceView::add_tab_for_code_file`. Owner asked for a split pane inside the current tab so the terminal stays visible next to the diff. `WorkspaceView::open_file_diff_in_new_pane` now constructs a `CodePane` and attaches it via `pane_group.add_pane_with_direction(Direction::Right, …)`. The `OpenFileDiffInNewTab` event/action names are left as-is to limit churn — their handler is the renamed `open_file_diff_in_new_pane`.
- **Reuse the existing diff pane on subsequent clicks.** A row click checks `PaneGroup::diff_pane_id` for an existing diff pane in the active tab. If present (and not hidden-for-undo-close), `CodeView::replace_with_single_path` strict-swaps its contents to the new path (closing other tabs in that pane). If absent, a fresh split pane is created and its `PaneId` is recorded. Stale IDs are tolerated — the lookup verifies the pane still exists via `code_pane_by_id`.
- **Right Code Review panel is workspace-level, not per-tab.** PRODUCT §1 says the panel toggles via `WorkspaceAction::ToggleRightPanel`, with no per-tab scoping called out. The original implementation stored `right_panel_open` / `is_right_panel_maximized` on each `PaneGroup`, so the panel disappeared when switching tabs. These now live on `WorkspaceState` (`is_code_review_panel_open` / `is_code_review_panel_maximized`); per-`PaneGroup` fields remain as mirrors kept in sync via `WorkspaceView::sync_code_review_panel_state_to_pane_groups`. The sync runs on every toggle, maximize, restore-from-snapshot, transferred-tab arrival, and `set_active_tab_index`. Panel **content** still scopes to the active tab's pane group (current behavior — each tab can have its own repo).
- **`expand_diffs` after `set_base`.** `LocalCodeEditorView::set_pending_diff_base_on_load` and its deferred-on-load handler now call `editor.expand_diffs(ctx)` after `set_base`, matching `CodeReviewView::apply_diff_to_code_editor`. Without it, removed lines never render and the user sees the working-tree content with no red decorations.

## Follow-up: dedicated diff viewer (5e)

5e closes the gap surfaced by 5a: clicking a row opened a regular code-editor tab with no diff base, so the user saw raw file contents with no red/green decorations. PR #60 lands the unified-inline form (5e.1) in a split pane inside the current tab; the side-by-side form (5e.2) remains a follow-up.

- **5e.1 (unified, in PR #60)** — `content_at_head` flows from `CodeReviewView` through `CodeReviewAction::OpenFileDiffInNewTab` and `RightPanelEvent::OpenFileDiffInNewTab` to `WorkspaceView::open_file_diff_in_new_pane`, which constructs a `CodePane` split, applies `set_pending_diff_base_on_load` on the resulting editor, and calls `expand_diffs` so hunks render inline. Subsequent row clicks reuse the existing diff pane (strict swap) via `PaneGroup::diff_pane_id`.
- **5e.2 (side-by-side)** — new two-editor `DiffPane` with synced scroll + width-aware switching to unified on narrow widths. The upstream `CodeDiffView` was removed in feature 02; rebuilding it cleanly is its own design surface.

## Spec deviations adopted during 5d impl

PRODUCT.md's Timeline section (§§18–23) was written before the 5e click→split-pane pivot landed, when "the inline diff" meant the in-panel inline expansion. After 5e the diff lives in a generic `CodePane<CodeView>` split off the active tab; the language doesn't quite map onto the new structure. The first cut of 5d put Timeline inside the right Code Review panel; owner feedback ([PR #63 review](https://github.com/timomak/twarp/pull/63)) redirected it to the **bottom of the left Project Explorer panel**, matching VS Code's Source Control Timeline placement. The rework adopts the following:

- **Timeline section lives at the bottom of the Project Explorer (left panel), not inside the Code Review (right) panel.** This is the architectural model VS Code ships and the owner explicitly named. The Project Explorer's content area becomes a vertical stack — file tree on top, collapsible Timeline pinned to the bottom. Each section has its own scroll surface, so scrolling commits doesn't drag the file tree.
- **Collapsed by default.** The Timeline header is always visible (chevron + `TIMELINE` label + optional `· <basename>`); clicking the header toggles expanded/collapsed. Collapsed = header-only, no body. Expanded = header + scrollable entry list.
- **Drag-resizable when expanded.** A `Resizable` (DragBarSide::Top, default 220px, bounded `(80px, 70% of window height)`) wraps the expanded section so the user can drag the divider above the Timeline header to give it more vertical space. The file tree above takes the remaining height. The handle is session-scoped — not persisted across restarts in this PR (follow-up via `PaneSettings` if the owner wants persistence).
- **Avatars are Gravatar images, not letter circles.** PRODUCT §19's "color hashed from email" is interpreted as VS Code does: the email is SHA-256-hashed (Gravatar's recommended modern scheme) and fed to `https://www.gravatar.com/avatar/<hash>?d=identicon&s=44`. The `Avatar` widget natively handles fetch + initial-letter fallback while loading; `d=identicon` means users without a Gravatar account still get a deterministic generated avatar instead of a blank.
- **Timeline focus tracks the active editor file, not Code Review sidebar clicks.** Subscribing to the active pane group's `ActiveFileModel` means switching tabs (or opening a file from the Project Explorer / cmd-P / file tree) refocuses the Timeline. This matches VS Code semantics and detaches Timeline from any Code Review interaction.
- **Click → read-only commit diff in a split pane (PRODUCT §21).** Clicking a Timeline entry fetches `git show <sha>:<path>` (post) and `git show <sha>^:<path>` (pre), writes the post content to a session-scoped tempdir (`tempfile::TempDir` → auto-deletes on workspace drop), and opens it through a new `WorkspaceView::open_file_diff_in_new_pane_with_options(..., read_only=true)` overload. `read_only` skips the 5b hunk Stage/Unstage button wiring and marks the editor's `InteractionState::Selectable` so the user can scroll/copy but not type. Subsequent clicks reuse the same diff pane (same path as 5e). For the first commit on a branch (`<sha>^` doesn't resolve), the base falls back to an empty string and the editor renders the file as fully added.
- **No `[Back to working diff]` link.** The user closes the commit-diff pane (or clicks another row in the sidebar) to return to the working diff. Adding a dedicated chrome bar would mean a new variant; the existing pane close gesture is sufficient.

## Spec deviations adopted during 5b impl

Spec text in PRODUCT §12 names two distinct surfaces — `[+] [↺]` on the **Changes** side, `[−]` on the **Staged Changes** side. The rework's single split-pane diff view (post-PR #60) collapses both sides into one HEAD-vs-working-tree diff that's shared between Staged and Changes row clicks (PRODUCT §11 risk mitigation: "Partial-stage entries get two indices; clicking either expands the same file's diff"). With one diff view per file, there's no natural place to gate which button shows.

- **All three buttons appear on every hunk in the diff pane.** Hover reveals `[+]` (stage), `[−]` (unstage), `[↺]` (revert) regardless of which sidebar row was clicked. Clicking a button on a hunk that's already in the requested state surfaces a `patch does not apply` error in the log + refreshes diff state; the next refresh removes the rendered hunk.
- **Discard-hunk semantics unchanged.** PRODUCT §12 specifies `git apply --reverse <patch>` for discard. The pre-existing `RevertDiffHunk` action does an in-buffer revert via `CodeEditorModel::reverse_diff_by_index` followed by the user saving — functionally equivalent for the user, but undoable from the editor. 5b leaves the existing path in place; promoting to a git-level discard is a follow-up.
- **The split-pane diff is the host surface, not the legacy in-panel inline diff.** The pre-existing `with_revert_diff_hunk_button()` calls inside `CodeReviewView::create_code_review_model_with_global_buffer` and `create_code_review_model` (lines ~3446, ~3534) are not extended — those construct editors for the legacy inline-diff list, which the rework deprecated in 5a. 5b's buttons are enabled via runtime setters from `WorkspaceView::open_file_diff_in_new_pane`.

## Why this is feature 05 (last user-facing scope)

Largest surface area; most UI; benefits the most from a stable foundation (post-AI-removal). Slotted just before the rebrand so it ships onto a tree that already reflects the fork's identity. The small markdown-render-default change at 03 is a default flip, not a structural feature, so it doesn't displace this one as the last big user-visible build.
