# 03 — Render markdown by default — TECH spec

## Overview

This PR locks in twarp's existing "render markdown by default" behavior with regression tests and a single explanatory comment. It introduces **no user-visible behavior change**. PRODUCT invariants 2 and 3 already hold today via `code.editor.prefer_markdown_viewer = true` + `FileNotebookView::new()` initializing `MarkdownDisplayMode::Rendered`; this work pins those defaults so a future refactor cannot silently flip them back to raw. The `CodeViewView`'s `MarkdownDisplayMode::Raw` initial state is **deliberately preserved** (PRODUCT invariant 5) — that pane is only reached via explicit code-view intent, and a code comment documents why.

## Context

Current routing and pane state (verified during PRODUCT spec investigation):

- `crates/warp_util/src/file_type.rs:129` — `is_markdown_file()` matches extensions `md` / `markdown` and filenames `README` / `CHANGELOG` / `LICENSE`, all case-insensitive.
- `app/src/util/file/external_editor/settings.rs:100` — `prefer_markdown_viewer: PreferMarkdownViewer { type: bool, default: true, ... }`. Synced to cloud.
- `app/src/util/openable_file_type.rs:163-230` — `resolve_file_target_*` family. Returns `FileTarget::MarkdownViewer(layout)` when `is_markdown && prefer_markdown_viewer`. Existing tests live in `#[cfg(test)] mod tests` at line 235+; `test_markdown_files` (line 316) and `test_resolve_file_target_markdown_viewer_precedence` (line 261) already cover happy paths but miss case variants, `LICENSE`, and the `prefer_markdown_viewer = false` branch.
- `app/src/notebooks/file/mod.rs:293` — `markdown_display_mode: MarkdownDisplayMode::Rendered` in `FileNotebookView::new`. Tests submodule at `app/src/notebooks/file/mod_tests.rs` does not yet assert this default.
- `app/src/code/view.rs:293` — `MarkdownToggleView::new(MarkdownDisplayMode::Raw, ctx)` in `update_markdown_mode_segmented_control`. This is the deliberately-preserved Raw default; needs an inline comment, not a code change.

See `PRODUCT.md` for the user-facing behavior contract.

## Proposed changes

1. **Extend `openable_file_type.rs` tests** to lock routing for the full markdown surface:
   - Case variants: `README.MD`, `notes.Markdown`, `readme`.
   - Missing filename: `LICENSE`.
   - The `prefer_markdown_viewer = false` branch returns `FileTarget::CodeEditor(..)` for `.md`.
   - Existing `test_markdown_files` and `test_resolve_file_target_markdown_viewer_precedence` are extended in place rather than duplicated.

2. **Add `EditorSettings` default test.** New test (in the same `openable_file_type.rs` tests module, or a sibling test in `external_editor/settings.rs` if that file has tests) asserts `PreferMarkdownViewer::default_value() == true`. Mirrors the existing `test_open_code_panels_file_editor_default_is_warp` (line 250) pattern.

3. **Add `FileNotebookView` default-mode test** in `app/src/notebooks/file/mod_tests.rs`. Construct a fresh `FileNotebookView` via `ctx.add_view(FileNotebookView::new)` in a test harness and assert `markdown_display_mode == MarkdownDisplayMode::Rendered`. Follow whatever pattern existing tests in that file use (likely a `TestAppContext`-style fixture).

4. **Document the deliberate CodePane Raw default.** One-line comment above `app/src/code/view.rs:293`, e.g.:
   ```rust
   // Raw is the deliberate default in CodePane: this pane is only reached
   // via explicit code-view intent ("Open as Code" or prefer_markdown_viewer
   // = false). Picking "Rendered" via this toggle swaps the pane to
   // MarkdownViewer (see CodeViewAction::RenderMarkdown). See twarp 03 spec.
   ```

5. **Optional integration test (defer if framework friction).** Use the `warp-integration-test` skill to add one e2e test walking PRODUCT smoke-test steps 2–4: open `.md` via file tree → assert MarkdownViewer pane → click "Raw" → assert CodePane → click "Rendered" → assert MarkdownViewer. If the existing integration harness already covers file-open-into-pane routing, hook into that fixture; otherwise skip and rely on the unit tests + manual smoke test.

No new types, no new settings, no production-code logic changes.

## Files touched

| Path | Change |
|---|---|
| `app/src/util/openable_file_type.rs` | Extend existing test cases; add `PreferMarkdownViewer` default test. |
| `app/src/notebooks/file/mod_tests.rs` | Add `FileNotebookView` default `MarkdownDisplayMode` test. |
| `app/src/code/view.rs` | Comment-only change above line 293 documenting deliberate Raw default. |
| `crates/integration/tests/*.rs` *(optional)* | One e2e test for the toggle pane swap, if the harness fits. |

## Test plan

| PRODUCT invariant | Verification |
|---|---|
| 1 (path detection) | Extended `test_markdown_files` covers `.MD`, `.Markdown`, `readme`, `LICENSE`. |
| 2 (default → rendered) | New `test_resolve_file_target_markdown_*` cases for each extension/filename with `prefer_markdown_viewer = true`. Plus new `FileNotebookView` default-mode test. |
| 3 (setting default `true`) | New `test_prefer_markdown_viewer_default_is_true`. |
| 4 (toggle is a pane swap, both directions) | Optional integration test; otherwise covered by code-reading + manual smoke step 3–4. |
| 5 (CodePane stays Raw) | New routing test for `prefer_markdown_viewer = false` returning `FileTarget::CodeEditor`. Plus the code comment makes intent explicit for future readers. |
| 6 (mode does not persist) | No test; manual smoke step 6 covers it. Adding a persistence test would require modeling session restore in a unit test, which is out of proportion for this PR. |
| 7 (detection is path-based) | Already covered by existing `is_markdown_file` tests; no new tests. |

Manual verification: run the full smoke-test checklist from `PRODUCT.md` against a local debug build before opening the PR for review.

## Parallelization

Not beneficial. The work is a handful of test additions and a one-line comment in a single PR, with no independent subtasks worth fanning out. A single agent edits all four files sequentially.

## Risks

- **False sense of coverage if the optional integration test is skipped.** Unit tests verify routing and default initial state, but cannot catch a regression where the toggle wire-up breaks (e.g. the `ReplaceWithCodePane` event stops firing). The manual smoke test must be run before review to close that gap. If integration coverage proves easy, prefer adding it.
- **Future refactor of `prefer_markdown_viewer` default.** If a contributor (twarp or upstream cherry-pick) changes the default to `false`, the new `test_prefer_markdown_viewer_default_is_true` will fail loudly. Intended outcome.
- **Upstream cherry-pick conflicts in `view.rs:293`.** Adding a comment block above this line creates a small textual conflict surface. Likelihood is low (this code is stable post-AI-removal) and the resolution is mechanical (keep both upstream's change and twarp's comment).
- **Large-file rendering performance.** Unchanged. We don't touch the markdown parser or `RichTextEditorView`. PRODUCT explicitly carries forward today's behavior on large files.

## Rollout

No feature flag, no migration, no telemetry. Plain merge to master. The just-merged feature 02 (AI removal) is already on master; this PR has no dependency on it beyond confirming the markdown render path is untangled from the deleted AI assistant transcript renderer (verified during PRODUCT spec investigation — none of the surfaces under test reference AI code).
