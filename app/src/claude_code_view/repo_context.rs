//! The composer context bar (#11): folder, git branch, diff size, PR number,
//! and CI status shown above the message input.
//!
//! Self-contained and best-effort. Rather than wire the pane into the
//! `GitStatusUpdateModel` (which needs a pre-watched repo and only knows the
//! working-tree diff, not the PR diff or PR/CI), the pane runs `git` and `gh`
//! in the user's login shell and parses the output. Anything missing — not a
//! repo, no `gh`, not signed in, no PR — just drops that field; the bar shows
//! whatever it could resolve (at least the folder name). The refresh runs off
//! the main thread and on a cadence the pane drives (open + each turn end), so
//! a slow `gh` network call never blocks the UI.

use std::path::Path;

/// Aggregated CI state for the branch's PR (#11). Derived from `gh`'s
/// `statusCheckRollup`, collapsed to the three states worth a one-glance chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CiState {
    Passing,
    Failing,
    Pending,
}

impl CiState {
    pub(super) fn label(self) -> &'static str {
        match self {
            CiState::Passing => "CI passing",
            CiState::Failing => "CI failing",
            CiState::Pending => "CI running",
        }
    }
}

/// The resolved context shown in the composer bar (#11). Every field is
/// independently optional — a non-repo cwd still yields a `folder`.
#[derive(Clone, Debug, Default)]
pub(super) struct RepoContext {
    pub folder: Option<String>,
    pub branch: Option<String>,
    pub added: Option<usize>,
    pub removed: Option<usize>,
    pub pr_number: Option<u64>,
    pub ci: Option<CiState>,
}

impl RepoContext {
    /// True when nothing beyond a bare folder name resolved — the caller can
    /// still render the folder, but there's no git/PR context to show.
    pub(super) fn is_effectively_empty(&self) -> bool {
        self.branch.is_none()
            && self.added.is_none()
            && self.removed.is_none()
            && self.pr_number.is_none()
            && self.ci.is_none()
    }
}

/// The last path component of `cwd`, the bar's folder label.
pub(super) fn folder_name(cwd: &Path) -> Option<String> {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

/// A login-shell snippet that prints the repo context in marker-delimited
/// sections we can parse back. Each tool's stderr is swallowed so a missing
/// `git`/`gh`, a non-repo dir, or an unauthenticated `gh` degrades to an empty
/// section rather than noise.
pub(super) fn build_command(cwd: &Path) -> String {
    // Single-quote the path and escape any embedded single quotes.
    let dir = cwd.to_string_lossy().replace('\'', r"'\''");
    format!(
        "cd '{dir}' 2>/dev/null || exit 0\n\
         echo '@@BRANCH@@'; git rev-parse --abbrev-ref HEAD 2>/dev/null\n\
         echo '@@DIFF@@'; git diff --shortstat 2>/dev/null\n\
         echo '@@PR@@'; gh pr view --json number,additions,deletions,statusCheckRollup 2>/dev/null\n"
    )
}

/// Parse the [`build_command`] output into a [`RepoContext`]. `folder` is
/// supplied by the caller (derived from the cwd, not the shell output).
pub(super) fn parse(output: &str, folder: Option<String>) -> RepoContext {
    let mut context = RepoContext {
        folder,
        ..Default::default()
    };

    let mut section = "";
    let mut branch_lines: Vec<&str> = Vec::new();
    let mut diff_line: Option<&str> = None;
    let mut pr_json = String::new();
    for line in output.lines() {
        match line.trim() {
            "@@BRANCH@@" => section = "branch",
            "@@DIFF@@" => section = "diff",
            "@@PR@@" => section = "pr",
            _ => match section {
                "branch" => branch_lines.push(line),
                "diff" if diff_line.is_none() && !line.trim().is_empty() => {
                    diff_line = Some(line)
                }
                "pr" => {
                    pr_json.push_str(line);
                    pr_json.push('\n');
                }
                _ => {}
            },
        }
    }

    context.branch = branch_lines
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && *line != "HEAD")
        .map(str::to_owned);

    if let Some(line) = diff_line {
        let (added, removed) = parse_shortstat(line);
        context.added = added;
        context.removed = removed;
    }

    // The PR JSON (when `gh` produced any) carries the PR's own diff, number,
    // and check rollup — preferred over the working-tree shortstat so the bar
    // matches the PR the way GitHub shows it.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(pr_json.trim()) {
        context.pr_number = value.get("number").and_then(|v| v.as_u64());
        if let Some(additions) = value.get("additions").and_then(|v| v.as_u64()) {
            context.added = Some(additions as usize);
        }
        if let Some(deletions) = value.get("deletions").and_then(|v| v.as_u64()) {
            context.removed = Some(deletions as usize);
        }
        context.ci = value
            .get("statusCheckRollup")
            .and_then(|v| v.as_array())
            .and_then(|checks| aggregate_ci(checks));
    }

    context
}

