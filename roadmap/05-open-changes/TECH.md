---
name: 05 — Open Changes panel
status: rework
supersedes: roadmap/05-open-changes/TECH.md@PR#56
---

# Open Changes panel — TECH

Companion to [PRODUCT.md](PRODUCT.md). Section numbers below refer to PRODUCT.md.

## Context

The first revision of this spec proposed a brand-new left-side panel module (`app/src/open_changes/`) with its own file watcher, refresh coordinator, view layer, and tool-panel integration. That was the wrong call: the existing right-side **Code Review panel** in `app/src/code_review/` already provides the file list, inline diff rendering, refresh hooks, and commit/push UI — the gap was just sidebar layout (flat list vs. staged/unstaged split), hunk-level affordances, in-progress-op handling, and Timeline.

This tech spec layers the rework directly into `app/src/code_review/`. No new module. No new file watcher. No new tool-panel variant.

Relevant files on master:

- `app/src/workspace/view.rs:14740-14867` — `render_right_panel_button`: Diff icon button in the workspace header, dispatches `WorkspaceAction::ToggleRightPanel`. Unchanged.
- `app/src/workspace/view/right_panel.rs:598-676` — `RightPanelView` host for the panel content. Unchanged.
- `app/src/code_review/code_review_view.rs:4795-4865` — `render_file_sidebar`: today's flat file list. **Primary rework target.**
- `app/src/code_review/code_review_view.rs:4871-4950` — `render_file_sidebar_row`: per-row rendering with file name, dir, +/- counts. Extended with status glyph; rendering moves into per-section helpers.
- `app/src/code_review/code_review_view.rs:7317-7346` — `CodeReviewAction::FileSelected` handler: expands the file inline below the sidebar via `CodeReviewEditorView`. Unchanged — `FileSelected` continues to be the click target from both new sections.
- `app/src/code_review/code_review_view.rs:5429` — `render_file_content`: inline diff rendering host. Unchanged.
- `app/src/code_review/diff_state.rs:366-386` — `DiffStateModel`: the model the sidebar reads from. **Primary data-model target.**
- `app/src/code_review/diff_state.rs:456` — `LoadedState`: holds `file_states: IndexMap<PathBuf, FileState>`. **Extended** with `staged: Vec<FileEntry>` and `changes: Vec<FileEntry>` derived from the same `git status --porcelain=v2` call the model already makes.
- `app/src/code_review/diff_state.rs:1085-1175` — `Repository` subscription + `DiffStateRepositoryUpdate::Invalidation` channel. **Reused** as the refresh trigger; no new watcher.
- `app/src/code_review/diff_state.rs:1876` — existing `git status --porcelain=2` call. Extended to surface the staged/unstaged columns separately into the new `LoadedState` fields.
- `app/src/code_review/code_review_header/mod.rs:52-89` — header with commit / push / PR buttons, gated by `FeatureFlag::GitOperationsInCodeReview`. **Unchanged**; PRODUCT §15–§17 inherit this verbatim.
- `app/src/code_review/code_review_view.rs:326-341` — `PrimaryGitActionMode` enum driving the header button label. **Extended** in 5c to add the `Continue rebase` / `Conclude merge` / `Continue cherry-pick` variants gated on `InProgressOp`.
- `app/src/code_review/telemetry_event.rs` — `CodeReviewTelemetryEvent`. **Reused**; any new events for the staged/unstaged split or Timeline land here.

Files **not** touched by this rework (and which the prior revision incorrectly proposed creating or modifying):

- ~`app/src/open_changes/`~ — module never lands. Parser carries over into `app/src/code_review/porcelain_v2.rs`; everything else (`view/`, `watcher.rs`, `refresh.rs`, etc.) is dropped.
- ~`ToolPanelView::OpenChanges`~ in `app/src/workspace/view/left_panel.rs` — not added.
- ~`LeftPanelDisplayedTab::OpenChanges`~ in `app/src/app_state.rs` — not added.
- ~`compute_left_panel_views` entry~ — not added.
- ~`BulkFilesystemWatcher` wrapper~ — not needed; the existing `Repository` subscription is the refresh trigger.

