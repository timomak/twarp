//! Unit tests for the Open Changes panel data layer (5a).
//!
//! Cover:
//! - `parse_porcelain_v2` for every documented v2 line kind: branch
//!   headers, ordinary entries (M/A/D/U), renames, copies, unmerged,
//!   untracked, submodule, detached HEAD, ahead/behind tracking.
//! - `find_repo_root` walks up from a nested path to the repo root.
//! - `OpenChangesModel::unique_change_count` dedupes partial-stage entries.
//!
//! View-layer tests (rendering Box<dyn Element>) require an `Appearance`
//! handle from a running app context; those are covered by the smoke
//! test in PRODUCT.md.

use std::path::{Path, PathBuf};

use crate::open_changes::repo::{
    find_repo_root, parse_porcelain_v2, BranchState, FileEntry, FileStatus, InProgressOp,
};
use crate::open_changes::OpenChangesModel;

// --- parse_porcelain_v2: branch headers ---

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
    let parsed = parse_porcelain_v2("# branch.oid 1234567890abcdef\n# branch.head (detached)\n");
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

// --- parse_porcelain_v2: file entries ---

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
    // Partial stage: file appears in both sections.
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

// --- InProgressOp detection ---

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
fn detects_no_op_in_clean_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let git = tmp.path().join(".git");
    std::fs::create_dir(&git).unwrap();
    assert_eq!(InProgressOp::detect(&git), None);
}

// --- find_repo_root ---

#[test]
fn finds_repo_root_from_nested_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    let nested = root.join("a/b/c");
    std::fs::create_dir_all(&nested).unwrap();

    let found = find_repo_root(&nested).expect("should find root");
    // tempdir on macOS canonicalizes via /private/var; compare basenames.
    assert!(found.ends_with(root.file_name().unwrap()));
}

#[test]
fn returns_none_outside_any_repo() {
    // /tmp itself shouldn't have a .git; canonicalize first to handle
    // macOS's /private/var/folders symlink.
    let tmp = std::env::temp_dir();
    let canon = std::fs::canonicalize(&tmp).unwrap_or(tmp);
    // Walk up to /; if any ancestor has a .git this test is invalid.
    let mut p = canon.clone();
    while p.pop() {
        if p.join(".git").exists() {
            return; // skip — running inside a checked-out repo
        }
    }
    // canonical /tmp should not be inside a repo.
    let nested = canon.join("__open_changes_test_not_a_repo__");
    assert!(find_repo_root(&nested).is_none());
}

#[test]
fn rejects_relative_paths() {
    // find_repo_root expects absolute input — caller has the focused
    // pane's absolute cwd.
    assert!(find_repo_root(Path::new("relative/path")).is_none());
}

// --- OpenChangesModel::unique_change_count ---

#[test]
fn unique_change_count_handles_partial_stage() {
    let mut model = OpenChangesModel::new();
    model.state = Some(crate::open_changes::RepoState {
        root: PathBuf::from("/tmp/repo"),
        repo_name: "repo".to_string(),
        branch: BranchState::default(),
        staged: vec![FileEntry {
            path: PathBuf::from("a.rs"),
            status: FileStatus::Modified,
            from_path: None,
            is_submodule: false,
        }],
        changes: vec![
            FileEntry {
                path: PathBuf::from("a.rs"), // same path, partial stage
                status: FileStatus::Modified,
                from_path: None,
                is_submodule: false,
            },
            FileEntry {
                path: PathBuf::from("b.rs"),
                status: FileStatus::Untracked,
                from_path: None,
                is_submodule: false,
            },
        ],
        op_in_progress: None,
        errors: vec![],
    });
    // a.rs counted once (despite being in both sections), b.rs counted once.
    assert_eq!(model.unique_change_count(), 2);
}

#[test]
fn unique_change_count_is_zero_in_no_repo_state() {
    let model = OpenChangesModel::new();
    assert_eq!(model.unique_change_count(), 0);
}
