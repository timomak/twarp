# 10 - File editor surface with go-to-definition (PRODUCT)

Companion to [TECH.md](TECH.md). Behavior is written as user-visible, testable invariants. This feature exposes the existing rich code editor as a first-class file-editing workflow, independent of code review.

## Summary

Users can open workspace files from the project file tree or the Open File palette into an editable Code pane, edit and save them with normal editor affordances, keep multiple files open as tabs, and navigate code with cmd+click go-to-definition. The workflow should feel like a lightweight in-app editor: file opening, dirty state, save, external reload, tabs, LSP hover, references, diagnostics, and definition navigation all work without sending the user through a code-review-only surface.

## Goals / Non-goals

**Goals**

1. File-tree clicks, file-tree keyboard activation, file-tree context-menu open actions, and Open File palette results can open local text/code files in a Code pane.
2. The opened Code pane is editable by default, uses the existing syntax highlighting and LSP-backed editor capabilities, and saves to the original file path with Cmd+S / Ctrl+S.
3. Dirty state is visible in the pane header and in tabs. Closing a dirty tab prompts before losing edits.
4. External file changes reload into open editors when safe and do not overwrite unsaved user edits.
5. Multiple files can live in one Code pane as tabs, with duplicate opens focusing the existing tab instead of creating duplicate tabs.
6. Cmd+click / Ctrl+click on a symbol uses LSP definition data to navigate to the target file and location in the same editing workflow.

**Non-goals**

1. No LSP completion, rename, code actions, outline pane, breadcrumbs, folding, minimap, or status-bar redesign.
2. No new LSP protocol implementation. Definition, hover, references, formatting, and diagnostics use existing LSP plumbing.
3. No remote-file editing. Remote project explorer entries that cannot be opened locally remain disabled or routed to the existing unavailable state.
4. No binary editor. Binary and system-generic files continue to route through existing file-target decisions, not through the text Code pane.
5. No real-time collaborative editing or merge UI for conflicting external changes.

## Figma

Figma: none provided.

## Behavior

### 10a - File-tree and Open File to Code pane

1. When the user clicks a local text/code file in the file tree, the file opens in a Code pane using the configured file-open layout. If the configured layout is split pane, the Code pane appears in the active workspace tab beside the current pane and receives focus. If the configured layout is new tab, the Code pane opens in a new workspace tab according to the user's new-tab placement setting.

2. Pressing Enter on a selected file-tree file performs the same open behavior as clicking that file. Keyboard selection, expansion, and collapse behavior in the file tree remain unchanged.

3. The file-tree context menu exposes explicit Code-pane open intents for text/code files:
   - "Open in new pane" opens the file in a Code pane split from the active pane.
   - "Open in new tab" opens the file in a Code pane in a new workspace tab.
   - These explicit intents bypass the user's external-editor preference for non-binary text/code files.

4. The Open File palette opens accepted file results in a Code pane using the same Code editing workflow. Palette line/column metadata, when present, positions the editor at that location after the file loads.

5. Files that should not be edited as text do not open as raw text in the Code pane. Images, markdown files configured to render by default, external-editor choices, system-default files, and binary files keep their existing routing behavior unless the user explicitly chooses a Code-pane open action that is valid for that file type.

6. Opening a directory from the file tree or palette does not create an empty Code editor. It keeps the existing directory behavior, such as opening a terminal tab rooted at that directory where that behavior already exists.

7. An opened file displays its filename in the pane header. Hovering the title or tab shows the path using the existing relative/full path convention for the workspace.

8. A newly opened file starts clean. Editing the file marks the active tab dirty, shows the dirty indicator in the single-file header or tab label, and updates close/quit warning state.

9. Cmd+S on macOS and Ctrl+S on other platforms saves the active Code tab. Saving writes to the tab's current file path. Successful saves clear the dirty indicator and show the existing "File saved." success toast. Failed saves keep the tab dirty and show the existing save failure affordance.

10. Save As remains available for untitled/new files and files that have no backing path. Saving an untitled file assigns the chosen path to the tab, updates syntax/language behavior from the new path, and then follows normal saved-file behavior.

11. Closing a dirty Code tab, closing a Code pane containing dirty tabs, or quitting with dirty Code tabs prompts the user before data loss. The prompt offers save, discard, and cancel. Cancel leaves the tab/pane open and preserves edits.

12. If the same file is opened again in the same workspace editing context, twarp focuses the already-open tab or pane and moves the cursor to the requested line/column when provided. It does not create duplicate editable buffers for the same path in the same tab group.

13. If the file changes on disk while its open buffer has no unsaved user edits, the visible editor updates to the new on-disk content. Cursor/scroll movement should be no more disruptive than the existing file reload behavior.

14. If the file changes on disk while the user has unsaved edits, twarp does not overwrite those edits. The user keeps the dirty buffer and receives the existing conflict/unsaved-change indication rather than a silent reload.

15. File-tree rename and delete events stay coherent with open Code tabs. Renaming an open file updates the tab path while preserving unsaved edits. Deleting an open file closes or invalidates the backing path through the existing delete/unsaved flow without saving stale edits to the old path.

16. LSP features that already work in `LocalCodeEditorView` are available in standalone Code panes for supported local files after their server is running: diagnostics underlines, hover documentation, find references, formatting on save when configured, and LSP footer controls.

