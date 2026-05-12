---
name: 05 — Open Changes panel
status: draft
---

# Open Changes panel — TECH

Companion to [PRODUCT.md](PRODUCT.md). Section numbers below refer to PRODUCT.md.

## Context

This feature builds a side-panel git-review surface modeled on VS Code's Source Control view. The user-visible contract is exhaustively pinned in PRODUCT.md (45 numbered invariants spanning panel layout, file lists, inline diff, stage / unstage / discard, commit, push / pull / fetch, file Timeline, refresh, and accessibility). This tech spec covers how the implementation fits into twarp's existing surfaces — tool-panel registration, the shell-out git wrapper, the gpui-style command-completion event dispatcher, the bulk filesystem watcher, the existing `DiffViewer`, and the toast / inline-banner UI primitives — and how the five sub-phases (5a–5e) compose without churn.

Almost nothing here is novel architecture; the work is wiring plus a single new module (`app/src/open_changes/`) that owns the repo-state model and per-sub-phase view code. The risk profile is dominated by (a) parsing `git status` / `git diff` output reliably, and (b) coordinating refresh across file-system events, command-completion events, and direct user actions without flicker or stale rows.

Relevant files on master:

- `app/src/workspace/view/left_panel.rs:164-176` — `pub enum ToolPanelView { ProjectExplorer, GlobalSearch { ... }, WarpDrive, Shortcuts, ConversationListView }`. Add `OpenChanges` here, next to `Shortcuts`.
- `app/src/workspace/view/left_panel.rs:74-144` — `LeftPanelAction` enum (currently includes `Shortcuts`, `ShortcutsAddNew`, `ShortcutsOpenInEditor`, `ShortcutsToggleRowMenu`, `ShortcutsDelete`, `ShortcutsBeginEdit`, `ShortcutsEditSave`). New `OpenChanges*` variants follow the same precedent.
- `app/src/workspace/view/left_panel.rs:809-884` — `create_toolbelt_button_config()`. New entry for `ToolPanelView::OpenChanges` with the source-control glyph and the panel-badge count (PRODUCT §5).
- `app/src/workspace/view/left_panel.rs:1091-1124` — `render_active_tool_panel_view()`. Dispatch arm for `OpenChanges` calls into `open_changes::view::render(...)`.
- `app/src/workspace/view.rs:17257` — `compute_left_panel_views(ctx)`. Append `ToolPanelView::OpenChanges` immediately after `ToolPanelView::Shortcuts` (PRODUCT §1).
- `app/src/app_state.rs:889` — `LeftPanelDisplayedTab` enum + `From<ToolPanelView> for LeftPanelDisplayedTab` mapping. Add `OpenChanges` variant; update the reverse mapping in `workspace/view.rs:3380-3385`.
- `app/src/util/git.rs:10-76` — `run_git_command(repo_path: &Path, args: &[&str]) -> Result<String>` and `run_git_command_with_env(...)`. **All git operations in this feature route through these wrappers** — no direct `git2` use, no inline `Command::new("git")`. The wrappers already handle PATH propagation for git hooks and git-lfs (relevant for §29 push and §27 pre-commit hook failures).
- `app/src/terminal/event.rs:205-216` — `BlockCompletedEvent`. Emitted when a command block finishes.
- `app/src/terminal/event.rs:287-320` — `UserBlockCompleted { command: String, exit_code, ... }` — holds the resolved command string (post-alias-expansion). Drives PRODUCT §38 (`git`-prefix post-command refresh).
- `app/src/terminal/event.rs:41-46` — `AfterBlockStarted { block_id, command, is_for_in_band_command }` — could be used for an "operation in flight" indicator if needed; v1 only consumes `UserBlockCompleted`.
- `app/src/terminal/model_events.rs:40-62` — `ModelEventDispatcher` plus the subscriber registration shape (gpui-style `ctx.emit` + `subscribe`). Pattern reused by the panel's refresh coordinator.
- `crates/watcher/src/lib.rs:137-227` — `BulkFilesystemWatcher` plus `BulkFilesystemWatcherEvent { added, modified, deleted, moved }` (102-114). Underlying engine: `notify-debouncer-full` forked at `warpdotdev/notify`.
- `app/src/shortcuts/watcher.rs:1-100` — `ShortcutsWatcher` wrapping `BulkFilesystemWatcher` (200ms debounce, parent-dir scope with file filter). **Direct precedent** for the panel's watcher (different filter scope: whole working tree minus `.git/` except a small allowlist — PRODUCT §40).
- `app/src/workspace/action.rs:428-431` — `WorkspaceAction::OpenFileInNewTab { full_path, line_and_column }`. Dispatched via `ctx.dispatch_typed_action_deferred(...)`. Used by §15's `[Open full diff in tab]` and §24's `[Resolve…]`.
- `app/src/workspace/action.rs:366-370` — `WorkspaceAction::OpenFilePath { path }` (system default app) and `OpenInExplorer { path }`. Not used directly but listed for orientation.
- `app/src/code/diff_viewer.rs:15-29` — `DiffViewer` with `DisplayMode { FullPane, Embedded { ... }, InlineBanner { max_height, is_expanded, is_dismissed } }`. **Reused for PRODUCT §§12–17 (inline diff view).** `Embedded` is the right mode for the always-attached diff below the file lists; `FullPane` is used for `[Open full diff in tab]`.
- `app/src/code/editor/diff.rs` — editor-level diff handling (3-context-line standard). Implementation reference for hunk-header rendering and add/remove tinting.
- `app/src/code_review/` — full review UI with diff viewing across multiple files. Already implements many of the visual conventions the panel needs (status glyph styling, hunk-button hover affordances). Lift styling primitives where reasonable instead of reimplementing.
- `app/src/workspace/toast_stack.rs:10-92` — `ToastStack::add_ephemeral_toast(toast, window_id, ctx)` and `add_persistent_toast(...)`. Used for "Committed `<sha>`" (§27), "Pushed (now in sync)" (§29), and the undo-delete toast (§20).
- `app/src/view_components/dismissible_toast.rs` — `DismissibleToast::default(msg)` / `success(...)` / `error(...)` constructors.
- `app/src/terminal/view/inline_banner/mod.rs:66-150` — `InlineBannerStyle { CallToAction, Recommendation, LowPriority, VeryLowPriority }` and `InlineBannerContent { title, content, buttons }`. **Used for**: commit-failure banner (§27, `CallToAction` style with `[Copy error]` button), push/pull failure banner (§29), no-upstream tooltip (§29 via the menu item's disabled tooltip, not a banner), `<state> in progress` banner (§22, `Recommendation` style), large-diff truncation banner (§15, `LowPriority` style).
- `app/src/terminal/view/inline_banner/passive_code_diff.rs` — usage reference for the inline banner with action buttons.
- `app/src/workspace/view/left_panel.rs:1987-1990` — example `ctx.dispatch_typed_action_deferred(WorkspaceAction::OpenFileInNewTab { ... })` dispatch from the file-tree view. Direct pattern for the panel's open-in-tab calls.

## Sub-phase split

PRODUCT.md's behavior surface is too large to fit in one PR; STATUS.md pre-declared a five-PR split that aligns naturally to the module's data-flow layers. Each sub-PR adds exactly one layer on top of the previous, and the public API between layers is small enough to fix before any view code consumes it.

- **5a — Panel scaffold + working/staged file lists.** `ToolPanelView::OpenChanges`, no-repo state, with-repo state, repo header, two section lists with status glyphs and counts, file watcher, post-command refresh, manual refresh. **No diff view, no staging actions, no commit, no Timeline.** Covers PRODUCT §§1–11 (panel surface + file rows), §§38–40 (refresh hooks), §41 (errors-never-silent at the model layer), §42 (virtualization), §44 (themed visuals), §45 (telemetry boundary).
- **5b — Inline diff view.** Hook `DiffViewer` into the panel under the section lists; render unified diffs for the focused file; support binary / deleted / renamed / large-file modes; keyboard navigation across hunks; re-focus on refresh. Covers PRODUCT §§12–17. No interactive stage/unstage/discard yet; hover affordances on hunks are visually present but no-op (or omitted entirely — pick during impl) until 5c.
- **5c — Stage / unstage / discard at file and hunk granularity.** Hover-action cluster on rows (§9), confirmation flow for discard (§20), undo for untracked-file delete (§20), file-level + hunk-level apply through `git apply --cached` / `git apply --reverse`. Conflict-row `[Resolve…]` opens the file via `OpenFileInNewTab`. In-progress merge / rebase / cherry-pick state surfaces (§22). Idempotence and race handling (§23). Covers PRODUCT §§18–24.
- **5d — Commit message input + commit / push / pull / fetch.** Commit input above the lists (§25), Commit button gating (§26), Commit action (§27), Amend toggle (§28), Push / Pull / Fetch / Refresh in the overflow menu (§§29–30), error banner with `[Copy error]` on any git failure (§41). Covers PRODUCT §§25–31, §41.
- **5e — File Timeline.** Timeline section below the lists, paged `git log` output for the focused file, click → commit-diff replaces the inline diff with a read-only view, rename tracking via `--follow`, local-only `↑` marker. Covers PRODUCT §§32–37. **Scope-cut candidate** per STATUS.md: if surface area runs over, the feature can ship as `merged` after 5d; Timeline becomes a follow-up.

This split is durable: 5a's public API (`RepoState`, `FileEntry`, `Section`, `RefreshCoordinator`, `git::status`) is exactly what 5b consumes; 5b's public API (`Diff`, `Hunk`, `DiffViewerHandle`) is exactly what 5c consumes; 5c's public API (`git::stage`, `git::unstage`, `git::discard`, `git::apply_hunk`) is exactly what 5d consumes for "mid-merge commit" plus its own new `git::commit`, `git::push`, etc.; 5e's Timeline is read-only and depends only on `git::log_for_path` plus the existing `Diff` type.

## Proposed changes

### 1. New module `app/src/open_changes/`

```
app/src/open_changes/
├── mod.rs              // public API: load(ctx), OpenChangesModel
├── repo.rs             // RepoState, FileEntry, Section, RepoStatus enum, parsers
├── refresh.rs          // RefreshCoordinator: debounce + coalesce + dispatch
├── watcher.rs          // OpenChangesWatcher wrapping BulkFilesystemWatcher
├── git.rs              // thin wrappers over util/git.rs::run_git_command for our verbs
├── diff.rs             // Diff, Hunk, diff-parser, large-file truncation
├── timeline.rs         // TimelineEntry, log paging
├── view/
│   ├── mod.rs          // OpenChangesPanelView (top-level View)
│   ├── header.rs       // repo header + in-progress-op banner
│   ├── sections.rs     // Staged Changes + Changes lists, row rendering
│   ├── diff_view.rs    // inline DiffViewer host + Back-to-working-diff control
│   ├── commit.rs       // message input + Commit/Amend + overflow menu
│   └── timeline.rs     // Timeline section + entry rendering
└── open_changes_tests.rs
```

The `view/` submodule is added incrementally: 5a ships `mod.rs`, `header.rs`, `sections.rs`. 5b adds `diff_view.rs`. 5c expands `sections.rs` with hover actions. 5d adds `commit.rs`. 5e adds `timeline.rs`.

### 2. Data structures (5a)

```rust
// open_changes/repo.rs

pub struct RepoState {
    pub root: PathBuf,           // git top-level dir
    pub repo_name: String,       // basename(root)
    pub branch: BranchState,
    pub staged: Vec<FileEntry>,
    pub changes: Vec<FileEntry>,
    pub op_in_progress: Option<InProgressOp>,
    pub errors: Vec<String>,     // verbatim git stderr from the most recent failed op
}

pub enum BranchState {
    Branch { name: String, upstream: Option<UpstreamTracking> },
    Detached { short_sha: String },
}

pub struct UpstreamTracking {
    pub remote_branch: String,
    pub ahead: u32,
    pub behind: u32,
}

pub enum InProgressOp { Merging { from: String }, Rebasing { onto: String },
                       CherryPicking { sha: String }, Bisecting }

pub struct FileEntry {
    pub path: PathBuf,              // relative to repo root
    pub status: FileStatus,
    pub from_path: Option<PathBuf>, // populated for renames (R, C)
    pub is_submodule: bool,
}

pub enum FileStatus { Modified, Added, Deleted, Renamed, Copied, Unmerged, Untracked }

// open_changes/mod.rs

pub struct OpenChangesModel {
    pub state: Option<RepoState>,         // None ↔ no-repo state (PRODUCT §3)
    pub focused_file: Option<PathBuf>,    // most recently clicked row (relative path)
    pub diff_cache: HashMap<(PathBuf, DiffKind), Diff>,  // populated lazily by 5b
    pub message_draft: String,            // populated by 5d
    pub amend: bool,                      // populated by 5d
    pub timeline: Option<TimelineState>,  // populated by 5e
    pub refresh_in_flight: bool,
}
```

`OpenChangesModel` is a singleton model, same shape as `ShortcutsModel` (post-4d) and `ToastStack`. It's owned by the workspace; the panel view subscribes to it for re-renders.

### 3. Repo discovery and panel-repo tracking (5a, PRODUCT §2)

`repo::find_repo_root(start: &Path) -> Option<PathBuf>` walks up directories looking for `.git` (as a directory **or** a file — `.git` is a file for worktrees / submodules). Implementation: pure-Rust traversal, no shell-out; cheap to call on every focus change. Result is cached per-(window, pane) until the pane's cwd changes.

A subscriber in `OpenChangesPanelView::new` watches the focused-pane signal (existing in workspace; locate during impl — likely `Workspace::active_terminal_view_handle` or equivalent observer). On focus change:

1. Read the focused pane's cwd.
2. Compute the new repo root.
3. If different from the current panel repo (or transitioning to/from None):
   - Tear down the previous `OpenChangesWatcher` (§5).
   - Tear down the previous post-command subscription scope (§6).
   - Start a new watcher rooted at the new repo's working tree.
   - Subscribe to command-completion for the new repo.
   - Trigger an immediate refresh (§4).

Per PRODUCT §2 last sentence, a `cd` *inside* the focused pane (without focus changing) also moves the panel repo if the cwd resolves to a different repo. The cwd signal is emitted by the terminal model (locate during impl; likely the same source `UserBlockCompleted` already correlates with). Handle in the same subscriber.

### 4. Refresh coordinator (5a, PRODUCT §7)

```rust
// open_changes/refresh.rs
pub struct RefreshCoordinator {
    pending: bool,
    in_flight: bool,
    debounce_handle: Option<TaskHandle>,
}

impl RefreshCoordinator {
    pub fn schedule(&mut self, ctx: &mut ViewContext<OpenChangesPanelView>);
}
```

Debounce window: 250ms per PRODUCT §7. Coalescing: if `in_flight` is true when `schedule` is called, set `pending = true` and return; when the in-flight refresh completes, if `pending`, kick exactly one more refresh.

A refresh runs the following git commands sequentially against the panel repo (all via `util::git::run_git_command`):

1. `git rev-parse --show-toplevel` (sanity: confirms the repo still exists; cheap).
2. `git status --porcelain=v2 --branch --untracked-files=all --renames` — populates `branch`, `upstream`, `ahead/behind`, `staged`, `changes`.
3. Detect in-progress op by looking for sentinel files under `.git/`:
   - `MERGE_HEAD` → `Merging`. The "from" branch comes from `MERGE_MSG` or `ORIG_HEAD` (locate the canonical source during impl).
   - `rebase-apply/` or `rebase-merge/` → `Rebasing`. Onto branch from `rebase-merge/onto` or equivalent.
   - `CHERRY_PICK_HEAD` → `CherryPicking`.
   - `BISECT_LOG` → `Bisecting`.

All three reads run inside one `ctx.spawn` task; the task posts its result back via `ctx.notify` and the panel re-renders. Errors at this layer are stashed into `RepoState::errors` and surfaced as a banner via 5d's error-banner mechanism (5a degrades gracefully without the banner — just logs).

`git status --porcelain=v2` output parsing lives in `repo.rs::parse_porcelain_v2(&str) -> ParsedStatus`. v2 is line-oriented and stable; we own the parser (no third-party crate). Renames produce `2 ` records with both `from` and `to` paths. Unmerged files produce `u ` records. Untracked files produce `? ` records.

Unit tests in `open_changes_tests.rs` exercise: modified file, added file, deleted file, rename, copy, untracked, unmerged with various conflict markers, submodule with content changes, submodule with only commit-pointer changes, file with embedded newlines in the path (v2 escape format), 0-byte file, and the empty-repo case (no HEAD yet).

### 5. File watcher (5a, PRODUCT §40)

`open_changes/watcher.rs::OpenChangesWatcher` wraps `BulkFilesystemWatcher` (`crates/watcher/src/lib.rs:137-227`) following the precedent in `app/src/shortcuts/watcher.rs`. Configuration:

- Root: the panel repo's working-tree top-level directory.
- Debounce: 200ms (consistent with `ShortcutsWatcher`).
- Filter: include everything **except** paths under `.git/`, with allowlisted exceptions for `.git/HEAD`, `.git/index`, `.git/MERGE_HEAD`, `.git/REBASE_HEAD` (and `.git/rebase-merge/`), `.git/CHERRY_PICK_HEAD`, `.git/BISECT_LOG`. Allowlisting state-sentinel files lets the panel notice when the user runs `git merge` etc. in the terminal even without the post-command hook firing.
- Recursive: yes.

Every dispatched `BulkFilesystemWatcherEvent` calls `RefreshCoordinator::schedule(ctx)`. The watcher's own debouncing plus the coordinator's 250ms coalescing means a burst of FS events produces at most one git invocation per ~250ms.

Switching the panel repo tears down the previous watcher (drop the `BulkFilesystemWatcher` handle) and starts a new one. The drop signal is propagated through gpui's task cancellation, so dangling FS events from the previous repo never trigger refreshes for the new repo.

### 6. Post-command refresh (5a, PRODUCT §38)

Subscribe to `ModelEventDispatcher` (`app/src/terminal/model_events.rs:40-62`). On `UserBlockCompleted { command, .. }`:

1. Resolve the pane's cwd at command-finish time.
2. Check whether the cwd is inside the panel repo's working tree (string prefix match against `RepoState::root`).
3. Check whether `command` (after the alias expansion already done upstream of `UserBlockCompleted`) begins with `git ` followed by a non-`-` token, or is exactly `git`. Exclude `git-lfs` and any other `git-` hyphenated form (PRODUCT §38).
4. If both checks pass, `RefreshCoordinator::schedule(ctx)`.

Subscription scope: the subscription is held by `OpenChangesPanelView`, so it lifetime-matches the panel. It does **not** require the panel to be visible — the model updates so the toolbelt badge stays current.

### 7. Toolbelt badge (5a, PRODUCT §5)

`create_toolbelt_button_config()` (`app/src/workspace/view/left_panel.rs:809-884`) gains a new arm for `ToolPanelView::OpenChanges`:

```rust
ToolPanelView::OpenChanges => ToolbeltButtonConfig {
    icon: Icon::SourceControl, // add the glyph to the icon enum
    tooltip: "Open Changes",
    action: LeftPanelAction::OpenChanges,
    badge: open_changes_model
        .read(ctx, |m, _| m.state.as_ref().map(|s| s.unique_change_count()))
        .filter(|n| *n > 0),
}
```

`unique_change_count(&self) -> usize` returns the count of distinct paths across `staged` and `changes` (a single file appearing in both, as happens with partial staging, counts once). The badge is hidden when count is 0 or the panel is in no-repo state.

### 8. Section rendering and row hover (5a, plus 5c extensions)

`view/sections.rs` renders two collapsible sections. Each row uses `gpui` hover styling to reveal the action cluster. 5a renders the cluster as a no-op placeholder (or omits it — pick the cleaner option during impl); 5c fills it in:

- Changes section: row shows `[+] [↺]` on hover.
- Staged Changes section: row shows `[−]` on hover.
- Conflict (`U`) row in either section: shows `[Resolve…]`.

Row click handler dispatches `LeftPanelAction::OpenChangesFocusFile { section, path }`. The handler updates `OpenChangesModel.focused_file` and, in 5b+, kicks off diff loading.

Status glyph rendering uses an enum-to-(char, color) match. Colors come from twarp's theme system (PRODUCT §44) — locate the theme accessors used by `code_review/` and reuse them.

Long paths (PRODUCT §8) truncate the directory portion from the left using a gpui text-truncation helper. Locate the helper used by tab titles during impl; if none exists at the right granularity, write one in `open_changes/view/util.rs`.

Row sort (PRODUCT §10) is stable lexicographic on the destination path (case-insensitive). Implementation: `Vec::sort_by_cached_key(|f| f.path.to_string_lossy().to_lowercase())`.

Virtualization (PRODUCT §42) kicks in when total visible row count exceeds 200. Use whatever gpui-side virtualization the existing tool panels use (Project Explorer is the canonical long-list view; reuse its scaffolding).

### 9. Inline diff view (5b, PRODUCT §§12–17)

`view/diff_view.rs` hosts a `DiffViewer` instance (`app/src/code/diff_viewer.rs:15-29`) in `DisplayMode::Embedded` mode below the section lists.

Diff loading is lazy and cached:

```rust
// open_changes/diff.rs

pub enum DiffKind { Working, Index, Commit(GitSha) }

pub struct Diff {
    pub path: PathBuf,
    pub kind: DiffKind,
    pub mode: DiffMode,
    pub hunks: Vec<Hunk>,
    pub truncated: Option<TruncationInfo>,
}

pub enum DiffMode { Text, Binary, PureRename { from: PathBuf, to: PathBuf },
                    Deleted, Untracked }

pub struct Hunk {
    pub old_start: u32, pub old_count: u32,
    pub new_start: u32, pub new_count: u32,
    pub function_context: Option<String>,
    pub lines: Vec<HunkLine>,
}

pub enum HunkLine { Context(String), Added(String), Removed(String) }

pub struct TruncationInfo { pub total_lines: usize, pub shown: usize }
```

When a file is focused (5b path), the panel kicks off:

```
match (kind, file.status) {
    (Working, Untracked) => synth_diff_from_full_content(),
    (Working, Deleted)   => git diff -- <path>          (all removed lines)
    (Working, _)         => git diff -- <path>,
    (Index, _)           => git diff --cached -- <path>,
    (Commit(sha), _)     => git show <sha> -- <path>,
}
```

Output is parsed by `diff.rs::parse_unified_diff(&str) -> Result<Diff>`. The parser handles standard unified-diff with hunk headers, `+`/`-`/` ` line prefixes, the `@@ -a,b +c,d @@ context` form, the `Binary files differ` form, `new file mode`/`deleted file mode`/`rename from`/`rename to` headers.

**Large-file truncation (PRODUCT §15):** the parser counts lines as it goes. At 50,000 lines or 5 MB, it stops appending and sets `truncated`. The view renders the `[Open full diff in tab]` link which dispatches `WorkspaceAction::OpenFileInNewTab` for a *synthesized* diff file (the full `git diff` output written to a temp file) — there is no twarp surface for "open this string in a new tab" today, but `OpenFileInNewTab` works for any path. Alternative: extend `WorkspaceAction` with a sibling `OpenContentInNewTab { name, content }` action. Pick during impl; recommend the path-based approach for v1 since it reuses an existing dispatch.

Hunk-header rendering (PRODUCT §14) is implemented by the existing `DiffViewer` for its `Embedded` mode; verify the hover-button slot is exposed for our use. If not, the cleanest path is a small wrapper component above `DiffViewer` that overlays the buttons absolutely-positioned per hunk. Note this during impl; the `DisplayMode::Embedded` shape was designed for this kind of host.

Re-focus on refresh (PRODUCT §17): on refresh completion, if `focused_file` is no longer in `staged` or `changes`, clear `focused_file` and collapse the diff view. Timeline retains its previous focus until a new row is clicked.

### 10. Stage / unstage / discard (5c, PRODUCT §§18–24)

Each verb is one `git` invocation through `util::git::run_git_command`:

| Verb | File-level | Hunk-level |
|---|---|---|
| Stage | `git add -- <path>` (or `git add -A -- <path>` for renames) | `git apply --cached <patch>` where `<patch>` is the synthesized one-hunk patch |
| Unstage | `git restore --staged -- <path>` (or `git rm --cached -- <path>` for initial commit when no HEAD) | `git apply --cached --reverse <patch>` |
| Discard (modified) | `git restore -- <path>` | `git apply --reverse <patch>` |
| Discard (untracked) | `std::fs::remove_file(<path>)` + capture content for undo | n/a |
| Discard (deleted, staged-as-deleted no, working-only) | `git restore -- <path>` | n/a |

Hunk patch synthesis lives in `diff.rs::hunk_to_patch(hunk, full_path) -> String`. Output is a valid `git apply`-compatible patch with the file header (`--- a/<path>` / `+++ b/<path>`) and one `@@` hunk. The patch is **not** sent through `git diff` round-trip — it's built directly from our `Hunk` struct so the lines we apply are exactly the lines we render.

Idempotence (PRODUCT §23): we don't pre-check whether the operation is a no-op. If `git apply` exits non-zero with a "patch does not apply" or "already applied" message, classify the error: "already applied" → silent success + refresh; "does not apply" → refresh + retry once with the freshly-parsed diff; second failure → toast `<op> no longer applies — refreshed.` and bail.

Discard confirmation (PRODUCT §20) is a gpui-rendered inline panel below the row. Pattern reference: 4d's right-click delete affordance in the Shortcuts panel (`app/src/workspace/view/left_panel.rs` shortcuts row code, post-4d). Same dismiss-on-outside-click behavior.

Undo for untracked-file delete: before deleting, read the file's bytes into a `Vec<u8>` and capture its mtime. Schedule a 10-second toast with `[Undo]`. On Undo, write the bytes back and `filetime::set_file_mtime` (already a dependency, verify during impl). Capture the path as a `PathBuf` so a rapid-fire delete + undo + delete cycle works.

In-progress-op banner (PRODUCT §22): rendered in `view/header.rs` below the repo header. Uses `InlineBannerStyle::Recommendation`. Commit-button label change is handled in 5d's `view/commit.rs` by reading `RepoState.op_in_progress`.

Conflict `[Resolve…]` dispatches `WorkspaceAction::OpenFileInNewTab { full_path: repo.root.join(&entry.path), line_and_column: None }`.

### 11. Commit / push / pull / fetch (5d, PRODUCT §§25–31)

Commit message input (`view/commit.rs::CommitMessageInput`) is a multi-line `Editor`-style component (locate the existing multi-line text input — used by the AI prompt input pre-removal? checked: probably `ClickableTextInput` extended to multi-line, or the editor handle from `code/editor/` for plain-text). Picks a single-component during impl. Updates `OpenChangesModel.message_draft` on every keystroke.

Commit gating (PRODUCT §26) is a pure function over `RepoState` and the input string:

```rust
fn can_commit(state: &RepoState, message: &str, amend: bool) -> CommitGate {
    let has_message = !message.trim().is_empty();
    let has_staged = !state.staged.is_empty();
    let merge_with_resolved_conflicts = state.op_in_progress.is_some()
        && state.staged.iter().chain(state.changes.iter())
            .all(|f| f.status != FileStatus::Unmerged);
    // ... per PRODUCT §26
}
```

Commit action (PRODUCT §27):

```rust
pub async fn commit(repo: &Path, message: &str, amend: bool) -> Result<GitSha, GitError> {
    let mut args = vec!["commit", "-F", "-"];
    if amend { args.push("--amend"); }
    let stdout = util::git::run_git_command_with_stdin(repo, &args, message.as_bytes()).await?;
    parse_sha_from_commit_output(&stdout)
}
```

`run_git_command_with_stdin` is a new helper alongside `run_git_command` (`app/src/util/git.rs`). Add it in 5d; it mirrors `run_git_command_with_env` but writes stdin. The current `run_git_command` impl pipes stdio via `command::r#async::Command` — adding stdin is a one-line addition.

On commit success: `ToastStack::add_ephemeral_toast(DismissibleToast::success("Committed <sha>: <subject>"), ...)` with a `[View]` button. Clicking `[View]` updates `OpenChangesModel.timeline.focused_sha = Some(sha)` (5e wires the timeline scroll-to).

On commit failure: capture stderr verbatim, push onto `RepoState.errors`, render `InlineBannerStyle::CallToAction` banner above the commit input with `[Copy error]` button. The banner persists until the user dismisses it or commits successfully.

Amend toggle (PRODUCT §28): when toggled on, fetch `git log -1 --format=%B` for the previous-commit message. If `message_draft` is non-empty, show the inline confirmation; otherwise prefill directly. Toggle-off restores the displaced draft from a saved-aside field on the model.

Push / Pull / Fetch (PRODUCT §§29–30):

- `git push` (no args — uses upstream).
- `git pull` (no args).
- `git fetch` (no args).
- "No upstream" detection: `git rev-parse --abbrev-ref @{u}` returns non-zero. Computed during refresh (§4) and cached on `BranchState::Branch::upstream`. Used to disable Push/Pull in the overflow menu.

Each operation shows a transient `Pushing…` / `Pulling…` / `Fetching…` indicator in the repo header (`view/header.rs` reads a transient `RepoState::active_op` field). On completion, toast or banner per PRODUCT §29. Refresh is automatically triggered by the file watcher (`.git/refs/...` changes) and post-command hook (the git command we just ran fires `UserBlockCompleted` — but wait, we shelled out through `run_git_command`, not through a pane, so `UserBlockCompleted` does **not** fire). Therefore the commit / push / pull / fetch handlers must call `RefreshCoordinator::schedule(ctx)` themselves on completion.

Overflow menu lives next to the Commit button. Use the existing menu primitive that powers other panel overflows (Project Explorer's per-row menu is the closest precedent; locate during impl).

### 12. File Timeline (5e, PRODUCT §§32–37)

`view/timeline.rs` renders below the diff view.

```rust
// open_changes/timeline.rs

pub struct TimelineState {
    pub path: PathBuf,
    pub entries: Vec<TimelineEntry>,
    pub next_page_token: Option<NextPage>,   // pagination cursor
    pub focused_sha: Option<GitSha>,         // active commit in the diff
    pub loading: bool,
}

pub struct TimelineEntry {
    pub sha: GitSha,
    pub short_sha: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: SystemTime,
    pub subject: String,
    pub is_rename_commit: bool,
    pub original_path_at_commit: Option<PathBuf>,
    pub is_local_only: bool,
}
```

Log fetching: `git log --follow --name-status --format=%H%x00%an%x00%ae%x00%at%x00%s -n <limit> --skip <offset> -- <path>`. The `--follow` flag handles rename tracking (PRODUCT §36). The `--name-status` line that includes `R<percent>` flags the rename commit. Parser lives in `timeline.rs::parse_log(&str) -> Vec<TimelineEntry>`.

Local-only detection (PRODUCT §37): one extra command on Timeline first-page load: `git log <upstream>..HEAD --format=%H -- <path>`. Result is a `HashSet<GitSha>`; entries with SHAs in the set get `is_local_only = true`. Re-run on refresh.

Paging: first load is `--skip 0 -n 20`; `[Load more]` increments `offset += 20` and concats results. Cursor invalidation: if a refresh detects new commits at `HEAD`, Timeline reloads from `offset 0` and discards the existing entries.

Click → commit-diff (PRODUCT §35): clicking a Timeline entry sets `OpenChangesModel.focused_file_diff_override = Some((path, Commit(sha)))`. The diff view's render logic prefers the override over the working-tree diff. `[Back to working diff]` clears the override.

Rename badge: rendered on entries with `is_rename_commit == true`. Tooltip on rows with `original_path_at_commit.is_some()` shows the original path.

Avatar circle: single-letter, deterministic color from `hash(author_email) % palette_size`. Palette is themed; reuse the avatar logic from `code_review/` if it exists, otherwise a small new helper in `view/util.rs`.

## Testing and validation

| PRODUCT § | Verification | Phase |
|-----------|--------------|-------|
| §1 (panel location) | View test: `compute_left_panel_views(ctx)` returns `[..., Shortcuts, OpenChanges, ...]`. Smoke prelude. | 5a |
| §2 (panel repo) | Repo-discovery unit: `find_repo_root` walks up; recognizes `.git/` dir, `.git` file (worktree), absent (`/tmp/x`). View integration test: focus-pane change triggers panel-repo swap. Smoke step 6. | 5a |
| §3 (no-repo state) | View unit: model with `state: None` renders the centered message; no other controls. Smoke step 1. | 5a |
| §4 (layout) | View snapshot test: with-repo state renders header → commit area placeholder → Staged Changes → Changes → Timeline placeholder in order. (Commit / Timeline are stubs in 5a, real in 5d / 5e.) | 5a / 5d / 5e |
| §5 (counts and badges) | Model unit: `unique_change_count()` dedupes a path that appears in both sections. View test: badge present iff count > 0. Smoke step 33. | 5a |
| §6 (clean tree empty state) | View unit: empty `staged` + empty `changes` renders `Working tree is clean.` hint; Commit button disabled. Smoke step 2. | 5a / 5d |
| §7 (refresh semantics) | Refresh coordinator unit: a burst of 10 `schedule()` calls produces 2 refreshes (1 immediate, 1 coalesced). | 5a |
| §8 (row content) | Parser unit: porcelain v2 lines for M/A/D/R/C/U/? each produce the expected `FileEntry`. View unit: status glyph for each status renders the right char + color. | 5a |
| §9 (hover actions) | View unit: hovering a Changes row reveals `[+] [↺]`; hovering a Staged Changes row reveals `[−]`; hovering a `U` row reveals only `[Resolve…]`. Smoke step 12 (and §17 for conflict row). | 5c |
| §10 (sort order) | Parser unit: sort is stable, lexicographic case-insensitive, rename uses destination. Smoke step 8. | 5a |
| §11 (row click → diff) | View integration: click row → `focused_file` updates; second click on focused row toggles diff collapsed; click on different row replaces. Smoke steps 7–8. | 5b |
| §12 (diff placement) | View snapshot: diff renders inline below the file list, both sections remain visible above. Smoke step 7. | 5b |
| §13 (diff format) | Diff parser unit: hunk headers, +/-/space lines, whitespace marker toggle. Smoke step 7. | 5b |
| §14 (hunk headers + buttons) | View unit: hunk header rendered with `@@ … @@` + function context; hover reveals `[+] [↺]` (Changes) or `[−]` (Staged). Smoke step 16. | 5b / 5c |
| §15 (binary/deleted/rename/large/untracked) | Diff parser unit: each `DiffMode` variant matches expected output for synthetic inputs. Smoke steps 10 (large), 11 (binary). | 5b |
| §16 (diff keyboard nav) | View unit: focus diff view, `j`/`k` move between hunks; `Enter` on a hunk header triggers stage; `s`/`d` shortcuts work. | 5b / 5c |
| §17 (re-focus on refresh) | Model unit: refresh that removes focused-file's entries clears `focused_file`; Timeline keeps its scope. | 5b |
| §18–§20 (stage/unstage/discard) | Git wrapper unit: each verb invokes the documented `git` args. Integration test: scratch repo + Workspace harness performs stage→commit→push end-to-end. Smoke steps 12–15. | 5c |
| §21 (hunk-level apply) | Patch-synthesis unit: `hunk_to_patch` round-trips through `git apply --check`. Smoke step 16. | 5c |
| §22 (in-progress op) | Sentinel detector unit: synthesize a `.git/MERGE_HEAD` etc. and assert detection; Commit button label changes appropriately. Smoke step 17. | 5c / 5d |
| §23 (idempotence and races) | Integration test: stage a file then click Stage again — no error, refresh fires once. Simulate "patch does not apply" by mutating the working tree mid-click. | 5c |
| §24 (resolve conflict row) | View unit: `[Resolve…]` dispatches `OpenFileInNewTab` with the full path. Smoke step 17. | 5c |
| §25 (commit message input) | View unit: input grows up to 8 lines then scrolls; character-count indicator appears when first line > 72. Smoke step 19. | 5d |
| §26 (commit gating) | `can_commit` pure-function unit: all eight gating combinations (message empty/not, staged empty/not, in-merge yes/no with conflicts yes/no). | 5d |
| §27 (commit action) | Integration test: commit happy path produces a new SHA, clears input, fires toast. Failure: pre-commit hook exits non-zero → banner with stderr, input retained. Smoke steps 18, 24. | 5d |
| §28 (amend) | Integration test: amend rewrites prior commit; amend prompt appears on non-empty draft. Smoke steps 20–21. | 5d |
| §29 (push) | Integration test: push happy path against a local bare-repo upstream; no-upstream tooltip case. Smoke steps 22–23. | 5d |
| §30 (pull/fetch/refresh) | Integration test: pull / fetch happy paths; manual refresh invocation. | 5d |
| §31 (no background mutation) | Audit test: grep `app/src/open_changes/` for `run_git_command` calls; confirm every call site is reachable only from a user-clicked handler or a read-only refresh path. | 5c / 5d |
| §32 (Timeline scope) | View unit: focused file → Timeline header shows `Timeline · <basename>`; no focus → centered hint. Smoke step 25. | 5e |
| §33 (Timeline entries) | Log parser unit: `git log --format` output produces correct fields including author/email/relative-time/subject. | 5e |
| §34 (paging) | View integration: `[Load more]` appends without scroll jump. Smoke step 28. | 5e |
| §35 (click → commit diff) | View integration: clicking Timeline entry replaces diff; `[Back to working diff]` restores. Smoke steps 26–27. | 5e |
| §36 (rename tracking) | Integration: `git mv` + Timeline → rename badge + tooltip on prior entries. Smoke step 29. | 5e |
| §37 (local-only marker) | Integration: local commit shows `↑`; push removes it. Smoke step 30. | 5e |
| §38 (post-command refresh) | Integration: run `git checkout -b foo` in a pane → `UserBlockCompleted` triggers refresh; `git-lfs ...` does not. Smoke step 31. | 5a |
| §39 (multi-pane racing) | Integration: open discard confirmation, fire a refresh → confirmation closes silently; commit input preserved. Smoke step 32. | 5a / 5c |
| §40 (watcher scope) | Watcher unit: events under `.git/objects/` are ignored; events on `.git/HEAD` and `.git/MERGE_HEAD` trigger refresh. | 5a |
| §41 (errors never silent) | Audit: every `run_git_command` error path either fills `RepoState.errors` or surfaces a banner. Integration test: corrupt `index` file → banner. | 5a / 5c / 5d |
| §42 (performance ceiling) | Bench test (manual): synthesize 10,000-file diff; assert panel scroll stays smooth (virtualization active). Diff bench: 10k-line file diff renders in < 200ms. | 5a / 5b |
| §43 (keyboard reachable) | View integration: Tab traverses commit input → Commit → Amend → overflow → headers → rows. Per-row keyboard shortcuts (`s`/`d`/`o`) fire. | 5c / 5d |
| §44 (themed visuals) | Audit: grep for `Color::rgb(` in `open_changes/view/`; expect zero hits. All colors come from theme tokens. | 5a–5e |
| §45 (telemetry) | Audit: no new telemetry events emitted from `open_changes/`. Existing terminal-side git-command events fire when commands hit a pane (not when we shell out — that's intentional). | 5a–5e |

New test files:

- 5a: `app/src/open_changes/open_changes_tests.rs` (porcelain v2 parser, repo discovery, refresh coordinator, watcher filter). Smoke prelude in PRODUCT §Smoke test ("5a" group) is the manual gate.
- 5b: extends `open_changes_tests.rs` with diff parser tests; adds `app/src/open_changes/view/diff_view_tests.rs` for the inline diff host.
- 5c: integration test `app/tests/open_changes_stage_commit_integration.rs` against a scratch repo built with `tempfile`. Patch-synthesis unit tests in `open_changes_tests.rs`.
- 5d: extends integration test to cover commit / amend / push / pull / fetch using a local bare-repo upstream.
- 5e: extends integration test with `git log` paging + rename tracking + local-only detection.

`./script/presubmit` must be green before opening each impl PR. The integration tests should be fast enough to run in presubmit (subsecond per case using `tempfile` + small fixtures); if they prove too slow, gate them behind a separate `cargo test --features open-changes-integration` invocation that CI runs separately, and keep presubmit limited to the unit tests.

## Risks and mitigations

- **Risk: `git status --porcelain=v2` output differs across git versions or is locale-sensitive.** Mitigation: parser asserts on the well-documented v2 grammar; integration tests run with `LC_ALL=C`. Fall back to `--porcelain=v1` only if a user reports a v2-parse failure; not worth pre-implementing.
- **Risk: `BulkFilesystemWatcher` floods on large repos (think `node_modules`).** Mitigation: scope the watcher to the repo working tree but rely on git itself for the ignored-files filtering (i.e. ignore the FS event but let the refresh decide what to surface). The 250ms refresh coalescing means even a 10k-event flood produces one git invocation.
- **Risk: Hunk-level `git apply --cached` fails because the working tree has drifted since render.** Mitigation: §23 — classify the error, refresh, retry once, then surface a clear "no longer applies" toast.
- **Risk: Commit-failure detection mistakes a non-zero exit code as failure when git wrote the commit and only the hook errored after.** Mitigation: parse git's stdout for `[<branch> <sha>] <subject>` first. If a SHA is present, treat as success-with-warning (surface the warning as a non-blocking banner). If absent, treat as failure and retain the message.
- **Risk: Commit message input loses focus on every refresh.** Mitigation: §39 — the refresh handler must never write to `OpenChangesModel.message_draft` or transition focus; assert with an integration test that types into the input while a refresh fires.
- **Risk: Discard-untracked-undo races a subsequent refresh that "sees" the deleted file as gone, then sees it re-appear.** Mitigation: Undo writes the file back via the same code path that the user's editor would; the subsequent refresh just re-discovers it. Confirm in integration: discard → undo → no toast, no banner, no flicker.
- **Risk: Renames are double-counted (one entry in Changes for the destination, one in Staged Changes for the source).** Mitigation: porcelain v2's `2 ` records carry both paths; the parser emits one `FileEntry` keyed by destination, with `from_path` populated.
- **Risk: Submodules with uncommitted changes inside them cause `git status` recursion.** Mitigation: parse the porcelain "submodule modified" indicator and render a single submodule row per PRODUCT §non-goals; never descend.
- **Risk: `--follow` interacts badly with paging.** Mitigation: paging cursor uses `--skip` rather than a SHA cursor; `--follow` is re-applied for each page and re-discovers history from HEAD. Slightly redundant work, but trivially correct.
- **Risk: `git log %x00`-delimited format breaks on author names containing NULs.** Mitigation: NULs in author fields are essentially never seen; parser asserts and falls back to a per-line split if the assertion fails (logs the offender). Document as a known fragility.
- **Risk: 5e's scope blows up.** Mitigation: STATUS.md pre-authorizes shipping `merged` without 5e. If 5d ships and 5e starts looking like its own multi-week effort, demote to follow-up.
- **Risk: Path encoding on Windows (the fork's primary target is macOS/Linux but parity matters).** Mitigation: paths flow as `PathBuf` everywhere; the porcelain v2 parser handles the quoted-path escape format for non-UTF-8 paths. Punt Windows-specific path-display issues to a follow-up if reported.
- **Risk: Commit-button `Continue rebase` semantics differ from a plain commit (no message input needed, may invoke `--continue` instead of `commit`).** Mitigation: PRODUCT §22 already specifies the variant labels; the gating function returns the variant alongside the bool so the handler dispatches the right git command.

## Follow-ups

- **Branch picker** in the repo header (out of scope per PRODUCT non-goals).
- **3-way merge UI** for conflict resolution (PRODUCT explicitly defers to external editor in v1).
- **Stash management** surface.
- **Persist commit-message drafts across restarts.**
- **LFS-aware diffs** (currently shows pointer file content; v1 doesn't fetch the blob).
- **GitHub / forge integration** (e.g. "create PR from this branch").
- **Multi-repo panel view** (currently scopes to one repo).
- **Submodule navigation** (currently single submodule row, no descent).
- **Line-level (sub-hunk) staging** — v1 is hunk-level only.
- **Inline conflict resolution UI** if the external-editor handoff proves friction.
- **Custom diff context lines** (3 hard-coded; could be user-configurable).
- **Per-pane / per-tab repo override** for users who want the panel to track a fixed repo regardless of focused-pane cwd.
- **Sticky branch tooltip** with full ahead/behind detail and a `[Sync]` action.

## Parallelization

The five sub-phases ship as five sequential PRs (5a → 5b → 5c → 5d → 5e) and are **not parallelizable**: each layer's public API is the next layer's consumer surface, and the realized API drifts during impl in ways that would cause merge churn if developed concurrently. Specifically:

- 5b consumes 5a's `RepoState`, `FileEntry`, `RefreshCoordinator`. The exact shape of these settles only after 5a ships.
- 5c consumes 5b's `Diff`, `Hunk`, and the diff-view focus model. Hunk-button placement depends on whether `DiffViewer::Embedded` exposes a hunk-level overlay slot (discovered during 5b).
- 5d depends on 5c's stage/unstage paths to mid-merge-resolve before commit can fire.
- 5e is the only sub-phase that *could* be partially parallelized with 5d (Timeline is read-only and orthogonal to commit). Not worth the coordination overhead for one-engineer twarp.

Within each sub-phase, the work is small enough (one new module, one new view) that splitting across agents would just create merge churn. Single-agent linear development is the right call.

If future scope adds independent diff backends (e.g. a virtual-FS diff for an "Open Changes" surface that doesn't require an on-disk repo), that becomes a per-backend parallelization opportunity. Out of scope for v1.
