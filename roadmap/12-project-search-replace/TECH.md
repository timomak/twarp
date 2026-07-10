# 12 - Project-wide search & replace technical spec

## Context

Feature 12 builds on code that is already present in the current tree for project-wide search, then adds a preview-first replace flow. The user-facing behavior is specified in `PRODUCT.md`; this document focuses on where implementation should land and how to validate it.

The existing search backend lives in `crates/twarp_ripgrep`. `crates/twarp_ripgrep/src/search.rs:21` defines the public match/submatch structs used by the UI, and `crates/twarp_ripgrep/src/search.rs:44` runs a JSON-emitting ripgrep subprocess with case, multiline, include, and exclude parameters. The backend intentionally quits on binary files and uses a line heap limit for single-line searches at `crates/twarp_ripgrep/src/search.rs:88`. Streaming search is already exposed through `search_streaming` at `crates/twarp_ripgrep/src/search.rs:224`, which spawns the current Warp binary with the `ripgrep-search` subcommand at `crates/twarp_ripgrep/src/search.rs:249`.

The current UI/model stack for search is `app/src/workspace/view/global_search`. `app/src/workspace/view/global_search/mod.rs` defines `SearchConfig` with regex, case sensitivity, include globs, and exclude globs. `app/src/workspace/view/global_search/model.rs:60` aborts the previous search, emits a fresh search id, converts literal queries with `regex::escape`, and calls `twarp_ripgrep::search::search_streaming`. Results are emitted incrementally and then batched after the first 50 rows at `app/src/workspace/view/global_search/model.rs:154`. The view stores query/filter editors, roots, stale-search state, result grouping, selection, and toggles at `app/src/workspace/view/global_search/view.rs:299`.

The left panel already exposes Global Search under the `GlobalSearch` feature flag and code settings. `app/src/workspace/view.rs:18416` adds the Search panel to local filesystems when the flag and setting are enabled, and `app/src/workspace/view.rs:20051` handles toggle/open actions. `app/src/workspace/view/left_panel.rs:612` renders the Search toolbelt entry, `app/src/workspace/view/left_panel.rs:690` creates and subscribes to a `GlobalSearchView`, and `app/src/workspace/view/left_panel.rs:999` converts a search `OpenMatch` event into `OpenFileWithTarget` with `CodePanelsFileOpenEntrypoint::GlobalSearch`.

The editor-open path needed for result clicks already exists. `app/src/code/view.rs:813` focuses an existing tab or opens a new one by path, and `app/src/code/view.rs:851` sets a pending `ScrollPosition::LineAndColumn` so files can jump after load. This is the endpoint 12a should continue to use.

File writes should go through the existing file/editor models rather than adding write behavior to `twarp_ripgrep`. `crates/twarp_files/src/lib.rs:662` owns file saves and emits `FileSaved`/`FailedToSave`. `app/src/code/global_buffer_model.rs:481` wraps those saves for tracked editor buffers and keeps file versions in sync. `app/src/code/view.rs:930` and `app/src/code/view.rs:1031` show the current editor save and unsaved-change patterns that replace must not bypass for open buffers.

Headless integration test patterns already exist for opening local files and asserting editor position. `crates/integration/src/test/file_tree.rs:36` verifies that a file opens in Warp's editor, and `crates/integration/src/test/goto_line.rs:120` verifies line/column jumps. Project search integration tests should follow that Builder/TestStep style.

## Proposed changes

### 12a - Project-wide search UI

1. Treat the existing `GlobalSearch` feature flag as the rollout gate for 12a. Do not introduce another 12a flag unless the current `GlobalSearch` flag cannot cover the full search surface.

2. Keep `GlobalSearchView` as the left-panel implementation rather than adding a second modal. It already has the required query field, regex/case toggles, include/exclude filters, grouped results, collapse/expand actions, keyboard navigation, and `OpenMatch` event shape.

3. Close any product gaps in the current search UI inside `app/src/workspace/view/global_search/view.rs` and `app/src/workspace/view/global_search/model.rs`:
   - Ensure invalid regex is surfaced as a visible error and does not leave stale current results.
   - Ensure empty query state does not show "No results found" before a real search.
   - Ensure capped-result messaging is tied to `MAX_MATCH_COUNT`.
   - Ensure remote/unsupported/local-fs-disabled states match `PRODUCT.md`.

