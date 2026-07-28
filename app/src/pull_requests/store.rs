//! twarp 21a: the Pull Requests store — a singleton model holding a per-repo
//! cache of GitHub pull requests fetched with the `gh` CLI. All subprocess
//! work runs on the background executor (mirroring
//! [`crate::skills_store::SkillsStoreModel`]); results stream back over a
//! channel and are applied on the foreground.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::code_review::github_author::parse_github_origin;

/// How many PRs one fetch requests from `gh pr list`.
const PR_LIST_LIMIT: u32 = 50;

/// The `--json` fields requested from `gh pr list`.
const PR_LIST_JSON_FIELDS: &str = "number,title,author,isDraft,state,reviewDecision,mergeable,updatedAt,url,headRefName,statusCheckRollup";

/// The state filter shown in the page header. `Draft` is `--state open`
/// narrowed client-side to draft PRs (gh has no draft state).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum PrStateFilter {
    #[default]
    Open,
    Closed,
    Draft,
    All,
}

impl PrStateFilter {
    pub const ALL: [PrStateFilter; 4] = [
        PrStateFilter::Open,
        PrStateFilter::Closed,
        PrStateFilter::Draft,
        PrStateFilter::All,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrStateFilter::Open => "Open",
            PrStateFilter::Closed => "Closed",
            PrStateFilter::Draft => "Draft",
            PrStateFilter::All => "All",
        }
    }

    /// Stable string used as the dropdown-item action payload.
    pub fn as_str(self) -> &'static str {
        match self {
            PrStateFilter::Open => "open",
            PrStateFilter::Closed => "closed",
            PrStateFilter::Draft => "draft",
            PrStateFilter::All => "all",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }

    /// The `gh pr list --state` value backing this filter.
    fn gh_state(self) -> &'static str {
        match self {
            PrStateFilter::Open | PrStateFilter::Draft => "open",
            PrStateFilter::Closed => "closed",
            PrStateFilter::All => "all",
        }
    }
}

/// Aggregated CI state of a PR's `statusCheckRollup` (local copy of the
/// `claude_code_view::repo_context` aggregation — that one is module-private).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrCiState {
    Passing,
    Failing,
    Pending,
}

/// The review decision reported by GitHub for a PR.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

impl PrReviewDecision {
    pub fn label(self) -> &'static str {
        match self {
            PrReviewDecision::Approved => "Approved",
            PrReviewDecision::ChangesRequested => "Changes requested",
            PrReviewDecision::ReviewRequired => "Review required",
        }
    }
}

/// One pull request row, parsed out of `gh pr list --json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrEntry {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub is_draft: bool,
    /// `OPEN` / `CLOSED` / `MERGED`.
    pub state: String,
    pub review_decision: Option<PrReviewDecision>,
    /// True iff GitHub reports `mergeable == "CONFLICTING"`.
    pub conflicting: bool,
    /// Raw `updatedAt` RFC 3339 timestamp (may be empty).
    pub updated_at: String,
    pub url: String,
    pub head_ref: String,
    pub ci: Option<PrCiState>,
}

/// Cached fetch state for one repo.
#[derive(Clone, Debug, Default)]
pub struct RepoPrData {
    pub prs: Vec<PrEntry>,
    pub error: Option<String>,
    pub loading: bool,
    /// True once at least one fetch has completed (success or error).
    pub fetched: bool,
}

/// One background fetch's result, streamed back to the model.
struct FetchResult {
    repo: PathBuf,
    /// The [`PullRequestsStoreModel::generation`] this fetch was issued for.
    /// Stale results (a newer fetch superseded this one) are dropped rather
    /// than applied against a selection/filter they no longer match.
    generation: u64,
    /// `Some` when this fetch also resolved the viewer login.
    viewer: Option<String>,
    outcome: Result<Vec<PrEntry>, String>,
}

/// Singleton owning the PR cache, the project list shown in the page's repo
/// picker, and the current selection/filter.
pub struct PullRequestsStoreModel {
    projects: Vec<PathBuf>,
    selected: Option<PathBuf>,
    filter: PrStateFilter,
    /// The `gh api user` login, used for the "Yours"/"Others" grouping.
    viewer: Option<String>,
    data: HashMap<PathBuf, RepoPrData>,
    /// Bumped on every [`Self::refresh`]; only a [`FetchResult`] carrying the
    /// latest generation is applied, so selection/filter changes made while a
    /// fetch is in flight always win.
    generation: u64,
    fetch_tx: async_channel::Sender<FetchResult>,
}

