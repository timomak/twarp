---
name: 05 — Open Changes panel
status: rework
supersedes: roadmap/05-open-changes/PRODUCT.md@PR#56
---

# Open Changes panel — PRODUCT

## Summary

Rework the existing right-side **Code Review panel** (toggled by the top-right Diff icon → `WorkspaceAction::ToggleRightPanel`) into a VS Code-style Source Control view. Today the panel shows a flat list of edited files; this feature splits the list into separate **Staged Changes** and **Changes** sections with per-file status glyphs, adds a per-file **Timeline**, and closes gaps in hunk-level staging and in-progress-op handling. The panel surface, toggle button, refresh path, inline-diff expansion, and commit/push controls are inherited from the existing panel.

## Why this revision

The first revision of this spec assumed a brand-new left-side panel tab next to "Custom shortcuts." That was wrong — the existing right-side Code Review panel already provides most of what feature 05 needed (file list, inline diff expansion, refresh on repo change, commit/push UI, discard affordances). Building a parallel surface duplicated infrastructure and skipped over working code. This revision reframes the feature as a focused rework of `app/src/code_review/`.

## Goals / Non-goals

**Goals**

- Preserve the existing right-side panel surface, toggle button, keyboard binding, and diff click-flow.
- Split the existing flat file sidebar into two sections (**Staged Changes** above, **Changes** below) with per-file status glyphs (M / A / D / R / C / U / ?).
- File and hunk-level **stage / unstage / discard** with VS Code parity (hover-revealed `[+] [−] [↺]` cluster, inline discard confirmation, untracked-file undo toast).
- In-progress merge / rebase / cherry-pick / bisect banner.
- Per-file **Timeline** section — paged `git log --follow` for the focused file, click → commit-diff replaces the inline diff for that file.
- Use the existing `DiffStateModel` invalidation path for refreshes — no new file watcher.

**Non-goals**

- No new panel host. The right-side Code Review panel stays; this is a sidebar + behavior rework, not a new surface.
- No replacement of the existing inline-diff expand-on-click flow. Click → expand inline, same as today.
- No replacement of the existing commit dialog (gated by `FeatureFlag::GitOperationsInCodeReview`). Verify behavior; do not redesign.
- No branch management, conflict-resolution UI, stash management, submodule traversal, LFS-aware diffs, multi-repo workspace view, or commit-draft persistence across restarts. Same non-goals as the prior revision.
- No 3-way merge UI; conflict resolution stays an external-editor handoff via `WorkspaceAction::OpenFileInNewTab`.

## Behavior

Section numbering picks up where the prior revision left off so cross-references from TECH.md and STATUS.md stay valid. Invariants tagged **[inherits]** describe behavior already present in the existing panel and assert that this feature must not regress it. Invariants tagged **[new]** are net-new work.

### Panel surface

1. **[inherits]** **Where it lives.** The panel is the existing right-side slide-over toggled by `WorkspaceAction::ToggleRightPanel` (top-right Diff icon button in the workspace header). No new tab in the left panel; no second toggle. The keyboard binding (`workspace:toggle_right_panel`) is unchanged.

2. **[inherits]** **Panel repo.** The panel scopes to the active pane group's git repo, derived from the focused-pane cwd. Switching focus to a pane inside a different repo retargets the panel. Walking up `.git` to find the working-tree top-level is `Repository` model behavior; do not duplicate.

3. **[inherits / verify]** **No-repo state.** Opening the panel in a non-repo context shows the existing no-repo affordance. If today's empty-state copy isn't `No git repo in the focused pane.` per the prior spec, leave it as-is — verify but don't redesign.

4. **[new]** **Sidebar split.** The existing flat file sidebar splits into two collapsible sections, top to bottom:
    - **Staged Changes  ·  N** — entries from `git status --porcelain=v2`'s staged column.
    - **Changes  ·  N** — entries from the working-tree column (modified, deleted, untracked, copy/rename destination).
    Each section header shows its file count. Both sections render even when empty (`· 0`), so the user can see "nothing staged, two unstaged changes" at a glance.

5. **[new]** **Status glyph column.** Each row gains a single-character status glyph on the left: `M` (modified), `A` (added), `D` (deleted), `R` (renamed), `C` (copied), `U` (unmerged, bold), `?` (untracked, italic). Color comes from the theme; no hard-coded colors.

6. **[inherits]** **+/- line counts per row.** The existing additions/deletions number stays on each row (right side, or wherever the existing layout puts it). It is informational, not the primary identifier.

7. **[new]** **Sort order.** Within each section, rows sort by full path lexicographically (case-insensitive). Renames sort by destination path. Sort is stable across refreshes.

8. **[inherits]** **Click → inline diff.** Left-clicking a row dispatches `CodeReviewAction::FileSelected(idx)` and the diff expands inline below the row, rendered by the existing `CodeReviewEditorView`. Click the same row again to collapse. Click a different row to switch the inline expansion.

9. **[inherits]** **Diff format.** Unified-diff with hunk headers, +/- line tinting, themed colors, expand/collapse — all the existing behavior carries over.

