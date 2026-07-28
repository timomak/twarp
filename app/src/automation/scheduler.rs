//! twarp 20d: local scheduled agent tasks (Automation > Scheduled Tasks).
//!
//! A [`SchedulerModel`] singleton owns the task list and run history (mirrored
//! to the `scheduled_tasks` / `scheduled_task_runs` SQLite tables) and runs a
//! 30-second tick loop on the background executor. A due task "fires" by
//! queueing a [`FireRequest`] and emitting [`SchedulerEvent::FireRequested`];
//! the workspace (see `WorkspaceView::handle_scheduler_fires`) drains the
//! queue on the main thread and opens a real Claude Code pane in a new tab,
//! pre-seeded with the task prompt, then restores the previously active tab so
//! the run happens in the background but stays fully inspectable.
//!
//! Completion is detected via `ClaudeCodeViewEvent::SessionEnded` (emitted
//! from the pane's `TranscriptEvent::Ended` handling) and reported back here
//! keyed by session id. A run whose first attempt fails is retried ONCE with
//! the task's fallback provider (if set and different); if that fails too the
//! run records `both_failed`. No further retries.
//!
//! Beachball rule: the tick loop and all time math run off the main thread /
//! are O(number of tasks); no filesystem work happens here.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Local, TimeZone, Utc};
use claude_code::driver::{AgentProvider, PermissionMode};
use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::{ModelEvent, PersistedScheduledTask, PersistedScheduledTaskRun};
use crate::GlobalResourceHandlesProvider;

/// Tick period of the scheduler loop.
pub const TICK_SECONDS: u64 = 30;

/// Run-history rows kept per task (in memory; SQLite prunes to the same
/// bound on insert).
const RUNS_KEPT_PER_TASK: usize = 20;

/// Serialize an [`AgentProvider`] for the `provider` columns.
pub fn provider_name(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "claude",
        AgentProvider::Codex => "codex",
    }
}

pub fn provider_from_name(name: &str) -> Option<AgentProvider> {
    match name {
        "claude" => Some(AgentProvider::Claude),
        "codex" => Some(AgentProvider::Codex),
        _ => None,
    }
}

/// One scheduled task, as edited on the Scheduled Tasks page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledTaskEntry {
    /// UUID string, stable across edits.
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub cwd: String,
    /// 5-field cron expression (min hour dom month dow).
    pub schedule: String,
    pub provider: AgentProvider,
    pub fallback_provider: Option<AgentProvider>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub enabled: bool,
    /// Fire a missed occurrence once at app launch instead of skipping it.
    pub catch_up: bool,
    /// Next due time (unix secs); `None` until computed.
    pub next_run_at: Option<i64>,
    pub created_at: i64,
}

impl ScheduledTaskEntry {
    fn from_persisted(row: PersistedScheduledTask) -> Option<Self> {
        Some(Self {
            id: row.id,
            name: row.name,
            prompt: row.prompt,
            cwd: row.cwd,
            schedule: row.schedule,
            provider: provider_from_name(&row.provider)?,
            fallback_provider: row
                .fallback_provider
                .as_deref()
                .and_then(provider_from_name),
            model: row.model,
            effort: row.effort,
            permission_mode: row
                .permission_mode
                .as_deref()
                .and_then(PermissionMode::from_cli_arg),
            enabled: row.enabled,
            catch_up: row.catch_up,
            next_run_at: row.next_run_at,
            created_at: row.created_at,
        })
    }

    fn to_persisted(&self) -> PersistedScheduledTask {
        PersistedScheduledTask {
            id: self.id.clone(),
            name: self.name.clone(),
            prompt: self.prompt.clone(),
            cwd: self.cwd.clone(),
            schedule: self.schedule.clone(),
            provider: provider_name(self.provider).to_owned(),
            fallback_provider: self.fallback_provider.map(|p| provider_name(p).to_owned()),
            model: self.model.clone(),
            effort: self.effort.clone(),
            permission_mode: self.permission_mode.map(|m| m.as_cli_arg().to_owned()),
            enabled: self.enabled,
            catch_up: self.catch_up,
            next_run_at: self.next_run_at,
            created_at: self.created_at,
        }
    }
}