impl PullRequestsStoreModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (fetch_tx, fetch_rx) = async_channel::unbounded::<FetchResult>();
        let _ = ctx.spawn_stream_local(
            fetch_rx,
            |model: &mut Self, result, ctx| model.apply_fetch(result, ctx),
            |_, _| {},
        );
        Self {
            projects: Vec::new(),
            selected: None,
            filter: PrStateFilter::default(),
            viewer: None,
            data: HashMap::new(),
            generation: 0,
            fetch_tx,
        }
    }

    pub fn projects(&self) -> &[PathBuf] {
        &self.projects
    }

    pub fn selected_repo(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    pub fn filter(&self) -> PrStateFilter {
        self.filter
    }

    pub fn viewer(&self) -> Option<&str> {
        self.viewer.as_deref()
    }

    /// Cached data for the selected repo, if any fetch has been started.
    pub fn selected_data(&self) -> Option<&RepoPrData> {
        self.data.get(self.selected.as_ref()?)
    }

    /// Called by the workspace when the page opens: the known project roots
    /// plus the preferred default (the active tab's repo). Always kicks off a
    /// refresh of the selection so the page opens with fresh data.
    pub fn set_projects(
        &mut self,
        projects: Vec<PathBuf>,
        default: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.projects = projects;
        let keep = |sel: &PathBuf, list: &[PathBuf]| list.contains(sel);
        let selected = default
            .filter(|d| keep(d, &self.projects))
            .or_else(|| self.selected.clone().filter(|s| keep(s, &self.projects)))
            .or_else(|| self.projects.first().cloned());
        self.selected = selected;
        self.refresh(ctx);
    }

    pub fn select_repo(&mut self, repo: PathBuf, ctx: &mut ModelContext<Self>) {
        if self.selected.as_ref() == Some(&repo) {
            return;
        }
        self.selected = Some(repo);
        self.refresh(ctx);
    }

    pub fn set_filter(&mut self, filter: PrStateFilter, ctx: &mut ModelContext<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.refresh(ctx);
    }

    /// Re-fetch the selected repo's PR list on the background executor. Always
    /// issues a new fetch — a refresh (or selection/filter change) while one
    /// is already in flight supersedes it: the generation bump makes the older
    /// result stale, and [`Self::apply_fetch`] drops it on arrival.
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(repo) = self.selected.clone() else {
            ctx.notify();
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.data.entry(repo.clone()).or_default().loading = true;
        ctx.notify();

        // Best-effort viewer lookup; retried on every refresh until it lands.
        let need_viewer = self.viewer.is_none();
        let filter = self.filter;
        let tx = self.fetch_tx.clone();
        ctx.background_executor()
            .spawn(async move {
                let viewer = need_viewer.then(|| fetch_viewer_login(&repo)).flatten();
                let outcome = fetch_pr_list(&repo, filter);
                let _ = tx
                    .send(FetchResult {
                        repo,
                        generation,
                        viewer,
                        outcome,
                    })
                    .await;
            })
            .detach();
    }

    fn apply_fetch(&mut self, result: FetchResult, ctx: &mut ModelContext<Self>) {
        // The viewer login is selection/filter-independent — keep it even
        // from a superseded fetch.
        if let Some(viewer) = result.viewer {
            self.viewer = Some(viewer);
        }
        if result.generation != self.generation {
            // Superseded: a newer fetch (different filter/repo, or just a
            // fresher refresh) is in flight and will clear `loading` when it
            // arrives.
            ctx.notify();
            return;
        }
        let entry = self.data.entry(result.repo).or_default();
        entry.loading = false;
        entry.fetched = true;
        match result.outcome {
            Ok(prs) => {
                entry.prs = prs;
                entry.error = None;
            }
            Err(error) => entry.error = Some(error),
        }
        ctx.notify();
    }
}

impl Entity for PullRequestsStoreModel {
    type Event = ();
}

impl SingletonEntity for PullRequestsStoreModel {}

/// Run a command in `cwd`, returning trimmed stdout on success and a
/// human-readable error otherwise. Blocking — background executor only.
fn run_in_repo(repo: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                format!("`{program}` was not found on your PATH.")
            } else {
                format!("failed to run `{program}`: {err}")
            }
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("`{program}` exited with {}", output.status)
        } else {
            stderr.to_owned()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Resolve the repo's ORIGIN remote (never upstream) to `owner/repo`.
fn github_slug(repo: &Path) -> Result<String, String> {
    let url = run_in_repo(repo, "git", &["remote", "get-url", "origin"])
        .map_err(|err| format!("Could not read the origin remote: {err}"))?;
    let (owner, name) = parse_github_origin(&url)
        .ok_or_else(|| format!("The origin remote is not a GitHub repository ({url})."))?;
    Ok(format!("{owner}/{name}"))
}

/// Blocking `gh pr list` fetch + parse. Background executor only.
fn fetch_pr_list(repo: &Path, filter: PrStateFilter) -> Result<Vec<PrEntry>, String> {
    let slug = github_slug(repo)?;
    let limit = PR_LIST_LIMIT.to_string();
    let json = run_in_repo(
        repo,
        "gh",
        &[
            "pr",
            "list",
            "--repo",
            &slug,
            "--state",
            filter.gh_state(),
            "--json",
            PR_LIST_JSON_FIELDS,
            "--limit",
            &limit,
        ],
    )?;
    let prs = parse_pr_list(&json)?;
    Ok(filter_entries(prs, filter))
}

/// Blocking `gh api user` lookup of the signed-in login. Best-effort: any
/// failure just disables the "Yours"/"Others" grouping.
fn fetch_viewer_login(repo: &Path) -> Option<String> {
    run_in_repo(repo, "gh", &["api", "user", "--jq", ".login"])
        .ok()
        .filter(|login| !login.is_empty())
}

/// Parse `gh pr list --json` output into row entries.
pub(crate) fn parse_pr_list(json: &str) -> Result<Vec<PrEntry>, String> {
    let values: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|err| format!("Could not parse `gh pr list` output: {err}"))?;
    Ok(values.iter().map(parse_pr_entry).collect())
}

