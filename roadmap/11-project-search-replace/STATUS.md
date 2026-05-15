# 11 — Project-wide search & replace

**Phase:** not-started
**Spec PR:** —
**Impl PRs:** —

## Scope

Search across the project from a dedicated UI (palette mode or side panel), backed by the existing `warp_ripgrep` crate. Click a result to open the file at the matched line in the editor. Replace-all with per-file preview before apply.

## Why this slot

Independent of 09 in principle, but result-click → open-file becomes useful only once the file-editor surface exists. The `warp_ripgrep` crate is already in the workspace but currently has no UI outside terminal commands.

## Sub-phases

- [ ] **11a — Project-wide search UI.** Search modal or side panel. Query input with case-sensitivity and regex toggles. Results grouped by file with line previews. Click → open file at match in editor pane.
- [ ] **11b — Replace.** Replace-all flow with per-file diff preview before apply. Confirm + commit changes to disk. Undo via the editor's existing undo system where possible.

## What's already built

- `warp_ripgrep` crate provides the search backend
- In-file find model (`app/src/code_review/find_model.rs`) is a UI pattern reference
- Suggestion UI patterns for displaying ranked results
- Editor file open/save (lands with 09)

## Notes

- Cross-file undo is non-trivial. If full undo is hard, fall back to "preview + confirm before apply" as the safety net; the editor's per-file undo still works on any file kept open after the replace.
- Replace previews should show context lines around each match, not just the matched range.
- No upstream cherry-pick conflicts expected — this wires an existing crate into a new UI.
