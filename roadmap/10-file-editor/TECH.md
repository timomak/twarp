# 10 - File editor surface with go-to-definition (TECH)

Companion to [PRODUCT.md](PRODUCT.md). This plan is scoped to the two sub-phases in [STATUS.md](STATUS.md): 10a file-tree/Open File to Code pane with save/reload, then 10b multi-file tabs plus cmd-click definition.

## Context

The implementation should use the existing code editor stack rather than creating a new file editor. `CodeView` already owns an editor tab group, active tab index, pane configuration, title updates, save actions, and tab rendering. It registers Cmd+S/Ctrl+S as `code_view:save`, plus Save As and close-tab actions (`app/src/code/view.rs:89`, `app/src/code/view.rs:145`, `app/src/code/view.rs:247`). A `CodeView` constructs `LocalCodeEditorView::new_with_global_buffer` for path-backed files, which gives every opened file a shared `GlobalBufferModel` buffer (`app/src/code/view.rs:406`, `app/src/code/view.rs:487`).

`LocalCodeEditorView` is the existing file-backed rich editor. It stores file metadata, dirty/base versions, LSP server state, hover state, diagnostics, context menu, and find-references view (`app/src/code/local_code_editor.rs:287`). It opens global buffers with `GlobalBufferModel::open`, subscribes to buffer events, sets syntax language by path, and initializes LSP after file load (`app/src/code/local_code_editor.rs:1214`, `app/src/code/local_code_editor.rs:1450`). It saves through `GlobalBufferModel::save` and falls back to Save As when no file ID exists (`app/src/code/local_code_editor.rs:1524`).

`GlobalBufferModel` is the shared buffer and file-watch integration point. It subscribes to `FileModelEvent`, populates buffers on initial load, applies external file updates, avoids overwriting user edits when versions conflict, saves through `FileModel`, discards unsaved changes by re-reading from disk, and sends LSP document sync events (`app/src/code/global_buffer_model.rs:99`, `app/src/code/global_buffer_model.rs:220`, `app/src/code/global_buffer_model.rs:368`, `app/src/code/global_buffer_model.rs:481`, `app/src/code/global_buffer_model.rs:543`). Do not introduce a second file watcher path for this feature.

The file tree already has the right event surface. `FileTreeAction` includes click, keyboard execute, open in new pane, and open in new tab (`app/src/code/file_tree/view.rs:97`). Rendered file-tree rows dispatch `ItemClicked`, and `open_file` resolves a `FileTarget` before emitting `FileTreeEvent::OpenFile` (`app/src/code/file_tree/view.rs:2043`, `app/src/code/file_tree/view.rs:2299`, `app/src/code/file_tree/view.rs:2984`). The left panel forwards `FileTreeEvent::OpenFile` as `LeftPanelEvent::OpenFileWithTarget`, and `Workspace::open_file_with_target` routes `FileTarget::CodeEditor(layout)` to `open_code(CodeSource::FileTree { ... })` (`app/src/workspace/view/left_panel.rs:1039`, `app/src/workspace/view.rs:5232`, `app/src/workspace/view.rs:5347`).

The Open File palette already emits `CommandPaletteEvent::OpenFile`, and workspace handling routes it through `open_code` with a line/column target when present (`app/src/search/command_palette/view.rs:916`, `app/src/workspace/view.rs:11930`). File search items also preserve line/column metadata in `CommandPaletteItemAction::OpenFile` (`app/src/search/command_palette/files/search_item.rs:79`).

`Workspace::open_code` already centralizes Code-pane placement. It sends telemetry, groups files into an existing Code pane when `FeatureFlag::TabbedEditorView` and `prefer_tabbed_editor_view` are both enabled, de-duplicates paths in the current pane group when grouping is off, creates `CodePane::new` or `CodePane::new_preview`, and opens the pane in either a new workspace tab or right split (`app/src/workspace/view.rs:6758`). `CodePane` registers/deregisters with `CodeManager`, forwards CodeView file-open/tab-change events to the active-file model and opened-files model, and snapshots multi-tab state for persistence (`app/src/pane_group/pane/code_pane.rs:37`, `app/src/pane_group/pane/code_pane.rs:67`, `app/src/pane_group/pane/code_pane.rs:120`, `app/src/pane_group/pane/code_pane.rs:210`).

Go-to-definition should reuse existing LSP logic. `LocalCodeEditorView::goto_definition_at_cursor` calls the existing LSP server `goto_definition` and emits `LocalCodeEditorEvent::GotoDefinition`; `CodeView` handles that event by registering external files if needed, opening/focusing the target path, applying line/column positioning, and focusing the editor (`app/src/code/local_code_editor.rs:1899`, `app/src/code/view.rs:593`). Cmd-hover already requests definitions and creates a `HoverableLink` whose click dispatches either `NavigateToTarget` or a lazy find-references action (`app/src/code/local_code_editor.rs:587`, `app/src/code/local_code_editor.rs:631`, `app/src/code/local_code_editor.rs:721`). The low-level editor already emits `MaybeClickOnHoveredLink` on cmd-click/Ctrl-click (`app/src/code/editor/view/actions.rs:1383`) and suppresses normal selection when cmd-clicking an underlined symbol (`app/src/code/editor/view.rs:1518`).

