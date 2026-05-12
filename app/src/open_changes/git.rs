//! Async git wrappers for the Open Changes panel.
//!
//! All git operations route through [`crate::util::git::run_git_command`],
//! which shells out via `command::r#async::Command`. There is no in-process
//! `git2` dependency — the wrapper is the single point where git
//! invocations happen, which keeps PATH propagation (for hooks like LFS)
//! consistent with the rest of twarp's git surfaces.

use std::path::Path;

use anyhow::Result;

use crate::open_changes::repo::{parse_porcelain_v2, InProgressOp, ParsedStatus, RepoState};
use crate::util::git::run_git_command;

/// Fetches `git status --porcelain=v2 --branch --untracked-files=all
/// --renames` for `repo_path`, parses it, and builds a [`RepoState`].
///
/// Errors propagate from [`run_git_command`]; the caller is expected to
/// stash the message into `RepoState::errors` and surface it (5a logs
/// only; 5d adds the banner).
pub async fn fetch_status(repo_path: &Path) -> Result<RepoState> {
    let stdout = run_git_command(
        repo_path,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "--renames",
        ],
    )
    .await?;

    let ParsedStatus {
        branch,
        staged,
        changes,
    } = parse_porcelain_v2(&stdout);

    let repo_name = repo_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let op_in_progress = InProgressOp::detect(&repo_path.join(".git"));

    Ok(RepoState {
        root: repo_path.to_path_buf(),
        repo_name,
        branch,
        staged,
        changes,
        op_in_progress,
        errors: Vec::new(),
    })
}
