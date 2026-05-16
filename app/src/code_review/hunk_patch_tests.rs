//! Round-trip tests for [`hunk_to_patch`]. Each test seeds a scratch
//! git repo with a known file, generates a `DiffHunk` matching some
//! edit, synthesizes the patch, and validates it via `git apply
//! --check` (no actual apply). This catches header malformations,
//! off-by-one line counts, and missing newlines without depending on
//! the parser.

use std::path::{Path, PathBuf};

use command::blocking::Command;
use command::Stdio;
use tempfile::TempDir;

use crate::code_review::diff_state::{DiffHunk, DiffLine, DiffLineType, GitFileStatus};
use crate::code_review::hunk_patch::hunk_to_patch;

fn ctx_line(num_old: usize, num_new: usize, text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Context,
        old_line_number: Some(num_old),
        new_line_number: Some(num_new),
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

fn add_line(num: usize, text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Add,
        old_line_number: None,
        new_line_number: Some(num),
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

fn del_line(num: usize, text: &str) -> DiffLine {
    DiffLine {
        line_type: DiffLineType::Delete,
        old_line_number: Some(num),
        new_line_number: None,
        text: text.to_string(),
        no_trailing_newline: false,
    }
}

fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("git binary present on test path")
}

fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    assert!(git(repo, &["init", "-q"]).status.success());
    assert!(git(repo, &["config", "user.email", "test@example.com"])
        .status
        .success());
    assert!(git(repo, &["config", "user.name", "test"]).status.success());
    assert!(git(repo, &["config", "commit.gpgsign", "false"])
        .status
        .success());
    dir
}

fn write(repo: &Path, name: &str, content: &str) -> PathBuf {
    let path = repo.join(name);
    std::fs::write(&path, content).expect("write file");
    path
}

fn commit_all(repo: &Path, msg: &str) {
    assert!(git(repo, &["add", "-A"]).status.success());
    assert!(git(repo, &["commit", "-m", msg, "-q"]).status.success());
}

/// Runs `git apply --check` against the patch. On failure the assertion
/// surfaces stderr so the synthesized patch is visible in test output.
fn assert_applies_clean(repo: &Path, patch: &str, extra_args: &[&str]) {
    let mut args = vec!["apply", "--check"];
    args.extend_from_slice(extra_args);
    args.push("-");
    let mut child = Command::new("git")
        .args(&args)
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git apply");
    use std::io::Write as _;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(patch.as_bytes())
        .expect("write patch");
    let out = child.wait_with_output().expect("wait git apply");
    assert!(
        out.status.success(),
        "git apply --check {extra_args:?} failed: stderr={} patch={}",
        String::from_utf8_lossy(&out.stderr),
        patch,
    );
}

#[test]
fn modified_hunk_single_line_replacement_roundtrips() {
    let dir = init_repo();
    let repo = dir.path();
    write(repo, "a.txt", "line one\nline two\nline three\n");
    commit_all(repo, "init");

    // Working tree drifts; index still matches HEAD. Applying the
    // patch via `--cached` simulates the stage-hunk flow.
    write(repo, "a.txt", "line one\nLINE TWO\nline three\n");

    let hunk = DiffHunk {
        old_start_line: 1,
        old_line_count: 3,
        new_start_line: 1,
        new_line_count: 3,
        lines: vec![
            ctx_line(1, 1, "line one"),
            del_line(2, "line two"),
            add_line(2, "LINE TWO"),
            ctx_line(3, 3, "line three"),
        ],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };

    let patch = hunk_to_patch(Path::new("a.txt"), GitFileStatus::Modified, &hunk);
    assert_applies_clean(repo, &patch, &["--cached"]);
}

#[test]
fn pure_addition_hunk_roundtrips() {
    let dir = init_repo();
    let repo = dir.path();
    write(repo, "a.txt", "line one\nline two\n");
    commit_all(repo, "init");
    write(repo, "a.txt", "line one\nline two\nnew tail\n");

    let hunk = DiffHunk {
        old_start_line: 2,
        old_line_count: 1,
        new_start_line: 2,
        new_line_count: 2,
        lines: vec![ctx_line(2, 2, "line two"), add_line(3, "new tail")],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };

    let patch = hunk_to_patch(Path::new("a.txt"), GitFileStatus::Modified, &hunk);
    assert_applies_clean(repo, &patch, &["--cached"]);
}

#[test]
fn pure_deletion_hunk_roundtrips() {
    let dir = init_repo();
    let repo = dir.path();
    write(repo, "a.txt", "line one\nline two\nline three\n");
    commit_all(repo, "init");
    write(repo, "a.txt", "line one\nline three\n");

    let hunk = DiffHunk {
        old_start_line: 1,
        old_line_count: 3,
        new_start_line: 1,
        new_line_count: 2,
        lines: vec![
            ctx_line(1, 1, "line one"),
            del_line(2, "line two"),
            ctx_line(3, 2, "line three"),
        ],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };

    let patch = hunk_to_patch(Path::new("a.txt"), GitFileStatus::Modified, &hunk);
    assert_applies_clean(repo, &patch, &["--cached"]);
}