17. When no LSP server is available for a file, editing and saving still work. LSP-only affordances are hidden, disabled, or no-op in the same way they are in the existing editor surface.

18. Remote or unsupported file-tree states remain explicit. Remote files that cannot be opened locally show the existing unavailable behavior and do not create empty local Code panes.

### 10b - Multi-file tabs and cmd+click definition

19. A Code pane can contain multiple open files as editor tabs. Opening another file while tabbed editing is enabled adds it to the existing editable Code pane for the active workspace tab, unless the user explicitly opens it in a new workspace tab or split pane.

20. Each editor tab shows the file icon, filename, active/inactive state, close affordance, and dirty indicator. Long names are clipped without pushing close buttons or pane controls out of the header.

21. Clicking an editor tab switches the active file without changing the surrounding workspace tab. Middle-clicking or clicking the close affordance closes that editor tab.

22. Reopening a file that is already present in the tab group focuses the existing tab. If a line/column target is provided, the editor jumps there after focusing.

23. Closing a saved tab removes it immediately. Closing a dirty tab prompts for save/discard/cancel. Saving from the prompt writes the file, then closes the tab if save succeeds. Cancel leaves the tab open and active.

24. Closing the last editor tab closes the Code pane. Closing saved tabs leaves dirty tabs open. Closing all tabs prompts through dirty tabs before removing the group.

25. Editor tabs can be reordered within the Code pane using the existing drag/drop tab behavior. Moving or merging tabs preserves dirty buffers and active selection.

26. The active file in a Code pane updates adjacent workspace state that follows the focused file, including the file tree's active-file highlight and any existing opened-files tracking.

27. Cmd+hover on macOS, and Ctrl+hover on platforms where Ctrl is the equivalent command modifier, underlines a symbol when LSP definition data is available. Moving away or releasing the modifier clears the underline according to existing hover behavior.

28. Cmd+click / Ctrl+click on an underlined symbol navigates to the LSP definition result. If the definition is in another file, twarp opens or focuses that file in the same Code tab group and moves the cursor to the returned line/column. If the definition is in the same file, twarp moves the cursor/scroll to that location without creating a duplicate tab.

29. If the LSP definition result points outside the current known workspace, twarp opens the target path in a Code tab and registers the external file with the LSP manager when supported so hover/references continue to work there.

30. If LSP returns multiple definitions, the first result is used for this feature. A richer chooser is out of scope.

31. If a cmd+click target resolves to the symbol's own definition and there is no different definition to navigate to, twarp shows the existing find-references card when references are available. If no references are available, nothing disruptive happens.

32. If no LSP server is available, the modifier-click gesture does not edit the file, does not change selection unexpectedly, and does not show an error toast. Normal text selection/cursor behavior remains intact.

33. Right-click context-menu "Go to definition" and Vim go-to-definition keep working and navigate through the same Code tab workflow as cmd+click.

34. Navigation preserves unsaved edits in all open tabs. Opening or focusing a definition target must not save, discard, or reload dirty buffers unless the user explicitly takes such an action.

## Smoke test

Run against a built twarp binary with a local repository containing at least two source files where one symbol has a valid LSP definition in the other file. Use a language with a configured/running LSP server, such as Rust in this repository.

### 10a - File-tree to Code pane, save, and reload

1. Start twarp in the local repository. Open the left project/file tree and click a local source file such as `app/src/code/view.rs`.
2. Verify the file opens in an editable Code pane, not in the code review panel. The pane title or tab shows the filename, and the editor has syntax highlighting.
3. Type a harmless temporary edit. Verify a dirty indicator appears in the pane header or active editor tab.
4. Press Cmd+S on macOS or Ctrl+S on another platform. Verify the dirty indicator clears and a save success toast appears.
5. Revert the temporary edit and save again. Verify the file on disk matches its original content.
6. With the file still open and clean, change the file externally using another shell/editor. Verify the open Code pane reloads the changed content.
7. Make an unsaved edit in twarp, then change the same file externally. Verify the unsaved twarp edit is not overwritten silently and the editor remains dirty.
8. Open the Open File palette, search for another source file, accept the result, and verify it opens in the same standalone Code editing workflow.
9. Try opening a remote/unavailable file-tree entry or a binary file if available. Verify it does not create a raw text Code editor with unusable binary content.

### 10b - Multi-file tabs and cmd+click definition

1. Open one source file from the file tree, then open a second source file from the file tree or Open File palette while tabbed editing is enabled.
2. Verify both files appear as tabs in the Code pane. Click each tab and confirm the active editor switches without losing content.
3. Edit one tab without saving. Verify only that tab shows a dirty indicator. Switch tabs and verify the dirty state persists.
4. Attempt to close the dirty tab. Choose Cancel and verify the tab remains open. Close it again, choose Save or Discard, and verify the chosen action is honored.
5. Reopen a file that is already open. Verify twarp focuses the existing editor tab instead of creating a duplicate tab.
6. In a file with a known symbol definition, hold Cmd/Ctrl and hover the symbol. Verify the symbol underlines when the LSP result is available.
7. Cmd+click / Ctrl+click the underlined symbol. Verify twarp opens or focuses the definition file in the Code tab group and places the cursor at the target line/column.
8. Use the context menu "Go to definition" or Vim go-to-definition on another symbol. Verify it navigates through the same Code tab workflow.
9. Stop or disable the LSP server, then repeat a modifier-click on a symbol. Verify editing remains usable and no disruptive error UI appears.
