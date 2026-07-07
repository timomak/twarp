//! Agents model for the Claude Code pane (twarp).
//!
//! Claude Code fans work out to sub-agents via the `Task` tool (shipped as
//! `Agent` on some builds): each call spawns an agent whose activity nests
//! under the launching card. In a busy session those cards scatter across
//! turns and scroll away, so — like the background scripts the pane already
//! surfaces — a long-running research or review agent is easy to lose track
//! of.
//!
//! This module derives a per-chat **agents view** from the transcript: it
//! walks the [`TranscriptItem`]s (descending into `Task` children, so an
//! agent's own sub-agents are listed too) and produces an ordered list the
//! pane renders as a floating status panel, twinned with the
//! background-scripts one. It is intentionally headless (no UI) so the
//! extraction and the wire-shape parsing can be unit-tested without a window
//! — the same split [`super::background_scripts`] uses.
//!
//! Agents normally run inline: the launching call's tool result arrives when
//! the agent finishes, so the card's [`ToolStatus`] is the state. Current
//! `claude` builds can also run an agent *in the background* — the launch
//! acknowledges immediately ("you will be notified…") and the terminal state
//! arrives later as a [`TaskNotification`], the same channel background Bash
//! uses. Such an agent stays [`AgentRunState::Running`] until its
//! notification — joined by `tool_use_id`, or by task id against the id
//! announced in the acknowledgement — retires it.
//!
//! Extraction is read-only and best-effort: twarp can observe the agents
//! Claude launched, but cannot itself spawn or stop them (those are the
//! model's tool calls). State the transcript doesn't make explicit degrades
//! to [`AgentRunState::Running`] rather than guessing.

use claude_code::{TaskNotification, TaskRunStatus, ToolStatus, TranscriptItem};
use serde_json::Value;

use super::tool_cards::is_subagent_tool;

/// Run state of a sub-agent, derived from the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentRunState {
    /// Launched and, as far as the transcript shows, still working. The
    /// default when nothing later marks it done.
    Running,
    /// The launching call completed (or the background notification reported
    /// success) — the agent returned its result.
    Finished,
    /// The launching call failed, or the notification reported failure.
    Failed,
    /// A `TaskStop` was issued for this agent, or its notification reported
    /// it stopped before finishing.
    Stopped,
}

impl AgentRunState {
    /// A short, human label for the status pill.
    pub(super) fn label(self) -> &'static str {
        match self {
            AgentRunState::Running => "running",
            AgentRunState::Finished => "finished",
            AgentRunState::Failed => "failed",
            AgentRunState::Stopped => "stopped",
        }
    }

    /// Whether the agent is still believed to be working (drives the running
    /// indicator and the panel's "N running" count).
    pub(super) fn is_active(self) -> bool {
        matches!(self, AgentRunState::Running)
    }
}

/// One sub-agent launched in this chat.
#[derive(Debug, Clone)]
pub(super) struct AgentRun {
    /// The launching tool-use id — a stable key for per-row UI state.
    pub id: String,
    /// The `subagent_type` dispatched to (`Explore`, `general-purpose`, …),
    /// when present.
    pub agent_type: Option<String>,
    /// The short human `description` Claude attached to the call.
    pub description: Option<String>,
    pub state: AgentRunState,
    /// The agent's returned result text (and, for background agents, the
    /// notification summary), newest last. Empty when nothing has come back.
    pub result: String,
}

impl AgentRun {
    /// The row label: "<type>: <description>", falling back to whichever is
    /// present — the same composition the transcript's tool card uses.
    pub(super) fn title(&self) -> String {
        match (self.agent_type.as_deref(), self.description.as_deref()) {
            (Some(kind), Some(desc)) => format!("{kind}: {desc}"),
            (None, Some(desc)) => desc.to_owned(),
            (Some(kind), None) => kind.to_owned(),
            (None, None) => "Agent".to_owned(),
        }
    }
}

