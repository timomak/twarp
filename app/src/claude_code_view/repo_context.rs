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

/// One individual status check on the branch's PR (#11), shown as a row in the
/// CI menu. `url` opens the check's run page on GitHub when present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CiCheck {
    pub name: String,
    pub state: CiState,
    pub url: Option<String>,
}

/// The resolved context shown in the composer bar (#11). Every field is
/// independently optional — a non-repo cwd still yields a `folder`.
#[derive(Clone, Debug, Default)]
pub(super) struct RepoContext {
    pub folder: Option<String>,
    pub branch: Option<String>,
    /// The repo's default branch (`main`/`master`/…), from `origin/HEAD`. Used
    /// as the branch fallback in a detached HEAD and offered in the switch menu.
    pub default_branch: Option<String>,
    /// Local branch names, most-recently-committed first — the branch menu's
    /// switch list.
    pub branches: Vec<String>,
    /// The repo's GitHub web URL (`https://github.com/owner/repo`), derived from
    /// `origin`. Lets the branch menu open `…/tree/<branch>`.
    pub repo_web_url: Option<String>,
    pub added: Option<usize>,
    pub removed: Option<usize>,
    pub pr_number: Option<u64>,
    /// The PR's own web URL, when one exists.
    pub pr_url: Option<String>,
    pub ci: Option<CiState>,
    /// Per-check CI detail for the CI menu (name · state · run URL).
    pub ci_checks: Vec<CiCheck>,
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

    /// The GitHub URL for the current branch (`…/tree/<branch>`), if both the
    /// repo URL and a branch resolved.
    pub(super) fn branch_web_url(&self) -> Option<String> {
        let base = self.repo_web_url.as_deref()?;
        let branch = self.branch.as_deref()?;
        Some(format!("{base}/tree/{branch}"))
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
         echo '@@DEFAULT@@'; git symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null\n\
         echo '@@BRANCHES@@'; git branch --format='%(refname:short)' --sort=-committerdate 2>/dev/null\n\
         echo '@@REMOTE@@'; git remote get-url origin 2>/dev/null\n\
         echo '@@DIFF@@'; git diff --shortstat 2>/dev/null\n\
         echo '@@PR@@'; gh pr view --json number,additions,deletions,url,statusCheckRollup 2>/dev/null\n"
    )
}

/// Convert an `origin` remote URL into its GitHub web URL, e.g.
/// `git@github.com:owner/repo.git` / `https://github.com/owner/repo.git` →
/// `https://github.com/owner/repo`. Returns `None` for unrecognised hosts.
fn remote_to_web_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let stripped = remote.strip_suffix(".git").unwrap_or(remote);
    if let Some(rest) = stripped.strip_prefix("git@") {
        // `git@host:owner/repo` → `https://host/owner/repo`
        let (host, path) = rest.split_once(':')?;
        Some(format!("https://{host}/{path}"))
    } else if stripped.starts_with("https://") || stripped.starts_with("http://") {
        Some(stripped.to_owned())
    } else if let Some(rest) = stripped.strip_prefix("ssh://git@") {
        // `ssh://git@host/owner/repo`
        Some(format!("https://{rest}"))
    } else {
        None
    }
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
    let mut default_lines: Vec<&str> = Vec::new();
    let mut branch_list: Vec<String> = Vec::new();
    let mut remote_line: Option<&str> = None;
    let mut diff_line: Option<&str> = None;
    let mut pr_json = String::new();
    for line in output.lines() {
        match line.trim() {
            "@@BRANCH@@" => section = "branch",
            "@@DEFAULT@@" => section = "default",
            "@@BRANCHES@@" => section = "branches",
            "@@REMOTE@@" => section = "remote",
            "@@DIFF@@" => section = "diff",
            "@@PR@@" => section = "pr",
            _ => match section {
                "branch" => branch_lines.push(line),
                "default" => default_lines.push(line),
                "branches" if !line.trim().is_empty() => branch_list.push(line.trim().to_owned()),
                "remote" if remote_line.is_none() && !line.trim().is_empty() => {
                    remote_line = Some(line)
                }
                "diff" if diff_line.is_none() && !line.trim().is_empty() => diff_line = Some(line),
                "pr" => {
                    pr_json.push_str(line);
                    pr_json.push('\n');
                }
                _ => {}
            },
        }
    }

    // `origin/HEAD` resolves to `origin/main`; keep just the branch name.
    context.default_branch = default_lines
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(|line| line.strip_prefix("origin/").unwrap_or(line).to_owned());
    context.branches = branch_list;
    context.repo_web_url = remote_line.and_then(remote_to_web_url);

    // The checked-out branch, falling back to the repo's default branch on a
    // detached HEAD so the bar still shows a meaningful ref (#11, "show master
    // even on the default branch").
    context.branch = branch_lines
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && *line != "HEAD")
        .map(str::to_owned)
        .or_else(|| context.default_branch.clone());

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
        context.pr_url = value.get("url").and_then(|v| v.as_str()).map(str::to_owned);
        if let Some(additions) = value.get("additions").and_then(|v| v.as_u64()) {
            context.added = Some(additions as usize);
        }
        if let Some(deletions) = value.get("deletions").and_then(|v| v.as_u64()) {
            context.removed = Some(deletions as usize);
        }
        if let Some(checks) = value.get("statusCheckRollup").and_then(|v| v.as_array()) {
            context.ci = aggregate_ci(checks);
            context.ci_checks = checks.iter().filter_map(parse_check).collect();
        }
    }

    context
}