## Sub-phase split (revised)

The original 5a–5e split assumed a from-scratch build. With the rework framing, the gap is smaller and naturally collapses to **four** sub-PRs, in order:

- **5a (revised) — Sidebar split.** `LoadedState` gains `staged` / `changes` buckets populated from `git status --porcelain=v2`. `render_file_sidebar` splits into two collapsible sections with status glyphs + count headers. Click → `FileSelected` continues to work (no changes to the expansion logic). Covers PRODUCT §§1–9, §§24–25, §27, §29 (verify), §30 (verify).
- **5b — Hunk-level staging affordances.** Hover-revealed `[+] [↺]` / `[−]` on hunk headers inside the expanded inline diff. Patch synthesis for `git apply --cached` / `--cached --reverse` / `--reverse`. Idempotence + race retry per PRODUCT §14. Covers PRODUCT §12.
- **5c — In-progress op banner + discard / unstage polish.** `InProgressOp` detection from `.git/MERGE_HEAD` / `rebase-merge` / `CHERRY_PICK_HEAD` / `BISECT_LOG`. Banner above the sidebar. Conflict-row `[Resolve…]`. Commit-button label gating via extended `PrimaryGitActionMode`. File-level discard/unstage hover affordances (file-level `[+]` / `[−]` cluster, inline discard confirmation, untracked-file undo toast). Covers PRODUCT §§10–11, §13–§14.
- **5d — File Timeline.** New `code_review/timeline.rs`. Paged `git log --follow` for the focused file. New Timeline section below the inline diff. Click → commit-diff replaces inline diff for that file; `[Back to working diff]` restores. Rename badge + `↑` local-only marker. Covers PRODUCT §§18–23.

5e from the prior split (commit / push / pull) is **dropped**: PRODUCT §§15–17 inherit the existing implementation behind `FeatureFlag::GitOperationsInCodeReview`. If verification turns up real gaps, those become follow-ups, not blockers.

## Proposed changes

### 1. Carry the v2 parser into code_review/

The 27-test porcelain v2 parser from the prior 5a (`open_changes/repo.rs::parse_porcelain_v2`) is the only piece of that work worth saving. It moves to `app/src/code_review/porcelain_v2.rs` with:

```rust
// app/src/code_review/porcelain_v2.rs

pub struct ParsedStatus {
    pub branch: BranchState,
    pub staged: Vec<FileEntry>,
    pub changes: Vec<FileEntry>,
}

pub struct FileEntry {
    pub path: PathBuf,
    pub status: FileStatus,
    pub from_path: Option<PathBuf>,   // rename source
    pub is_submodule: bool,
}

pub enum FileStatus { Modified, Added, Deleted, Renamed, Copied, Unmerged, Untracked }

pub enum BranchState {
    Branch { name: String, upstream: Option<UpstreamTracking> },
    Detached { short_sha: String },
}

pub fn parse_porcelain_v2(text: &str) -> ParsedStatus;
```

Unit tests carry over verbatim. The `InProgressOp::detect` helper (`.git/MERGE_HEAD` sniff) moves alongside.

`find_repo_root` from the prior revision is **not** carried — the existing `Repository` model already resolves the repo root.

### 2. Extend `DiffStateModel::LoadedState`

```rust
// app/src/code_review/diff_state.rs (additions to LoadedState near :456)

pub struct LoadedState {
    pub file_states: IndexMap<PathBuf, FileState>,    // existing
    pub total_additions: usize,                       // existing
    pub total_deletions: usize,                       // existing
    pub files_changed: usize,                         // existing

    // 5a additions:
    pub staged: Vec<porcelain_v2::FileEntry>,
    pub changes: Vec<porcelain_v2::FileEntry>,
    pub in_progress_op: Option<porcelain_v2::InProgressOp>,
}
```

The status fetch around `diff_state.rs:1876` already runs `git status --porcelain=2` and parses it into `file_states`. Augment that path to also run `parse_porcelain_v2` on the same output and populate `staged` / `changes`. Cost: one extra parse pass over the same string. Don't add a second git invocation.

