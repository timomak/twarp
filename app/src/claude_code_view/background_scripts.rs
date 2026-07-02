//! Background-scripts model for the Claude Code pane (twarp feature 07).
//!
//! Claude Code can launch a shell command *in the background* — a `Bash` tool
//! call with `run_in_background: true`. The launch returns immediately with a
//! shell id; the process keeps running, and Claude later polls it (`BashOutput`,
//! matched by `bash_id`) or stops it (`KillShell`, matched by `shell_id`). In the
//! flat transcript those are three unrelated tool cards scattered across turns,
//! so a long-running dev server or watcher is easy to lose track of.
//!
//! This module derives a per-chat **background-scripts view** from the
//! transcript: it walks the [`TranscriptItem`]s (descending into `Task`
//! children), pairs each background launch with its follow-up polls/kills by
//! shell id, and produces an ordered list the pane renders as a floating status
//! panel. It is intentionally headless (no UI) so the extraction and the fragile
//! wire-shape parsing can be unit-tested without a window — the same split
//! [`super::tool_cards`] uses for its formatting helpers.
//!
//! Extraction is read-only and best-effort: twarp can observe the background
//! scripts Claude launched, but cannot itself start, poll, or kill them (those
//! are the model's tool calls). State that the transcript doesn't make explicit
//! degrades to [`BackgroundScriptState::Running`] rather than guessing.

use claude_code::{ToolStatus, TranscriptItem};
use serde_json::Value;

/// Run state of a background script, derived from the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackgroundScriptState {
    /// The launching `Bash` call itself failed — the script never started.
    LaunchFailed,
    /// Launched and, as far as the transcript shows, still running. The default
    /// when nothing later marks it killed or finished.
    Running,
    /// A `KillShell` / `KillBash` was issued for this shell.
    Killed,
    /// A `BashOutput` poll reported the shell completed / exited.
    Finished,
}

impl BackgroundScriptState {
    /// A short, human label for the status pill.
    pub(super) fn label(self) -> &'static str {
        match self {
            BackgroundScriptState::LaunchFailed => "failed to start",
            BackgroundScriptState::Running => "running",
            BackgroundScriptState::Killed => "stopped",
            BackgroundScriptState::Finished => "finished",
        }
    }

    /// Whether the process is still believed to be alive (drives the running
    /// indicator and the panel's "N running" count).
    pub(super) fn is_active(self) -> bool {
        matches!(self, BackgroundScriptState::Running)
    }
}

/// One background script launched in this chat.
#[derive(Debug, Clone)]
pub(super) struct BackgroundScript {
    /// The launching tool-use id — a stable key for per-row UI state, unique even
    /// when two scripts share a command line.
    pub id: String,
    /// The shell / bash id Claude assigned, when parseable from the launch
    /// result. `None` leaves follow-up polls/kills unmatchable, but the launch
    /// row still renders.
    pub shell_id: Option<String>,
    /// The command line (`Bash`'s `command`).
    pub command: String,
    /// The human description Claude attached to the call, when present.
    pub description: Option<String>,
    pub state: BackgroundScriptState,
    /// Captured output, newest last: the launch acknowledgement followed by every
    /// `BashOutput` poll for this shell. Empty when nothing has been captured.
    pub output: String,
}

