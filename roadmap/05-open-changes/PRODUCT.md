---
name: 05 — Open Changes panel
status: draft
---

# Open Changes panel — PRODUCT

## Summary

A built-in side panel for reviewing the current repo's changes without leaving twarp — modeled on VS Code's Source Control view, behavior-for-behavior where it makes sense. The panel lives in the left side panel next to "Custom shortcuts", scopes itself to a single git repo derived from the focused pane's working directory, and lets the user inspect, stage, unstage, discard, commit, push, pull, and view per-file history. The aim is parity with VS Code's panel for the operations a terminal user already does dozens of times a day, so reviewing a diff before committing never requires switching out of the terminal.

## Goals / Non-goals

**Goals**

- A side-panel surface — slotted next to "Custom shortcuts" — that always reflects the git state of one specific repo (the "panel repo") without the user typing any git commands.
- Two clearly separated file lists: **Staged Changes** (the index) and **Changes** (the working tree), each with a file count badge.
- Click a file to view a unified diff inline; click a hunk header to navigate; basic syntax-aware coloring inherited from twarp's existing diff/markdown rendering.
- Stage / unstage / discard at **file** granularity and at **hunk** granularity, with consistent hover affordances on every row.
- A commit message input + Commit button + secondary push / pull / fetch controls — enough to ship the "review → commit → push" loop end-to-end inside the panel.
- A file Timeline (per-file commit history) that mirrors VS Code's Timeline section: focused-file scoped, click a commit to see its diff for that file.
- Live refresh as the working tree changes (file watcher) and as the user runs git commands in any pane (post-command hook), so the panel never goes stale.
- All operations route through twarp's existing in-process git plumbing — no new daemons, no shelling out to `git`, no language-server-style sidecar.
- Friendly handling of "no repo" and "no changes" states: the panel always shows *something* coherent.

**Non-goals**