/// A string field on a tool input, trimmed; `None` when missing/blank.
fn str_field(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Whether a completed launch acknowledgement says the agent kept running in
/// the background (so completion arrives later as a notification, and the
/// tool result must not be read as the agent's result). Conservative: only
/// explicit phrasings count, so an ordinary agent result that merely mentions
/// "background" isn't mistaken for a launch ack.
fn ack_reports_background(ack: &str) -> bool {
    let lower = ack.to_ascii_lowercase();
    [
        "running in the background",
        "running in background",
        "you will be notified",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Parse the agent/task id out of a background launch's acknowledgement — a
/// *labeled* id ("ID: abc123", "agent-id=abc123"). Requiring the `:`/`=`
/// separator avoids latching onto prose. `None` when nothing id-shaped is
/// found (the notification then only matches by `tool_use_id`).
fn parse_task_id(ack: &str) -> Option<String> {
    let is_id_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let lower = ack.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("id") {
        let after = search_from + rel + "id".len();
        let rest = &ack[after..];
        let sep = rest.trim_start_matches([' ', '\t']);
        if let Some(value) = sep.strip_prefix([':', '=']) {
            let token: String = value
                .chars()
                .skip_while(|&c| !is_id_char(c))
                .take_while(|&c| is_id_char(c))
                .collect();
            if !token.is_empty() {
                return Some(token);
            }
        }
        search_from = after;
    }
    None
}

/// Append text to an agent's captured result, separated by a blank line so a
/// launch ack followed by a notification summary reads in order.
fn append_result(agent: &mut AgentRun, more: &str) {
    let more = more.trim();
    if more.is_empty() {
        return;
    }
    if !agent.result.is_empty() {
        agent.result.push_str("\n\n");
    }
    agent.result.push_str(more);
}

/// Collect every sub-agent launched in `items` (descending into `Task`
/// children), in launch order, then retire background agents from the harness
/// [`TaskNotification`]s.
pub(super) fn collect(
    items: &[TranscriptItem],
    notifications: &[TaskNotification],
) -> Vec<AgentRun> {
    let mut agents: Vec<AgentRunEntry> = Vec::new();
    walk(items, &mut agents);
    for notification in notifications {
        apply_notification(&mut agents, notification);
    }
    agents.into_iter().map(|entry| entry.agent).collect()
}

/// An [`AgentRun`] plus the background task id parsed from its launch ack —
/// collection-internal, so the UI-facing struct doesn't carry matching state.
struct AgentRunEntry {
    agent: AgentRun,
    task_id: Option<String>,
}

/// Retire the background agent a terminal-state notification belongs to.
/// Matched by the launch's `tool_use` id when the notification carries one
/// (exact), else by the task id against the id parsed from the launch
/// acknowledgement. Only a Running agent transitions — a stop or failure
/// already observed in the transcript is sticky — but the summary is always
/// captured.
fn apply_notification(agents: &mut [AgentRunEntry], notification: &TaskNotification) {
    let by_tool_use =
        |e: &AgentRunEntry| notification.tool_use_id.as_deref() == Some(e.agent.id.as_str());
    let by_task_id =
        |e: &AgentRunEntry| e.task_id.as_deref() == Some(notification.task_id.as_str());
    let index = agents
        .iter()
        .position(by_tool_use)
        // Newest-first, in case a task id recycles.
        .or_else(|| agents.iter().rposition(by_task_id));
    let Some(entry) = index.map(|i| &mut agents[i]) else {
        return;
    };
    if entry.agent.state == AgentRunState::Running {
        entry.agent.state = match notification.status {
            TaskRunStatus::Completed => AgentRunState::Finished,
            TaskRunStatus::Failed => AgentRunState::Failed,
            TaskRunStatus::Stopped => AgentRunState::Stopped,
        };
    }
    if let Some(summary) = &notification.summary {
        append_result(&mut entry.agent, summary);
    }
}

fn walk(items: &[TranscriptItem], agents: &mut Vec<AgentRunEntry>) {
    for item in items {
        let TranscriptItem::Tool {
            id,
            name,
            input,
            status,
            output,
            children,
        } = item
        else {
            continue;
        };

        if is_subagent_tool(name) {
            let ack = output.as_ref().map(|o| o.text.as_str()).unwrap_or("");
            let background = ack_reports_background(ack);
            let state = match status {
                ToolStatus::Failed => AgentRunState::Failed,
                // A background launch's tool result is only the
                // acknowledgement — the agent is still working until its
                // notification arrives.
                ToolStatus::Completed if !background => AgentRunState::Finished,
                _ => AgentRunState::Running,
            };
            let mut agent = AgentRun {
                id: id.clone(),
                agent_type: str_field(input, "subagent_type"),
                description: str_field(input, "description"),
                state,
                result: String::new(),
            };
            append_result(&mut agent, ack);
            agents.push(AgentRunEntry {
                agent,
                task_id: background.then(|| parse_task_id(ack)).flatten(),
            });
        } else if name == "TaskStop" {
            // A stop call for a background agent, matched by task id
            // (newest-first, in case an id recycles).
            if let Some(task_id) = str_field(input, "task_id").or_else(|| str_field(input, "id")) {
                if let Some(entry) = agents
                    .iter_mut()
                    .rev()
                    .find(|e| e.task_id.as_deref() == Some(task_id.as_str()))
                {
                    if entry.agent.state == AgentRunState::Running {
                        entry.agent.state = AgentRunState::Stopped;
                    }
                }
            }
        }

        // An agent can itself dispatch agents; its nested calls live in the
        // parent card's children (PRODUCT §19).
        if !children.is_empty() {
            walk(children, agents);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_code::ToolOutput;
    use serde_json::json;

    fn task(id: &str, input: Value, status: ToolStatus, output: Option<&str>) -> TranscriptItem {
        TranscriptItem::Tool {
            id: id.to_owned(),
            name: "Task".to_owned(),
            input,
            status,
            output: output.map(|t| ToolOutput {
                text: t.to_owned(),
                summary: None,
            }),
            children: Vec::new(),
        }
    }

    fn explore_input() -> Value {
        json!({ "subagent_type": "Explore", "description": "trace capture flow" })
    }

    #[test]
    fn non_agent_tools_are_not_collected() {
        let items = vec![TranscriptItem::Tool {
            id: "b1".to_owned(),
            name: "Bash".to_owned(),
            input: json!({ "command": "ls" }),
            status: ToolStatus::Completed,
            output: None,
            children: Vec::new(),
        }];
        assert!(collect(&items, &[]).is_empty());
    }

    #[test]
    fn running_task_reads_running_with_type_and_description() {
        let items = vec![task("t1", explore_input(), ToolStatus::Running, None)];
        let agents = collect(&items, &[]);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type.as_deref(), Some("Explore"));
        assert_eq!(agents[0].description.as_deref(), Some("trace capture flow"));
        assert_eq!(agents[0].state, AgentRunState::Running);
        assert!(agents[0].state.is_active());
        assert_eq!(agents[0].title(), "Explore: trace capture flow");
    }

    #[test]
    fn completed_task_reads_finished_with_result() {
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Completed,
            Some("Found it in capture.rs"),
        )];
        let agents = collect(&items, &[]);
        assert_eq!(agents[0].state, AgentRunState::Finished);
        assert_eq!(agents[0].result, "Found it in capture.rs");
    }

    #[test]
    fn failed_task_reads_failed() {
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Failed,
            Some("boom"),
        )];
        assert_eq!(collect(&items, &[])[0].state, AgentRunState::Failed);
    }

    #[test]
    fn agent_tool_name_is_collected_too() {
        let items = vec![TranscriptItem::Tool {
            id: "a1".to_owned(),
            name: "Agent".to_owned(),
            input: json!({ "description": "review the diff" }),
            status: ToolStatus::Running,
            output: None,
            children: Vec::new(),
        }];
        let agents = collect(&items, &[]);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].title(), "review the diff");
    }

    #[test]
    fn nested_agents_are_collected_in_order() {
        let child = task(
            "inner",
            json!({ "description": "sub" }),
            ToolStatus::Running,
            None,
        );
        let items = vec![TranscriptItem::Tool {
            id: "outer".to_owned(),
            name: "Task".to_owned(),
            input: explore_input(),
            status: ToolStatus::Running,
            output: None,
            children: vec![child],
        }];
        let agents = collect(&items, &[]);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "outer");
        assert_eq!(agents[1].id, "inner");
    }

    #[test]
    fn missing_type_and_description_fall_back_to_generic_title() {
        let items = vec![task("t1", Value::Null, ToolStatus::Running, None)];
        assert_eq!(collect(&items, &[])[0].title(), "Agent");
    }

    /// The acknowledgement a background agent launch returns — completion
    /// arrives later as a task notification.
    const BACKGROUND_ACK: &str = "Async agent launched with ID: agent-3f9. \
        You will be notified when it completes.";

    fn notification(
        task_id: &str,
        tool_use_id: Option<&str>,
        status: TaskRunStatus,
        summary: Option<&str>,
    ) -> TaskNotification {
        TaskNotification {
            task_id: task_id.to_owned(),
            tool_use_id: tool_use_id.map(str::to_owned),
            status,
            summary: summary.map(str::to_owned),
        }
    }

    #[test]
    fn background_launch_stays_running_despite_completed_status() {
        // The launch's tool result is only the ack; the agent is still working.
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Completed,
            Some(BACKGROUND_ACK),
        )];
        assert_eq!(collect(&items, &[])[0].state, AgentRunState::Running);
    }

    #[test]
    fn notification_by_tool_use_id_finishes_background_agent() {
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Completed,
            Some(BACKGROUND_ACK),
        )];
        let notes = [notification(
            "agent-3f9",
            Some("t1"),
            TaskRunStatus::Completed,
            Some("Agent finished: found 3 call sites"),
        )];
        let agents = collect(&items, &notes);
        assert_eq!(agents[0].state, AgentRunState::Finished);
        assert!(
            agents[0].result.contains("3 call sites"),
            "summary captured"
        );
    }

    #[test]
    fn notification_matches_by_task_id_when_tool_use_id_missing() {
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Completed,
            Some(BACKGROUND_ACK),
        )];
        let notes = [notification("agent-3f9", None, TaskRunStatus::Failed, None)];
        assert_eq!(collect(&items, &notes)[0].state, AgentRunState::Failed);
    }

    #[test]
    fn notification_for_unknown_task_is_ignored() {
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Completed,
            Some(BACKGROUND_ACK),
        )];
        let notes = [notification(
            "other",
            Some("toolu_other"),
            TaskRunStatus::Completed,
            None,
        )];
        assert_eq!(
            collect(&items, &notes)[0].state,
            AgentRunState::Running,
            "an unmatched notification must not retire someone else's agent"
        );
    }

    #[test]
    fn task_stop_marks_background_agent_stopped() {
        let items = vec![
            task(
                "t1",
                explore_input(),
                ToolStatus::Completed,
                Some(BACKGROUND_ACK),
            ),
            TranscriptItem::Tool {
                id: "stop1".to_owned(),
                name: "TaskStop".to_owned(),
                input: json!({ "task_id": "agent-3f9" }),
                status: ToolStatus::Completed,
                output: None,
                children: Vec::new(),
            },
        ];
        assert_eq!(collect(&items, &[])[0].state, AgentRunState::Stopped);
    }

    #[test]
    fn notification_does_not_reanimate_stopped_agent() {
        // The stop observed in the transcript is sticky; the notification that
        // follows must not flip the state, but its summary is still captured.
        let items = vec![
            task(
                "t1",
                explore_input(),
                ToolStatus::Completed,
                Some(BACKGROUND_ACK),
            ),
            TranscriptItem::Tool {
                id: "stop1".to_owned(),
                name: "TaskStop".to_owned(),
                input: json!({ "task_id": "agent-3f9" }),
                status: ToolStatus::Completed,
                output: None,
                children: Vec::new(),
            },
        ];
        let notes = [notification(
            "agent-3f9",
            Some("t1"),
            TaskRunStatus::Completed,
            Some("late summary"),
        )];
        let agents = collect(&items, &notes);
        assert_eq!(agents[0].state, AgentRunState::Stopped);
        assert!(agents[0].result.contains("late summary"));
    }

    #[test]
    fn inline_result_mentioning_background_is_not_a_launch_ack() {
        // An inline agent whose *result* happens to mention "background" must
        // still read finished — only explicit ack phrasings count.
        let items = vec![task(
            "t1",
            explore_input(),
            ToolStatus::Completed,
            Some("The daemon forks itself into a background process."),
        )];
        assert_eq!(collect(&items, &[])[0].state, AgentRunState::Finished);
    }

    #[test]
    fn two_agents_keep_launch_order_and_distinct_ids() {
        let items = vec![
            task(
                "a",
                json!({ "description": "first" }),
                ToolStatus::Running,
                None,
            ),
            task(
                "b",
                json!({ "description": "second" }),
                ToolStatus::Running,
                None,
            ),
        ];
        let agents = collect(&items, &[]);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "a");
        assert_eq!(agents[1].description.as_deref(), Some("second"));
    }

    #[test]
    fn parse_task_id_handles_phrasings() {
        assert_eq!(
            parse_task_id("launched with ID: agent-3f9.").as_deref(),
            Some("agent-3f9")
        );
        assert_eq!(parse_task_id("agent-id=abc123").as_deref(), Some("abc123"));
        assert_eq!(parse_task_id("the id is abc123 now"), None);
        assert_eq!(parse_task_id("nothing useful"), None);
    }
}