/// Outcome of one run (one scheduled fire, spanning both attempts).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Running,
    Success,
    Error,
    /// Both the primary and the fallback provider failed.
    BothFailed,
}

impl RunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            RunOutcome::Running => "running",
            RunOutcome::Success => "success",
            RunOutcome::Error => "error",
            RunOutcome::BothFailed => "both_failed",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "running" => Some(RunOutcome::Running),
            "success" => Some(RunOutcome::Success),
            "error" => Some(RunOutcome::Error),
            "both_failed" => Some(RunOutcome::BothFailed),
            _ => None,
        }
    }
}

/// One run-history row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRunEntry {
    pub id: String,
    pub task_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub provider_used: AgentProvider,
    pub outcome: RunOutcome,
    /// The pane session backing the run, once spawned — lets the history
    /// strip offer "View session".
    pub session_id: Option<String>,
    pub summary: Option<String>,
}

impl TaskRunEntry {
    fn from_persisted(row: PersistedScheduledTaskRun) -> Option<Self> {
        Some(Self {
            id: row.id,
            task_id: row.task_id,
            started_at: row.started_at,
            finished_at: row.finished_at,
            provider_used: provider_from_name(&row.provider_used)?,
            outcome: RunOutcome::from_str(&row.outcome)?,
            session_id: row.session_id,
            summary: row.summary,
        })
    }

    fn to_persisted(&self) -> PersistedScheduledTaskRun {
        PersistedScheduledTaskRun {
            id: self.id.clone(),
            task_id: self.task_id.clone(),
            started_at: self.started_at,
            finished_at: self.finished_at,
            provider_used: provider_name(self.provider_used).to_owned(),
            outcome: self.outcome.as_str().to_owned(),
            session_id: self.session_id.clone(),
            summary: self.summary.clone(),
        }
    }
}

/// A queued request to open a pane for one run attempt. Drained by the
/// workspace on the main thread.
#[derive(Clone, Debug)]
pub struct FireRequest {
    pub run_id: String,
    pub task_id: String,
    pub provider: AgentProvider,
    pub prompt: String,
    pub cwd: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: PermissionMode,
    pub task_name: String,
}

/// Events emitted by the scheduler singleton.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedulerEvent {
    /// One or more [`FireRequest`]s are queued; a workspace should drain
    /// [`SchedulerModel::take_pending_fires`] and open the panes.
    FireRequested,
}

/// A run attempt currently backed by a live pane session.
#[derive(Clone, Debug)]
struct ActiveRun {
    run_id: String,
    task_id: String,
    /// 1 = primary provider, 2 = fallback.
    attempt: u8,
}

// ---------------------------------------------------------------------------
// Cron helpers (pure; unit-tested below)
// ---------------------------------------------------------------------------

/// Parse a 5-field cron expression (min hour dom month dow) by prepending a
/// seconds field for the `cron` crate. 6/7-field expressions are rejected so
/// the stored canonical form stays 5-field.
pub fn parse_schedule(expr: &str) -> Result<cron::Schedule, String> {
    let expr = expr.trim();
    let fields = expr.split_whitespace().count();
    if fields != 5 {
        return Err(format!(
            "Expected a 5-field cron expression (minute hour day month weekday), got {fields} field(s)."
        ));
    }
    cron::Schedule::from_str(&format!("0 {expr}")).map_err(|err| format!("Invalid cron: {err}"))
}

/// Human-facing validation: `Ok(())` or an inline error string.
pub fn validate_schedule(expr: &str) -> Result<(), String> {
    parse_schedule(expr).map(|_| ())
}

/// The next occurrence strictly after `after` (unix secs), in local time.
pub fn next_run_after(expr: &str, after_unix: i64) -> Option<i64> {
    let schedule = parse_schedule(expr).ok()?;
    let after: DateTime<Local> = Local.timestamp_opt(after_unix, 0).single()?;
    schedule.after(&after).next().map(|dt| dt.timestamp())
}