`in_progress_op` is set by reading `.git/`-sentinel files via `InProgressOp::detect`. The `Repository` model already exposes the git directory path; if not, a one-line `repo_root.join(".git")` walk does it.

### 3. Sidebar split (`render_file_sidebar`)

Replace the current flat-list body with two collapsible sections:

```rust
// app/src/code_review/code_review_view.rs (around :4795)

fn render_file_sidebar(&self, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let state = self.diff_state.read(app, |m, _| m.loaded_state().cloned());
    let Some(state) = state else {
        // Existing no-repo / loading affordance
        return self.render_file_sidebar_empty(appearance);
    };

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(8.0);

    if let Some(op) = state.in_progress_op.as_ref() {
        column = column.with_child(self.render_in_progress_banner(op, appearance));
    }

    column = column
        .with_child(self.render_sidebar_section("Staged Changes", &state.staged, true, appearance))
        .with_child(self.render_sidebar_section("Changes", &state.changes, false, appearance));

    Container::new(column.finish()).finish()
}

fn render_sidebar_section(
    &self,
    label: &str,
    files: &[FileEntry],
    is_staged: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // Header: `<label>  ·  N`
    // Body: one row per file via `render_file_sidebar_row` with a new `section: SidebarSection`
    //       parameter so the row's hover affordances know whether it's staged or unstaged.
    // ...
}
```

