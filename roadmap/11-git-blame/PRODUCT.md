# 11 - Git blame & per-line history

## Summary

Show Git blame metadata directly in the file editor gutter so a user can see who last touched each line without leaving the editor. Each committed line shows an author, short commit hash, and relative date; clicking the author or hash opens commit details and the relevant diff for that commit.

## Problem

Twarp already has a file-editing workflow, diff indicators, and commit timeline surfaces, but a user still has to leave the editor or run `git blame` manually to understand why a line exists. The blame gutter should make line ownership and history visible at the point where the user is reading or editing code.

## Goals

1. Make per-line authorship visible in the editor with minimal disruption to line numbers, diff indicators, comments, and text selection.
2. Keep the editor responsive while Git data is loading, stale, unavailable, or expensive to compute.
3. Provide a direct path from a blamed line to the commit message and the commit diff relevant to the open file.
4. Keep partially implemented work dark behind a feature flag until both sub-phases are complete enough for users.

## Non-goals

1. Do not build a full commit browser, branch history graph, or multi-file commit review surface in this feature.
2. Do not add editing operations such as revert-line, cherry-pick, checkout, or open PR from a blame entry.
3. Do not require GitHub. GitHub linking is optional and additive when the origin remote can be resolved.
4. Do not show blame for synthetic temp files, new unsaved files, binary files, or files outside a detected Git repository.

## Figma

Figma: none provided.

## Behavior

1. When Git blame is enabled and a local text file in a detected Git repository is opened in the regular file editor, the editor gutter reserves a blame area next to the existing line number and diff indicator area.

2. The editor content, line numbers, cursor, selection, syntax highlighting, diagnostics, comments, and diff indicators render immediately. Blame data loads asynchronously and must never block opening, scrolling, typing, saving, search, or selection.

3. While blame data is loading for a file, the blame area is blank. The UI does not show a blocking spinner in the gutter and does not show a toast just because blame is pending.

4. Once blame data is available, each visible current-file line that has committed blame shows:
   - The Git author display name.
   - A short commit hash, using the first 7 hexadecimal characters.
   - A relative author date such as `3d ago`, `2mo ago`, or `1y ago`.

5. The blame annotation belongs to the current line, not to a visual row of wrapped text. If a long source line wraps onto multiple screen rows, the blame text appears once, aligned with the first visual row for that line.

6. Blame annotations are shown only for visible file lines. Collapsed hidden ranges do not render one blame row per hidden line; when the user expands the range, the now-visible lines show their annotations if blame data exists.

7. Deleted lines shown as temporary diff blocks do not show blame annotations in this feature. The blame gutter describes the current contents of the open file.

8. If consecutive lines share the same commit, each line still resolves to that commit. The UI may visually de-emphasize repeated values, but clicking any visible author or hash opens the correct commit details for that line.

9. If the author name, hash, or date would overflow the available gutter width, the blame text truncates before it overlaps code text, line numbers, diff indicators, or gutter buttons. Line numbers and diff indicators have higher priority than blame text.

10. If the editor is too narrow to show blame legibly, the blame area may collapse or hide, but the editor remains fully usable and the code text must not be pushed into an unusable width.

11. Blame colors follow the active theme. The annotation should be subdued compared with code text, and the clickable author/hash affordance should be visible on hover or focus without introducing hard-coded colors.

12. A line whose current buffer content is newly inserted or edited and not yet attributable to a committed revision shows `(uncommitted)` instead of a stale author/hash/date. The UI must never display an old commit hash for a line that the user has changed in the current buffer.

13. For dirty buffers, unchanged lines may continue to show their last known committed blame when the mapping is unambiguous. Lines with ambiguous mapping after insertions, deletions, or replacements should prefer `(uncommitted)` or a blank blame value over stale blame.

14. Saving the file refreshes blame for the saved contents. If saved changes are still uncommitted relative to `HEAD`, those changed lines continue to show `(uncommitted)` until they are committed.