## Proposed changes

### 10a - File-tree -> Code pane wiring, save, and reload

1. Audit the existing file-tree and palette routing end to end before changing code. The expected happy path is:
   - `FileTreeView` click/Enter/context menu resolves a `FileTarget`.
   - `LeftPanelView` forwards `OpenFileWithTarget`.
   - `Workspace::open_file_with_target` routes `FileTarget::CodeEditor(layout)` to `Workspace::open_code`.
   - `CodePane::new` creates `CodeView::new`.
   - `CodeView` creates `LocalCodeEditorView::new_with_global_buffer`.
   Implementation should patch only broken links in this chain.

2. Ensure ordinary file-tree clicks and keyboard activation can land in a standalone editable Code pane when the resolved target is `FileTarget::CodeEditor`. Preserve existing routing for markdown viewer, image viewer, external editor, system default, system generic, directories, and binary files. Do not change `resolve_file_target_with_editor_choice` semantics unless the current behavior violates PRODUCT 10a.

3. Keep context-menu "Open in new pane" and "Open in new tab" as explicit Code-pane intents by continuing to pass `EditorLayout::SplitPane` / `EditorLayout::NewTab` to file-target resolution. For binary files, keep the current binary guard in `FileTreeView::open_file` so explicit Code-pane actions do not render binary bytes as text.

4. Preserve one `CodeSource::FileTree { path }` per file-tree open. That keeps telemetry and `CodeManager` de-duplication distinct from `CodeSource::Link` palette/link opens. If a later implementer finds duplicate focus behavior inconsistent between FileTree and Link sources, fix it in `CodeManager`/`Workspace::open_code` using file path matching, not by erasing source identity.

5. Use `CodeView`'s existing save path. Cmd+S/Ctrl+S should dispatch `CodeViewAction::SaveFile`, call `CodeView::save_local`, then `LocalCodeEditorView::save_local`. Do not add a workspace-level save action for this feature unless the current focus context prevents the existing binding from firing inside standalone Code panes.

6. Leave external reload in `GlobalBufferModel`. If 10a exposes a bug where clean standalone Code panes fail to update after filesystem changes, fix the `FileModelEvent::FileUpdated` -> `GlobalBufferModelEvent::BufferUpdatedFromFileEvent` path or the local editor's handling of that event. Avoid adding reload state to `CodeView`.

7. Preserve dirty/close behavior in `CodeView`: `has_unsaved_changes`, `remove_tab_with_confirmation`, `clear_tab_group_with_intent`, `close_saved_tabs`, and quit-warning integration already provide the product behavior. Patch only missing title/header updates if a standalone file-tree-opened pane fails to display the dirty state.

8. Keep active-file propagation in `CodePane::attach` so file-tree highlight/opened-file tracking follows file opens and tab switches. If file-tree click opens a Code pane but the active-file highlight does not move, repair the `CodeViewEvent::FileOpened` / `TabChanged` forwarding rather than directly mutating file-tree state from workspace.

9. Persistence should use the existing `CodePaneSnapShot::Local` tab snapshot path. If standalone Code panes restored from session do not reopen their tabs, fix `CodeView::restore` / `CodePane::snapshot`, not a parallel app-state format.

10. No new user setting is required for 10a. Respect existing settings:
    - `code.editor.open_file_layout` for palette/default layout.
    - `code.editor.open_code_panels_file_editor` for default project explorer editor choice.
    - `code.editor.prefer_markdown_viewer` for markdown routing.
    - `code.editor.format_on_save` for LSP formatting before save.

### 10b - Multi-file tabs and cmd+click gesture

1. Keep tabs inside `CodeView`. `CodeView::open_or_focus_existing`, `open_new_tab_for_path`, `focus_existing_tab_if_present`, `render_tab_bar_with_draggable`, and `remove_tab_with_confirmation` already express the desired tab behavior (`app/src/code/view.rs:794`, `app/src/code/view.rs:848`, `app/src/code/view.rs:1728`). Implementers should harden these paths rather than adding a second tab model.

2. Match the sub-phase rollout to existing tab gating. If `FeatureFlag::TabbedEditorView` plus `prefer_tabbed_editor_view` is still the intended rollout control, 10b should ensure the twarp OSS/dev binary enables or exposes the feature in the desired channel. If the feature is considered always-on for twarp after 10b, remove only the minimal gating needed and document the behavior in the implementation PR.

3. When grouping into an existing Code pane, preserve the current exclusion of code review diff panes and read-only timeline commit diff panes. Project/file-tree opens should not pollute a purpose-specific diff pane.

4. Preserve de-duplication by file path within a CodeView tab group. A repeated open should focus the existing tab and apply pending line/column scroll. This path is also used by LSP definition navigation, so regressions here affect 10b.

5. Keep tab close semantics in `CodeView`. Dirty close confirmation should remain per tab, and "close all" should walk dirty tabs through the existing prompt flow. Make only focused fixes if the prompt does not appear for standalone file-editor tabs.