`render_file_sidebar_row` gains:
- A `section: SidebarSection { Staged, Changes }` parameter.
- A leading status glyph column (`M`/`A`/`D`/`R`/`C`/`U`/`?`) themed.
- Hover-revealed action cluster on the right: `[+] [↺]` for Changes rows, `[−]` for Staged rows, `[Resolve…]` for `U` rows. (5a renders the cluster as placeholders if the actions aren't wired yet; 5c wires them.)
- Click handler unchanged: dispatches `CodeReviewAction::FileSelected(idx)` with the global index into the file list. **Indexing requires care**: the existing handler keys by `usize` into a flat list; the rework needs a stable mapping from `(section, in-section-idx)` to the global index. Simplest: keep one flat `Vec<FileEntry>` underneath and have the two sections render slices into it.

The +/- additions/deletions count rendering stays on the right of each row (PRODUCT §6 [inherits]) — that's the existing column; it's just behind the new hover cluster.

### 4. New actions

`CodeReviewAction` (existing enum in `code_review/mod.rs` or `code_review_view.rs`) gains:

```rust
StageFile(usize),
UnstageFile(usize),
DiscardFile(usize),
StageHunk { file_idx: usize, hunk_idx: usize },     // 5b
UnstageHunk { file_idx: usize, hunk_idx: usize },   // 5b
DiscardHunk { file_idx: usize, hunk_idx: usize },   // 5b
OpenConflictResolve(usize),                          // 5c
TimelineLoadMore,                                    // 5d
TimelineSelectCommit(GitSha),                        // 5d
TimelineBackToWorkingDiff,                           // 5d
```

Handlers route to thin async wrappers over `util::git::run_git_command`. The existing model's invalidation path picks up the resulting state change automatically.

### 5. In-progress op banner (5c)

```rust
fn render_in_progress_banner(&self, op: &InProgressOp, appearance: &Appearance) -> Box<dyn Element> {
    // Use InlineBannerStyle::Recommendation
    // Text: "<Op> in progress — resolve conflicts then commit, or run `<abort-cmd>` in the terminal."
}
```

Banner renders above the sidebar, below the existing panel header. Commit button label and gating in `PrimaryGitActionMode` extend to recognize `op_in_progress.is_some()` and switch to `Continue rebase` / `Conclude merge` / `Continue cherry-pick`.

### 6. Hunk-level affordances (5b)

The existing `CodeReviewEditorView` renders the diff inline. Hunk headers are rendered by the editor's diff layer (`app/src/code/editor/diff.rs`). 5b adds an overlay button cluster to each hunk header — locate the existing hover-action slot if `CodeReviewEditorView` exposes one; otherwise add a small wrapper component that absolutely-positions the cluster.

Patch synthesis (`hunk_to_patch(hunk, full_path) -> String`) lives in `app/src/code_review/hunk_patch.rs`. Output is a valid `git apply`-compatible patch with the file header and one `@@` hunk. Apply via `util::git::run_git_command(repo, &["apply", flags..., "-"])` with the patch on stdin (extend `run_git_command` with a stdin-capable variant if it doesn't already have one — same helper 5d needs for commit).

### 7. Timeline (5d)

```
app/src/code_review/
├── timeline.rs           // TimelineEntry, paged log fetch, rename + ahead detection
└── code_review_view.rs   // new render_timeline_section + handler arms
```

`TimelineEntry` mirrors the prior spec's shape: SHA, short SHA, author name + email, timestamp, subject, `is_rename_commit`, `original_path_at_commit`, `is_local_only`.

Log fetch: `git log --follow --name-status --format=%H%x00%an%x00%ae%x00%at%x00%s -n 20 --skip <offset> -- <path>`. Local-only detection: `git log <upstream>..HEAD --format=%H -- <path>` → `HashSet<GitSha>`. Run on Timeline first-page load; cache until refresh.

The Timeline render lives below the inline diff. When the user clicks an entry, the inline diff swaps to a `git show <sha> -- <path>` view; the existing `CodeReviewEditorView` already renders unified diff from a string source, so feed it the `git show` output.

## Testing and validation

| PRODUCT § | Verification | Phase |
|---|---|---|
| §1 (panel surface) | Manual: top-right Diff icon still toggles the panel. Smoke 1. | 5a |
| §2 (panel repo) | Manual: focus pane in other repo → panel retargets (existing behavior; verify). Smoke 1. | 5a |
| §3 (no-repo state) | Manual: open panel outside repo. Verify existing affordance. | 5a |
| §4 (sidebar split) | View test: `LoadedState { staged: [a], changes: [b] }` renders two section headers with correct counts. Smoke 1–3. | 5a |
| §5 (status glyph) | Unit: each `FileStatus` maps to expected char + theme color slot. Smoke 2–4. | 5a |
| §6 (+/- counts) | Manual: existing +/- column still renders. Smoke 4. | 5a |
| §7 (sort) | Unit: `parse_porcelain_v2` sort is case-insensitive lexicographic, stable. Smoke 6. | 5a |
| §8 (click → inline diff) | Manual: click row → diff expands inline. Smoke 7. | 5a |
| §9 (diff format) | Manual: existing diff render unchanged. | 5a |
| §10 (file hover actions) | View test + manual: hover row → correct cluster appears. Smoke 12–13. | 5c |
| §11 (file operations) | Integration test: scratch repo, click `[+]` → file moves to Staged Changes; click `[−]` → moves back; click `[↺]` → reverts. Smoke 12–13. | 5c |
| §12 (hunk operations) | Integration test: patch synthesis round-trips through `git apply --check`. Smoke 7–9. | 5b |
| §13 (in-progress banner) | Integration: induce merge conflict; banner + commit-button label change. Smoke 10–11. | 5c |
| §14 (idempotence / race retry) | Integration: drift working tree mid-click → retry happens. | 5b, 5c |
| §15–§17 (commit / push / pull) | Manual: existing behavior unchanged. | (verify in 5a; no impl PR needed) |
| §18 (Timeline scope) | View test: focused file → header reflects basename. Smoke 14. | 5d |
| §19 (Timeline entries) | Log parser unit: %x00-delimited format parses correctly. | 5d |
| §20 (paging) | Manual: `[Load more]`. Smoke 17. | 5d |
| §21 (click → commit diff) | Integration: click Timeline entry → diff swaps. Smoke 15–16. | 5d |
| §22 (rename tracking) | Integration: `git mv` + commit, focus renamed file. Smoke 18. | 5d |
| §23 (local-only marker) | Integration: commit without push → marker appears; push → marker clears. Smoke 19. | 5d |
| §24 (refresh trigger) | Manual: terminal `git checkout` triggers refresh. Smoke 20. | 5a |
| §25 (staged + changes from one fetch) | Unit: porcelain v2 parser populates both buckets. Smoke 2–5. | 5a |
| §26 (errors never silent) | Manual: induce a git failure (corrupt index); banner with stderr appears. | (verify, no impl) |
| §27 (performance) | Manual: synthesize ~1k-file repo; sidebar scrolls smoothly. | 5a |
| §28 (keyboard) | View tests: per-row `s`/`d` keyboard shortcuts fire. Smoke step manual. | 5c |
| §29 (themed visuals) | Audit: no `Color::rgb(` in new code. | 5a, 5b, 5c, 5d |
| §30 (telemetry) | Audit: any new event lands on `CodeReviewTelemetryEvent`; no new channel. | 5a |

Test files:

- 5a: `app/src/code_review/porcelain_v2_tests.rs` (carry over 27 parser tests). Snapshot/view tests for the new sidebar layout extend `app/src/code_review/code_review_view_tests.rs` (or its nearest existing analog).
- 5b: `app/src/code_review/hunk_patch_tests.rs` (patch synthesis round-trip through `git apply --check`).
- 5c: integration tests against a scratch repo built with `tempfile`.
- 5d: log-parser unit tests + scratch-repo integration test for `--follow` + `<upstream>..HEAD` queries.

`./script/presubmit` must pass before each impl PR opens. The integration tests use `tempfile` + subsecond fixtures; if they prove too slow for presubmit, gate behind a feature flag and run them in a separate CI job.

## Risks and mitigations

- **Risk:** `LoadedState` is read by many call sites (file-list rendering, commit dialog, header buttons). Adding `staged` / `changes` fields without breaking those readers requires care. **Mitigation:** the new fields are additive; existing `file_states` continues to be the source for everything except sidebar rendering. The sidebar reader switches first; other readers are untouched.
- **Risk:** `FileSelected(usize)` indexes a flat list, but the sidebar now renders two sections. **Mitigation:** keep one underlying `Vec<FileEntry>` (e.g., `staged ++ changes`) and have the sections render slices into it. The `usize` index continues to be a flat index. Partial-stage entries (same path in both sections) get **two** indices; clicking either expands the same file's diff.
- **Risk:** Conflict rows in both sections create double-rendering. **Mitigation:** §13's "deduped by path" rule applies — the underlying list has one conflict entry, rendered into both section views by reference.
- **Risk:** Hunk-button rendering may not have a host slot in the existing diff renderer. **Mitigation:** discovered during 5b; fall back to an absolute-positioned overlay wrapper if `CodeReviewEditorView` doesn't expose one.
- **Risk:** Existing commit dialog gated by `FeatureFlag::GitOperationsInCodeReview` may behave unexpectedly on platforms / accounts without the flag. **Mitigation:** PRODUCT §§15–17 are flagged as `[inherits]` — verify against the flag's two states (on and off) during 5a smoke, file any gaps as follow-ups rather than blockers.
- **Risk:** The existing panel may run `git status --porcelain=2` (v2 with `=2`, not `=2 --renames`). Renames may surface differently than the prior spec assumed. **Mitigation:** explicit `--renames` flag added when extending the call site; verify behavior with a `git mv` smoke step.
- **Risk:** Carrying the parser tests across `app/src/open_changes/` → `app/src/code_review/porcelain_v2.rs` may produce a noisy `git mv`-shaped diff. **Mitigation:** include the move as the first commit in the 5a impl PR; the rest of the PR is the layered additions.

## Follow-ups

- Verify and (if needed) polish the existing commit / push / pull / fetch behavior so PRODUCT §§15–17 match VS Code parity. If gaps emerge, file as 5e or post-05 work.
- Tighten the `Repository` invalidation cadence if the panel feels laggy on large repos. Measure first; don't pre-optimize.
- Pre-populate Timeline cache so the first `[Load more]` is instant.
- Stash management surface (still out of scope).
- 3-way merge UI (still out of scope; external editor handoff remains the contract).
- Branch picker in the header (still out of scope).

## Parallelization

The four sub-phases ship as four sequential PRs. They're not parallelizable: 5b/5c/5d all consume 5a's `LoadedState` extensions and the new `SidebarSection`-aware row rendering. Single-engineer twarp doesn't benefit from sub-agent parallelism here.