15. When the repository state changes outside the editor in a way that can affect blame, such as commit, checkout, rebase, reset, or external file update, the visible blame data is invalidated and refreshed. During refresh, the editor remains usable.

16. Blame results are scoped to the exact repository and file path. Switching tabs, renaming a file, moving between repositories, or opening the same filename from another repository must not reuse blame from the wrong file.

17. If Git is unavailable, the file is not tracked, the file is outside a repository, the blame command fails, or the file has no commits yet, the blame gutter stays blank except for `(uncommitted)` lines where the editor can identify local-only content. These cases should not show a persistent error toast.

18. If a blame request completes after the user has switched files or the file version has changed, the stale result is ignored.

19. Clicking the author or short hash for a committed line opens a commit detail popover anchored near the clicked blame annotation. Clicking the relative date alone does not need to open the popover.

20. The commit detail popover shows a loading state if commit details are not already cached. The loading state is local to the popover and must not disable the editor.

21. When loaded, the popover shows:
   - Commit subject and body.
   - Author name and email when available.
   - Full commit hash.
   - Absolute author date and relative date.
   - The repository-relative file path.
   - The diff for the clicked commit as it applies to the open file.

22. Commit messages may contain Markdown-like text, but the popover must remain readable even when the message is plain text, empty, very long, or contains unusual whitespace.

23. The diff in the popover is read-only. Added and removed lines are visually distinct and selectable/copyable as text. Very large diffs may be truncated with a clear `Diff truncated` note instead of making the popover unbounded.

24. If the clicked commit no longer exists, the file path was renamed in a way the detail fetch cannot resolve, or Git returns an error for the detail request, the popover shows a compact failure state with the commit hash and an explanation that details could not be loaded.

25. If the repository origin is a GitHub remote that can be resolved to owner and repository, the popover includes an `Open on GitHub` action for the commit. If no GitHub URL can be built, the action is omitted.

26. Only one blame popover is open per editor. Opening another blame entry replaces the previous popover.

27. Clicking outside the popover, pressing Escape, switching files, closing the editor, or disabling blame closes the popover. Closing it returns focus to the editor without moving the cursor or changing the selection.

28. Keyboard users can focus a visible author/hash annotation and open it with Enter or Space. Escape closes the popover. The popover content must be reachable for reading and copying without trapping focus permanently.

29. Text selection, copy, search, line-number selection, comment gutter buttons, diff hunk buttons, and code navigation continue to behave as they did before blame was enabled.

30. The feature is available only when the Git blame feature flag is enabled. When the flag is disabled, no blame commands run and the editor gutter is unchanged from the current behavior.

## Smoke test

### 11a - Blame parser + gutter rendering

1. Build and launch the `warp-oss`/twarp binary with the Git blame feature flag enabled.
2. Create a temporary Git repository with a text file containing at least three lines; commit line 1 as author `Alice`, then change line 2 and commit as author `Bob`.
3. Open the repository and the committed file in twarp.
4. Verify the editor still shows normal line numbers, syntax highlighting, and any diff indicators, and that the blame gutter shows `Alice` on line 1 and `Bob` on line 2 with 7-character hashes and relative dates.
5. Edit line 1 without saving and verify line 1 changes to `(uncommitted)` instead of continuing to show Alice's old hash.
6. Save the file without committing and verify line 1 still shows `(uncommitted)` while unchanged committed lines keep their committed blame.
7. Commit the saved change, refocus or reopen the file, and verify line 1 refreshes to the new commit's author/hash/date.

### 11b - Commit detail popover

1. In the same file, click the author or hash for Bob's blamed line.
2. Verify a popover opens near the clicked gutter annotation and shows Bob's commit subject/body, author, full hash, absolute date, relative date, repository-relative file path, and the diff for that commit in the open file.
3. If the test repository has a GitHub `origin`, verify the popover shows an `Open on GitHub` action; otherwise verify that action is omitted.
4. Press Escape and verify the popover closes, focus returns to the editor, and the cursor/selection did not move.
