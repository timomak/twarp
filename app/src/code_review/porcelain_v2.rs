//! Parser for `git status --porcelain=v2 --branch` output, used by the
//! Code Review panel sidebar to split rows into Staged Changes /
//! Changes sections (PRODUCT §§4–7).
//!
//! See <https://git-scm.com/docs/git-status#_porcelain_format_version_2>
//! for the line grammar. The parser is line-oriented and permissive —
//! unknown lines are skipped rather than fatally failing, since v2 is
//! documented to add new line kinds in future git releases.
//!
//! The input is the **newline-separated** form (no `-z` flag). Callers
//! that already run with `-z` should issue a separate fetch for the
//! sidebar split until the existing call sites are reconciled in a
//! follow-up.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchState {
    Branch {
        name: String,
        upstream: Option<UpstreamTracking>,
    },
    Detached {
        short_sha: String,
    },
}

impl Default for BranchState {
    fn default() -> Self {
        BranchState::Branch {
            name: String::new(),
            upstream: None,
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
    /// Detected by looking for sentinel files under `.git/`. PRODUCT §13.
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

    /// Human-readable verb for the in-progress-op banner.
    pub fn label(&self) -> &'static str {
        match self {
            InProgressOp::Merging => "Merge",
            InProgressOp::Rebasing => "Rebase",
            InProgressOp::CherryPicking => "Cherry-pick",
            InProgressOp::Bisecting => "Bisect",
        }
    }

    /// Shell command the user can paste to abort the op.
    pub fn abort_command(&self) -> &'static str {
        match self {
            InProgressOp::Merging => "git merge --abort",
            InProgressOp::Rebasing => "git rebase --abort",
            InProgressOp::CherryPicking => "git cherry-pick --abort",
            InProgressOp::Bisecting => "git bisect reset",
        }
    }