/// Pull one [`CiCheck`] out of a `statusCheckRollup` entry. Handles both
/// `CheckRun` items (`name` + `detailsUrl`) and `StatusContext` items
/// (`context` + `targetUrl`). Returns `None` for entries with no recognisable
/// name.
fn parse_check(check: &serde_json::Value) -> Option<CiCheck> {
    let name = check
        .get("name")
        .or_else(|| check.get("context"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_owned();
    let url = check
        .get("detailsUrl")
        .or_else(|| check.get("targetUrl"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(CiCheck {
        name,
        state: classify_check(check),
        url,
    })
}

/// Classify a single check into a [`CiState`]: a terminal failure conclusion is
/// `Failing`, a clean terminal conclusion is `Passing`, anything not yet
/// completed is `Pending`.
fn classify_check(check: &serde_json::Value) -> CiState {
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
    match conclusion.as_deref().or(state.as_deref()) {
        Some(
            "FAILURE" | "TIMED_OUT" | "ERROR" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED",
        ) => CiState::Failing,
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => CiState::Passing,
        _ if status.as_deref() == Some("COMPLETED") => CiState::Passing,
        _ => CiState::Pending,
    }
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
    // Any failure wins, then any still-running, else passing.
    let states = checks.iter().map(classify_check);
    let mut any_pending = false;
    for state in states {
        match state {
            CiState::Failing => return Some(CiState::Failing),
            CiState::Pending => any_pending = true,
            CiState::Passing => {}
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

    #[test]
    fn parses_branches_default_and_remote() {
        let output = "@@BRANCH@@\nmaster\n@@DEFAULT@@\norigin/master\n@@BRANCHES@@\nmaster\nfeature/x\nfix/y\n@@REMOTE@@\ngit@github.com:timomak/twarp.git\n@@DIFF@@\n@@PR@@\n";
        let context = parse(output, Some("twarp".to_owned()));
        assert_eq!(context.branch.as_deref(), Some("master"));
        assert_eq!(context.default_branch.as_deref(), Some("master"));
        assert_eq!(context.branches, vec!["master", "feature/x", "fix/y"]);
        assert_eq!(
            context.repo_web_url.as_deref(),
            Some("https://github.com/timomak/twarp")
        );
        assert_eq!(
            context.branch_web_url().as_deref(),
            Some("https://github.com/timomak/twarp/tree/master")
        );
    }

    #[test]
    fn detached_head_falls_back_to_default_branch() {
        // `git rev-parse --abbrev-ref HEAD` prints `HEAD` when detached.
        let output = "@@BRANCH@@\nHEAD\n@@DEFAULT@@\norigin/main\n@@BRANCHES@@\nmain\n@@REMOTE@@\nhttps://github.com/o/r.git\n@@DIFF@@\n@@PR@@\n";
        let context = parse(output, None);
        assert_eq!(context.branch.as_deref(), Some("main"));
    }

    #[test]
    fn parses_pr_url_and_individual_checks() {
        let output = "@@BRANCH@@\nfeat/x\n@@DIFF@@\n@@PR@@\n{\"number\":12,\"url\":\"https://github.com/o/r/pull/12\",\"additions\":3,\"deletions\":1,\"statusCheckRollup\":[{\"name\":\"build\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\",\"detailsUrl\":\"https://ci/1\"},{\"context\":\"lint\",\"state\":\"FAILURE\",\"targetUrl\":\"https://ci/2\"}]}\n";
        let context = parse(output, None);
        assert_eq!(
            context.pr_url.as_deref(),
            Some("https://github.com/o/r/pull/12")
        );
        assert_eq!(context.ci, Some(CiState::Failing));
        assert_eq!(context.ci_checks.len(), 2);
        assert_eq!(context.ci_checks[0].name, "build");
        assert_eq!(context.ci_checks[0].state, CiState::Passing);
        assert_eq!(context.ci_checks[0].url.as_deref(), Some("https://ci/1"));
        assert_eq!(context.ci_checks[1].name, "lint");
        assert_eq!(context.ci_checks[1].state, CiState::Failing);
        assert_eq!(context.ci_checks[1].url.as_deref(), Some("https://ci/2"));
    }

    #[test]
    fn remote_url_variants() {
        assert_eq!(
            remote_to_web_url("git@github.com:o/r.git"),
            Some("https://github.com/o/r".to_owned())
        );
        assert_eq!(
            remote_to_web_url("https://github.com/o/r.git"),
            Some("https://github.com/o/r".to_owned())
        );
        assert_eq!(
            remote_to_web_url("ssh://git@github.com/o/r.git"),
            Some("https://github.com/o/r".to_owned())
        );
        assert_eq!(remote_to_web_url("/local/path"), None);
    }
}
