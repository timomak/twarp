//! Data model for the Open Changes panel.
//!
//! `RepoState` is the single source of truth a refresh produces; the view
//! reads it for every render. `parse_porcelain_v2` is the workhorse that
//! turns `git status --porcelain=v2 --branch` output into [`FileEntry`]
//! rows. Unit tests in `open_changes_tests.rs` pin its behavior against
//! the documented v2 grammar.

use std::path::{Path, PathBuf};

/// Complete snapshot of the panel repo at the time of the most recent
/// refresh. PRODUCT §4 (layout).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoState {
    /// Absolute path to the repo's top-level working-tree directory.
    pub root: PathBuf,
    /// Basename of [`root`]. Cached so the header doesn't recompute it
    /// on every render.
    pub repo_name: String,
    /// Branch name (or detached-HEAD short SHA), plus upstream tracking.
    pub branch: BranchState,
    /// Files with changes in the index (`git diff --cached`).
    pub staged: Vec<FileEntry>,
    /// Files with changes in the working tree (`git diff`) plus untracked.
    pub changes: Vec<FileEntry>,
    /// `Some(op)` when the repo is mid-merge / mid-rebase /
    /// mid-cherry-pick / mid-bisect (PRODUCT §22). `None` otherwise.
    pub op_in_progress: Option<InProgressOp>,
    /// Verbatim git stderr from the most recent failed operation, surfaced
    /// as a banner by 5d. Populated by 5c/5d; 5a only reads `git status`,
    /// which generally doesn't fail in user-facing ways.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchState {
    /// Normal branch checkout with optional upstream tracking.
    Branch {
        name: String,
        upstream: Option<UpstreamTracking>,
    },
    /// Detached HEAD; carries the short SHA for display.
    Detached { short_sha: String },
}

impl Default for BranchState {
    fn default() -> Self {
        BranchState::Branch {
            name: String::new(),
            upstream: None,
        }
    }
}