/// The `run_in_background: true` flag on a `Bash` input (defensive — absent or a
/// non-bool reads as foreground).
fn is_background_bash(name: &str, input: &Value) -> bool {
    name == "Bash"
        && input
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
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

/// The shell id a follow-up `BashOutput` / `KillShell` call targets. Claude uses
/// `bash_id` for polls and `shell_id` for kills; accept either (plus a bare
/// `id`) so a wire rename on one doesn't silently unmatch the rows.
fn referenced_shell_id(input: &Value) -> Option<String> {
    str_field(input, "bash_id")
        .or_else(|| str_field(input, "shell_id"))
        .or_else(|| str_field(input, "id"))
}

/// Parse the shell id out of a background launch's acknowledgement text. The
/// exact phrasing has drifted across `claude` builds, so match loosely: prefer
/// an explicit `bash_…` token, else the word right after an "id" marker. `None`
/// when nothing id-shaped is found (follow-ups then can't match this script).
fn parse_shell_id(ack: &str) -> Option<String> {
    let is_id_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    // Most reliable: the conventional `bash_<n>` shell handle, wherever it sits.
    if let Some(pos) = ack.find("bash_") {
        let token: String = ack[pos..].chars().take_while(|&c| is_id_char(c)).collect();
        if token.len() > "bash_".len() {
            return Some(token);
        }
    }
    // Fallback: a *labeled* id — "id" followed by a `:` or `=` separator and the
    // value token (e.g. "ID: abc123", "shell-id=abc123"). Requiring the separator
    // avoids latching onto prose ("id is abc" would otherwise yield "is").
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

/// Whether a `BashOutput` poll result signals the shell is no longer running.
/// Conservative: only an explicit status marker counts, so ordinary output that
/// happens to contain the word "completed" doesn't falsely retire a live script.
fn poll_reports_done(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "<status>completed",
        "<status>killed",
        "status: completed",
        "status: killed",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Append captured output to a script, keeping a blank-line separator between
/// the launch ack and successive polls so the expanded box reads in order.
fn append_output(script: &mut BackgroundScript, more: &str) {
    let more = more.trim();
    if more.is_empty() {
        return;
    }
    if !script.output.is_empty() {
        script.output.push_str("\n\n");
    }
    script.output.push_str(more);
}

/// Collect every background script launched in `items` (descending into `Task`
/// children), in launch order. A `Bash` call with `run_in_background: true` is a
/// launch; its later `BashOutput` / `KillShell` calls — matched by shell id —
/// refine its [`state`](BackgroundScript::state) and append to its output.
pub(super) fn collect(items: &[TranscriptItem]) -> Vec<BackgroundScript> {
    let mut scripts: Vec<BackgroundScript> = Vec::new();
    walk(items, &mut scripts);
    scripts
}

/// Index of the script owning `shell_id`, searching newest-first so a recycled
/// shell handle attributes to its most recent launch.
fn script_for_shell<'a>(
    scripts: &'a mut [BackgroundScript],
    shell_id: &str,
) -> Option<&'a mut BackgroundScript> {
    scripts
        .iter_mut()
        .rev()
        .find(|s| s.shell_id.as_deref() == Some(shell_id))
}

fn walk(items: &[TranscriptItem], scripts: &mut Vec<BackgroundScript>) {
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

        if is_background_bash(name, input) {
            if let Some(command) = str_field(input, "command") {
                let ack = output.as_ref().map(|o| o.text.as_str()).unwrap_or("");
                let state = if *status == ToolStatus::Failed {
                    BackgroundScriptState::LaunchFailed
                } else if poll_reports_done(ack) {
                    // A short-lived command (a `cd`, a one-shot script) can exit
                    // before Claude ever polls it — some builds report that in
                    // the launch acknowledgement itself. Honor an explicit status
                    // marker here so such scripts don't sit at "running" forever.
                    BackgroundScriptState::Finished
                } else {
                    BackgroundScriptState::Running
                };
                let mut script = BackgroundScript {
                    id: id.clone(),
                    shell_id: parse_shell_id(ack),
                    command,
                    description: str_field(input, "description"),
                    state,
                    output: String::new(),
                };
                append_output(&mut script, ack);
                scripts.push(script);
            }
        } else if name == "BashOutput" {
            if let Some(shell_id) = referenced_shell_id(input) {
                if let Some(script) = script_for_shell(scripts, &shell_id) {
                    if let Some(out) = output {
                        append_output(script, &out.text);
                        // A kill is sticky; only a Running script retires on a
                        // poll that reports completion.
                        if script.state == BackgroundScriptState::Running
                            && poll_reports_done(&out.text)
                        {
                            script.state = BackgroundScriptState::Finished;
                        }
                    }
                }
            }
        } else if name == "KillShell" || name == "KillBash" {
            if let Some(shell_id) = referenced_shell_id(input) {
                if let Some(script) = script_for_shell(scripts, &shell_id) {
                    if script.state == BackgroundScriptState::Running {
                        script.state = BackgroundScriptState::Killed;
                    }
                }
            }
        }

        // A background script can be launched by a `Task` sub-agent; its nested
        // calls live in the parent card's children (PRODUCT §19).
        if !children.is_empty() {
            walk(children, scripts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_code::ToolOutput;
    use serde_json::json;

    fn launch(id: &str, command: &str, ack: &str) -> TranscriptItem {
        TranscriptItem::Tool {
            id: id.to_owned(),
            name: "Bash".to_owned(),
            input: json!({ "command": command, "run_in_background": true }),
            status: ToolStatus::Completed,
            output: Some(ToolOutput {
                text: ack.to_owned(),
                summary: None,
            }),
            children: Vec::new(),
        }
    }

    fn tool(name: &str, input: Value, output: Option<&str>) -> TranscriptItem {
        TranscriptItem::Tool {
            id: format!("{name}_call"),
            name: name.to_owned(),
            input,
            status: ToolStatus::Completed,
            output: output.map(|t| ToolOutput {
                text: t.to_owned(),
                summary: None,
            }),
            children: Vec::new(),
        }
    }

    #[test]
    fn foreground_bash_is_not_a_background_script() {
        let items = vec![tool("Bash", json!({ "command": "ls" }), Some("a.txt"))];
        assert!(collect(&items).is_empty());
        let explicit_false = vec![tool(
            "Bash",
            json!({ "command": "ls", "run_in_background": false }),
            None,
        )];
        assert!(collect(&explicit_false).is_empty());
    }

    #[test]
    fn background_launch_starts_running_with_command() {
        let items = vec![launch(
            "t1",
            "npm run dev",
            "Running in background with ID: bash_1",
        )];
        let scripts = collect(&items);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].command, "npm run dev");
        assert_eq!(scripts[0].shell_id.as_deref(), Some("bash_1"));
        assert_eq!(scripts[0].state, BackgroundScriptState::Running);
    }

    #[test]
    fn failed_launch_reads_failed_to_start() {
        let items = vec![TranscriptItem::Tool {
            id: "t1".to_owned(),
            name: "Bash".to_owned(),
            input: json!({ "command": "false", "run_in_background": true }),
            status: ToolStatus::Failed,
            output: None,
            children: Vec::new(),
        }];
        assert_eq!(
            collect(&items)[0].state,
            BackgroundScriptState::LaunchFailed
        );
    }

    #[test]
    fn kill_shell_marks_stopped() {
        let items = vec![
            launch("t1", "tail -f log", "Started bash_7"),
            tool("KillShell", json!({ "shell_id": "bash_7" }), Some("killed")),
        ];
        let scripts = collect(&items);
        assert_eq!(scripts.len(), 1, "kill is not its own script row");
        assert_eq!(scripts[0].state, BackgroundScriptState::Killed);
    }

    #[test]
    fn bash_output_completion_marker_finishes_script() {
        let items = vec![
            launch("t1", "make build", "ID: bash_3"),
            tool(
                "BashOutput",
                json!({ "bash_id": "bash_3" }),
                Some("build ok\n<status>completed</status>"),
            ),
        ];
        let scripts = collect(&items);
        assert_eq!(scripts[0].state, BackgroundScriptState::Finished);
        assert!(scripts[0].output.contains("build ok"));
    }

    #[test]
    fn launch_ack_reporting_done_finishes_without_a_poll() {
        // A fast command whose launch acknowledgement already carries a status
        // marker is finished immediately — no BashOutput poll required.
        let items = vec![launch(
            "t1",
            "cd /tmp",
            "started bash_1\n<status>completed</status>",
        )];
        assert_eq!(collect(&items)[0].state, BackgroundScriptState::Finished);
    }

    #[test]
    fn bash_output_without_status_marker_stays_running() {
        // Ordinary streamed output that merely mentions "completed" must not
        // retire a live script.
        let items = vec![
            launch("t1", "watch", "bash_9"),
            tool(
                "BashOutput",
                json!({ "bash_id": "bash_9" }),
                Some("task 3 completed, still watching…"),
            ),
        ];
        assert_eq!(collect(&items)[0].state, BackgroundScriptState::Running);
    }

    #[test]
    fn poll_after_kill_does_not_reanimate() {
        let items = vec![
            launch("t1", "server", "bash_2"),
            tool("KillShell", json!({ "shell_id": "bash_2" }), None),
            tool(
                "BashOutput",
                json!({ "bash_id": "bash_2" }),
                Some("late output"),
            ),
        ];
        let scripts = collect(&items);
        assert_eq!(scripts[0].state, BackgroundScriptState::Killed);
        assert!(
            scripts[0].output.contains("late output"),
            "output still captured"
        );
    }

    #[test]
    fn nested_task_child_launch_is_collected() {
        let items = vec![TranscriptItem::Tool {
            id: "task1".to_owned(),
            name: "Task".to_owned(),
            input: Value::Null,
            status: ToolStatus::Running,
            output: None,
            children: vec![launch("child", "cargo watch", "bash_5")],
        }];
        let scripts = collect(&items);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].command, "cargo watch");
    }

    #[test]
    fn two_scripts_keep_launch_order_and_distinct_ids() {
        let items = vec![
            launch("a", "first", "bash_1"),
            launch("b", "second", "bash_2"),
        ];
        let scripts = collect(&items);
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].id, "a");
        assert_eq!(scripts[1].command, "second");
    }

    #[test]
    fn parse_shell_id_handles_phrasings() {
        // The conventional `bash_<n>` handle is found wherever it sits.
        assert_eq!(
            parse_shell_id("Command running with ID: bash_42").as_deref(),
            Some("bash_42")
        );
        assert_eq!(parse_shell_id("started bash_7").as_deref(), Some("bash_7"));
        // A labeled, separator-delimited id is the fallback.
        assert_eq!(
            parse_shell_id("shell-id: abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(parse_shell_id("ID=xyz-9").as_deref(), Some("xyz-9"));
        // Prose without a `bash_` handle or a labeled id yields nothing, rather
        // than latching onto a stray word ("id is abc" must not return "is").
        assert_eq!(parse_shell_id("shell id is abc123 now"), None);
        assert_eq!(parse_shell_id("nothing useful"), None);
    }
}