- Branch management UI (create / switch / delete branches, manage remotes). The panel covers the *Source Control* surface, not the *Branches* surface; v1 users continue to use the terminal for branch ops.
- Conflict resolution UI. Merge / rebase / cherry-pick conflicts surface as a banner pointing the user to resolve in their editor of choice; no inline 3-way merge tool in v1.
- Multi-repo workspaces in a single panel view. The panel scopes to one repo at a time (the one derived from focused-pane cwd). If the user wants to look at a different repo, they focus a pane inside that repo.
- Submodule traversal. Submodule changes show as a single "modified submodule" row with the SHA delta; their internal diffs are not browsable from this panel.
- Stash management (list / apply / drop stashes). Stash is a terminal workflow in v1.
- Pull-request authoring, GitHub integrations, or anything that talks to a forge over the network beyond plain git push/pull/fetch.
- Inline editing of working-tree files from the diff view. The diff view is read-only — to edit a file, the user opens it in a tab (`WorkspaceAction::OpenFileInNewTab`) or in their external editor.
- Preserving in-flight commit message drafts across twarp restarts in v1. The message input clears on quit; persistence is a follow-up.
- LFS-aware diffs. LFS pointer files diff as text (showing the pointer); binary blobs show a placeholder per §15. v1 does not fetch LFS content for the diff view.
- Worktrees other than the user's primary checkout. The panel reflects `HEAD` of the current worktree; secondary `git worktree`s are out of scope.
- Custom Timeline providers (VS Code's extensibility hook for non-git history sources). v1's Timeline is git-only.

## Behavior

### Panel surface

1. **Where it lives.** The panel is a new tool-panel view, "Open Changes", added to the left side panel **immediately to the right of "Custom shortcuts"** in the panel switcher. Opening the left panel and switching to "Open Changes" shows the view; the panel is dismissed and switched by the same gestures that govern any other left-panel view. The view is reachable via the same keyboard / menu / mouse paths as the existing tool-panel views and gets a distinct toolbelt icon (the source-control / branch glyph).

2. **Panel repo.** The view's content always scopes to exactly one git repo, called the "panel repo". The panel repo is determined by walking up from the focused pane's current working directory until a `.git` directory (or file, for worktrees / submodules) is found. If the focused pane's cwd is inside a git repo, that repo is the panel repo. If the focused pane's cwd is **not** inside a git repo, see §3. The panel repo updates automatically when the user focuses a pane whose cwd resolves to a different repo, or when the focused pane's cwd changes (e.g. `cd` to a subdirectory of another repo).

3. **No-repo state.** When no pane is focused, or the focused pane's cwd is not inside a git repo, the panel shows a single centered message: `No git repo in the focused pane.` plus a one-line hint: `Open Changes follows the focused pane's working directory.` No other controls render. Focusing a pane inside a repo immediately replaces the no-repo state with the normal view (§4).

4. **Layout (with-repo state).** Top to bottom, the panel shows:
    1. **Repo header.** One line: `<repo-name> · <branch-name>`. `<repo-name>` is the basename of the repo's top-level directory; `<branch-name>` is the current branch, or `(detached HEAD: <short-sha>)` when detached, or `(rebasing onto <branch>)` / `(merging from <branch>)` / `(cherry-picking <short-sha>)` when an interrupted op is in progress (see §22). Clicking the repo name reveals nothing in v1 (reserved for future branch picker); clicking the branch name opens a tooltip showing the upstream tracking branch and ahead/behind counts (`↑2 ↓1` style).
    2. **Commit area.** A multi-line text input for the commit message and one primary button: **Commit**. Push / pull / fetch are reachable via an adjacent overflow menu (§30).
    3. **Staged Changes section.** Collapsible header `Staged Changes  ·  <count>`, then one row per staged file. Empty when count is 0; the section header still renders so the user can see there is nothing staged.
    4. **Changes section.** Collapsible header `Changes  ·  <count>`, then one row per working-tree changed file.
    5. **Timeline section.** Collapsible header `Timeline`; body is described in §§32–37. Collapsed by default until the user focuses a file (§32).

5. **Counts and badges.** Each section header shows its file count. The panel's toolbelt icon shows an overall badge with the **total** changed-file count (staged + working tree, deduplicated by path) when ≥ 1, hidden when 0. The badge matches the styling of unread-notification badges used elsewhere in twarp.

6. **Empty state (clean tree).** When the panel repo has zero staged changes and zero working-tree changes, both section headers render with `· 0`, the Commit button is disabled, the commit message input is enabled (typing a message is allowed but Commit stays disabled while there's nothing to commit), and a one-line hint shows beneath the headers: `Working tree is clean.` Timeline still renders normally for whichever file the user focuses.

7. **Refresh semantics.** The panel re-reads git state automatically whenever:
    - A file under the panel repo's working tree is created, modified, renamed, or deleted (file watcher).
    - A git command finishes running in any twarp pane whose cwd is inside the panel repo (post-command hook on commands beginning with `git ` after shell-alias expansion — see §38).
    - The user clicks the manual refresh affordance in the panel's overflow menu.
    - The panel repo itself changes (focused pane moves between repos per §2).
    
    Refresh is debounced (≤ 250ms coalescing window) so a burst of file-system events produces at most one refresh. A refresh in progress while another is requested coalesces — the in-flight refresh completes, then exactly one more fires.

### File rows

8. **Row content.** Each file row in the Staged Changes and Changes sections shows:
    - A **status glyph** on the left, single character, color-coded: `M` (modified, yellow), `A` (added, green), `D` (deleted, red), `R` (renamed, blue), `C` (copied, blue), `U` (unmerged / conflict, red, bold), `?` (untracked, green, italic).
    - The **file path** relative to the repo top-level. The path is rendered with the basename in normal weight and the directory portion in a dimmer shade trailing the basename (`mod.rs  src/workspace/view/`). Very long paths truncate the directory portion from the **left** with `…`; the basename is always visible.
    - On hover, a row reveals an **action cluster** flush right (§9).
    - The row is clickable as a whole (§11).

9. **Hover actions per row.**
    - **Changes (working tree) row:** `[+]` Stage file, `[↺]` Discard file. Both are icon buttons with tooltips.
    - **Staged Changes row:** `[−]` Unstage file. (No discard from the staged section — to discard, unstage first.)
    - **Conflict (`U`) row:** No stage / unstage / discard buttons. Instead a single `[Resolve…]` button that opens the file in a new tab (§24).

10. **Row sort order.** Within each section, rows sort by full path lexicographically (case-insensitive). Renames sort by their **destination** path. The sort order is stable across refreshes so a row doesn't jump when its sibling changes.

11. **Row click → diff.** Left-clicking a row makes that file the **focused file**: the row gains a focused-row highlight, the diff view (§12) renders the file's diff inline below the section list, and the Timeline section (§32) updates to that file. Left-clicking the focused row a second time collapses the diff view without losing focus. Clicking a different row replaces the focused file and re-renders the diff.

12. **Diff view placement.** The diff view renders **inline below the file list**, expanding the panel's scroll area; it does not pop out into a separate tab and does not cover the other section. Both Staged Changes and Changes sections remain visible above; the diff view scrolls with the panel. The diff view's height is bounded only by the diff content — long diffs make the panel longer; the user scrolls.

### Diff view

13. **Diff format.** The diff view shows a **unified diff** with two columns of context: line numbers on the left (old / new pair), the diff content on the right. Added lines are tinted green, removed lines tinted red, context lines uncolored. Whitespace-only lines render with a low-contrast `·` marker on each whitespace position when the user toggles "show whitespace" in the overflow menu (off by default).

14. **Hunk headers.** Each hunk starts with a header row showing `@@ -old_start,old_count +new_start,new_count @@ [function-context]`, dimmed. To the right of the hunk header, on hover, two buttons appear:
    - In the **Changes** diff view: `[+]` Stage hunk, `[↺]` Discard hunk.
    - In the **Staged Changes** diff view: `[−]` Unstage hunk.
    Hunk-level operations operate on the exact lines shown in the hunk; partial-line hunk staging is not supported in v1 (line-level granularity is a follow-up).

15. **Binary, deleted, renamed, large files.**
    - **Binary file:** the diff view renders `Binary file changed.` with no content body. Stage / unstage / discard at file level still work; hunk-level operations are not offered.
    - **Deleted file:** the diff view renders the full prior content with every line marked removed. The hunk-stage button stages the deletion.
    - **Pure rename (no content change):** the diff view renders `Renamed: <old-path> → <new-path>` with no body. File-level operations work; hunk-level is not offered.
    - **Rename with content change:** the rename line above, then the content diff between old and new.
    - **Large file** (diff > 50,000 lines or > 5 MB): the diff view renders the first 1,000 lines with a banner above: `Large diff truncated — showing first 1,000 lines of N.` The banner has an `[Open full diff in tab]` link that opens the diff in a new tab via twarp's diff viewer (same surface the panel uses; just unbounded). File-level operations still work in the panel.
    - **Untracked file (`?`):** the diff view renders the file's full content with every line marked added.

16. **Diff view focus and keyboard.** When the diff view is focused, `j` / `k` (or `↓` / `↑`) move between hunks; `Enter` on a hunk header triggers its primary action (Stage hunk in Changes, Unstage hunk in Staged Changes); `Space` toggles the row's expanded/collapsed state in the file list. `Esc` returns focus to the file row. Mouse wheel scrolls normally; the panel does not capture wheel events outside the diff bounds.

17. **Re-focus on refresh.** After a refresh, if the focused file still has changes, it remains focused and the diff re-renders. If the focused file is gone from both sections (e.g. user staged the only hunk, or discarded the change), focus clears and the diff view collapses; Timeline keeps showing the previously focused file until the user focuses a different one (§32).

### Stage / unstage / discard

18. **Stage file.** Clicking `[+]` on a Changes row stages the full file: equivalent to `git add -- <path>` for tracked/modified/deleted files, `git add -- <path>` (intent-to-add then full add) for untracked files. For a rename, both sides of the rename stage atomically.

19. **Unstage file.** Clicking `[−]` on a Staged Changes row unstages the full file: equivalent to `git restore --staged -- <path>` (or the equivalent against `HEAD` for the initial commit when no `HEAD` exists yet, e.g. `git rm --cached`).

20. **Discard file.** Clicking `[↺]` on a Changes row pops a small inline confirmation directly beneath the row: `Discard changes to <basename>?  [Cancel] [Discard]`. Clicking outside the confirmation, or pressing Escape, cancels. Clicking Discard:
    - For a **tracked, modified** file: reverts the working-tree content to match the index (or `HEAD`, if not staged) — equivalent to `git restore -- <path>`.
    - For a **tracked, deleted** file: restores the file from the index — equivalent to `git restore -- <path>`.
    - For an **untracked (`?`)** file: deletes the file from disk. The confirmation text changes to `Delete untracked file <basename>?` to signal the irreversibility. After delete, an undo toast appears (`Deleted <basename>. [Undo]`) for 10 seconds; clicking Undo restores the file contents and modification time exactly as they were before the delete.
    - For a **directory of untracked files** (when the row is a directory grouping — not in v1; v1 only shows individual files): not applicable.

21. **Stage / unstage / discard hunk.** Same semantics as their file-level counterparts but applied to the lines in one hunk. The implementation uses git's plumbing to apply / unapply a synthetic patch limited to that hunk; on failure (e.g. the hunk no longer applies cleanly because the file changed mid-operation), the operation is refused with an inline toast `Hunk no longer applies — refresh and try again.` and a refresh is triggered.

22. **In-progress operations (merge / rebase / cherry-pick).** When the panel repo is in a `MERGING`, `REBASING`, `CHERRY-PICKING`, or `BISECTING` state:
    - The repo header reflects the state (§4).
    - A banner appears below the header: `<state> in progress — resolve conflicts then commit, or run <abort-command> in the terminal.` (`<abort-command>` is `git merge --abort`, `git rebase --abort`, etc.)
    - Conflicted files appear in **both** sections with `U` glyph; the conflict is the same row identity, not duplicated.
    - The Commit button is enabled only when zero conflict (`U`) rows remain. Its label changes to `Continue rebase` / `Conclude merge` / `Continue cherry-pick` as appropriate; clicking it runs the equivalent of `git <op> --continue`.

23. **Idempotence and races.** Stage / unstage / discard operations on a file or hunk that is already in the requested state are no-ops (no error, no refresh needed beyond the one that already showed the state). If a stage / unstage / discard fails because the underlying state changed between render and click (e.g. the user ran `git add .` in a pane between when the panel rendered and when they clicked Stage), the panel refreshes and retries the operation against the new state; if the operation no longer applies, an inline toast surfaces (`<op> no longer applies — refreshed.`) and the row updates.

24. **Resolve (conflict row).** Clicking `[Resolve…]` on a conflict row dispatches `WorkspaceAction::OpenFileInNewTab` for the conflicted file, opening it in a new twarp tab with conflict markers visible. The panel itself does **not** present a 3-way merge UI in v1. After the user fixes and saves the file (and stages it via terminal or panel), the row leaves the conflict state on the next refresh.

### Commit, push, pull, fetch

25. **Commit message input.** Multi-line text input above the section lists. Placeholder: `Message (press <chord> to commit)` where `<chord>` is the platform-appropriate shortcut: `⌘Enter` on macOS, `Ctrl+Enter` elsewhere. The input grows to fit content up to 8 lines, then scrolls internally. The input supports the same Markdown rendering as commit-message editors elsewhere in twarp (basic monospace; no inline rendering — commit messages are plain text). A live character-count indicator shows beneath the input only when the **first line** exceeds 72 characters (`First line: 78 chars (recommended ≤ 72)`); below that threshold the count is hidden.

26. **Commit gating.** The Commit button is enabled iff **all** of the following hold:
    - The panel repo is not in a no-changes state (§6), **or** it is in a merge/rebase/cherry-pick state with zero conflicts remaining (§22).
    - At least one staged change exists, **or** the panel repo is mid-merge with all conflicts resolved and `git commit` would succeed (in which case the staged-changes count may technically be zero but the merge produces the commit).
    - The commit message input is non-empty (whitespace-only doesn't count) **unless** the repo is mid-merge (where git would generate the default merge message); in that case the input can be left empty and `Commit` will use git's default merge message, but typing a message overrides it.

27. **Commit action.** Clicking Commit (or pressing the chord from §25 while the input is focused):
    - Runs the equivalent of `git commit -F -` with the input contents as the message, against the panel repo. No `--amend`, no `--signoff`, no `--no-verify` from this surface in v1.
    - On success: clears the message input, refreshes the panel, shows a transient `Committed <short-sha>: <first-line>` toast at the bottom of the panel with a `[View]` link that focuses the new commit in Timeline (§32).
    - On failure (pre-commit hook rejection, signing failure, anything `git commit` would surface): the message input retains its content, an inline error banner appears above the input showing the verbatim git stderr, and the panel does not refresh state until the user dismisses the banner or successfully commits. The banner has a `[Copy error]` button.

28. **Amend.** A small `[Amend]` toggle sits next to the Commit button. While Amend is on:
    - The button label changes to `Commit (Amend)`.
    - The commit message input prepopulates with the previous commit's message **once** when the toggle is first switched on (without overwriting any text the user already typed — if the input is non-empty the user is asked inline: `Replace draft with previous commit message? [Replace] [Keep draft]`).
    - The commit operation passes `--amend`.
    - Toggling Amend off restores any draft message that was displaced.
    
    Amend is disabled when there is no previous commit on the branch (initial commit case).

29. **Push.** The overflow menu's `Push` entry pushes the current branch to its upstream. If the branch has no upstream configured, `Push` is disabled and a tooltip explains `No upstream — run \`git push -u\` in the terminal to set one.` Pushing shows a transient `Pushing…` indicator in the repo header; on success, `Pushed (now in sync)` toast; on failure, an inline error banner with verbatim git stderr (same shape as §27 failure).

30. **Pull, fetch, refresh.** Overflow menu entries:
    - `Pull` — `git pull` against the upstream. Disabled with the same tooltip as Push when no upstream is set.
    - `Fetch` — `git fetch` for the current remote. Always enabled when a remote exists.
    - `Refresh` — manually re-reads working-tree and index state (§7).
    Each shows a transient indicator in the repo header during its run and a success / failure toast on completion. None of these operations modify the message input or Amend state.

31. **No background mutation outside operations.** The panel never silently runs `git add`, `git reset`, `git commit`, `git push`, `git pull`, or any other state-mutating git op unless the user explicitly clicks the corresponding affordance (§§18–30). File-watcher refreshes are read-only.

### Timeline (file history)

32. **Scope.** The Timeline section shows the focused file's commit history. "Focused file" means the row most recently clicked in either section (§11), even if that row is no longer present (e.g. after discard). If no file has ever been focused in the current panel session, Timeline shows a centered hint: `Click a file to see its history.` The Timeline section header reflects the focused file: `Timeline · <basename>` when a file is focused, `Timeline` otherwise.

33. **Entries.** Each Timeline entry is one commit that touched the focused file, in reverse chronological order. An entry shows:
    - **Author avatar** (single-letter circle keyed to author name when no avatar URL is available; consistent color per author).
    - **Author name** (truncated with `…` if needed).
    - **Relative time** (`2 hours ago`, `3 days ago`, `5 mins ago`). Hovering reveals the absolute timestamp.
    - **Commit subject** (first line of the commit message), truncated with `…`.
    - The entry is a single horizontal row.

34. **Paging.** Timeline loads the most recent 20 entries initially. A `[Load more]` button at the bottom appends the next 20. There is no upper bound; the user can keep loading. Each `[Load more]` shows a spinner during fetch.

35. **Click → diff for that commit.** Left-clicking a Timeline entry replaces the inline diff view with the **focused file's diff at that commit** (i.e. `git show <sha> -- <path>` semantics). The diff is read-only and shows no Stage / Unstage / Discard buttons. A small `[Back to working diff]` link appears above the diff to return to the live working-tree / index diff for that file. Clicking another Timeline entry replaces the diff again. Clicking the same entry a second time has no effect.

36. **Rename tracking.** When the focused file has been renamed in history, Timeline follows the rename (`git log --follow`). Commits prior to the rename show the entry's original path in a tooltip on the row. A small badge (`R`) appears on entries that *are* the rename commit.

37. **Local-only vs pushed.** Each Timeline entry whose commit is **ahead** of the upstream branch shows an `↑` marker next to the relative time; pushed commits show no marker. When the repo has no upstream configured, no markers appear.

### Refresh hooks and cross-pane consistency

38. **Post-command refresh.** twarp already knows when a command finishes in a pane and what command it was. The panel subscribes to that signal and triggers a refresh (§7) when the finished command, after shell-alias expansion, begins with `git ` and the pane's cwd is inside the panel repo. The match is exact-prefix on `git` (e.g. `git`, `git status`, `git commit -m "x"`, `git-lfs ...` is **not** matched). Aliases that expand to a `git`-prefixed command are matched after expansion; aliases that don't expand to `git` (e.g. a `g` alias to a custom script) are not matched and the user must use manual refresh.

39. **Multi-pane racing.** If the user is mid-edit in the commit message input or has an inline confirmation open (e.g. Discard confirmation) when a refresh fires, the refresh applies to the lists and diff view but does **not** alter the commit input state, the Amend toggle, or any open confirmation. If a row is removed by the refresh and that row had an open confirmation, the confirmation closes silently.

40. **File watcher scope.** The watcher subscribes to the panel repo's top-level working tree, excluding `.git/` (except `HEAD`, `index`, `MERGE_HEAD`, `REBASE_HEAD`, `CHERRY_PICK_HEAD`, `BISECT_LOG` for state detection per §22). Switching the panel repo (§2) tears down the previous watcher and starts a new one.

### Errors, performance, accessibility

41. **Git errors are never silent.** Any git operation that fails — whether triggered by the user (commit, push, stage hunk) or by an internal refresh (e.g. corrupt index) — surfaces a banner with the verbatim git stderr. The banner has a `[Copy error]` button. The panel never swallows a git error.

42. **Performance ceiling.** The panel must stay responsive on repos with up to **10,000 changed files** (the working-tree + staged file count). File rows render virtualized when total row count exceeds 200, so scrolling stays smooth. Diff rendering for a single file targets ≤ 200ms on a 10,000-line diff (which triggers the truncation banner per §15).

43. **Keyboard reachable.** Every action with a mouse affordance has a keyboard path:
    - Tab moves through commit message → Commit → Amend toggle → overflow menu → section headers → rows in render order.
    - On a focused row: `Enter` toggles diff expand/collapse; `s` stages (Changes) / unstages (Staged Changes); `d` discards (Changes); `o` opens the file in a new tab.
    - On a focused hunk header (after `Enter` on the row, then `j`/`k` to a hunk): `s` stages / unstages the hunk; `d` discards the hunk (Changes only).
    - `⌘Enter` / `Ctrl+Enter` from anywhere in the commit message commits.

44. **Themed visuals.** Status glyph colors, diff add/remove tints, focused-row highlight, banner backgrounds, and avatar colors all come from twarp's active theme. No hard-coded colors.

45. **No telemetry beyond existing channels.** Panel actions (stage, unstage, commit, push) emit the same telemetry their terminal equivalents already do — they share the in-process git plumbing — and the panel itself emits no additional per-action telemetry. Opening the panel registers a single "tool panel opened" event consistent with how other tool panels report.

## Smoke test

Run against a freshly built twarp binary inside a real git repo (use the twarp repo itself or a scratch repo with a few commits). Chord names below are macOS; substitute Ctrl for ⌘ on Linux/Windows. "Open the panel" means: open the left side panel and switch to the "Open Changes" tab next to "Custom shortcuts".

### 5a — Panel scaffold + working/staged file lists

1. Launch twarp. Focus a pane whose cwd is **not** in a git repo (e.g. `cd ~`). Open the panel. The view shows `No git repo in the focused pane.` with the hint underneath; no sections render.

2. `cd` into a git repo with a clean working tree. The panel switches to the with-repo state: repo header shows `<repo-name> · <branch>`, Staged Changes and Changes sections each render with `· 0`, and `Working tree is clean.` shows below.

3. In the terminal, `touch newfile.txt`. Within ~250ms, the panel refreshes: Changes section shows `· 1`, and one row appears: `?  newfile.txt`.

4. Run `git add newfile.txt` in the terminal. The row moves to Staged Changes: `?` → `A`, count moves from Changes `· 1` to Staged Changes `· 1`.

5. Modify an existing tracked file in the terminal (e.g. `echo hi >> README.md`). A second row appears in Changes: `M  README.md`. Staged Changes still shows `A  newfile.txt`.

6. Focus a different pane whose cwd is in a **different** git repo. The panel switches to that repo automatically; the previous repo's rows are gone and the new repo's state renders.

### 5b — Inline diff view

7. With Changes showing `M  README.md`, click the row. The row gains a focused highlight and a unified diff renders inline below the section list with green/red tinting. Hunk headers show `@@ … @@` and dim.

8. Click the focused `M  README.md` row again. The diff collapses but the row stays focused. Click it once more — the diff re-expands.

9. Click `A  newfile.txt` in Staged Changes (after a `git restore --staged` test, re-stage to have content). The diff shows the file's content with every line as an addition (green).

10. Stage a 12,000-line change to a file (`yes "line" | head -n 12000 > big.txt && git add big.txt`). Click the row. The diff view shows the first 1,000 lines with the banner: `Large diff truncated — showing first 1,000 lines of 12,000.` Click `[Open full diff in tab]` — a new twarp tab opens with the full diff.

11. Stage a binary file (`cp some.png . && git add some.png`). Click the row. The diff renders `Binary file changed.` with no body. No hunk-level affordances appear on hover.

### 5c — Stage / unstage / discard

12. Hover the `M  README.md` row in Changes. `[+]` and `[↺]` appear flush right. Click `[+]`. The row moves to Staged Changes (`M`). Counts update.

13. Hover the `M  README.md` row now in Staged Changes. Only `[−]` appears. Click it. The row returns to Changes.

14. Click `[↺]` on a tracked-modified row. An inline confirmation appears beneath the row: `Discard changes to README.md?  [Cancel] [Discard]`. Click Cancel — no change. Click `[↺]` again, then click outside the confirmation — it dismisses without acting. Click `[↺]` again, then click Discard. The file's working-tree content reverts to match the index (verify with `git diff` in the terminal).

15. Click `[↺]` on an untracked (`?`) row. Confirmation text now reads `Delete untracked file <name>?`. Click Discard. The file is deleted on disk (verify with `ls`). An undo toast appears with `[Undo]`. Click Undo within 10 seconds — the file reappears with original content and mtime.

16. With a 3-hunk modified file focused and the diff showing all three hunks, hover the second hunk's header. `[+]` and `[↺]` appear next to the header. Click `[+]`. The file appears in **both** Changes and Staged Changes (partial stage); the diff view's Changes hunks now show only hunks 1 and 3. Click into the Staged Changes copy of the file — its diff shows only hunk 2.

17. Set up a merge conflict (`git merge` a divergent branch). The conflicted file row shows `U` (bold red) in both sections; conflict row hover shows only `[Resolve…]` (no stage/unstage/discard). The repo header reads `(merging from <branch>)`; a banner says `Merge in progress — resolve conflicts then commit, or run \`git merge --abort\` in the terminal.` Click `[Resolve…]` — the file opens in a new twarp tab with conflict markers. Resolve the conflict, save, `git add` the file in the terminal. The `U` row disappears, the Commit button is now labeled `Conclude merge` and enabled.

### 5d — Commit message input + commit / push / pull

18. Stage at least one change. Type a commit message — `Test commit from panel`. Click Commit. The toast `Committed <short-sha>: Test commit from panel` appears; the message input clears; rows for the just-committed files disappear. Run `git log -1` in the terminal — the new commit is there with the exact message and `Co-Authored-By` line absent (the panel does not add co-author trailers).

19. Type a long first-line commit message (`This is a very long subject line that intentionally exceeds the seventy-two character recommendation`). The indicator below the input shows `First line: 100 chars (recommended ≤ 72)`. Shorten to ≤ 72 chars; the indicator disappears.

20. Click `[Amend]`. The toggle highlights; the button label changes to `Commit (Amend)`. The message input prepopulates with the previous commit's full message (the empty input case, no draft to displace). Edit the message; click Commit (Amend). `git log -1 --pretty=fuller` shows the rewritten message on the prior commit; no new commit was created.

21. Repeat (20) but type a draft message **before** clicking Amend. An inline prompt appears: `Replace draft with previous commit message? [Replace] [Keep draft]`. Click Keep draft. The input retains the draft. Toggle Amend off and on again — the prompt does not re-appear within the same session for the same draft; toggling off restores the draft.

22. Configure a remote with `git remote add origin <local-path>` and push once via the terminal so the branch has an upstream. Open the overflow menu — `Push`, `Pull`, `Fetch`, `Refresh` are listed. Click Push. Repo header briefly shows `Pushing…`; on success, `Pushed (now in sync)` toast.

23. Delete the upstream (`git branch --unset-upstream`). Open the overflow menu — `Push` is disabled with the no-upstream tooltip. `Pull` is also disabled. `Fetch` and `Refresh` remain enabled.

24. Trigger a pre-commit hook failure (add a hook that exits non-zero). Stage a change, type a message, click Commit. The message input retains the text, a banner appears above with the verbatim hook output and a `[Copy error]` button. Click Copy error — the clipboard contains the stderr.

### 5e — File Timeline

25. Focus a file with multiple commits in history. The Timeline section header updates to `Timeline · <basename>`. The most recent 20 commits for that file render: avatar circle, author, relative time, subject. The most recent commit is at the top.

26. Click an older Timeline entry. The inline diff view replaces the working/staged diff with the commit's diff for that file. A `[Back to working diff]` link appears above. No stage/unstage/discard affordances render in this read-only mode.

27. Click `[Back to working diff]`. The inline diff returns to the working-tree / index diff.

28. Click `[Load more]` at the bottom of Timeline. The next 20 entries append, no jump in scroll position.

29. Focus a file that has been renamed (e.g. `git mv old.txt new.txt && git commit`). Timeline entries from before the rename show the old path in a tooltip; the rename commit itself shows a small `R` badge.

30. Push the branch so upstream catches up. Make and commit one new local change to a file that has Timeline entries; the new Timeline entry shows `↑` next to its relative time. Push. Refresh the panel — the `↑` marker is gone.

### Cross-cutting (any sub-phase)

31. With the panel open, run `git checkout -b new-branch` in any pane inside the panel repo. The post-command refresh fires; the repo header's branch name updates to `new-branch` without any user interaction in the panel.

32. With the panel open and a Discard confirmation visible, run `git add .` in a pane. The lists refresh, the discard confirmation closes silently, and no operations fire from the cancelled confirmation.

33. Open the panel; the toolbelt icon shows a numeric badge equal to the unique file count across both sections. Stage all changes; the badge equals the staged count. Commit; the badge clears (assuming a clean tree post-commit).
