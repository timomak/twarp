# 11 - Git blame & per-line history

## Context

`PRODUCT.md` defines the user-facing behavior. The feature spans the local file editor, gutter rendering, Git command helpers, and a small commit-detail overlay.

Relevant current code:

- `app/src/util/git.rs:13` exposes `run_git_command`, with local-filesystem and wasm stubs. `app/src/util/git.rs:86` exposes `run_git_command_with_stdin`, which is useful if blame needs current buffer contents rather than only the saved working-tree file.
- `app/src/code_review/porcelain_v2.rs:143` is the best local template for a permissive, unit-tested Git porcelain parser. It skips unknown lines and keeps parsing instead of failing the whole feature.
- `app/src/code/local_code_editor.rs:287` owns file-editor state around a `CodeEditorView`. It already has file metadata, save/load state, LSP hover state, and local overlays.
- `app/src/code/local_code_editor.rs:350` subscribes to `CodeEditorEvent`s. `app/src/code/local_code_editor.rs:1453` subscribes to `GlobalBufferModelEvent`s and emits `FileLoaded`/`FileSaved`. `app/src/code/local_code_editor.rs:1646` exposes the loaded file path.
- `app/src/code/editor/view.rs:105` defines editor events. `app/src/code/editor/view.rs:181` defines display options that are passed into the wrapper. `app/src/code/editor/view.rs:492` shows the current pattern for enabling gutter affordances from parent views.
- `app/src/code/editor/element.rs:65` has a fixed gutter width today. `app/src/code/editor/element.rs:624` builds per-line gutter elements from render blocks. `app/src/code/editor/element.rs:1224` renders the line-number/gutter-button content for each line.
- `app/src/code/editor/element.rs:338` and `app/src/code/editor/element.rs:367` show the current gutter click model: the wrapper reports a `GutterRange` to the editor click handler.
- `app/src/code/editor/diff.rs:529` exposes `DiffStatus`, and `app/src/code/editor/diff.rs:545` computes changed ranges against the editor's base content. Regular editor reset already sets base content at `app/src/code/editor/model.rs:1796`.
- `repo_metadata::DetectedRepositories` is already used to resolve repo roots for active files in `app/src/workspace/view/left_panel.rs:1110` and `app/src/workspace/view.rs:10662`.
- `app/src/code_review/git_status_update.rs:225` refreshes repo metadata after git-relevant events and emits metadata changes from `app/src/code_review/git_status_update.rs:243`.
- `app/src/workspace/view/left_panel.rs:1211` resolves GitHub `origin`, and `app/src/code_review/github_author.rs:45` parses GitHub remote URLs.
- `app/src/workspace/view.rs:10553` fetches pre/post commit contents with `git show` and opens a read-only diff pane. 11b should reuse the same Git primitives and path handling where possible.
- `app/src/code/language_server_extension.rs:471` is the closest existing pattern for a local editor popover with scrollable formatted content.
- `crates/twarp_features/src/lib.rs:780` shows where neighboring twarp git feature flags are declared; `app/src/bin/oss.rs:23` force-enables twarp OSS flags that should be on in the default dev binary.

## Proposed changes

### Shared rollout and module shape

Add a dedicated `FeatureFlag::GitBlame` and gate all fetching, gutter rendering, and popover work behind it. During 11a, keep the feature dark unless the worker deliberately enables it for local smoke testing. After 11b is complete, the implementation can add the flag to `TWARP_OSS_FLAGS` if the roadmap expects blame to be on in the default twarp dev binary.

Add a new local module under `app/src/code/`, for example `app/src/code/git_blame.rs`, and export it from `app/src/code/mod.rs`. Keep Git blame parsing and commit-detail parsing out of `LocalCodeEditorView` so the behavior is unit-testable without rendering the app.

Core types:

- `BlameLine`: 0-based current line index plus either `Committed(BlameCommitRef)` or `Uncommitted`.
- `BlameCommitRef`: full SHA, short SHA, author name, author email, author timestamp, author timezone, summary, and original filename when porcelain reports it.
- `ParsedBlame`: line-indexed `Vec<BlameLine>` plus any parse diagnostics useful for debug logging.
- `CommitDetail`: full SHA, author fields, date fields, message, optional GitHub URL, repository-relative path, and file-scoped patch text.
- `BlameCacheKey`: repo root, repository-relative file path, buffer/base content version, and `HEAD` OID. Include the OID so checkout/rebase/reset cannot reuse blame from the previous history.

### 11a - Blame parser + gutter rendering

Git fetch:

1. Resolve `(repo_root, relative_path)` from `LocalCodeEditorView::file_path()` and `DetectedRepositories`.
2. Fetch `HEAD` with `git rev-parse HEAD`. If the repo has no commits, return an empty committed blame result and let dirty-line handling show `(uncommitted)` where possible.
3. Run `git blame --porcelain -- <relative_path>` for clean saved content. If the implementation needs the current unsaved buffer for a manual refresh path, use `run_git_command_with_stdin` with `git blame --porcelain --contents - -- <relative_path>`.
4. Never run blame on every keystroke. Load on file load, path change, explicit repo invalidation, and save completion.

Parser:

1. Parse porcelain headers of the form `<sha> <original-line> <final-line> [group-size]`.
2. Parse known metadata fields: `author`, `author-mail`, `author-time`, `author-tz`, `summary`, and `filename`.
3. Treat all-zero SHAs as `Uncommitted`.
4. Keep a commit metadata table by SHA because regular porcelain omits repeated commit metadata inside a group.
5. Ignore unknown fields and malformed records when possible, matching the permissive style in `porcelain_v2.rs`.
6. Add focused unit tests for grouped records, repeated commits, zero SHA/uncommitted lines, missing optional fields, filenames with spaces, unknown fields, and malformed trailing records.