/// A short human summary of a 5-field cron expression for the task list, e.g.
/// "Hourly at :15", "Daily at 09:30", "Weekly on Mon at 09:30"; falls back to
/// the raw expression for anything irregular.
pub fn schedule_summary(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let [min, hour, dom, month, dow] = fields.as_slice() else {
        return expr.to_owned();
    };
    let simple = |s: &str| s.parse::<u32>().ok();
    match (simple(min), simple(hour), *dom, *month, *dow) {
        (Some(m), None, "*", "*", "*") if *hour == "*" => format!("Hourly at :{m:02}"),
        (Some(m), Some(h), "*", "*", "*") => format!("Daily at {h:02}:{m:02}"),
        (Some(m), Some(h), "*", "*", dow) => {
            let day = match dow {
                "0" | "7" | "SUN" | "Sun" | "sun" => Some("Sun"),
                "1" | "MON" | "Mon" | "mon" => Some("Mon"),
                "2" | "TUE" | "Tue" | "tue" => Some("Tue"),
                "3" | "WED" | "Wed" | "wed" => Some("Wed"),
                "4" | "THU" | "Thu" | "thu" => Some("Thu"),
                "5" | "FRI" | "Fri" | "fri" => Some("Fri"),
                "6" | "SAT" | "Sat" | "sat" => Some("Sat"),
                _ => None,
            };
            match day {
                Some(day) => format!("Weekly on {day} at {h:02}:{m:02}"),
                None => expr.to_owned(),
            }
        }
        _ => expr.to_owned(),
    }
}

/// The fallback state machine: what to do when an attempt finishes (pure;
/// unit-tested). Attempt 1 failing with a distinct fallback provider retries
/// exactly once; a failed attempt 2 is `both_failed`. No further retries.
#[derive(Debug, PartialEq, Eq)]
enum AttemptResolution {
    Finished(RunOutcome),
    RetryWithFallback(AgentProvider),
}

fn resolve_attempt(
    success: bool,
    attempt: u8,
    fallback: Option<AgentProvider>,
) -> AttemptResolution {
    if success {
        AttemptResolution::Finished(RunOutcome::Success)
    } else if attempt == 1 {
        match fallback {
            Some(fallback) => AttemptResolution::RetryWithFallback(fallback),
            None => AttemptResolution::Finished(RunOutcome::Error),
        }
    } else {
        AttemptResolution::Finished(RunOutcome::BothFailed)
    }
}

fn now_unix() -> i64 {
    Utc::now().timestamp()
}

/// What the launch pass decided for one task (pure; unit-tested).
#[derive(Debug, PartialEq, Eq)]
enum LaunchDecision {
    /// Fire one missed occurrence now, then schedule the next.
    CatchUp { next_run_at: Option<i64> },
    /// Just roll `next_run_at` forward past `now`.
    Roll { next_run_at: Option<i64> },
    /// Nothing due; keep (or fill in) `next_run_at`.
    Keep { next_run_at: Option<i64> },
}

/// The at-launch missed-run policy: a stored `next_run_at` in the past fires
/// once iff `catch_up`, else rolls forward silently. A task with no stored
/// `next_run_at` (fresh row) never fires at launch — it just gets one.
fn launch_decision(task: &ScheduledTaskEntry, now: i64) -> LaunchDecision {
    let next_from_now = next_run_after(&task.schedule, now);
    if !task.enabled {
        return LaunchDecision::Keep {
            next_run_at: task.next_run_at,
        };
    }
    match task.next_run_at {
        Some(due) if due <= now => {
            if task.catch_up {
                LaunchDecision::CatchUp {
                    next_run_at: next_from_now,
                }
            } else {
                LaunchDecision::Roll {
                    next_run_at: next_from_now,
                }
            }
        }
        Some(due) => LaunchDecision::Keep {
            next_run_at: Some(due),
        },
        None => LaunchDecision::Keep {
            next_run_at: next_from_now,
        },
    }
}

