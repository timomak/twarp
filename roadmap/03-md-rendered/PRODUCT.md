# 03 — Render markdown by default — PRODUCT spec

## Summary

When a user opens a markdown file (`.md`, `.markdown`, or one of `README` / `CHANGELOG` / `LICENSE` with no extension) in twarp through any default code path, the file renders. The user can flip to raw via an always-visible per-pane toggle. The behavioral default is "rendered"; this spec locks that contract so it cannot silently regress to "raw."

## Background

Investigation found that today's default open path already routes markdown files to the rendered viewer. The two relevant surfaces are:

- **MarkdownViewer pane** (`FileNotebookView`) — renders. Default initial mode: `Rendered`.
- **Code editor pane** (`CodeViewView`) — raw. Default initial mode: `Raw`.

Routing between them is controlled by the existing setting `code.editor.prefer_markdown_viewer` (boolean, default `true`). The default routing for `.md` lands in MarkdownViewer.

The per-pane "Rendered / Raw" segmented toggle is not an in-pane mode flip — picking the opposite mode **swaps the pane type**. "Rendered" in CodePane closes that pane and opens a MarkdownViewer; "Raw" in MarkdownViewer closes it and opens a CodePane. There is no third "raw text shown inside MarkdownViewer" state.

This means: the implementation contract is **routing-level**, not in-pane. Code editor staying Raw-default is intentional, because the only entry points to it for `.md` files are explicit code-view intents — "Open as Code," `prefer_markdown_viewer = false`, restoring a CodePane snapshot — and overriding those would undo the user's explicit choice.

## Behavior

1. A file whose path matches `is_markdown_file()` (extensions `.md` / `.markdown` case-insensitive, or filename `README` / `CHANGELOG` / `LICENSE` case-insensitive with no extension) is treated as markdown for routing purposes.

2. Opening a markdown file through any default code path — file-tree click, file-tree keyboard activation, workspace "open file" routing, code review file open, "Open in New Tab" — routes to the MarkdownViewer pane and renders the file. This is the headline contract: **default open → rendered**.

3. The `code.editor.prefer_markdown_viewer` setting governs the routing decision. Its default is `true` and must stay `true`. Users who flip it to `false` opt out of the default and accept that markdown files will open into the code editor pane (raw).

4. The "Rendered / Raw" segmented control in the pane header is always present when the open file is markdown, in both MarkdownViewer and CodeEditor panes. Selecting the non-active mode replaces the current pane with the corresponding pane type, preserving the file path and code source. Cancelling an "unsaved changes" prompt on this swap leaves the user in the current pane.

5. Explicit code-view entry points keep raw-default behavior:
   - "Open as Code" action from MarkdownViewer → CodePane → Raw.
   - File open while `prefer_markdown_viewer = false` → CodePane → Raw.
   - Restoring a persisted CodePane snapshot that holds a markdown file → CodePane → Raw.

6. The mode chosen by the user via the segmented toggle does not persist across sessions or across reopens — closing and reopening a file resets to the default for that surface (Rendered for MarkdownViewer, Raw for CodePane). Persistence is out of scope.

7. Markdown detection is path-based only. File contents that "look like markdown" inside a non-markdown extension are not rendered; conversely, a `.md` file with non-markdown contents still routes to MarkdownViewer (which renders whatever it parses, malformed or otherwise).

## Setting

Use the existing key:

- **`code.editor.prefer_markdown_viewer`** — boolean, default `true`, synced to cloud. No new setting introduced. The "Code → Open Markdown files in Warp's Markdown Viewer by default" switch in settings remains the single user-facing control for the routing default.

## Out of scope

- Changing which extensions/filenames count as markdown (`is_markdown_file()` is left alone).
- Changing the rendered style, theme, syntax support, or which markdown features render (tables, code fences, etc.).
- Adding rendering to surfaces that don't render today: `cat foo.md` terminal output, file-path hover previews, inline preview tooltips. If those surfaces exist later, they're a separate feature.
- Persisting the per-pane Rendered/Raw choice across sessions or per-file.
- Flipping the CodePane initial mode to `Rendered` for `.md` files — explicit code-view intent stays raw.
- Auto-rendering files whose contents look like markdown but whose path doesn't match `is_markdown_file()`.

## Edge cases

- **Empty `.md` files.** Open through default path → MarkdownViewer → renders an empty document. No error, no fallback to raw.
- **Very large `.md` files.** No new size threshold introduced. Whatever the MarkdownViewer / `parse_markdown` pipeline does today on a large file is preserved.
- **`.MD`, `.Markdown`, `README.md`, `readme`, `License`.** All match `is_markdown_file()` (case-insensitive). Route to MarkdownViewer.
- **`README` with no extension and non-markdown contents** (e.g. plain text or shell script). Routes to MarkdownViewer; rendered output reflects whatever the markdown parser produces.
- **Broken or partial markdown.** Renders best-effort; no fallback to raw, no error toast.
- **File opened from terminal output** (clicking a detected path). Routes through the same default opener and renders.
- **`prefer_markdown_viewer = false` user.** Markdown files open in CodePane, raw, with the toggle visible so they can opt back into rendered per-file.
- **Restoring a window with a previously-open `.md` file in a CodePane snapshot.** Stays in CodePane (Raw). Persistence respects the snapshot, not the global default.

## Smoke test

Run against a built twarp binary. Use a workspace containing `README.md` (non-trivial markdown — headings, list, code fence), `EMPTY.md` (zero bytes), and `Notes.MARKDOWN` (uppercase extension).

1. Open twarp with the workspace. Confirm settings show `code.editor.prefer_markdown_viewer = true` (default).
2. Click `README.md` in the file tree. It opens in the MarkdownViewer pane, rendered (headings styled, list bulleted, code fence in a code block). The pane header shows a "Rendered / Raw" segmented control with "Rendered" selected.
3. Click "Raw" in the segmented control. The pane swaps to the code editor showing the raw markdown source. The toggle shows "Raw" selected.
4. Click "Rendered" in the segmented control. The pane swaps back to MarkdownViewer, rendered. Repeat once to confirm the swap is stable.
5. Right-click `README.md` in the file tree → "Open as Code" (or equivalent action). Verify it opens in the code editor pane with "Raw" selected — explicit code intent is preserved.
6. Close `README.md`, then reopen it from the file tree. Verify the default Rendered state returns (mode does not persist).
7. Open `EMPTY.md` via the file tree. Verify it opens in MarkdownViewer with an empty rendered body, no error.
8. Open `Notes.MARKDOWN` via the file tree. Verify it routes to MarkdownViewer and renders (case-insensitive extension match).
9. In settings, toggle "Open Markdown files in Warp's Markdown Viewer by default" off. Open `README.md` from the file tree. Verify it now opens in the code editor pane (Raw). Toggle the setting back on; reopen → MarkdownViewer (Rendered).
10. Open `README.md` via a terminal command's clickable file path (e.g. `ls` output, or `echo $(pwd)/README.md` and click). Verify it routes to MarkdownViewer, Rendered.