### Stage / unstage / discard

10. **[mixed]** **File-level hover actions.** On hover, each row reveals an action cluster on the right:
    - **Changes row:** `[+]` Stage file, `[↺]` Discard file. `[↺]` may already exist via `FeatureFlag::DiscardPerFileAndAllChanges`; if so, keep its behavior and add `[+]`.
    - **Staged Changes row:** `[−]` Unstage file.
    - **Conflict (`U`) row:** `[Resolve…]` only; clicking dispatches `WorkspaceAction::OpenFileInNewTab` on the conflicted path. No stage/unstage/discard affordances.

11. **[mixed]** **File-level operations.** Stage = `git add -- <path>`; unstage = `git restore --staged -- <path>` (or `git rm --cached` for initial-commit case); discard-modified = `git restore -- <path>`; discard-untracked = `fs::remove_file(<path>)` with a 10-second undo toast that restores the file's bytes and mtime. The discard confirmation shows inline beneath the row (`Discard changes to <basename>? [Cancel] [Discard]`); the untracked variant reads `Delete untracked file <basename>?` to signal irreversibility.

12. **[new]** **Hunk-level affordances.** Hovering a hunk header inside the inline diff reveals the same `[+] [↺]` (in Changes) or `[−]` (in Staged Changes) cluster, scoped to that hunk. Stage hunk = `git apply --cached <synthesized-patch>`; unstage hunk = `git apply --cached --reverse <patch>`; discard hunk = `git apply --reverse <patch>`. If the patch no longer applies (working tree drifted since render), refresh + retry once; on second failure, toast `Hunk no longer applies — refresh and try again.`

13. **[new]** **In-progress op banner.** When `.git/MERGE_HEAD` / `.git/rebase-merge` / `.git/CHERRY_PICK_HEAD` / `.git/BISECT_LOG` exists, the panel renders a banner above the sidebar: `<Op> in progress — resolve conflicts then commit, or run \`<abort-cmd>\` in the terminal.` Conflict rows appear in both sections with the `U` glyph (deduped by path). Commit button label changes to `Continue rebase` / `Conclude merge` / `Continue cherry-pick`.

14. **[inherits]** **Idempotence and race handling.** Stage/unstage/discard on an already-stage/unstaged/cleared file is a no-op (no error). Operations that fail because the underlying state drifted between render and click trigger a refresh and surface a toast; this matches the existing panel's behavior and should not regress.

### Commit, push, pull, fetch

15. **[inherits]** **Commit input + button.** The existing header commit affordance (gated by `FeatureFlag::GitOperationsInCodeReview`) stays. The commit message input, commit button enablement, amend toggle, character-count indicator, and stderr error banner all carry over from the existing implementation. No redesign in this feature.

16. **[inherits]** **Push / pull / fetch.** The existing git operations menu surfaces these. Verify the no-upstream tooltip behavior matches the prior spec (§29) — if it doesn't, that's a future polish, not a 05 blocker.

17. **[inherits]** **No background mutation.** The panel never silently runs a state-mutating git op; mutations only happen on explicit user action. Verify and do not regress.

### Timeline

18. **[new]** **Scope.** A new section below the inline diff. Header: `Timeline · <basename>` when a file is focused, `Timeline` (with a hint) otherwise. Tracks the most-recently-clicked file even if that file is no longer in either section.

19. **[new]** **Entries.** Each entry: author-avatar circle (single letter, color hashed from email), author name, relative time (with absolute on hover), commit subject. Rendered in reverse chronological order; first page = 20 entries.

20. **[new]** **Paging.** `[Load more]` at the bottom appends the next 20.

21. **[new]** **Click → commit-diff.** Clicking a Timeline entry replaces the inline diff with the focused file's diff at that commit (`git show <sha> -- <path>`). A `[Back to working diff]` link above the diff returns to the working/index diff. Read-only — no stage/unstage/discard in the commit-diff view.

22. **[new]** **Rename tracking.** `git log --follow` for the focused file. Entries from before the rename show the original path in a tooltip; the rename commit gets a small `R` badge.

23. **[new]** **Local-only marker.** Entries whose commit is ahead of upstream show an `↑` next to the relative time. No marker when there's no upstream configured.

### Refresh, errors, performance

24. **[inherits]** **Refresh trigger.** The existing `DiffStateModel` / `Repository` invalidation path is the refresh trigger. Do **not** add a new `BulkFilesystemWatcher`; reuse the existing subscription. PRODUCT §7 / §38 from the prior revision (250ms coalescing, post-command refresh, etc.) is whatever the existing panel does — verify and inherit.

25. **[mixed]** **Refresh produces staged + changes.** The model's status fetch (whatever the existing panel calls today; likely `git status --porcelain=v2`) populates both sections. The parser must distinguish staged-only, unstaged-only, partial-stage (file appears in both with same path), and unmerged (deduped to single conflict row visible in both sections).

26. **[inherits]** **Errors never silent.** Any git operation failure surfaces a verbatim-stderr banner with a `[Copy error]` button. Existing implementation; verify.

