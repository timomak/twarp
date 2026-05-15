# 09 — File editor surface with go-to-definition

**Phase:** not-started
**Spec PR:** —
**Impl PRs:** —

## Scope

Expose twarp's existing `crates/editor/` rich-text surface as a first-class file-editing workflow, not a code-review subsidiary. Open any file via the existing file tree or "Open File" palette into a `Code` pane; multi-file tabs with dirty indicator and Cmd+S to save; reload on external change via the existing file watcher. Wire cmd+click on a symbol to LSP `definition()` — the protocol call is already implemented in `app/src/code/local_code_editor.rs`, so this is gesture wiring + UX, not protocol work. Hover, find-references, and diagnostics ride along because they're already wired to the same editor surface.

## Why this slot

Foundation for the IDE direction (10 and 11 depend on it). Placed after 08 (rebrand) because wiring touches `app/src/code/`, `crates/editor/`, and `crates/lsp/` — all of which would be churned during the rebrand rename pass.

## Sub-phases

- [ ] **9a — File-tree → code pane wiring.** Click a file in the existing file tree (or trigger "Open File" palette mode) to open it in a `Code` pane. Save (Cmd+S) with dirty-state indicator. Reload on external change via existing watcher.
- [ ] **9b — Multi-file tabs + cmd+click gesture.** Multiple open files as tabs within the window. Cmd+click on a symbol triggers LSP `definition()` — verify whether the gesture is already wired in code-review context vs. needs a new binding. Tab close confirms when dirty.

## What's already built (audited 2026-05-14)

- `crates/editor/` — rich text surface with cursors, selection, undo/redo, multi-cursor, vim mode (937-LOC handler)
- `crates/lsp/` — manager + service with `definition()`, `hover()`, `references()`, `format()` all callable; diagnostics already rendered as dashed underlines via `app/src/code/language_server_extension.rs`
- Pane system supports `PaneType::Code`; non-terminal panes already used for Settings, Welcome, code review, etc.
- File tree (`crates/repo_metadata/src/file_tree_store.rs`) with gitignore + watcher
- Tree-sitter syntax highlighting, bracket matching, auto-indent
- Suggestion UI ready to host LSP completion in a future feature

## Notes

- 9a is the gating piece — without it, every existing LSP feature stays trapped inside the code review panel.
- Out of scope for this feature: LSP completion, rename, outline pane, breadcrumbs, status bar, code folding. Those become candidate follow-on features if the IDE direction continues past this trio (09–11).
- No upstream cherry-pick conflicts expected — the changes are workflow wiring, not modifications to widely-touched surfaces.
