//! twarp 21a: the Pull Requests store — a singleton model holding a per-repo
//! cache of GitHub pull requests fetched with the `gh` CLI. All subprocess
//! work runs on the background executor (mirroring
//! [`crate::skills_store::SkillsStoreModel`]); results stream back over a
//! channel and are applied on the foreground.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::code_review::github_author::parse_github_origin;
use crate::pull_requests::diff::{parse_pr_diff, parse_review_threads, PrFileDiff, PrReviewThread};

/// How many PRs one fetch requests from `gh pr list`.
const PR_LIST_LIMIT: u32 = 50;

/// The `--json` fields requested from `gh pr list`.
const PR_LIST_JSON_FIELDS: &str = "number,title,author,isDraft,state,reviewDecision,mergeable,updatedAt,url,headRefName,statusCheckRollup";

/// The `--json` fields requested from `gh pr view` for the detail page (21b).
const PR_DETAIL_JSON_FIELDS: &str = "number,title,body,author,state,isDraft,mergeable,mergeStateStatus,reviewDecision,baseRefName,headRefName,additions,deletions,changedFiles,url,createdAt,statusCheckRollup";

/// How many timeline nodes (comments / reviews) one detail fetch requests.
/// Older items beyond this are dropped; the UI notes the truncation.
const TIMELINE_PAGE_SIZE: u32 = 50;

/// GraphQL query fetching the PR-level conversation timeline: issue comments
/// plus reviews (with their top-level bodies, states, and line-comment
/// counts — the latter lets 21c render "N comments on files" one-liners for
/// body-less COMMENTED reviews).
const TIMELINE_QUERY: &str = "\
query($owner: String!, $name: String!, $number: Int!, $last: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      comments(last: $last) {
        totalCount
        nodes { author { login } createdAt body }
      }
      reviews(last: $last) {
        totalCount
        nodes { author { login } createdAt body state comments { totalCount } }
      }
    }
  }
}";

/// How many review threads (and comments per thread) one Files fetch requests.
const REVIEW_THREADS_PAGE_SIZE: u32 = 100;
const THREAD_COMMENTS_PAGE_SIZE: u32 = 50;

/// GraphQL query fetching the line-anchored review threads for the Files tab
/// (21c).
const REVIEW_THREADS_QUERY: &str = "\
query($owner: String!, $name: String!, $number: Int!, $first: Int!, $comments: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: $first) {
        totalCount
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          startLine
          diffSide
          comments(first: $comments) {
            nodes { author { login } createdAt body }
          }
        }
      }
    }
  }
}";

/// GraphQL mutation replying to one review thread by node id (21d).
const THREAD_REPLY_MUTATION: &str = "\
mutation($threadId: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $threadId, body: $body}) {
    comment { id }
  }
}";

/// GraphQL mutations resolving / unresolving one review thread (21d).
const RESOLVE_THREAD_MUTATION: &str = "\
mutation($threadId: ID!) {
  resolveReviewThread(input: {threadId: $threadId}) { thread { id } }
}";
const UNRESOLVE_THREAD_MUTATION: &str = "\
mutation($threadId: ID!) {
  unresolveReviewThread(input: {threadId: $threadId}) { thread { id } }
}";

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

/// A merge strategy for `gh pr merge` (21b merge box).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrMergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl PrMergeMethod {
    pub const ALL: [PrMergeMethod; 3] = [
        PrMergeMethod::Merge,
        PrMergeMethod::Squash,
        PrMergeMethod::Rebase,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrMergeMethod::Merge => "Merge",
            PrMergeMethod::Squash => "Squash",
            PrMergeMethod::Rebase => "Rebase",
        }
    }

    /// Stable string used as the action payload.
    pub fn as_str(self) -> &'static str {
        match self {
            PrMergeMethod::Merge => "merge",
            PrMergeMethod::Squash => "squash",
            PrMergeMethod::Rebase => "rebase",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// The `gh pr merge` flag backing this method.
    fn gh_flag(self) -> &'static str {
        match self {
            PrMergeMethod::Merge => "--merge",
            PrMergeMethod::Squash => "--squash",
            PrMergeMethod::Rebase => "--rebase",
        }
    }
}