4. Keep result-click routing through `GlobalSearchEvent::OpenMatch` -> `LeftPanelEvent::OpenFileWithTarget` -> `WorkspaceView::open_file_with_target` -> `CodeView::open_or_focus_existing`. This preserves existing editor target resolution, Markdown-vs-code behavior, telemetry, and line/column jumping.

5. Keep search roots owned by the left-panel working-directories model. Do not ask the search view to scan arbitrary filesystem roots outside the current workspace unless a later spec adds root selection.

6. Add or tighten tests:
   - Unit tests for literal escaping, regex mode, case sensitivity, include/exclude parsing, stale search ids, and submatch-to-column conversion.
   - A headless integration test that opens a local workspace, opens Search, types a query, observes saved result positions, clicks a match, and asserts the editor pane opens at the expected line/column.

### 12b - Replace

1. Add replace state to the search surface behind a dark gate if 12b lands before the entire preview/apply path is complete. A reasonable gate name is `ProjectSearchReplace`; remove or promote it only when the preview and apply flow is stable. Keep search-only 12a usable while 12b is incomplete.

2. Introduce a replace model adjacent to the current search model, for example `app/src/workspace/view/global_search/replace_model.rs`, rather than expanding `twarp_ripgrep` into a mutating crate. The model should own:
   - Preview generation from the current query/config/results.
   - Per-file included/excluded state.
   - Apply-in-progress state and double-submit prevention.
   - Per-file apply outcome: applied, skipped stale, failed.

3. Represent previews with stable, file-oriented data:
   - Root path and file path.
   - Search config fingerprint: query, regex flag, case flag, include globs, exclude globs, roots.
   - Original file content version or file modification metadata when available.
   - Match spans in old-file byte offsets, plus line/column display positions.
   - Context lines before and after each match.
   - Replacement text.

4. Generate previews by reading current file contents, not by trusting old streamed line snippets. Search results are enough to seed candidate files, but preview generation must recompute matches against the current file content so byte spans are correct and stale search results cannot write wrong ranges.

5. Use the Rust `regex` crate for regex-mode preview matching to match the backend's regex semantics as closely as possible. In literal mode, escape the query before matching, as search already does. Do not implement capture-group replacement in the first pass unless the UI and tests explicitly document it; otherwise insert replacement text literally.

6. Apply replacements per file from the end of the file toward the beginning so earlier spans are not shifted before they are processed. Before writing, verify that every previewed old span still contains the expected old text. If verification fails for a file, skip that file and report it as stale.

7. For an open file with a tracked editor buffer, prefer applying through the editor/global-buffer path so the visible buffer updates and normal undo can work. If no public multi-edit API is available at implementation time, the conservative fallback is:
   - Write unopened files through `FileModel`.
   - For open files, either apply through the existing editor edit API or block apply for that file with a clear skipped reason. Do not overwrite open unsaved edits by writing stale disk content.

8. For unopened files, write through `FileModel` or a small helper in the same file-service layer so file watcher/version events stay consistent with the rest of Warp. Avoid direct ad hoc `std::fs::write` calls from the view.

9. Render the preview in `GlobalSearchView` or a sibling view under `app/src/workspace/view/global_search`, reusing existing diff/editor components where practical. `app/src/code/editor/diff.rs` already contains diff status and replacement rendering primitives; use them if they fit a compact per-file preview, but do not couple replace preview to code-review state.

10. After apply, mark current search results stale or re-run search with the same config. Prefer re-running when the search root count and file count are small enough; otherwise show the apply summary and require explicit refresh.

11. Telemetry should distinguish search opened/query started from replace preview generated, replace confirmed, replace canceled, replace applied, replace skipped stale, and replace failed. Use existing telemetry patterns in the search/left-panel code if adding events is in scope for the 12b implementation sub-phase.

## Testing and validation

1. `crates/twarp_ripgrep` unit coverage:
   - Literal, regex, case-sensitive, case-insensitive, include, exclude, binary-file skip, and multiline query behavior.
   - Streaming parser behavior for match messages with multiple submatches.

2. `GlobalSearch` model/view unit coverage:
   - A newer search id suppresses older progress.
   - Filter-only changes re-run a search.
   - Invalid regex produces a failed/error state.
   - Result grouping and cap behavior match `PRODUCT.md` behavior 11, 12, 17, and 18.

3. 12a integration coverage:
   - Create a temp local project with multiple matching files.
   - Open Search via `WorkspaceAction::ToggleGlobalSearch` or `OpenGlobalSearch`.
   - Type a query, wait for result saved positions, click a match, and assert pane count/title plus cursor line/column using the existing file-tree/goto-line helpers.
   - Repeat with include/exclude filters and case/regex toggles.