6. Cmd-click should reuse the existing hoverable-link path:
   - `CodeEditorViewAction::MouseHovered { cmd: true }` -> `LocalCodeEditorView::definition_for_hovered_range`.
   - `LocalCodeEditorView::fetch_definition_for_hover` -> `HoverableLink` with `NavigateToTarget` or `FetchAndShowFindReferences`.
   - `CodeEditorViewAction::MaybeClickOnHoveredLink` -> `CodeEditorModel::maybe_click_on_hovered_link`.
   - `LocalCodeEditorAction::NavigateToTarget` -> `LocalCodeEditorEvent::GotoDefinition`.
   - `CodeView` opens/focuses the target tab.
   If cmd-click is failing in standalone Code panes, fix the event/focus/modifier path between these existing steps; do not issue a new direct LSP request on mouse up.

7. Preserve right-click context menu and Vim go-to-definition behavior by routing them through `goto_definition_at_cursor` and the same `LocalCodeEditorEvent::GotoDefinition` handler. Cmd-click should be a gesture over the same navigation behavior, not a separate navigation system.

8. Keep find-references fallback semantics. When the definition target is the same symbol/location, the hover link's on-click should continue to fetch references lazily and show `FindReferencesView`. Do not add a definition chooser in 10b.

9. External definition targets should continue to call `LspManagerModel::maybe_register_external_file` before opening the target path. This preserves hover/references in files outside the current workspace where supported.

10. Add telemetry only if an existing event clearly covers this workflow. `LspTelemetryEvent::GotoDefinition` and `CodePaneOpened` already exist; avoid new telemetry unless needed to distinguish cmd-click from context-menu/Vim.

## Testing and validation

### Unit and model tests

1. Add/extend focused tests around `CodeView` tab behavior where practical:
   - Opening a path already in the tab group focuses the existing tab and does not append a duplicate (PRODUCT 12, 22).
   - Dirty state is tab-scoped and `contains_unsaved_changes` aggregates across tabs (PRODUCT 8, 20, 23).
   - Removing the last tab closes the pane through the existing pane event (PRODUCT 24).

2. Add/extend `GlobalBufferModel` tests only if 10a changes reload behavior. Cover clean-buffer external reload and dirty-buffer conflict preservation (PRODUCT 13, 14).

3. Add/extend file-tree view tests only if click/context-menu routing changes. The target is that file actions emit `FileTreeEvent::OpenFile` with the intended `FileTarget` and layout (PRODUCT 1, 2, 3, 5).

4. Add/extend editor gesture tests where the existing test harness can drive `CodeEditorViewAction` directly:
   - Cmd-hover sets a `HoverableLink` when definition data resolves (PRODUCT 27).
   - Cmd-click on the hovered link dispatches the `NavigateToTarget` path instead of normal text selection (PRODUCT 28, 32).
   - Same-location definition falls back to references when references are present (PRODUCT 31).

### Headless/offscreen integration tests

1. Prefer a narrow integration test using the existing integration framework only if it can run headlessly on this worker. The test should open a local fixture repo, open a file via the file tree or command palette, edit text, trigger save, and assert the file changed on disk (PRODUCT 1, 8, 9).

2. Add a tab-focused integration test only if the framework can inspect `CodeView` without a real display. It should open two files, switch tabs, close a saved tab, and verify the remaining tab is active (PRODUCT 19, 21, 24).

3. Real-display visual checks for tab header clipping, hover underline, and mouse cmd-click can be left to the primary Mac UX gate if they cannot run headlessly here. The implementation PR should state which visual behaviors were not locally exercised.

### Manual smoke

Use the `PRODUCT.md` smoke test exactly. The 10a smoke test validates file-tree/palette open, dirty state, save, reload, conflict preservation, and routing exclusions. The 10b smoke test validates editor tabs, dirty tab close prompts, duplicate-open focus, cmd-hover/cmd-click definition, context-menu/Vim navigation, and no-LSP behavior.

### Required checks for implementation branches

Run the standard worker checks for code implementation branches:

1. `cargo build --bin warp-oss`
2. `cargo fmt -- --check`
3. `cargo clippy --workspace -- -D warnings`

This spec-only branch does not require those Rust checks because it edits only roadmap markdown.

## Risks and mitigations

1. **Accidentally changing file-type routing.** Keep all file-target decisions in the existing `resolve_file_target_*` helpers and only force Code-pane routing for explicit Code open actions.
2. **Overwriting unsaved edits on external reload.** Treat `GlobalBufferModel` version checks as the source of truth and add tests if 10a touches reload logic.
3. **Polluting code-review diff panes with normal editor tabs.** Preserve `Workspace::open_code`'s diff-pane exclusion when tab grouping is enabled.
4. **Cmd-click racing hover resolution.** Reuse `HoverableLink` so clicks only navigate when definition data has already produced an underlined range. The gesture should no-op cleanly when there is no link.
5. **Line/column off-by-one errors.** Keep the existing conversion boundary: LSP locations are zero-based; `LineAndColumnArg` is one-based for line numbers. Add targeted tests around navigation if this path changes.