/// One CI check row parsed from `statusCheckRollup` (21b Checks tab).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrCheck {
    pub name: String,
    pub state: PrCiState,
    /// Web URL for the check's details page (may be empty).
    pub details_url: String,
    /// Human "4m 12s" duration when start/completion timestamps exist.
    pub duration: Option<String>,
}

/// The full PR detail parsed from `gh pr view --json` (21b).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    /// Raw markdown description body.
    pub body: String,
    pub author: String,
    /// `OPEN` / `CLOSED` / `MERGED`.
    pub state: String,
    pub is_draft: bool,
    /// `MERGEABLE` / `CONFLICTING` / `UNKNOWN`.
    pub mergeable: String,
    /// `CLEAN` / `BLOCKED` / `BEHIND` / `DIRTY` / `UNSTABLE` / …
    pub merge_state_status: String,
    pub review_decision: Option<PrReviewDecision>,
    pub base_ref: String,
    pub head_ref: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub url: String,
    pub created_at: String,
    pub checks: Vec<PrCheck>,
}

/// The state of one PR review in the timeline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

impl PrReviewState {
    pub fn label(self) -> &'static str {
        match self {
            PrReviewState::Approved => "approved",
            PrReviewState::ChangesRequested => "requested changes",
            PrReviewState::Commented => "reviewed",
            PrReviewState::Dismissed => "review dismissed",
        }
    }
}

/// What kind of conversation entry a timeline item is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrTimelineKind {
    Comment,
    Review(PrReviewState),
}

/// One PR-level conversation item (issue comment or review body), 21b.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrTimelineItem {
    pub author: String,
    /// Raw RFC 3339 timestamp (sorts chronologically as a string).
    pub created_at: String,
    /// Markdown body; may be empty for state-only reviews (Approved etc).
    pub body: String,
    pub kind: PrTimelineKind,
    /// For reviews: how many line comments the review carries (Files tab).
    pub file_comments: u64,
}

/// Cached Files-tab state for the one open PR (21c): the parsed per-file
/// diff plus the line-anchored review threads.
#[derive(Clone, Debug, Default)]
pub struct PrFilesData {
    pub files: Vec<PrFileDiff>,
    pub threads: Vec<PrReviewThread>,
    /// True when GitHub reported more review threads than one page holds.
    pub threads_truncated: bool,
    pub error: Option<String>,
    pub loading: bool,
    /// True once at least one files fetch completed (success or error).
    pub fetched: bool,
}

/// Cached detail-fetch state for the one open PR.
#[derive(Clone, Debug, Default)]
pub struct PrDetailData {
    pub detail: Option<PrDetail>,
    pub timeline: Vec<PrTimelineItem>,
    /// True when GitHub reported more comments/reviews than one page holds.
    pub timeline_truncated: bool,
    pub error: Option<String>,
    pub loading: bool,
    /// True once at least one detail fetch completed (success or error).
    pub fetched: bool,
    /// True while a merge / mark-ready subprocess is running.
    pub mutating: bool,
    /// The last mutation's gh error output, if it failed.
    pub mutation_error: Option<String>,
    /// The Files tab's diff + review threads (fetched on first tab open).
    pub files: PrFilesData,
}

/// A successful detail fetch's payload.
struct DetailPayload {
    detail: PrDetail,
    timeline: Vec<PrTimelineItem>,
    truncated: bool,
}

/// A successful Files (diff + threads) fetch's payload.
struct FilesPayload {
    files: Vec<PrFileDiff>,
    threads: Vec<PrReviewThread>,
    threads_truncated: bool,
}

/// A background detail-fetch or mutation result, streamed back to the model.
enum DetailMessage {
    Fetch {
        repo: PathBuf,
        number: u64,
        generation: u64,
        outcome: Result<DetailPayload, String>,
    },
    Files {
        repo: PathBuf,
        number: u64,
        generation: u64,
        outcome: Result<FilesPayload, String>,
    },
    Mutation {
        repo: PathBuf,
        number: u64,
        outcome: Result<(), String>,
    },
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
    /// The one open detail view's data, keyed by (repo, PR number). Only one
    /// PR detail is open at a time (list ↔ detail navigation is page-level).
    detail: Option<((PathBuf, u64), PrDetailData)>,
    /// Bumped on every detail fetch/close so stale results are dropped.
    detail_generation: u64,
    /// Bumped on every Files fetch/close; separate from `detail_generation`
    /// so a detail refresh doesn't invalidate an in-flight Files fetch.
    files_generation: u64,
    detail_tx: async_channel::Sender<DetailMessage>,
}