Editor state and invalidation:

1. Add `BlameState` to `LocalCodeEditorView`, not to the shared low-level editor crate. The local editor has file identity, save/load events, and repo context.
2. Store the latest request token/version with each async blame request. When a request completes, apply it only if the active file path, repo root, `HEAD` OID, and relevant content version still match.
3. On `GlobalBufferModelEvent::BufferLoaded`, request initial blame after the file path and base content version are known.
4. On `GlobalBufferModelEvent::FileSaved` and successful external file reload, invalidate and refetch blame.
5. Subscribe to the repo status/update path already used by Git chips or diff state. On commit/index/working-tree metadata changes, clear the cache entry for the active file and refetch if the editor is still visible.
6. On user edits, do not fetch. Instead, mark changed/inserted lines from the editor's diff status as `Uncommitted`. If mapping from cached blame to the edited buffer is ambiguous, prefer `Uncommitted` or blank over stale committed blame.

Gutter rendering:

1. Extend `CodeEditorViewDisplayOptions` with optional blame annotations and a click callback/event path.
2. Pass blame annotations into `EditorWrapper::new` alongside the existing line-number and diff display data.
3. Replace the single hard-coded `GUTTER_WIDTH` assumption with a width derived from line-number width plus optional blame width plus existing diff/button affordance spacing. Preserve the current line-number-only width when blame is disabled.
4. Extend `render_gutter_element` to render line number and blame text in the same per-line row. The blame text should be subdued, truncated, and lower priority than line numbers and diff/comment buttons.
5. Add a blame click target. A practical shape is `GutterRange::Blame { line: EditorLineLocation, sha: String }`, with `CodeEditorEvent::BlameAnnotationClicked { path, line, sha }` or a local-editor event that carries enough context for 11b.
6. Do not attach blame to temporary deletion blocks or collapsed hidden-section rows.

### 11b - Commit detail popover

Fetch:

1. Reuse the same `(repo_root, relative_path)` resolution and request-token pattern as 11a.
2. Fetch structured commit metadata with a parse-friendly command such as `git show -s --format=%H%x00%an%x00%ae%x00%aI%x00%B <sha>`.
3. Fetch the file-scoped patch with `git show --format= --patch --find-renames --find-copies <sha> -- <relative_path>`. Keep the scope to the open file; this matches the product goal of per-line file history and keeps the popover bounded.
4. Fetch `origin` once per repo with `git remote get-url origin` and reuse `parse_github_origin` to build `https://github.com/<owner>/<repo>/commit/<sha>` when possible.
5. Cache commit details by `(repo_root, relative_path, sha)` for the session so repeated clicks are instant.

UI:

1. Add local editor state for `BlamePopoverState`: closed, loading, loaded, and failed.
2. When the editor receives a blame-click event, open the popover immediately in loading state and kick off the detail fetch unless cached.
3. Render the popover from `LocalCodeEditorView::render`, similar to the LSP hover tooltip: a bounded, scrollable container with formatted message text and a read-only diff section.
4. Anchor the popover near the clicked gutter annotation. If the current event stack cannot expose a stable save position for gutter text, store the last click position in wrapper state and position the overlay relative to the editor wrapper.
5. Close the popover on Escape, outside click, active file change, editor close, or flag disable. Do not move the text cursor as a side effect.
6. Keep popover actions minimal: close and optional `Open on GitHub`. Do not add commit mutation actions.

## Testing and validation

Unit tests:

1. Add parser tests in the new blame module for PRODUCT Behavior 4, 8, 12, 17, and 18.
2. Add local Git-command tests under `#[cfg(all(test, feature = "local_fs"))]` using a temp repository with multiple authors, saved uncommitted changes, no commits, and a non-Git file.
3. Add editor element/view tests for the blame-disabled path to prove the gutter remains identical when `GitBlame` is off.
4. Add editor rendering/click tests for a blame-enabled gutter row if the existing element test harness can inspect text/click dispatch without a real display.
5. Add popover state tests for cached commit details, stale request discard, failed detail fetch, and GitHub URL construction.

Manual smoke validation:

1. Run the 11a and 11b smoke steps from `PRODUCT.md` against a built twarp binary.
2. Verify narrow editor widths, long author names, wrapped source lines, and dirty buffers.
3. Verify switching tabs while a blame request is in flight does not apply stale annotations.
4. Verify `cargo fmt -- --check`, targeted Rust tests, `cargo build --bin warp-oss`, and `cargo clippy --workspace -- -D warnings` before handoff.

Risks and mitigations:

1. Expensive blame on large files: fetch asynchronously, cache by file/version/HEAD, and avoid per-keystroke subprocesses.
2. Stale attribution on dirty buffers: dirty-line overlay must prefer `(uncommitted)` or blank over stale committed metadata whenever mapping is uncertain.
3. Gutter layout regressions: blame is off by default via the `text_editing.code_editor_git_blame` setting (`app/src/settings/editor.rs`), and the gutter keeps its pre-blame width/rendering exactly while the setting is off.
4. Popover scope creep: keep 11b file-scoped and read-only; reuse the existing read-only diff-pane work only as a reference, not as a second full review surface inside the popover.