/// Pull insertion/deletion counts out of a `git diff --shortstat` line, e.g.
/// `" 3 files changed, 12 insertions(+), 4 deletions(-)"`. Either count may be
/// absent (an all-additions or all-deletions diff omits the other clause).
fn parse_shortstat(line: &str) -> (Option<usize>, Option<usize>) {
    let mut added = None;
    let mut removed = None;
    let tokens: Vec<&str> = line.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if token.starts_with("insertion") {
            added = index
                .checked_sub(1)
                .and_then(|i| tokens.get(i))
                .and_then(|n| n.parse().ok());
        } else if token.starts_with("deletion") {
            removed = index
                .checked_sub(1)
                .and_then(|i| tokens.get(i))
                .and_then(|n| n.parse().ok());
        }
    }
    (added, removed)
}

/// Collapse `gh`'s `statusCheckRollup` array into one [`CiState`]. The array
/// mixes `CheckRun` items (`status` + `conclusion`) and `StatusContext` items
/// (`state`); any failure wins, then any still-running, else passing. An empty
/// array (no checks) yields `None`.
fn aggregate_ci(checks: &[serde_json::Value]) -> Option<CiState> {
    if checks.is_empty() {
        return None;
    }
    let mut any_pending = false;
    for check in checks {
        let conclusion = check
            .get("conclusion")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_uppercase);
        let state = check
            .get("state")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_uppercase);
        let status = check
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_uppercase);

        let verdict = conclusion.as_deref().or(state.as_deref());
        match verdict {
            Some("FAILURE" | "TIMED_OUT" | "ERROR" | "CANCELLED" | "STARTUP_FAILURE"
            | "ACTION_REQUIRED") => return Some(CiState::Failing),
            Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => {}
            // No verdict yet, or an in-progress status — still pending.
            _ => {
                let completed = status.as_deref() == Some("COMPLETED");
                if !completed {
                    any_pending = true;
                }
            }
        }
        if status.as_deref().is_some_and(|s| s != "COMPLETED") {
            any_pending = true;
        }
    }
    Some(if any_pending {
        CiState::Pending
    } else {
        CiState::Passing
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_diff_and_pr_with_ci() {
        let output = "@@BRANCH@@\nfeature/x\n@@DIFF@@\n 2 files changed, 12 insertions(+), 4 deletions(-)\n@@PR@@\n{\"number\":86,\"additions\":748,\"deletions\":10,\"statusCheckRollup\":[{\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}]}\n";
        let context = parse(output, Some("twarp".to_owned()));
        assert_eq!(context.folder.as_deref(), Some("twarp"));
        assert_eq!(context.branch.as_deref(), Some("feature/x"));
        // PR diff overrides the working-tree shortstat.
        assert_eq!(context.added, Some(748));
        assert_eq!(context.removed, Some(10));
        assert_eq!(context.pr_number, Some(86));
        assert_eq!(context.ci, Some(CiState::Passing));
    }

    #[test]
    fn working_tree_diff_when_no_pr() {
        let output = "@@BRANCH@@\nmain\n@@DIFF@@\n 1 file changed, 5 insertions(+)\n@@PR@@\n";
        let context = parse(output, Some("twarp".to_owned()));
        assert_eq!(context.branch.as_deref(), Some("main"));
        assert_eq!(context.added, Some(5));
        assert_eq!(context.removed, None);
        assert_eq!(context.pr_number, None);
        assert_eq!(context.ci, None);
    }

    #[test]
    fn pending_and_failing_ci_aggregate() {
        let pending = vec![serde_json::json!({"status":"IN_PROGRESS","conclusion":null})];
        assert_eq!(aggregate_ci(&pending), Some(CiState::Pending));
        let failing = vec![
            serde_json::json!({"status":"COMPLETED","conclusion":"SUCCESS"}),
            serde_json::json!({"status":"COMPLETED","conclusion":"FAILURE"}),
        ];
        assert_eq!(aggregate_ci(&failing), Some(CiState::Failing));
        assert_eq!(aggregate_ci(&[]), None);
    }

    #[test]
    fn non_repo_cwd_yields_only_folder() {
        let output = "@@BRANCH@@\n@@DIFF@@\n@@PR@@\n";
        let context = parse(output, Some("tmp".to_owned()));
        assert_eq!(context.folder.as_deref(), Some("tmp"));
        assert!(context.is_effectively_empty());
    }
}