impl PullRequestsStoreModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (fetch_tx, fetch_rx) = async_channel::unbounded::<FetchResult>();
        let _ = ctx.spawn_stream_local(
            fetch_rx,
            |model: &mut Self, result, ctx| model.apply_fetch(result, ctx),
            |_, _| {},
        );
        let (detail_tx, detail_rx) = async_channel::unbounded::<DetailMessage>();
        let _ = ctx.spawn_stream_local(
            detail_rx,
            |model: &mut Self, message, ctx| model.apply_detail_message(message, ctx),
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
            detail: None,
            detail_generation: 0,
            files_generation: 0,
            detail_tx,
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

    /// Detail data for `number` in the currently selected repo, if a detail
    /// fetch has been started for it.
    pub fn detail_data(&self, number: u64) -> Option<&PrDetailData> {
        let ((repo, n), data) = self.detail.as_ref()?;
        (Some(repo.as_path()) == self.selected_repo() && *n == number).then_some(data)
    }

    /// Drop the open detail (back-to-list). In-flight results become stale.
    pub fn close_detail(&mut self, ctx: &mut ModelContext<Self>) {
        self.detail = None;
        self.detail_generation += 1;
        self.files_generation += 1;
        ctx.notify();
    }

    /// Fetch the Files-tab data (diff + review threads) if it hasn't been
    /// fetched or started yet. Called when the Files tab opens.
    pub fn ensure_files(&mut self, number: u64, ctx: &mut ModelContext<Self>) {
        let up_to_date = self
            .detail_data(number)
            .is_some_and(|data| data.files.fetched || data.files.loading);
        if !up_to_date {
            self.fetch_files(number, ctx);
        }
    }

    /// (Re-)fetch the PR diff + review threads on the background executor.
    pub fn fetch_files(&mut self, number: u64, ctx: &mut ModelContext<Self>) {
        let Some(repo) = self.selected.clone() else {
            return;
        };
        let key = (repo.clone(), number);
        let Some((existing, data)) = self.detail.as_mut() else {
            return;
        };
        if *existing != key {
            return;
        }
        data.files.loading = true;
        self.files_generation += 1;
        let generation = self.files_generation;
        ctx.notify();

        let tx = self.detail_tx.clone();
        ctx.background_executor()
            .spawn(async move {
                let outcome = fetch_pr_files(&repo, number);
                let _ = tx
                    .send(DetailMessage::Files {
                        repo,
                        number,
                        generation,
                        outcome,
                    })
                    .await;
            })
            .detach();
    }

    /// (Re-)fetch the detail + timeline for `number` in the selected repo on
    /// the background executor. Opening a different PR replaces the cached
    /// detail; refreshing the same PR keeps stale data visible while loading.
    pub fn fetch_detail(&mut self, number: u64, ctx: &mut ModelContext<Self>) {
        let Some(repo) = self.selected.clone() else {
            return;
        };
        let key = (repo.clone(), number);
        match &mut self.detail {
            Some((existing, data)) if *existing == key => data.loading = true,
            _ => {
                self.detail = Some((
                    key,
                    PrDetailData {
                        loading: true,
                        ..Default::default()
                    },
                ));
            }
        }
        self.detail_generation += 1;
        let generation = self.detail_generation;
        ctx.notify();

        let tx = self.detail_tx.clone();
        ctx.background_executor()
            .spawn(async move {
                let outcome = fetch_pr_detail(&repo, number);
                let _ = tx
                    .send(DetailMessage::Fetch {
                        repo,
                        number,
                        generation,
                        outcome,
                    })
                    .await;
            })
            .detach();
    }

    /// Run `gh pr merge` with the chosen strategy. On success the detail and
    /// the list both refetch; on failure gh's error message is surfaced.
    pub fn merge_pr(&mut self, number: u64, method: PrMergeMethod, ctx: &mut ModelContext<Self>) {
        self.run_mutation(
            number,
            move |repo, slug| {
                let n = number.to_string();
                run_in_repo(
                    repo,
                    "gh",
                    &["pr", "merge", &n, "--repo", slug, method.gh_flag()],
                )
                .map(|_| ())
            },
            ctx,
        );
    }

    /// Run `gh pr ready` to take a draft PR out of draft.
    pub fn mark_ready(&mut self, number: u64, ctx: &mut ModelContext<Self>) {
        self.run_mutation(
            number,
            move |repo, slug| {
                let n = number.to_string();
                run_in_repo(repo, "gh", &["pr", "ready", &n, "--repo", slug]).map(|_| ())
            },
            ctx,
        );
    }

    /// Post a PR-level comment (`gh pr comment --body-file -`, body piped via
    /// stdin so no shell-quoting issues). Returns true when the mutation was
    /// actually started (21d).
    pub fn comment_pr(&mut self, number: u64, body: String, ctx: &mut ModelContext<Self>) -> bool {
        self.run_mutation(
            number,
            move |repo, slug| {
                let n = number.to_string();
                run_in_repo_with_stdin(
                    repo,
                    "gh",
                    &["pr", "comment", &n, "--repo", slug, "--body-file", "-"],
                    &body,
                )
                .map(|_| ())
            },
            ctx,
        )
    }

    /// Reply to one inline review thread by GraphQL node id (21d). The id was
    /// fetched from the origin-slug reviewThreads query, so the write stays
    /// pinned to the fork.
    pub fn reply_thread(
        &mut self,
        number: u64,
        thread_id: String,
        body: String,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.run_mutation(
            number,
            move |repo, _slug| {
                run_graphql_mutation(
                    repo,
                    THREAD_REPLY_MUTATION,
                    &[("threadId", &thread_id), ("body", &body)],
                )
            },
            ctx,
        )
    }

    /// Resolve / unresolve one review thread by GraphQL node id (21d).
    pub fn set_thread_resolved(
        &mut self,
        number: u64,
        thread_id: String,
        resolved: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let mutation = if resolved {
            RESOLVE_THREAD_MUTATION
        } else {
            UNRESOLVE_THREAD_MUTATION
        };
        self.run_mutation(
            number,
            move |repo, _slug| run_graphql_mutation(repo, mutation, &[("threadId", &thread_id)]),
            ctx,
        )
    }

    /// Submit a batched review (verdict + summary + drafted line comments) as
    /// one REST call: `gh api repos/{slug}/pulls/{n}/reviews --input -` with
    /// the JSON payload (built by
    /// [`crate::pull_requests::review::build_review_payload`]) piped via
    /// stdin. Creates the review and its comments atomically (21d).
    pub fn submit_review(
        &mut self,
        number: u64,
        payload: String,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        self.run_mutation(
            number,
            move |repo, slug| {
                let endpoint = format!("repos/{slug}/pulls/{number}/reviews");
                run_in_repo_with_stdin(
                    repo,
                    "gh",
                    &["api", "--method", "POST", &endpoint, "--input", "-"],
                    &payload,
                )
                .map(|_| ())
            },
            ctx,
        )
    }

    /// Spawn one mutating gh call for the open detail PR on the background
    /// executor. `run` receives the repo path and its resolved origin slug
    /// (fork discipline: every mutating gh call pins `--repo <origin slug>`).
    /// Returns true when the mutation was actually started (false when
    /// another one is already in flight or the detail doesn't match).
    fn run_mutation(
        &mut self,
        number: u64,
        run: impl FnOnce(&Path, &str) -> Result<(), String> + Send + 'static,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(repo) = self.selected.clone() else {
            return false;
        };
        let Some((key, data)) = self.detail.as_mut() else {
            return false;
        };
        if *key != (repo.clone(), number) || data.mutating {
            return false;
        }
        data.mutating = true;
        data.mutation_error = None;
        ctx.notify();

        let tx = self.detail_tx.clone();
        ctx.background_executor()
            .spawn(async move {
                let outcome = github_slug(&repo).and_then(|slug| run(&repo, &slug));
                let _ = tx
                    .send(DetailMessage::Mutation {
                        repo,
                        number,
                        outcome,
                    })
                    .await;
            })
            .detach();
        true
    }

    fn apply_detail_message(&mut self, message: DetailMessage, ctx: &mut ModelContext<Self>) {
        match message {
            DetailMessage::Fetch {
                repo,
                number,
                generation,
                outcome,
            } => {
                if generation != self.detail_generation {
                    return; // Superseded (or the detail was closed).
                }
                let Some((key, data)) = self.detail.as_mut() else {
                    return;
                };
                if *key != (repo, number) {
                    return;
                }
                data.loading = false;
                data.fetched = true;
                match outcome {
                    Ok(payload) => {
                        data.detail = Some(payload.detail);
                        data.timeline = payload.timeline;
                        data.timeline_truncated = payload.truncated;
                        data.error = None;
                    }
                    Err(error) => data.error = Some(error),
                }
                ctx.notify();
            }
            DetailMessage::Files {
                repo,
                number,
                generation,
                outcome,
            } => {
                if generation != self.files_generation {
                    return; // Superseded (or the detail was closed).
                }
                let Some((key, data)) = self.detail.as_mut() else {
                    return;
                };
                if *key != (repo, number) {
                    return;
                }
                data.files.loading = false;
                data.files.fetched = true;
                match outcome {
                    Ok(payload) => {
                        data.files.files = payload.files;
                        data.files.threads = payload.threads;
                        data.files.threads_truncated = payload.threads_truncated;
                        data.files.error = None;
                    }
                    Err(error) => data.files.error = Some(error),
                }
                ctx.notify();
            }
            DetailMessage::Mutation {
                repo,
                number,
                outcome,
            } => {
                let Some((key, data)) = self.detail.as_mut() else {
                    return;
                };
                if *key != (repo, number) {
                    return;
                }
                data.mutating = false;
                let files_fetched = data.files.fetched;
                match outcome {
                    Ok(()) => {
                        // The world changed: refetch the detail, the list, and
                        // (if it was open) the Files tab's diff + threads.
                        self.fetch_detail(number, ctx);
                        if files_fetched {
                            self.fetch_files(number, ctx);
                        }
                        self.refresh(ctx);
                    }
                    Err(error) => data.mutation_error = Some(error),
                }
                ctx.notify();
            }
        }
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

/// Like [`run_in_repo`], but pipes `stdin` into the child. Blocking —
/// background executor only.
fn run_in_repo_with_stdin(
    repo: &Path,
    program: &str,
    args: &[&str],
    stdin: &str,
) -> Result<String, String> {
    use std::io::Write;
    let mut child = std::process::Command::new(program)
        .args(args)
        .current_dir(repo)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                format!("`{program}` was not found on your PATH.")
            } else {
                format!("failed to run `{program}`: {err}")
            }
        })?;
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(stdin.as_bytes())
            .map_err(|err| format!("failed to write to `{program}`: {err}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("failed to run `{program}`: {err}"))?;
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

/// Run one `gh api graphql` mutation with string variables. GraphQL errors
/// come back with exit status != 0 from gh, surfacing via stderr.
fn run_graphql_mutation(
    repo: &Path,
    mutation: &str,
    variables: &[(&str, &str)],
) -> Result<(), String> {
    let query = format!("query={mutation}");
    let mut args: Vec<String> = vec!["api".into(), "graphql".into(), "-f".into(), query];
    for (key, value) in variables {
        args.push("-f".into());
        args.push(format!("{key}={value}"));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_in_repo(repo, "gh", &arg_refs).map(|_| ())
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

/// Blocking detail + timeline fetch for one PR. Background executor only.
fn fetch_pr_detail(repo: &Path, number: u64) -> Result<DetailPayload, String> {
    let slug = github_slug(repo)?;
    let n = number.to_string();
    let json = run_in_repo(
        repo,
        "gh",
        &[
            "pr",
            "view",
            &n,
            "--repo",
            &slug,
            "--json",
            PR_DETAIL_JSON_FIELDS,
        ],
    )?;
    let detail = parse_pr_detail(&json)?;

    let (owner, name) = slug
        .split_once('/')
        .ok_or_else(|| format!("Unexpected repo slug: {slug}"))?;
    let query = format!("query={TIMELINE_QUERY}");
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={name}");
    let number_arg = format!("number={number}");
    let last_arg = format!("last={TIMELINE_PAGE_SIZE}");
    let timeline_json = run_in_repo(
        repo,
        "gh",
        &[
            "api",
            "graphql",
            "-f",
            &query,
            "-F",
            &owner_arg,
            "-F",
            &name_arg,
            "-F",
            &number_arg,
            "-F",
            &last_arg,
        ],
    )?;
    let (timeline, truncated) = parse_timeline(&timeline_json)?;
    Ok(DetailPayload {
        detail,
        timeline,
        truncated,
    })
}

/// Blocking Files-tab fetch for one PR: the unified diff (`gh pr diff`) plus
/// the line-anchored review threads (GraphQL). Background executor only.
fn fetch_pr_files(repo: &Path, number: u64) -> Result<FilesPayload, String> {
    let slug = github_slug(repo)?;
    let n = number.to_string();
    let diff = run_in_repo(repo, "gh", &["pr", "diff", &n, "--repo", &slug])?;
    let files = parse_pr_diff(&diff);

    let (owner, name) = slug
        .split_once('/')
        .ok_or_else(|| format!("Unexpected repo slug: {slug}"))?;
    let query = format!("query={REVIEW_THREADS_QUERY}");
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={name}");
    let number_arg = format!("number={number}");
    let first_arg = format!("first={REVIEW_THREADS_PAGE_SIZE}");
    let comments_arg = format!("comments={THREAD_COMMENTS_PAGE_SIZE}");
    let threads_json = run_in_repo(
        repo,
        "gh",
        &[
            "api",
            "graphql",
            "-f",
            &query,
            "-F",
            &owner_arg,
            "-F",
            &name_arg,
            "-F",
            &number_arg,
            "-F",
            &first_arg,
            "-F",
            &comments_arg,
        ],
    )?;
    let (threads, threads_truncated) = parse_review_threads(&threads_json)?;
    Ok(FilesPayload {
        files,
        threads,
        threads_truncated,
    })
}

/// Parse `gh pr view --json` output into a [`PrDetail`].
pub(crate) fn parse_pr_detail(json: &str) -> Result<PrDetail, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| format!("Could not parse `gh pr view` output: {err}"))?;
    let str_field = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let u64_field = |key: &str| value.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let checks = value
        .get("statusCheckRollup")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(parse_check_row)
        .collect();
    Ok(PrDetail {
        number: u64_field("number"),
        title: str_field("title"),
        body: str_field("body"),
        author: value
            .get("author")
            .and_then(|a| a.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
        state: str_field("state"),
        is_draft: value
            .get("isDraft")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        mergeable: str_field("mergeable"),
        merge_state_status: str_field("mergeStateStatus"),
        review_decision: parse_review_decision(
            value.get("reviewDecision").and_then(|v| v.as_str()),
        ),
        base_ref: str_field("baseRefName"),
        head_ref: str_field("headRefName"),
        additions: u64_field("additions"),
        deletions: u64_field("deletions"),
        changed_files: u64_field("changedFiles"),
        url: str_field("url"),
        created_at: str_field("createdAt"),
        checks,
    })
}

/// Parse one `statusCheckRollup` item into a check row. CheckRun items carry
/// `name`/`detailsUrl`/timestamps; classic StatusContext items carry
/// `context`/`targetUrl`.
fn parse_check_row(check: &serde_json::Value) -> PrCheck {
    let str_of = |key: &str| check.get(key).and_then(|v| v.as_str()).unwrap_or_default();
    let name = match str_of("name") {
        "" => str_of("context"),
        name => name,
    };
    let details_url = match str_of("detailsUrl") {
        "" => str_of("targetUrl"),
        url => url,
    };
    PrCheck {
        name: name.to_owned(),
        state: classify_check(check),
        details_url: details_url.to_owned(),
        duration: check_duration(str_of("startedAt"), str_of("completedAt")),
    }
}

/// "4m 12s"-style duration between two RFC 3339 timestamps, if both parse.
fn check_duration(started: &str, completed: &str) -> Option<String> {
    let start = chrono::DateTime::parse_from_rfc3339(started).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(completed).ok()?;
    let secs = (end - start).num_seconds();
    if secs < 0 {
        return None;
    }
    Some(if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    })
}

/// Parse the [`TIMELINE_QUERY`] response into oldest-first conversation items
/// plus a truncation flag. Pending reviews and empty `COMMENTED` reviews are
/// skipped; body-less Approved/Changes-requested reviews are kept (the state
/// itself is the content), and body-less `COMMENTED` reviews that carry line
/// comments are kept so the UI can link them to the Files tab (21c).
pub(crate) fn parse_timeline(json: &str) -> Result<(Vec<PrTimelineItem>, bool), String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| format!("Could not parse the timeline response: {err}"))?;
    let pr = value
        .pointer("/data/repository/pullRequest")
        .ok_or_else(|| "The timeline response has no pull request.".to_owned())?;

    let connection = |key: &str| {
        let nodes = pr
            .pointer(&format!("/{key}/nodes"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let total = pr
            .pointer(&format!("/{key}/totalCount"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let truncated = total > nodes.len() as u64;
        (nodes, truncated)
    };
    let node_common = |node: &serde_json::Value| {
        let author = node
            .pointer("/author/login")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let created_at = node
            .get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let body = node
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        (author, created_at, body)
    };

    let (comment_nodes, comments_truncated) = connection("comments");
    let (review_nodes, reviews_truncated) = connection("reviews");

    let mut items: Vec<PrTimelineItem> = comment_nodes
        .iter()
        .map(|node| {
            let (author, created_at, body) = node_common(node);
            PrTimelineItem {
                author,
                created_at,
                body,
                kind: PrTimelineKind::Comment,
                file_comments: 0,
            }
        })
        .collect();
    for node in &review_nodes {
        let state = match node.get("state").and_then(|v| v.as_str()).unwrap_or("") {
            "APPROVED" => PrReviewState::Approved,
            "CHANGES_REQUESTED" => PrReviewState::ChangesRequested,
            "COMMENTED" => PrReviewState::Commented,
            "DISMISSED" => PrReviewState::Dismissed,
            _ => continue, // PENDING and unknown states are not shown.
        };
        let (author, created_at, body) = node_common(node);
        let file_comments = node
            .pointer("/comments/totalCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if body.trim().is_empty() && state == PrReviewState::Commented && file_comments == 0 {
            // A body-less "commented" review with no line comments has
            // nothing to show.
            continue;
        }
        items.push(PrTimelineItem {
            author,
            created_at,
            body,
            kind: PrTimelineKind::Review(state),
            file_comments,
        });
    }
    // RFC 3339 UTC timestamps sort chronologically as strings.
    items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok((items, comments_truncated || reviews_truncated))
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
    fn parses_gh_pr_view_detail_json() {
        let json = r###"{
            "number": 42,
            "title": "Add the thing",
            "body": "## Summary\nDoes the thing.",
            "author": {"login": "timomak"},
            "state": "OPEN",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "reviewDecision": "APPROVED",
            "baseRefName": "master",
            "headRefName": "feat/thing",
            "additions": 120,
            "deletions": 7,
            "changedFiles": 5,
            "url": "https://github.com/timomak/twarp/pull/42",
            "createdAt": "2026-07-20T10:00:00Z",
            "statusCheckRollup": [
                {
                    "name": "build",
                    "status": "COMPLETED",
                    "conclusion": "SUCCESS",
                    "startedAt": "2026-07-20T10:00:00Z",
                    "completedAt": "2026-07-20T10:04:12Z",
                    "detailsUrl": "https://ci.example/build"
                },
                {
                    "context": "legacy-status",
                    "state": "FAILURE",
                    "targetUrl": "https://ci.example/legacy"
                }
            ]
        }"###;
        let detail = parse_pr_detail(json).unwrap();
        assert_eq!(detail.number, 42);
        assert_eq!(detail.body, "## Summary\nDoes the thing.");
        assert_eq!(detail.author, "timomak");
        assert_eq!(detail.mergeable, "MERGEABLE");
        assert_eq!(detail.merge_state_status, "CLEAN");
        assert_eq!(detail.base_ref, "master");
        assert_eq!(detail.head_ref, "feat/thing");
        assert_eq!(
            (detail.additions, detail.deletions, detail.changed_files),
            (120, 7, 5)
        );
        assert_eq!(detail.checks.len(), 2);

        let build = &detail.checks[0];
        assert_eq!(build.name, "build");
        assert_eq!(build.state, PrCiState::Passing);
        assert_eq!(build.details_url, "https://ci.example/build");
        assert_eq!(build.duration.as_deref(), Some("4m 12s"));

        // Classic StatusContext shape: `context`/`targetUrl`/`state`.
        let legacy = &detail.checks[1];
        assert_eq!(legacy.name, "legacy-status");
        assert_eq!(legacy.state, PrCiState::Failing);
        assert_eq!(legacy.details_url, "https://ci.example/legacy");
        assert_eq!(legacy.duration, None);
    }

    #[test]
    fn detail_parse_tolerates_missing_fields() {
        let detail = parse_pr_detail(r#"{"number": 3}"#).unwrap();
        assert_eq!(detail.number, 3);
        assert_eq!(detail.body, "");
        assert!(detail.checks.is_empty());
        assert!(parse_pr_detail("not json").is_err());
    }

    #[test]
    fn check_duration_buckets() {
        assert_eq!(
            check_duration("2026-07-20T10:00:00Z", "2026-07-20T10:00:45Z").as_deref(),
            Some("45s")
        );
        assert_eq!(
            check_duration("2026-07-20T10:00:00Z", "2026-07-20T11:30:00Z").as_deref(),
            Some("1h 30m")
        );
        // Reversed or unparseable timestamps yield no duration.
        assert_eq!(
            check_duration("2026-07-20T10:00:00Z", "2026-07-20T09:00:00Z"),
            None
        );
        assert_eq!(check_duration("", "2026-07-20T10:00:00Z"), None);
    }

    #[test]
    fn parses_graphql_timeline_oldest_first() {
        let json = r#"{
            "data": {"repository": {"pullRequest": {
                "comments": {
                    "totalCount": 2,
                    "nodes": [
                        {"author": {"login": "alice"}, "createdAt": "2026-07-21T00:00:00Z", "body": "Looks interesting"},
                        {"author": {"login": "bob"}, "createdAt": "2026-07-23T00:00:00Z", "body": "Ping"}
                    ]
                },
                "reviews": {
                    "totalCount": 4,
                    "nodes": [
                        {"author": {"login": "carol"}, "createdAt": "2026-07-22T00:00:00Z", "body": "Nit inside", "state": "CHANGES_REQUESTED"},
                        {"author": {"login": "carol"}, "createdAt": "2026-07-24T00:00:00Z", "body": "", "state": "APPROVED"},
                        {"author": {"login": "dave"}, "createdAt": "2026-07-22T12:00:00Z", "body": "", "state": "COMMENTED"},
                        {"author": {"login": "erin"}, "createdAt": "2026-07-25T00:00:00Z", "body": "", "state": "COMMENTED", "comments": {"totalCount": 3}}
                    ]
                }
            }}}
        }"#;
        let (items, truncated) = parse_timeline(json).unwrap();
        // The empty COMMENTED review is dropped; the body-less APPROVED
        // review and the COMMENTED review carrying line comments are kept.
        assert_eq!(items.len(), 5);
        assert!(!truncated);
        assert_eq!(
            items.iter().map(|i| i.author.as_str()).collect::<Vec<_>>(),
            ["alice", "carol", "bob", "carol", "erin"]
        );
        assert_eq!(
            items[1].kind,
            PrTimelineKind::Review(PrReviewState::ChangesRequested)
        );
        assert_eq!(
            items[3].kind,
            PrTimelineKind::Review(PrReviewState::Approved)
        );
        assert_eq!(items[0].kind, PrTimelineKind::Comment);
        assert_eq!(
            items[4].kind,
            PrTimelineKind::Review(PrReviewState::Commented)
        );
        assert_eq!(items[4].file_comments, 3);
    }

    #[test]
    fn timeline_truncation_and_errors() {
        let json = r#"{
            "data": {"repository": {"pullRequest": {
                "comments": {"totalCount": 80, "nodes": [
                    {"author": {"login": "a"}, "createdAt": "2026-07-21T00:00:00Z", "body": "x"}
                ]},
                "reviews": {"totalCount": 0, "nodes": []}
            }}}
        }"#;
        let (items, truncated) = parse_timeline(json).unwrap();
        assert_eq!(items.len(), 1);
        assert!(truncated);

        assert!(parse_timeline(r#"{"data": {"repository": null}}"#).is_err());
        assert!(parse_timeline("nope").is_err());
    }

    #[test]
    fn merge_method_round_trip() {
        for method in PrMergeMethod::ALL {
            assert_eq!(PrMergeMethod::from_str(method.as_str()), Some(method));
        }
        assert_eq!(PrMergeMethod::Squash.gh_flag(), "--squash");
        assert_eq!(PrMergeMethod::from_str("bogus"), None);
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