// ---------------------------------------------------------------------------
// The singleton
// ---------------------------------------------------------------------------

/// Singleton owning scheduled tasks + run history and the tick loop.
pub struct SchedulerModel {
    tasks: Vec<ScheduledTaskEntry>,
    /// All run history, newest first (bounded per task).
    runs: Vec<TaskRunEntry>,
    /// Fires queued for the workspace to open, oldest first.
    pending_fires: Vec<FireRequest>,
    /// Live runs keyed by pane session id (filled in on spawn).
    active_by_session: HashMap<String, ActiveRun>,
}

impl SchedulerModel {
    pub fn new(
        persisted_tasks: Vec<PersistedScheduledTask>,
        persisted_runs: Vec<PersistedScheduledTaskRun>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let tasks: Vec<ScheduledTaskEntry> = persisted_tasks
            .into_iter()
            .filter_map(ScheduledTaskEntry::from_persisted)
            .collect();
        let mut runs: Vec<TaskRunEntry> = persisted_runs
            .into_iter()
            .filter_map(TaskRunEntry::from_persisted)
            .collect();
        runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));

        let mut model = Self {
            tasks,
            runs,
            pending_fires: Vec::new(),
            active_by_session: HashMap::new(),
        };

        // Runs persisted as `running` belong to sessions the previous app
        // instance was driving; they can never complete now.
        let now = now_unix();
        let orphaned: Vec<String> = model
            .runs
            .iter()
            .filter(|run| run.outcome == RunOutcome::Running)
            .map(|run| run.id.clone())
            .collect();
        for id in orphaned {
            model.update_run(&id, ctx, |run| {
                run.outcome = RunOutcome::Error;
                run.finished_at = Some(now);
                run.summary = Some("Interrupted by app quit.".to_owned());
            });
        }

        // At-launch missed-run pass.
        let mut catch_up_fires: Vec<String> = Vec::new();
        let mut changed = false;
        for task in &mut model.tasks {
            let decision = launch_decision(task, now);
            let next = match &decision {
                LaunchDecision::CatchUp { next_run_at } => {
                    catch_up_fires.push(task.id.clone());
                    *next_run_at
                }
                LaunchDecision::Roll { next_run_at } | LaunchDecision::Keep { next_run_at } => {
                    *next_run_at
                }
            };
            if task.next_run_at != next {
                task.next_run_at = next;
                changed = true;
            }
        }
        if changed {
            model.persist_tasks(ctx);
        }
        for task_id in catch_up_fires {
            model.fire(&task_id, 1, None, ctx);
        }

        model.spawn_tick_loop(ctx);
        model
    }

    pub fn tasks(&self) -> &[ScheduledTaskEntry] {
        &self.tasks
    }

    pub fn get(&self, id: &str) -> Option<&ScheduledTaskEntry> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Run history for one task, newest first.
    pub fn runs_for(&self, task_id: &str) -> Vec<&TaskRunEntry> {
        self.runs.iter().filter(|r| r.task_id == task_id).collect()
    }

    /// The newest run of a task, if any.
    pub fn last_run(&self, task_id: &str) -> Option<&TaskRunEntry> {
        self.runs.iter().find(|r| r.task_id == task_id)
    }

    /// Whether a task currently has a live (running) attempt.
    pub fn is_running(&self, task_id: &str) -> bool {
        self.active_by_session
            .values()
            .any(|active| active.task_id == task_id)
            || self
                .pending_fires
                .iter()
                .any(|fire| fire.task_id == task_id)
    }

    pub fn name_taken(&self, name: &str, except_id: Option<&str>) -> bool {
        self.tasks
            .iter()
            .any(|t| t.name == name && Some(t.id.as_str()) != except_id)
    }

    /// Insert or (matched by id) replace a task, recompute its next run, then
    /// persist.
    pub fn upsert(&mut self, mut entry: ScheduledTaskEntry, ctx: &mut ModelContext<Self>) {
        entry.next_run_at = entry
            .enabled
            .then(|| next_run_after(&entry.schedule, now_unix()))
            .flatten();
        if let Some(existing) = self.tasks.iter_mut().find(|t| t.id == entry.id) {
            *existing = entry;
        } else {
            self.tasks.push(entry);
        }
        self.tasks.sort_by(|a, b| a.name.cmp(&b.name));
        self.persist_tasks(ctx);
    }

    pub fn delete(&mut self, id: &str, ctx: &mut ModelContext<Self>) {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        if self.tasks.len() == before {
            return;
        }
        self.runs.retain(|r| r.task_id != id);
        self.active_by_session
            .retain(|_, active| active.task_id != id);
        self.pending_fires.retain(|fire| fire.task_id != id);
        self.persist_tasks(ctx);
        self.send_model_event(
            ModelEvent::DeleteScheduledTaskRuns {
                task_id: id.to_owned(),
            },
            ctx,
        );
    }

    pub fn toggle_enabled(&mut self, id: &str, ctx: &mut ModelContext<Self>) {
        let now = now_unix();
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        task.enabled = !task.enabled;
        task.next_run_at = task
            .enabled
            .then(|| next_run_after(&task.schedule, now))
            .flatten();
        self.persist_tasks(ctx);
    }

    /// Fire a task immediately (the row's "Run now").
    pub fn run_now(&mut self, id: &str, ctx: &mut ModelContext<Self>) {
        if self.get(id).is_none() || self.is_running(id) {
            return;
        }
        self.fire(id, 1, None, ctx);
    }

    /// Take (and clear) the queued fire requests. Called by the workspace
    /// that handles [`SchedulerEvent::FireRequested`]; the drain makes the
    /// hand-off single-consumer even with several windows subscribed.
    pub fn take_pending_fires(&mut self) -> Vec<FireRequest> {
        std::mem::take(&mut self.pending_fires)
    }

    /// The workspace opened a pane for `run_id`; bind the run to its session
    /// so `SessionEnded` events can find it.
    pub fn on_fire_spawned(
        &mut self,
        run_id: &str,
        session_id: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(run) = self.runs.iter().find(|r| r.id == run_id) else {
            return;
        };
        // A run row whose provider no longer matches the task's primary is the
        // fallback attempt (finish_attempt_inner rewrote provider_used).
        let attempt = match self.get(&run.task_id) {
            Some(task) if run.provider_used != task.provider => 2,
            _ => 1,
        };
        let task_id = run.task_id.clone();
        let run_id = run_id.to_owned();
        self.update_run(&run_id.clone(), ctx, |run| {
            run.session_id = Some(session_id.clone());
        });
        self.active_by_session.insert(
            session_id,
            ActiveRun {
                run_id,
                task_id,
                attempt,
            },
        );
        ctx.notify();
    }

    /// The workspace could not open a pane for `run_id` (spawn failure).
    pub fn on_fire_failed(&mut self, run_id: &str, reason: String, ctx: &mut ModelContext<Self>) {
        let Some(run) = self.runs.iter().find(|r| r.id == run_id).cloned() else {
            return;
        };
        self.finish_attempt(&run.id, false, reason, ctx);
    }

    /// A pane session ended; if it backs one of our runs, record the outcome
    /// (and maybe fire the fallback).
    pub fn on_session_ended(
        &mut self,
        session_id: &str,
        success: bool,
        summary: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(active) = self.active_by_session.remove(session_id) else {
            return;
        };
        self.finish_attempt_inner(active, success, summary, ctx);
    }

    /// Finish the attempt backing `run_id` (used for spawn failures, where no
    /// session id ever existed).
    fn finish_attempt(
        &mut self,
        run_id: &str,
        success: bool,
        summary: String,
        ctx: &mut ModelContext<Self>,
    ) {
        // Reconstruct the ActiveRun shape for a run that never got a session.
        let Some(run) = self.runs.iter().find(|r| r.id == run_id) else {
            return;
        };
        let attempt = match self.get(&run.task_id) {
            Some(task) if run.provider_used != task.provider => 2,
            _ => 1,
        };
        let active = ActiveRun {
            run_id: run_id.to_owned(),
            task_id: run.task_id.clone(),
            attempt,
        };
        self.finish_attempt_inner(active, success, summary, ctx);
    }

    fn finish_attempt_inner(
        &mut self,
        active: ActiveRun,
        success: bool,
        summary: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let now = now_unix();
        let fallback = self.get(&active.task_id).and_then(|task| {
            task.fallback_provider
                .filter(|fallback| *fallback != task.provider)
        });

        match resolve_attempt(success, active.attempt, fallback) {
            AttemptResolution::Finished(outcome) => {
                self.update_run(&active.run_id, ctx, |run| {
                    run.outcome = outcome;
                    run.finished_at = Some(now);
                    run.summary = Some(summary.clone());
                });
            }
            AttemptResolution::RetryWithFallback(fallback) => {
                // Retry ONCE with the fallback provider, on the same run row.
                self.update_run(&active.run_id, ctx, |run| {
                    run.provider_used = fallback;
                    run.summary = Some(format!("Primary provider failed: {summary}"));
                });
                let task_id = active.task_id.clone();
                self.fire(&task_id, 2, Some(active.run_id.clone()), ctx);
            }
        }
        ctx.notify();
    }

    /// Queue one fire attempt. `attempt` 1 creates a fresh run row; attempt 2
    /// reuses `existing_run_id` (the fallback retry).
    fn fire(
        &mut self,
        task_id: &str,
        attempt: u8,
        existing_run_id: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(task) = self.get(task_id).cloned() else {
            return;
        };
        let provider = if attempt >= 2 {
            match task.fallback_provider {
                Some(p) if p != task.provider => p,
                _ => return,
            }
        } else {
            task.provider
        };

        let run_id = match existing_run_id {
            Some(id) => id,
            None => {
                let run = TaskRunEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    task_id: task.id.clone(),
                    started_at: now_unix(),
                    finished_at: None,
                    provider_used: provider,
                    outcome: RunOutcome::Running,
                    session_id: None,
                    summary: None,
                };
                let id = run.id.clone();
                self.insert_run(run, ctx);
                id
            }
        };

        // Scheduled runs never bypass permissions: an unattended session must
        // not get carte blanche just because the stored mode says so.
        let permission_mode = match task.permission_mode {
            Some(PermissionMode::BypassPermissions) | None => PermissionMode::Default,
            Some(mode) => mode,
        };

        self.pending_fires.push(FireRequest {
            run_id,
            task_id: task.id.clone(),
            provider,
            prompt: task.prompt.clone(),
            cwd: task.cwd.clone(),
            model: task.model.clone(),
            effort: task.effort.clone(),
            permission_mode,
            task_name: task.name.clone(),
        });
        ctx.notify();
        ctx.emit(SchedulerEvent::FireRequested);
    }

    /// The 30s tick: fire every enabled task whose `next_run_at` has passed,
    /// then advance it.
    fn on_tick(&mut self, ctx: &mut ModelContext<Self>) {
        // Re-announce fires queued before any workspace subscribed (e.g. the
        // at-launch catch-up pass runs before windows exist).
        if !self.pending_fires.is_empty() {
            ctx.emit(SchedulerEvent::FireRequested);
        }
        let now = now_unix();
        let due: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.enabled && task.next_run_at.is_some_and(|due| due <= now))
            .map(|task| task.id.clone())
            .collect();
        if due.is_empty() {
            return;
        }
        let mut fired = Vec::new();
        for id in due {
            if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
                task.next_run_at = next_run_after(&task.schedule, now);
            }
            // A task still running its previous occurrence skips this one
            // (its next_run_at advanced above, so it won't spin).
            if !self.is_running(&id) {
                fired.push(id);
            }
        }
        self.persist_tasks(ctx);
        for id in fired {
            self.fire(&id, 1, None, ctx);
        }
    }

    fn spawn_tick_loop(&self, ctx: &mut ModelContext<Self>) {
        let (tick_tx, tick_rx) = async_channel::unbounded::<()>();
        let _ = ctx.spawn_stream_local(
            tick_rx,
            |model: &mut Self, (), ctx| model.on_tick(ctx),
            |_, _| {},
        );
        ctx.background_executor()
            .spawn(async move {
                loop {
                    twarpui::r#async::Timer::after(std::time::Duration::from_secs(TICK_SECONDS))
                        .await;
                    if tick_tx.send(()).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
    }

    fn insert_run(&mut self, run: TaskRunEntry, ctx: &mut ModelContext<Self>) {
        self.send_model_event(
            ModelEvent::UpsertScheduledTaskRun {
                run: run.to_persisted(),
            },
            ctx,
        );
        let task_id = run.task_id.clone();
        self.runs.insert(0, run);
        // In-memory prune mirroring the SQLite bound.
        let mut kept = 0;
        self.runs.retain(|r| {
            if r.task_id != task_id {
                return true;
            }
            kept += 1;
            kept <= RUNS_KEPT_PER_TASK
        });
        ctx.notify();
    }

    fn update_run(
        &mut self,
        run_id: &str,
        ctx: &mut ModelContext<Self>,
        mutate: impl FnOnce(&mut TaskRunEntry),
    ) {
        let Some(run) = self.runs.iter_mut().find(|r| r.id == run_id) else {
            return;
        };
        mutate(run);
        let persisted = run.to_persisted();
        self.send_model_event(ModelEvent::UpsertScheduledTaskRun { run: persisted }, ctx);
        ctx.notify();
    }

    fn persist_tasks(&self, ctx: &mut ModelContext<Self>) {
        ctx.notify();
        self.send_model_event(
            ModelEvent::ReplaceScheduledTasks {
                tasks: self
                    .tasks
                    .iter()
                    .map(ScheduledTaskEntry::to_persisted)
                    .collect(),
            },
            ctx,
        );
    }

    fn send_model_event(&self, event: ModelEvent, ctx: &mut ModelContext<Self>) {
        let handles = GlobalResourceHandlesProvider::as_ref(ctx).get();
        if let Some(sender) = &handles.model_event_sender {
            if let Err(err) = sender.send(event) {
                log::error!("Failed to persist scheduler state: {err}");
            }
        }
    }
}

impl Entity for SchedulerModel {
    type Event = SchedulerEvent;
}

impl SingletonEntity for SchedulerModel {}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(schedule: &str) -> ScheduledTaskEntry {
        ScheduledTaskEntry {
            id: "t1".to_owned(),
            name: "test".to_owned(),
            prompt: "do things".to_owned(),
            cwd: "/tmp".to_owned(),
            schedule: schedule.to_owned(),
            provider: AgentProvider::Claude,
            fallback_provider: None,
            model: None,
            effort: None,
            permission_mode: None,
            enabled: true,
            catch_up: false,
            next_run_at: None,
            created_at: 0,
        }
    }

    #[test]
    fn validates_five_field_cron_only() {
        assert!(validate_schedule("0 9 * * *").is_ok());
        assert!(validate_schedule("*/15 * * * *").is_ok());
        assert!(validate_schedule("30 9 * * 1").is_ok());
        assert!(validate_schedule("").is_err());
        assert!(validate_schedule("not cron").is_err());
        assert!(
            validate_schedule("0 0 9 * * *").is_err(),
            "6-field rejected"
        );
        assert!(validate_schedule("61 9 * * *").is_err());
    }

    #[test]
    fn next_run_advances_strictly_past_after() {
        // Hourly at minute 0: the next run after any time is the next hour
        // boundary, strictly in the future.
        let now = now_unix();
        let next = next_run_after("0 * * * *", now).unwrap();
        assert!(next > now);
        assert!(next - now <= 3600);
        assert_eq!(next % 60, 0);
        // Advancing from the computed occurrence lands on the following one.
        let next2 = next_run_after("0 * * * *", next).unwrap();
        assert_eq!(next2 - next, 3600);
    }

    #[test]
    fn launch_decision_rolls_or_catches_up_missed_runs() {
        let now = now_unix();

        // Missed occurrence, catch_up off: roll forward, no fire.
        let mut t = task("0 * * * *");
        t.next_run_at = Some(now - 1000);
        let d = launch_decision(&t, now);
        match d {
            LaunchDecision::Roll { next_run_at } => assert!(next_run_at.unwrap() > now),
            other => panic!("expected Roll, got {other:?}"),
        }

        // Missed occurrence, catch_up on: fire once, then next.
        t.catch_up = true;
        match launch_decision(&t, now) {
            LaunchDecision::CatchUp { next_run_at } => assert!(next_run_at.unwrap() > now),
            other => panic!("expected CatchUp, got {other:?}"),
        }

        // Future occurrence: kept as-is.
        t.next_run_at = Some(now + 1000);
        assert_eq!(
            launch_decision(&t, now),
            LaunchDecision::Keep {
                next_run_at: Some(now + 1000)
            }
        );

        // Fresh row (no next_run_at): never fires at launch, gets one.
        t.next_run_at = None;
        t.catch_up = true;
        match launch_decision(&t, now) {
            LaunchDecision::Keep { next_run_at } => assert!(next_run_at.unwrap() > now),
            other => panic!("expected Keep, got {other:?}"),
        }

        // Disabled: untouched even when overdue.
        t.enabled = false;
        t.next_run_at = Some(now - 1000);
        assert_eq!(
            launch_decision(&t, now),
            LaunchDecision::Keep {
                next_run_at: Some(now - 1000)
            }
        );
    }

    #[test]
    fn schedule_summaries() {
        assert_eq!(schedule_summary("15 * * * *"), "Hourly at :15");
        assert_eq!(schedule_summary("30 9 * * *"), "Daily at 09:30");
        assert_eq!(schedule_summary("30 9 * * 1"), "Weekly on Mon at 09:30");
        assert_eq!(schedule_summary("*/5 * * * *"), "*/5 * * * *");
    }

    #[test]
    fn provider_roundtrip_and_outcomes() {
        assert_eq!(provider_from_name("claude"), Some(AgentProvider::Claude));
        assert_eq!(provider_from_name("codex"), Some(AgentProvider::Codex));
        assert_eq!(provider_from_name("gemini"), None);
        for outcome in [
            RunOutcome::Running,
            RunOutcome::Success,
            RunOutcome::Error,
            RunOutcome::BothFailed,
        ] {
            assert_eq!(RunOutcome::from_str(outcome.as_str()), Some(outcome));
        }
    }

    #[test]
    fn fallback_state_machine() {
        use AttemptResolution::*;
        // Success finishes regardless of attempt / fallback.
        assert_eq!(
            resolve_attempt(true, 1, Some(AgentProvider::Codex)),
            Finished(RunOutcome::Success)
        );
        assert_eq!(
            resolve_attempt(true, 2, None),
            Finished(RunOutcome::Success)
        );
        // First failure with a fallback: retry once.
        assert_eq!(
            resolve_attempt(false, 1, Some(AgentProvider::Codex)),
            RetryWithFallback(AgentProvider::Codex)
        );
        // First failure without a fallback: plain error.
        assert_eq!(resolve_attempt(false, 1, None), Finished(RunOutcome::Error));
        // Second failure: both_failed, even with a fallback configured — no
        // retry loops.
        assert_eq!(
            resolve_attempt(false, 2, Some(AgentProvider::Codex)),
            Finished(RunOutcome::BothFailed)
        );
    }

    #[test]
    fn persisted_task_roundtrip() {
        let mut t = task("30 9 * * 1");
        t.fallback_provider = Some(AgentProvider::Codex);
        t.model = Some("sonnet".to_owned());
        t.effort = Some("medium".to_owned());
        t.permission_mode = Some(PermissionMode::AcceptEdits);
        t.next_run_at = Some(123);
        let back = ScheduledTaskEntry::from_persisted(t.to_persisted()).unwrap();
        assert_eq!(t, back);
    }
}