fn parse_pr_entry(value: &serde_json::Value) -> PrEntry {
    let str_field = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let checks = value
        .get("statusCheckRollup")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    PrEntry {
        number: value.get("number").and_then(|v| v.as_u64()).unwrap_or(0),
        title: str_field("title"),
        author: value
            .get("author")
            .and_then(|a| a.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
        is_draft: value
            .get("isDraft")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        state: str_field("state"),
        review_decision: parse_review_decision(
            value.get("reviewDecision").and_then(|v| v.as_str()),
        ),
        conflicting: value.get("mergeable").and_then(|v| v.as_str()) == Some("CONFLICTING"),
        updated_at: str_field("updatedAt"),
        url: str_field("url"),
        head_ref: str_field("headRefName"),
        ci: aggregate_ci(checks),
    }
}

fn parse_review_decision(raw: Option<&str>) -> Option<PrReviewDecision> {
    match raw? {
        "APPROVED" => Some(PrReviewDecision::Approved),
        "CHANGES_REQUESTED" => Some(PrReviewDecision::ChangesRequested),
        "REVIEW_REQUIRED" => Some(PrReviewDecision::ReviewRequired),
        _ => None,
    }
}

/// Apply the client-side part of a filter (`Draft` narrows open PRs).
pub(crate) fn filter_entries(prs: Vec<PrEntry>, filter: PrStateFilter) -> Vec<PrEntry> {
    match filter {
        PrStateFilter::Draft => prs.into_iter().filter(|pr| pr.is_draft).collect(),
        _ => prs,
    }
}

/// v1 grouping: "Yours" = authored by the signed-in viewer, "Others" = the
/// rest. With no known viewer everything lands in "Others".
pub(crate) fn group_prs<'a>(
    prs: &'a [PrEntry],
    viewer: Option<&str>,
) -> (Vec<&'a PrEntry>, Vec<&'a PrEntry>) {
    match viewer {
        Some(viewer) if !viewer.is_empty() => prs.iter().partition(|pr| pr.author == viewer),
        _ => (Vec::new(), prs.iter().collect()),
    }
}

/// Classify one `statusCheckRollup` item (local copy of
/// `claude_code_view::repo_context::classify_check`): a terminal failure
/// conclusion is `Failing`, a clean terminal conclusion is `Passing`, anything
/// not yet completed is `Pending`.
fn classify_check(check: &serde_json::Value) -> PrCiState {
    let upper = |key: &str| {
        check
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_ascii_uppercase)
    };
    let conclusion = upper("conclusion");
    let state = upper("state");
    let status = upper("status");
    match conclusion.as_deref().or(state.as_deref()) {
        Some(
            "FAILURE" | "TIMED_OUT" | "ERROR" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED",
        ) => PrCiState::Failing,
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => PrCiState::Passing,
        _ if status.as_deref() == Some("COMPLETED") => PrCiState::Passing,
        _ => PrCiState::Pending,
    }
}

/// Collapse a `statusCheckRollup` array into one state: any failure wins, then
/// any still-running, else passing. No checks yields `None`.
fn aggregate_ci(checks: &[serde_json::Value]) -> Option<PrCiState> {
    if checks.is_empty() {
        return None;
    }
    let mut any_pending = false;
    for state in checks.iter().map(classify_check) {
        match state {
            PrCiState::Failing => return Some(PrCiState::Failing),
            PrCiState::Pending => any_pending = true,
            PrCiState::Passing => {}
        }
    }
    Some(if any_pending {
        PrCiState::Pending
    } else {
        PrCiState::Passing
    })
}