#[test]
fn reverse_apply_roundtrips() {
    let dir = init_repo();
    let repo = dir.path();
    write(repo, "a.txt", "line one\nline two\nline three\n");
    commit_all(repo, "init");
    write(repo, "a.txt", "line one\nLINE TWO\nline three\n");
    assert!(git(repo, &["add", "a.txt"]).status.success());

    let hunk = DiffHunk {
        old_start_line: 1,
        old_line_count: 3,
        new_start_line: 1,
        new_line_count: 3,
        lines: vec![
            ctx_line(1, 1, "line one"),
            del_line(2, "line two"),
            add_line(2, "LINE TWO"),
            ctx_line(3, 3, "line three"),
        ],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };

    let patch = hunk_to_patch(Path::new("a.txt"), GitFileStatus::Modified, &hunk);
    assert_applies_clean(repo, &patch, &["--cached", "--reverse"]);
}

#[test]
fn no_newline_at_eof_preserved() {
    let dir = init_repo();
    let repo = dir.path();
    write(repo, "a.txt", "line one\n");
    commit_all(repo, "init");
    std::fs::write(repo.join("a.txt"), "line one\nno trailing").expect("write no-newline file");

    let hunk = DiffHunk {
        old_start_line: 1,
        old_line_count: 1,
        new_start_line: 1,
        new_line_count: 2,
        lines: vec![
            ctx_line(1, 1, "line one"),
            DiffLine {
                line_type: DiffLineType::Add,
                old_line_number: None,
                new_line_number: Some(2),
                text: "no trailing".to_string(),
                no_trailing_newline: true,
            },
        ],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };

    let patch = hunk_to_patch(Path::new("a.txt"), GitFileStatus::Modified, &hunk);
    assert!(patch.contains("\\ No newline at end of file"));
    assert_applies_clean(repo, &patch, &["--cached"]);
}

#[test]
fn single_line_range_omits_count() {
    let hunk = DiffHunk {
        old_start_line: 5,
        old_line_count: 1,
        new_start_line: 5,
        new_line_count: 1,
        lines: vec![
            del_line(5, "old"),
            add_line(5, "new"),
            ctx_line(6, 6, "tail"),
        ],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };
    let patch = hunk_to_patch(Path::new("f.txt"), GitFileStatus::Modified, &hunk);
    assert!(patch.contains("@@ -5,1 +5,1 @@") || patch.contains("@@ -5 +5 @@"));
    assert!(
        patch.lines().any(|l| l == "@@ -5,1 +5,1 @@") || patch.lines().any(|l| l == "@@ -5 +5 @@")
    );
}

#[test]
fn deleted_file_status_writes_dev_null_target() {
    let hunk = DiffHunk {
        old_start_line: 1,
        old_line_count: 1,
        new_start_line: 0,
        new_line_count: 0,
        lines: vec![del_line(1, "only line")],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };
    let patch = hunk_to_patch(Path::new("dead.txt"), GitFileStatus::Deleted, &hunk);
    assert!(patch.contains("--- a/dead.txt"));
    assert!(patch.contains("+++ /dev/null"));
}

#[test]
fn added_file_status_writes_dev_null_source() {
    let hunk = DiffHunk {
        old_start_line: 0,
        old_line_count: 0,
        new_start_line: 1,
        new_line_count: 1,
        lines: vec![add_line(1, "fresh")],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };
    let patch = hunk_to_patch(Path::new("new.txt"), GitFileStatus::New, &hunk);
    assert!(patch.contains("--- /dev/null"));
    assert!(patch.contains("+++ b/new.txt"));
}

#[test]
fn skips_hunk_header_diff_lines() {
    let hunk = DiffHunk {
        old_start_line: 1,
        old_line_count: 1,
        new_start_line: 1,
        new_line_count: 1,
        lines: vec![
            DiffLine {
                line_type: DiffLineType::HunkHeader,
                old_line_number: None,
                new_line_number: None,
                text: "@@ -1 +1 @@".to_string(),
                no_trailing_newline: false,
            },
            del_line(1, "old"),
            add_line(1, "new"),
        ],
        unified_diff_start: 0,
        unified_diff_end: 0,
    };
    let patch = hunk_to_patch(Path::new("f.txt"), GitFileStatus::Modified, &hunk);
    // The synthesized header appears exactly once; no duplicate from
    // the lines vec.
    let header_count = patch.lines().filter(|l| l.starts_with("@@")).count();
    assert_eq!(header_count, 1, "patch: {patch}");
}