impl BranchState {
    /// One-line label for the repo header (PRODUCT §4). For detached
    /// HEAD, renders as `(detached HEAD: <sha>)`.
    pub fn display_label(&self) -> String {
        match self {
            BranchState::Branch { name, .. } => name.clone(),
            BranchState::Detached { short_sha } => {
                format!("(detached HEAD: {short_sha})")
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpstreamTracking {
    pub remote_branch: String,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InProgressOp {
    Merging,
    Rebasing,
    CherryPicking,
    Bisecting,
}

impl InProgressOp {
    /// Detected by looking for sentinel files under `.git/`. PRODUCT §22.
    pub fn detect(git_dir: &Path) -> Option<Self> {
        if git_dir.join("MERGE_HEAD").exists() {
            Some(InProgressOp::Merging)
        } else if git_dir.join("rebase-apply").is_dir() || git_dir.join("rebase-merge").is_dir() {
            Some(InProgressOp::Rebasing)
        } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
            Some(InProgressOp::CherryPicking)
        } else if git_dir.join("BISECT_LOG").exists() {
            Some(InProgressOp::Bisecting)
        } else {
            None
        }
    }

    /// Display label for the in-progress-op banner (PRODUCT §22).
    pub fn label(&self) -> &'static str {
        match self {
            InProgressOp::Merging => "Merge",
            InProgressOp::Rebasing => "Rebase",
            InProgressOp::CherryPicking => "Cherry-pick",
            InProgressOp::Bisecting => "Bisect",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Path relative to the repo root.
    pub path: PathBuf,
    pub status: FileStatus,
    /// For renames / copies, the source path. None otherwise.
    pub from_path: Option<PathBuf>,
    /// True for submodule entries (porcelain-v2 `sub` field starting with `S`).
    pub is_submodule: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
}

impl FileStatus {
    /// Single-character glyph rendered in the status column (PRODUCT §8).
    pub fn glyph(&self) -> char {
        match self {
            FileStatus::Modified => 'M',
            FileStatus::Added => 'A',
            FileStatus::Deleted => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Copied => 'C',
            FileStatus::Unmerged => 'U',
            FileStatus::Untracked => '?',
        }
    }
}

/// Walk up from `start` looking for a `.git` directory or file. Returns
/// the directory **containing** `.git` (i.e. the repo's working-tree
/// top-level) when found.
///
/// `.git` may be a directory (normal checkout) or a file (worktrees,
/// submodules); both count as "found". PRODUCT §2.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur: PathBuf = if start.is_absolute() {
        start.to_path_buf()
    } else {
        // The caller is expected to pass an absolute path. Bail rather
        // than mixing in a `getcwd` here — the panel always has the
        // focused pane's absolute cwd available.
        return None;
    };

    loop {
        let dot_git = cur.join(".git");
        if dot_git.exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Parsed output of `git status --porcelain=v2 --branch
/// --untracked-files=all --renames`. The parser is line-oriented and
/// permissive — unknown lines are skipped rather than fatally failing,
/// since porcelain v2 is documented to add new line kinds in future git
/// releases.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedStatus {
    pub branch: BranchState,
    pub staged: Vec<FileEntry>,
    pub changes: Vec<FileEntry>,
}

/// Parse porcelain-v2 status output into a [`ParsedStatus`]. See
/// <https://git-scm.com/docs/git-status#_porcelain_format_version_2> for
/// the line grammar.
pub fn parse_porcelain_v2(text: &str) -> ParsedStatus {
    let mut result = ParsedStatus::default();
    let mut branch_name: Option<String> = None;
    let mut detached = false;
    let mut head_sha: Option<String> = None;
    let mut upstream_branch: Option<String> = None;
    let mut ahead: u32 = 0;
    let mut behind: u32 = 0;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            if rest == "(detached)" {
                detached = true;
            } else {
                branch_name = Some(rest.trim().to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.oid ") {
            // First 7 chars (or whole string if shorter).
            let sha = rest.trim();
            let short: String = sha.chars().take(7).collect();
            head_sha = Some(short);
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            upstream_branch = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // Format: "+N -M"
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                ahead = parts[0].trim_start_matches('+').parse().unwrap_or(0);
                behind = parts[1].trim_start_matches('-').parse().unwrap_or(0);
            }
        } else if let Some(rest) = line.strip_prefix("1 ") {
            parse_v2_ordinary_entry(rest, &mut result);
        } else if let Some(rest) = line.strip_prefix("2 ") {
            parse_v2_rename_entry(rest, &mut result);
        } else if let Some(rest) = line.strip_prefix("u ") {
            parse_v2_unmerged_entry(rest, &mut result);
        } else if let Some(rest) = line.strip_prefix("? ") {
            result.changes.push(FileEntry {
                path: PathBuf::from(rest.trim()),
                status: FileStatus::Untracked,
                from_path: None,
                is_submodule: false,
            });
        }
        // Other line kinds (`!` ignored, `# stash.count`, future `# ...`
        // headers) are skipped silently.
    }

    result.branch = match (detached, branch_name, head_sha) {
        (true, _, Some(sha)) => BranchState::Detached { short_sha: sha },
        (false, Some(name), _) => BranchState::Branch {
            name,
            upstream: upstream_branch.map(|remote_branch| UpstreamTracking {
                remote_branch,
                ahead,
                behind,
            }),
        },
        _ => BranchState::default(),
    };

    // Sort by path lexicographically (case-insensitive), stable so a row
    // doesn't jump on every refresh when a sibling changes. PRODUCT §10.
    result
        .staged
        .sort_by_cached_key(|f| f.path.to_string_lossy().to_lowercase());
    result
        .changes
        .sort_by_cached_key(|f| f.path.to_string_lossy().to_lowercase());

    result
}

fn parse_v2_ordinary_entry(rest: &str, result: &mut ParsedStatus) {
    // `1 XY sub mH mI mW hH hI path`
    // 8 space-separated tokens; the last (`path`) may contain spaces, so
    // we splitn-7 first and take the remainder as the path.
    let parts: Vec<&str> = rest.splitn(8, ' ').collect();
    if parts.len() < 8 {
        return;
    }
    let xy = parts[0];
    let sub = parts[1];
    let path = parts[7];
    if xy.len() < 2 || path.is_empty() {
        return;
    }
    let x = xy.chars().next().unwrap_or('.');
    let y = xy.chars().nth(1).unwrap_or('.');
    let is_submodule = sub.starts_with('S');
    let path_buf = PathBuf::from(path);

    if let Some(status) = xy_char_to_status(x, false) {
        result.staged.push(FileEntry {
            path: path_buf.clone(),
            status,
            from_path: None,
            is_submodule,
        });
    }
    if let Some(status) = xy_char_to_status(y, false) {
        result.changes.push(FileEntry {
            path: path_buf,
            status,
            from_path: None,
            is_submodule,
        });
    }
}

fn parse_v2_rename_entry(rest: &str, result: &mut ParsedStatus) {
    // `2 XY sub mH mI mW hH hI Xscore PATH\tORIGPATH`
    // 9 leading tokens then a tab-separated pair as the final field.
    let parts: Vec<&str> = rest.splitn(9, ' ').collect();
    if parts.len() < 9 {
        return;
    }
    let xy = parts[0];
    let sub = parts[1];
    let path_pair = parts[8];
    if xy.len() < 2 {
        return;
    }
    let x = xy.chars().next().unwrap_or('.');
    let y = xy.chars().nth(1).unwrap_or('.');
    let is_submodule = sub.starts_with('S');

    let mut path_parts = path_pair.splitn(2, '\t');
    let new_path = path_parts.next().unwrap_or("").trim();
    let old_path = path_parts.next().unwrap_or("").trim();
    if new_path.is_empty() {
        return;
    }
    let from = if old_path.is_empty() {
        None
    } else {
        Some(PathBuf::from(old_path))
    };

    if let Some(status) = xy_char_to_status(x, true) {
        result.staged.push(FileEntry {
            path: PathBuf::from(new_path),
            status,
            from_path: from.clone(),
            is_submodule,
        });
    }
    if let Some(status) = xy_char_to_status(y, true) {
        result.changes.push(FileEntry {
            path: PathBuf::from(new_path),
            status,
            from_path: from,
            is_submodule,
        });
    }
}

fn parse_v2_unmerged_entry(rest: &str, result: &mut ParsedStatus) {
    // `u XY sub m1 m2 m3 mW h1 h2 h3 path`
    // 10 fields total after the leading `u `. The XY field encodes the
    // conflict states for the two sides; for the panel we collapse to
    // `Unmerged`.
    let parts: Vec<&str> = rest.splitn(10, ' ').collect();
    if parts.len() < 10 {
        return;
    }
    let path = parts[9];
    if path.is_empty() {
        return;
    }
    let entry = FileEntry {
        path: PathBuf::from(path),
        status: FileStatus::Unmerged,
        from_path: None,
        is_submodule: false,
    };
    result.staged.push(entry.clone());
    result.changes.push(entry);
}

fn xy_char_to_status(c: char, is_rename: bool) -> Option<FileStatus> {
    match c {
        '.' => None,
        'M' => Some(FileStatus::Modified),
        'A' => Some(FileStatus::Added),
        'D' => Some(FileStatus::Deleted),
        'R' => Some(FileStatus::Renamed),
        'C' => Some(FileStatus::Copied),
        'U' => Some(FileStatus::Unmerged),
        // Some git versions encode renames with non-R XY chars; default
        // to Renamed on rename lines, Modified elsewhere.
        _ if is_rename => Some(FileStatus::Renamed),
        _ => Some(FileStatus::Modified),
    }
}