/// A coarse "3d ago"-style relative label for an RFC 3339 timestamp; empty on
/// unparseable input. Static render — no live-updating timer.
pub(crate) fn relative_updated_at(updated_at: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(updated_at) else {
        return String::new();
    };
    let secs = (now - then.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0);
    match secs {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        86_400..=2_591_999 => format!("{}d ago", secs / 86_400),
        _ => format!("{}mo ago", secs / 2_592_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> &'static str {
        r#"[
            {
                "number": 12,
                "title": "Fix the widget",
                "author": {"login": "timomak"},
                "isDraft": false,
                "state": "OPEN",
                "reviewDecision": "APPROVED",
                "mergeable": "MERGEABLE",
                "updatedAt": "2026-07-27T12:00:00Z",
                "url": "https://github.com/timomak/twarp/pull/12",
                "headRefName": "fix/widget",
                "statusCheckRollup": [
                    {"status": "COMPLETED", "conclusion": "SUCCESS"},
                    {"state": "SUCCESS"}
                ]
            },
            {
                "number": 13,
                "title": "Draft thing",
                "author": {"login": "someoneelse"},
                "isDraft": true,
                "state": "OPEN",
                "reviewDecision": "REVIEW_REQUIRED",
                "mergeable": "CONFLICTING",
                "updatedAt": "2026-07-20T12:00:00Z",
                "url": "https://github.com/timomak/twarp/pull/13",
                "headRefName": "draft/thing",
                "statusCheckRollup": [
                    {"status": "IN_PROGRESS"},
                    {"conclusion": "FAILURE"}
                ]
            }
        ]"#
    }

    #[test]
    fn parses_gh_pr_list_json() {
        let prs = parse_pr_list(sample_json()).unwrap();
        assert_eq!(prs.len(), 2);

        let first = &prs[0];
        assert_eq!(first.number, 12);
        assert_eq!(first.title, "Fix the widget");
        assert_eq!(first.author, "timomak");
        assert!(!first.is_draft);
        assert_eq!(first.review_decision, Some(PrReviewDecision::Approved));
        assert!(!first.conflicting);
        assert_eq!(first.head_ref, "fix/widget");
        assert_eq!(first.ci, Some(PrCiState::Passing));

        let second = &prs[1];
        assert!(second.is_draft);
        assert!(second.conflicting);
        assert_eq!(
            second.review_decision,
            Some(PrReviewDecision::ReviewRequired)
        );
        // A failure anywhere in the rollup wins over the in-progress check.
        assert_eq!(second.ci, Some(PrCiState::Failing));
    }

    #[test]
    fn parse_tolerates_missing_fields() {
        let prs = parse_pr_list(r#"[{"number": 7}]"#).unwrap();
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].author, "");
        assert_eq!(prs[0].review_decision, None);
        assert_eq!(prs[0].ci, None);
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_pr_list("gh: command failed").is_err());
    }

    #[test]
    fn draft_filter_is_client_side() {
        let prs = parse_pr_list(sample_json()).unwrap();
        let drafts = filter_entries(prs.clone(), PrStateFilter::Draft);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].number, 13);
        assert_eq!(filter_entries(prs, PrStateFilter::Open).len(), 2);
    }

    #[test]
    fn groups_by_viewer_login() {
        let prs = parse_pr_list(sample_json()).unwrap();
        let (yours, others) = group_prs(&prs, Some("timomak"));
        assert_eq!(yours.len(), 1);
        assert_eq!(yours[0].number, 12);
        assert_eq!(others.len(), 1);

        let (yours, others) = group_prs(&prs, None);
        assert!(yours.is_empty());
        assert_eq!(others.len(), 2);
    }

    #[test]
    fn ci_aggregation_pending_and_empty() {
        let checks = vec![serde_json::json!({"status": "IN_PROGRESS"})];
        assert_eq!(aggregate_ci(&checks), Some(PrCiState::Pending));
        assert_eq!(aggregate_ci(&[]), None);
    }

    #[test]
    fn relative_updated_at_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-28T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(relative_updated_at("2026-07-27T23:59:30Z", now), "just now");
        assert_eq!(relative_updated_at("2026-07-27T23:30:00Z", now), "30m ago");
        assert_eq!(relative_updated_at("2026-07-27T12:00:00Z", now), "12h ago");
        assert_eq!(relative_updated_at("2026-07-20T00:00:00Z", now), "8d ago");
        assert_eq!(relative_updated_at("2026-01-01T00:00:00Z", now), "6mo ago");
        assert_eq!(relative_updated_at("not-a-date", now), "");
    }

    #[test]
    fn filter_round_trip_and_gh_state() {
        for filter in PrStateFilter::ALL {
            assert_eq!(PrStateFilter::from_str(filter.as_str()), Some(filter));
        }
        assert_eq!(PrStateFilter::Draft.gh_state(), "open");
        assert_eq!(PrStateFilter::All.gh_state(), "all");
    }
}