    /// Label for the panel's primary action button while the op is in
    /// progress (PRODUCT §13). Bisect has no natural "commit to continue"
    /// action, so the existing Commit label stays.
    pub fn primary_action_label(&self) -> Option<&'static str> {
        match self {
            InProgressOp::Merging => Some("Conclude merge"),
            InProgressOp::Rebasing => Some("Continue rebase"),
            InProgressOp::CherryPicking => Some("Continue cherry-pick"),
            InProgressOp::Bisecting => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub status: FileStatus,
    pub from_path: Option<PathBuf>,
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
    /// Single-character glyph rendered in the sidebar's status column
    /// (PRODUCT §5).
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedStatus {
    pub branch: BranchState,
    pub staged: Vec<FileEntry>,
    pub changes: Vec<FileEntry>,
}

/// Parse newline-separated porcelain-v2 status output.
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
            let sha = rest.trim();
            let short: String = sha.chars().take(7).collect();
            head_sha = Some(short);
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            upstream_branch = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
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

    // Sort by path, case-insensitive lexicographic, stable (PRODUCT §7).
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
        _ if is_rename => Some(FileStatus::Renamed),
        _ => Some(FileStatus::Modified),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_head() {
        let parsed = parse_porcelain_v2("# branch.oid abcdef0123456789\n# branch.head main\n");
        match parsed.branch {
            BranchState::Branch { name, upstream } => {
                assert_eq!(name, "main");
                assert!(upstream.is_none());
            }
            other => panic!("expected Branch, got {other:?}"),
        }
    }

    #[test]
    fn parses_detached_head() {
        let parsed =
            parse_porcelain_v2("# branch.oid 1234567890abcdef\n# branch.head (detached)\n");
        match parsed.branch {
            BranchState::Detached { short_sha } => {
                assert_eq!(short_sha, "1234567");
            }
            other => panic!("expected Detached, got {other:?}"),
        }
    }

    #[test]
    fn parses_upstream_tracking() {
        let parsed = parse_porcelain_v2(
            "# branch.oid abc\n\
             # branch.head feature\n\
             # branch.upstream origin/feature\n\
             # branch.ab +2 -1\n",
        );
        match parsed.branch {
            BranchState::Branch {
                name,
                upstream: Some(up),
            } => {
                assert_eq!(name, "feature");
                assert_eq!(up.remote_branch, "origin/feature");
                assert_eq!(up.ahead, 2);
                assert_eq!(up.behind, 1);
            }
            other => panic!("expected Branch with upstream, got {other:?}"),
        }
    }

    #[test]
    fn parses_modified_unstaged() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 .M N... 100644 100644 100644 abc def README.md\n",
        );
        assert!(parsed.staged.is_empty());
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].path, PathBuf::from("README.md"));
        assert_eq!(parsed.changes[0].status, FileStatus::Modified);
    }

    #[test]
    fn parses_staged_added() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 A. N... 100644 100644 100644 abc def newfile.rs\n",
        );
        assert_eq!(parsed.staged.len(), 1);
        assert!(parsed.changes.is_empty());
        assert_eq!(parsed.staged[0].path, PathBuf::from("newfile.rs"));
        assert_eq!(parsed.staged[0].status, FileStatus::Added);
    }

    #[test]
    fn parses_staged_and_unstaged_same_file() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 MM N... 100644 100644 100644 abc def src/lib.rs\n",
        );
        assert_eq!(parsed.staged.len(), 1);
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.staged[0].path, PathBuf::from("src/lib.rs"));
        assert_eq!(parsed.changes[0].path, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn parses_deleted_unstaged() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 .D N... 100644 100644 000000 abc def gone.txt\n",
        );
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].status, FileStatus::Deleted);
    }

    #[test]
    fn parses_untracked() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             ? untracked.txt\n",
        );
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].path, PathBuf::from("untracked.txt"));
        assert_eq!(parsed.changes[0].status, FileStatus::Untracked);
    }

    #[test]
    fn parses_rename() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             2 R. N... 100644 100644 100644 abc def R100 new.rs\told.rs\n",
        );
        assert_eq!(parsed.staged.len(), 1);
        let entry = &parsed.staged[0];
        assert_eq!(entry.path, PathBuf::from("new.rs"));
        assert_eq!(entry.from_path.as_deref(), Some(Path::new("old.rs")));
        assert_eq!(entry.status, FileStatus::Renamed);
    }

    #[test]
    fn parses_unmerged_conflict() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             u UU N... 100644 100644 100644 100644 abc def ghi conflicted.rs\n",
        );
        // Unmerged appears in BOTH sections so the user can spot it in either view.
        assert_eq!(parsed.staged.len(), 1);
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.staged[0].status, FileStatus::Unmerged);
        assert_eq!(parsed.staged[0].path, PathBuf::from("conflicted.rs"));
    }

    #[test]
    fn parses_submodule_entry() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 .M S.M. 160000 160000 160000 abc def vendor/sub\n",
        );
        assert_eq!(parsed.changes.len(), 1);
        assert!(parsed.changes[0].is_submodule);
    }

    #[test]
    fn parses_paths_with_spaces() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 .M N... 100644 100644 100644 abc def path with spaces.md\n",
        );
        assert_eq!(parsed.changes[0].path, PathBuf::from("path with spaces.md"));
    }

    #[test]
    fn parses_empty_output_as_no_changes() {
        let parsed = parse_porcelain_v2("");
        assert!(parsed.staged.is_empty());
        assert!(parsed.changes.is_empty());
    }

    #[test]
    fn skips_unknown_lines_gracefully() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             # branch.something-new value\n\
             ! ignored.txt\n\
             1 .M N... 100644 100644 100644 abc def real.txt\n",
        );
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].path, PathBuf::from("real.txt"));
    }

    #[test]
    fn entries_sort_case_insensitive_lexicographic() {
        let parsed = parse_porcelain_v2(
            "# branch.head main\n\
             1 .M N... 100644 100644 100644 abc def Zzz.txt\n\
             1 .M N... 100644 100644 100644 abc def aaa.txt\n\
             1 .M N... 100644 100644 100644 abc def Mmm.txt\n",
        );
        let paths: Vec<_> = parsed
            .changes
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(paths, vec!["aaa.txt", "Mmm.txt", "Zzz.txt"]);
    }

    #[test]
    fn detects_merging() {
        let tmp = tempfile::tempdir().unwrap();
        let git = tmp.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        std::fs::write(git.join("MERGE_HEAD"), "abc").unwrap();
        assert_eq!(InProgressOp::detect(&git), Some(InProgressOp::Merging));
    }

    #[test]
    fn detects_rebasing() {
        let tmp = tempfile::tempdir().unwrap();
        let git = tmp.path().join(".git");
        std::fs::create_dir_all(git.join("rebase-merge")).unwrap();
        assert_eq!(InProgressOp::detect(&git), Some(InProgressOp::Rebasing));
    }

    #[test]
    fn detects_cherry_picking() {
        let tmp = tempfile::tempdir().unwrap();
        let git = tmp.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        std::fs::write(git.join("CHERRY_PICK_HEAD"), "abc").unwrap();
        assert_eq!(
            InProgressOp::detect(&git),
            Some(InProgressOp::CherryPicking)
        );
    }

    #[test]
    fn detects_bisecting() {
        let tmp = tempfile::tempdir().unwrap();
        let git = tmp.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        std::fs::write(git.join("BISECT_LOG"), "abc").unwrap();
        assert_eq!(InProgressOp::detect(&git), Some(InProgressOp::Bisecting));
    }

    #[test]
    fn detects_no_op_in_clean_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let git = tmp.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        assert_eq!(InProgressOp::detect(&git), None);
    }
}
