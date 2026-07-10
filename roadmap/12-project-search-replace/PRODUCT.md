# 12 - Project-wide search & replace

## Summary

Project-wide search and replace gives users a dedicated Warp surface for finding text across the current local project, opening matches directly in the editor, and applying confirmed replacements across files with a preview-first workflow.

## Figma

Figma: none provided.

## Goals

1. Let users search across all current local project roots without running a terminal command.
2. Make each search result actionable: selecting a match opens the file in Warp's editor at the matched line and column.
3. Make replace-all safe by requiring a per-file preview and explicit confirmation before disk writes.
4. Keep the workflow local-filesystem only until remote project file editing has an equivalent safe write path.

## Non-goals

1. Do not replace terminal `rg`, shell pipelines, or external editor integrations.
2. Do not provide a project-wide transactional undo guarantee across every changed file.
3. Do not search binary files or files that the search backend cannot read as text.
4. Do not make replace modify files before the user has reviewed and confirmed the preview.

## Behavior

### 12a - Project-wide search UI

1. A local workspace exposes a Search entry in the left tool panel when project search is enabled. Opening Search focuses the search query field by default.

2. If the user opens Search from selected editor or terminal text, the query field is prefilled with that selected text and the search surface is focused.

3. The search surface searches the current local project roots associated with the workspace. If multiple roots are present, results are grouped under their root directory. Nested roots are searched only once so the same file does not produce duplicate results.

4. Remote sessions, unsupported sessions, or workspaces without a local filesystem do not silently fail. The Search surface shows a clear unavailable state and no search is started.

5. The search form includes:
   - A query input.
   - A case-sensitivity toggle.
   - A regex toggle.
   - A files-to-include input for comma-separated glob patterns.
   - A files-to-exclude input for comma-separated glob patterns.

6. Literal search is the default. When regex is off, special regex characters in the query match themselves. When regex is on, the query is interpreted as a regular expression.

7. Case-insensitive search is the default. When case sensitivity is enabled, only exact-case matches are returned.

8. Include/exclude filters re-run the search even when the query text has not changed. Include filters narrow the searched files; exclude filters remove files from the searched set and win over includes for the same path.

9. Search starts after the user provides a non-empty query and pauses briefly after typing. Editing the query or toggles cancels or supersedes older in-flight results; stale results from older searches are never appended to the current result set.

10. While a search is running, results stream into the list as they are found. The UI remains usable while the search is in progress.

11. Results are grouped by directory, then by file. Each file row shows the file path and the number of matches in that file. Each match row shows the matched line preview and highlights the matched range.

12. Directory and file groups can be collapsed and expanded. Collapsing a group hides its child rows without discarding the search result data.

13. Keyboard navigation supports moving through the result list, collapsing and expanding directory/file groups, returning focus to the query field, and opening the selected match.

14. Clicking a match or pressing Enter on a selected match opens that file in Warp's editor at the matched line. If the match has a column, the editor scrolls to the matched column as well.

15. Opening a match in a file already open in the current code pane focuses the existing tab and jumps to the match instead of opening a duplicate tab.

16. If a query produces no results, the Search surface shows an empty state that suggests adjusting the query or filters.

17. If a result set is capped for performance, the Search surface tells the user that only a subset of matches is shown and that they should narrow the query.

18. Invalid regex input does not crash the surface. The user sees an error state, the previous valid results are cleared or marked stale, and no replacement flow can start from the invalid query.

19. Search respects the backend's binary-file and oversized-line safety behavior. Skipped files do not block the rest of the search.

20. Closing or switching away from Search does not mutate files, discard open editor state, or start replacement.

### 12b - Replace

21. Replace is available from the Search surface only after a valid non-empty search query has produced at least one current result.

22. The replace form includes a replacement text input and a Replace all action. Replacement text may be empty, which means "delete each match".

23. Replace uses the current search query, regex/case toggles, include/exclude filters, and project roots. If any of those inputs change after the preview is generated, the preview is marked stale and must be regenerated before apply.

24. Activating Replace all first opens a preview, not a write operation.

25. The preview is grouped by file and shows every file that would change. Each file preview includes enough surrounding context lines for the user to understand the change, with removed text and replacement text visually distinguished.

26. The preview shows a total count of files and matches that will change. Files with zero current matches are omitted.

27. The user can exclude an entire file from the pending replace before applying. Excluded files remain visible as excluded until the preview is regenerated or the flow is canceled.

28. The user can cancel from preview without changing any file.

29. Confirming the preview applies replacements only to the included files and only to matches represented by the current preview.

30. Before applying a file, Warp verifies that the file content still matches the previewed search spans. If a file changed on disk or in an open editor such that the preview no longer matches, that file is skipped and reported as not applied; other valid files may still apply.

31. Apply results are reported after confirmation: changed files, skipped files, and failed files are distinguishable.

32. For files already open in Warp's editor, the applied changes are reflected in the open buffer. Where the editor supports it, those changes participate in the file's normal undo history.

33. For files not open in Warp's editor, the changes are written to disk and file watchers update any later-opened editor buffer normally.

34. If a file cannot be written because of permissions, deletion, encoding, or another I/O error, that file is not partially written. The error is shown in the apply result and does not hide successful changes in other files.

35. Replacement never modifies binary files, unsupported remote files, or files outside the searched local roots.

36. Regex replacement follows the behavior documented in the UI. If capture-group replacement is not supported in the first implementation, replacement text is inserted literally even when regex search is enabled, and the UI must not imply capture expansion.

37. After a successful apply, the search results refresh or are clearly marked stale so the user does not act on pre-replace results as if they were current.

38. If the user starts a new search while a replace preview is open, the preview is dismissed or marked stale before another apply can happen.

39. Replace cannot be triggered while search is still running unless the preview is generated from a completed, current result set.

40. Replace actions are protected against double-submit. Repeated confirmation clicks while apply is in progress do not write the same replacement twice.

## Smoke test

### 12a - Project-wide search UI

1. Build and launch a local twarp binary with project search enabled.
2. Open a local workspace whose current directory contains at least two text files with the same token, for example `needle`.
3. Open the left-panel Search entry and type `needle`.
4. Verify results stream in, are grouped by file, show highlighted line previews, and show a total result count.
5. Toggle case sensitivity and verify the result set updates according to exact-case matching.
6. Toggle regex, search for a simple regex such as `need(le)?`, and verify matching results remain clickable.
7. Enter an include glob that matches only one file and verify results narrow to that file.
8. Click a match and verify the file opens in Warp's editor at the matched line and column.
9. Re-open Search, collapse and expand a file group, and verify the result rows hide and return without re-running the search.

### 12b - Replace

1. In the same local workspace, create two text files containing `needle` and one file that does not.
2. Search for `needle`, enter replacement text `thread`, and activate Replace all.
3. Verify a preview opens before any file changes on disk.
4. Verify the preview is grouped by file, shows context around each match, and shows removed/replacement text.
5. Exclude one file from the preview and confirm the replace.
6. Verify only the included file is changed on disk, the excluded file still contains `needle`, and the unchanged file is untouched.
7. Open the changed file in Warp's editor and verify the replacement is visible.
8. Repeat with an empty replacement string and verify matches are deleted only after preview confirmation.
9. Generate a preview, edit one previewed file externally before confirming, then confirm and verify that file is reported as skipped rather than being overwritten with stale spans.