27. **[inherits]** **Performance ceiling.** The panel must stay responsive on repos with up to 10,000 changed files. Use whatever virtualization the existing sidebar uses; if it's flat-list-only and breaks at 200+ rows, gate at 200 and add virtualization in a follow-up.

### Accessibility & telemetry

28. **[mixed]** **Keyboard reachable.** Tab traversal through the sidebar must visit each row in order. Per-row keyboard shortcuts (`s` to stage / unstage, `d` to discard, `Enter` to expand/collapse the diff, `⌘Enter` / `Ctrl+Enter` to commit from the message input) match the prior spec where feasible; missing shortcuts can ship in a polish pass.

29. **[inherits]** **Themed visuals.** Status glyph colors, banners, avatars, focused-row highlight — all come from the active theme. No hard-coded colors. Lift the existing theme accessors used by the panel.

30. **[inherits]** **Telemetry.** The existing `CodeReviewTelemetryEvent` channel carries telemetry for this surface. Do not add a parallel channel; if a new event is needed for the staged/unstaged split or Timeline, add it to `CodeReviewTelemetryEvent`.

## Smoke test

Run against a freshly built twarp binary inside a real git repo (use the twarp repo itself or a scratch repo with a few commits). Chord names below are macOS; substitute Ctrl for ⌘ on Linux/Windows. **"Open the panel"** means: click the top-right Diff icon in the workspace header (or press the `workspace:toggle_right_panel` keybinding).

### 5a — Sidebar split

1. Launch twarp. Open the panel inside a clean git repo. Two section headers render: `Staged Changes  ·  0` and `Changes  ·  0`. No file rows.
2. In the terminal, `touch newfile.txt`. The panel refreshes; `Changes` count goes to 1 and a row `?  newfile.txt` appears (untracked status glyph).
3. `git add newfile.txt`. The row migrates from Changes to Staged Changes; its glyph becomes `A`.
4. Modify a tracked file in the terminal (`echo hi >> README.md`). A row `M  README.md` appears in Changes; the staged `A newfile.txt` stays in Staged Changes.
5. Stage a hunk of `README.md` via the panel: hover the hunk header → click `[+]`. The file appears in **both** Staged Changes (the staged hunk) and Changes (the remaining unstaged hunk).
6. Sort check: stage four files with names `Aaa.rs`, `bbb.rs`, `Mmm.rs`, `zzz.rs`. The Staged Changes section lists them in case-insensitive lexicographic order.

### 5b — Hunk-level staging affordances

7. Click a row in Changes to expand its diff. Hover a hunk header → `[+]` and `[↺]` appear. Click `[+]`. The hunk migrates to the staged side; the Staged Changes section shows the file with the hunk's content; Changes shows only the remaining hunks.
8. Hover the staged file's hunk header → `[−]` appears. Click it. Hunk migrates back to Changes.
9. With a 3-hunk modified file, stage hunk 2. The file appears in both sections; the Changes side shows hunks 1 and 3; the Staged side shows hunk 2.

### 5c — In-progress op banner + discard polish

10. Create a merge conflict (`git merge` a divergent branch with conflicting changes). The repo header reads `(merging from <branch>)`; a banner says `Merge in progress — resolve conflicts then commit, or run \`git merge --abort\` in the terminal.` Conflicted file rows show `U` glyph in both sections; hover them and only `[Resolve…]` appears.
11. Click `[Resolve…]` on a conflict row. The file opens in a new twarp tab with conflict markers visible. Resolve and `git add` in the terminal. The `U` row disappears; the Commit button is now labeled `Conclude merge` and enabled.
12. Discard a tracked modification: click `[↺]`, confirm. The file's working-tree content reverts to match the index (verify with `git diff`).
13. Discard an untracked file: click `[↺]`, confirm. The file is removed from disk. An undo toast appears with `[Undo]`; click within 10 seconds → the file reappears with original content and mtime.

### 5d — Timeline

14. Focus a file with multiple commits in history. The Timeline section header shows `Timeline · <basename>`. 20 most-recent entries render with avatar, author, relative time, subject.
15. Click an older Timeline entry. The inline diff replaces the working/staged diff with the commit's diff for that file. A `[Back to working diff]` link appears.
16. Click `[Back to working diff]`. The inline diff returns to the working-tree view.
17. Click `[Load more]`. The next 20 entries append without scroll jump.
18. `git mv old.txt new.txt && git commit`. Focus the renamed file. Timeline entries from before the rename show the old path in a tooltip; the rename commit shows a small `R` badge.
19. Make and commit a local change (no push). The new Timeline entry shows `↑` next to its relative time. Push the branch. Refresh — the `↑` marker is gone.

### Cross-cutting

20. Open the panel; run `git checkout -b new-branch` in any pane inside the repo. Within a few hundred ms the repo header's branch name updates to `new-branch` (existing `Repository` subscriber trigger).
21. Run `git add .` in a pane while the panel has a Discard confirmation visible. The lists refresh, the confirmation closes silently, no operations fire from the cancelled confirmation.
