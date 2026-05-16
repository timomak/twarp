use std::path::Path;

use command::r#async::Command;
use command::Stdio;
use tempfile::TempDir;

use super::{detect_current_branch, detect_current_branch_display, run_git_command_with_stdin};

/// Helper: run a git command inside the given repo directory.
async fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run git");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Creates a temp git repo with one commit and returns `(dir_handle, repo_path)`.
async fn init_repo() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().to_path_buf();

    git(&path, &["init", "-b", "main"]).await;
    git(&path, &["config", "user.email", "test@test.com"]).await;
    git(&path, &["config", "user.name", "Test"]).await;
    git(&path, &["commit", "--allow-empty", "-m", "initial"]).await;

    (dir, path)
}

#[tokio::test]
async fn on_normal_branch_returns_branch_name() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "-b", "feature-xyz"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "feature-xyz");
    assert_eq!(
        detect_current_branch_display(&repo).await.unwrap(),
        "feature-xyz"
    );
}

#[tokio::test]
async fn detached_head_raw_returns_head() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "HEAD");
}

#[tokio::test]
async fn detached_head_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(
        result, "HEAD",
        "display variant should not return literal HEAD"
    );
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

#[tokio::test]
async fn detached_tag_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["tag", "v1.0"]).await;
    git(&repo, &["checkout", "v1.0"]).await;

    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(result, "HEAD");
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

/// twarp 5b: round-trip a synthesized one-hunk patch through
/// `run_git_command_with_stdin` to validate the stdin plumbing —
/// not the patch content. After applying with `--cached`, the file
/// should appear in the index with the new content while the working
/// tree is unchanged. A failure here means stdin piping broke; a
/// failure in [`crate::code_review::hunk_patch_tests`] means the patch
/// itself is malformed.
#[tokio::test]
async fn run_git_command_with_stdin_applies_patch_to_index() {
    let (_dir, repo) = init_repo().await;
    std::fs::write(repo.join("a.txt"), "line one\nline two\n").unwrap();
    git(&repo, &["add", "a.txt"]).await;
    git(&repo, &["commit", "-m", "seed"]).await;
    std::fs::write(repo.join("a.txt"), "line one\nLINE TWO\n").unwrap();

    let patch = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
 line one
-line two
+LINE TWO
";

    let out = run_git_command_with_stdin(&repo, &["apply", "--cached", "-"], patch)
        .await
        .expect("apply --cached succeeds");
    assert!(out.is_empty(), "apply prints nothing on success");

    let cached = git(&repo, &["diff", "--cached", "a.txt"]).await;
    assert!(cached.contains("+LINE TWO"));
    assert!(cached.contains("-line two"));
}

#[tokio::test]
async fn run_git_command_with_stdin_surfaces_stderr_on_failure() {
    let (_dir, repo) = init_repo().await;
    std::fs::write(repo.join("a.txt"), "line one\n").unwrap();
    git(&repo, &["add", "a.txt"]).await;
    git(&repo, &["commit", "-m", "seed"]).await;

    // Hunk's `-` side references a line that doesn't exist.
    let bogus = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,1 @@
-DOES NOT EXIST
+REPLACEMENT
";

    let err = run_git_command_with_stdin(&repo, &["apply", "--cached", "-"], bogus)
        .await
        .expect_err("apply on bogus patch fails");
    let msg = err.to_string();
    assert!(
        msg.contains("patch") || msg.contains("apply"),
        "expected stderr to mention apply failure, got: {msg}"
    );
}