4. Replace model unit coverage:
   - Preview generation for literal search, regex search, empty replacement, multiple matches on one line, UTF-8 text, no trailing newline, and CRLF inputs.
   - Apply from the end of file preserves correct spans when replacements change string length.
   - Stale preview detection skips a changed file.
   - Per-file exclusion prevents writes.
   - Failed write for one file does not prevent reporting successful writes for other files.

5. 12b integration coverage:
   - Preview appears before disk changes.
   - Confirm writes only included files.
   - Excluded files remain unchanged.
   - Open editor buffers reflect applied changes or are explicitly skipped if unsaved/open-buffer support is deferred.
   - Double-confirm during apply does not duplicate replacements.

6. Manual smoke validation must follow the `PRODUCT.md` `## Smoke test` section against a built `warp-oss` binary on a machine with a real display. This worker node is headless, so implementation workers should run Rust unit/headless integration tests here and leave real-display UX gates to the supervisor.

## Sub-phase breakdown

### 12a - Project-wide search UI

Scope: finish and validate the search-only left-panel experience. Work should stay in `app/src/workspace/view/global_search`, left-panel/workspace routing, search telemetry if needed, `crates/twarp_ripgrep` tests, and headless integration tests. This phase should not add replace UI or mutate files.

Exit criteria:

1. Search is available in local workspaces behind the existing `GlobalSearch` gate.
2. Query, regex, case sensitivity, include, and exclude controls work.
3. Results are grouped by file with highlighted line previews.
4. Clicking or pressing Enter on a match opens the file in the editor at line/column.
5. Empty, loading, capped, invalid-regex, remote/unsupported, and no-results states match `PRODUCT.md`.

### 12b - Replace

Scope: add preview-first replace-all on top of current search results. Work should stay in the global-search module, file/global-buffer/editor write paths, telemetry if needed, and tests. This phase owns disk mutation and must stay dark until preview, confirm, stale-skip, and error reporting all work.

Exit criteria:

1. Replace can generate a per-file preview from the current search config.
2. Users can exclude files, cancel safely, or confirm.
3. Confirm applies only included, still-current previews.
4. Open buffers are updated through editor/global-buffer APIs where possible; otherwise they are skipped with an explicit reason.
5. Apply summaries distinguish applied, skipped stale, and failed files.
6. Search results refresh or become clearly stale after apply.

## Risks and mitigations

1. Stale spans can corrupt files if replace trusts old line snippets. Mitigation: preview and apply must recompute/verify spans against current file content before writing.

2. Open unsaved buffers can be overwritten if replace writes directly to disk. Mitigation: route open files through editor/global-buffer APIs or skip them with a clear reason.

3. Regex replacement semantics can surprise users. Mitigation: start with literal replacement text unless capture expansion is explicitly implemented, labeled, and tested.

4. Large repositories can produce too many results/previews. Mitigation: keep search caps, stream results, and require narrowing before replace if preview generation would exceed reasonable limits.

5. Cross-file undo is not guaranteed. Mitigation: preview + confirm is the safety boundary, and per-file editor undo is best-effort only for files changed through open editor buffers.

## Parallelization

Do not split 12a and 12b into parallel implementation agents on one branch. The roadmap requires one sub-phase per branch, and 12b depends on the search surface, result grouping, current-search fingerprinting, and editor-open routing from 12a.

Within 12a, parallel agents are unlikely to reduce wall-clock time because the work is concentrated in `app/src/workspace/view/global_search`, `app/src/workspace/view/left_panel.rs`, and one integration test. A single worker should finish the search UI and tests to avoid conflicting edits in the same view/model files.

Within 12b, a dispatcher may use parallel agents only after a lead worker defines the preview model API. If used, keep ownership disjoint:

1. `replace-model` worker: own `app/src/workspace/view/global_search/replace_model.rs` and unit tests for preview/apply span logic.
2. `replace-ui` worker: own `app/src/workspace/view/global_search/view.rs` preview rendering and user actions, consuming the model API without changing its internals.
3. `replace-integration` worker: own `crates/integration/src/test/project_search_replace.rs` and test registration after the model/UI shape is stable.

These workers should land as one coordinated 12b branch or as strictly ordered PRs against the fork, never against upstream. The model API must merge first; UI and integration work should rebase on that API rather than inventing separate preview state.
