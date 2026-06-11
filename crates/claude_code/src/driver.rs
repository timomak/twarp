//! Subprocess driver for the local `claude` CLI (PRODUCT §8–§22, §52–§57).
//!
//! Spawns `claude -p --input-format stream-json --output-format stream-json
//! --verbose`, parses its JSONL output defensively line-by-line, and emits
//! [`TranscriptEvent`]s — the UI never sees raw `claude` JSON. Used by the
//! Claude Code panel; headless and unit-testable here.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
pub use async_process::Child;
use async_process::{ChildStderr, ChildStdin, ChildStdout};
use futures::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use futures::stream::Stream;
use serde_json::{json, Value};

use crate::{EndReason, TodoItem, TodoStatus, ToolOutput, TranscriptEvent, Usage};

/// Permission mode passed to `claude --permission-mode`. The CLI argument
/// names are the ones Claude Code itself accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    /// Prompt for each tool. Default. Without interactive prompt handling
    /// (PRODUCT §39 — wire protocol is undocumented, see TECH §Risks),
    /// `default` mode blocks. The selector lets the user pick a non-prompting
    /// mode for the session.
    Default,
    /// File edits proceed without prompting; bash/network still prompt.
    AcceptEdits,
    /// Read-only / plan mode — model can read and reason but not modify.
    Plan,
    /// Skip all prompts. Convenient for the smoke test; the trade-off is the
    /// session can run any tool without confirmation.
    BypassPermissions,
}

impl PermissionMode {
    pub fn as_cli_arg(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::BypassPermissions => "bypassPermissions",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Prompt for each tool",
            Self::AcceptEdits => "Auto-accept edits",
            Self::Plan => "Plan / read-only",
            Self::BypassPermissions => "Skip prompts",
        }
    }

    pub const ALL: [PermissionMode; 4] = [
        PermissionMode::BypassPermissions,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::Default,
    ];
}

/// Options for [`spawn_session`].
#[derive(Clone, Debug)]
pub struct SpawnOptions {
    pub cwd: PathBuf,
    pub model: Option<String>,
    /// `--effort <level>` — settable but not echoed back by the headless
    /// stream (see the #74 notes), so the pane treats it as write-only.
    pub effort: Option<String>,
    pub resume_session_id: Option<String>,
    /// Pin a fresh session's id (`--session-id`, PRODUCT §41): the pane owns
    /// its session identity from birth, so the raw-CLI toggle and mode
    /// restarts never hit a "no id yet" window. Ignored when resuming —
    /// `--resume` continues the existing id (`--fork-session` is never
    /// passed).
    pub session_id: Option<String>,
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
}

/// A live `claude` session: the child process, a writer for user messages on
/// its stdin, and a stream of [`TranscriptEvent`]s parsed off its stdout.
///
/// Drop kills the child (the spawn sets `kill_on_drop(true)`, PRODUCT §15).
pub struct SpawnedSession {
    pub child: Child,
    pub stdin: ChildStdin,
    pub events: Pin<Box<dyn Stream<Item = TranscriptEvent> + Send>>,
}

/// Spawn `claude` with stream-json IO. PRODUCT §8: the session is one
/// long-lived process driven multi-turn via stdin.
pub fn spawn_session(opts: SpawnOptions) -> Result<SpawnedSession> {
    let mut cmd = command::r#async::Command::new("claude");
    cmd.arg("-p")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--permission-mode")
        .arg(opts.permission_mode.as_cli_arg());

    if let Some(model) = &opts.model {
        cmd.arg("--model").arg(model);
    }
    if let Some(effort) = &opts.effort {
        cmd.arg("--effort").arg(effort);
    }
    if let Some(id) = &opts.resume_session_id {
        cmd.arg("--resume").arg(id);
    } else if let Some(id) = &opts.session_id {
        // A fresh session under a pane-chosen id (PRODUCT §41).
        cmd.arg("--session-id").arg(id);
    }
    if !opts.allowed_tools.is_empty() {
        cmd.arg("--allowedTools").arg(opts.allowed_tools.join(","));
    }

    cmd.current_dir(&opts.cwd)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn `claude`: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Failed to capture claude stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture claude stdout"))?;
    // stderr is drained only after stdout EOF (the process is exiting then) so
    // a startup failure — bad --resume id, auth problem — surfaces verbatim
    // instead of as a bare "ended unexpectedly" (PRODUCT §30, §37).
    let stderr = child.stderr.take();

    let events = event_stream(stdout, stderr);
    Ok(SpawnedSession {
        child,
        stdin,
        events: Box::pin(events),
    })
}

/// Send a SIGINT to the live `claude` process to interrupt the current turn
/// without ending the session (PRODUCT §11). Best-effort: Unix only — on
/// other platforms Stop falls back to ending the session via drop.
pub fn interrupt(child: &Child) {
    #[cfg(unix)]
    {
        let pid = child.id();
        // Signal sending is async-signal-safe; no Rust safety concern.
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child;
        log::warn!("Stop is not implemented on this platform; drop the session to terminate.");
    }
}

/// Write a user turn into the live session's stdin in the JSONL shape
/// `claude --input-format stream-json` expects (PRODUCT §16).
pub async fn send_user_message(stdin: &mut ChildStdin, text: &str) -> Result<()> {
    let line = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": text,
        },
    })
    .to_string();
    stdin
        .write_all(line.as_bytes())
        .await
        .context("write user message to claude stdin")?;
    stdin
        .write_all(b"\n")
        .await
        .context("write newline to claude stdin")?;
    stdin.flush().await.context("flush claude stdin")?;
    Ok(())
}

struct StreamState {
    reader: Option<BufReader<ChildStdout>>,
    stderr: Option<ChildStderr>,
    buffered: VecDeque<TranscriptEvent>,
}

/// Cap on the stderr tail surfaced when the process dies (PRODUCT §30 —
/// verbatim, but bounded so a runaway stderr can't flood the transcript).
const STDERR_TAIL_MAX_BYTES: usize = 4 * 1024;

impl StreamState {
    /// The terminal event once stdout is done: drain the (now-EOF'd) stderr
    /// and surface it verbatim if the process left an explanation — a bad
    /// `--resume` id, an auth failure (PRODUCT §30, §37). Empty stderr keeps
    /// the generic Exited notice.
    async fn end_event(&mut self) -> TranscriptEvent {
        self.reader = None;
        let mut tail = String::new();
        if let Some(stderr) = self.stderr.take() {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader.read_line(&mut line).await.is_ok_and(|n| n > 0) {
                if tail.len() + line.len() > STDERR_TAIL_MAX_BYTES {
                    break;
                }
                tail.push_str(&line);
                line.clear();
            }
        }
        let tail = tail.trim();
        if tail.is_empty() {
            TranscriptEvent::Ended {
                reason: EndReason::Exited,
            }
        } else {
            TranscriptEvent::Ended {
                reason: EndReason::Error(tail.to_owned()),
            }
        }
    }
}

fn event_stream(
    stdout: ChildStdout,
    stderr: Option<ChildStderr>,
) -> impl Stream<Item = TranscriptEvent> + Send {
    let state = StreamState {
        reader: Some(BufReader::new(stdout)),
        stderr,
        buffered: VecDeque::new(),
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            // Drain the buffer first — a single JSONL line can map to several
            // TranscriptEvents (e.g. an assistant turn with text + tool_use).
            if let Some(evt) = state.buffered.pop_front() {
                return Some((evt, state));
            }
            let mut line = String::new();
            let read = {
                let reader = state.reader.as_mut()?;
                reader.read_line(&mut line).await
            };
            match read {
                Ok(0) => {
                    // EOF — `claude` exited. Surface it once (with the stderr
                    // tail when there is one), then stop the stream so
                    // spawn_stream_local fires its on_done callback.
                    let event = state.end_event().await;
                    return Some((event, state));
                }
                Ok(_) => {}
                Err(err) => {
                    log::warn!("claude stdout read failed: {err}");
                    let event = state.end_event().await;
                    return Some((event, state));
                }
            }
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(err) => {
                    // PRODUCT §53: a non-JSON line is dropped and noted, not
                    // fatal.
                    log::warn!("claude: dropped non-JSON line: {err}");
                    continue;
                }
            };
            parse_event_into(&value, &mut state.buffered);
        }
    })
}

/// Translate one parsed stream-json value into zero or more
/// [`TranscriptEvent`]s. Defensive: unknown event types and missing optional
/// fields are tolerated (PRODUCT §53).
fn parse_event_into(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    let Some(ty) = value.get("type").and_then(|v| v.as_str()) else {
        log::warn!("claude: event without `type` field, dropped");
        return;
    };
    match ty {
        "system" => parse_system(value, out),
        "assistant" => parse_assistant(value, out),
        "user" => parse_user_event(value, out),
        "result" => parse_result(value, out),
        "stream_event" => {
            // 7c doesn't request `--include-partial-messages`; ignore deltas
            // if they ever arrive.
        }
        other => log::debug!("claude: ignoring unknown event type `{other}`"),
    }
}

fn parse_system(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    if value.get("subtype").and_then(|v| v.as_str()) != Some("init") {
        return;
    }
    let Some(session_id) = value
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
    else {
        return;
    };
    let cwd = value
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_default();
    let str_field = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    out.push_back(TranscriptEvent::SessionInit {
        session_id,
        cwd,
        model: str_field("model"),
        permission_mode: str_field("permissionMode"),
        fast_mode: str_field("fast_mode_state"),
    });
}

fn parse_assistant(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    // Set when this message comes from a `Task` sub-agent: the id of the Task
    // tool call that spawned it. Child tool calls nest under that card
    // (PRODUCT §19); sub-agent prose/thinking is internal monologue — its
    // product comes back as the Task's tool_result — so it is not rendered as
    // main-transcript content.
    let parent_id = value
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    for block in content {
        let Some(ty) = block.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        match ty {
            "text" if parent_id.is_none() => {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        // Whole-message path (PRODUCT §17): one delta + done.
                        // If we ever opt into `--include-partial-messages`,
                        // multiple deltas come through `stream_event` and this
                        // arm only emits the final consolidated text.
                        out.push_back(TranscriptEvent::AssistantTextDelta {
                            text: text.to_owned(),
                        });
                        out.push_back(TranscriptEvent::AssistantTextDone);
                    }
                }
            }
            "thinking" if parent_id.is_none() => {
                // `claude` emits empty thinking blocks (signature-only) before
                // tool calls; a card for them would be an empty artifact
                // (PRODUCT §22: a turn with no thinking shows no card).
                if let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) {
                    if !thinking.trim().is_empty() {
                        out.push_back(TranscriptEvent::Thinking {
                            text: thinking.to_owned(),
                            duration: None,
                        });
                    }
                }
            }
            "tool_use" => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                if !id.is_empty() && !name.is_empty() {
                    // 7f: a top-level `TodoWrite` IS the session's task list —
                    // route it to the in-place Todos item instead of a tool
                    // card (PRODUCT §17, §23). A sub-agent's TodoWrite stays a
                    // nested card: its list belongs to the sub-agent, not the
                    // session. Malformed input falls back to the tool card so
                    // something always renders (PRODUCT §29).
                    if name == "TodoWrite" && parent_id.is_none() {
                        if let Some(todos) = parse_todos(&input) {
                            out.push_back(TranscriptEvent::Todos(todos));
                            continue;
                        }
                    }
                    out.push_back(TranscriptEvent::ToolCall {
                        id,
                        name,
                        input,
                        parent_id: parent_id.clone(),
                    });
                }
            }
            "text" | "thinking" => {
                // Sub-agent prose/thinking (parent_id set) — skipped, see above.
            }
            _ => {
                // Unknown content-block type — skip (PRODUCT §53).
            }
        }
    }
}

fn parse_user_event(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    // `user` events on the way back from claude carry tool results, not user
    // turns (the user's own messages are echoed via stdin only).
    let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let id = block
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        if id.is_empty() {
            continue;
        }
        let is_error = block
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let text = extract_tool_result_text(block.get("content"));
        out.push_back(TranscriptEvent::ToolResult {
            id,
            output: ToolOutput {
                text,
                summary: None,
            },
            is_error,
        });
    }
}

/// Parse a `TodoWrite` input into the task list (PRODUCT §23). `None` when the
/// shape is unfamiliar — the caller falls back to a plain tool card (§29).
/// Items missing text are skipped; unknown statuses degrade to pending.
fn parse_todos(input: &Value) -> Option<Vec<TodoItem>> {
    let todos = input.get("todos")?.as_array()?;
    Some(
        todos
            .iter()
            .filter_map(|todo| {
                let text = todo
                    .get("content")
                    .or_else(|| todo.get("text"))?
                    .as_str()?
                    .to_owned();
                let status = match todo.get("status").and_then(|v| v.as_str()) {
                    Some("in_progress") => TodoStatus::InProgress,
                    Some("completed") => TodoStatus::Completed,
                    _ => TodoStatus::Pending,
                };
                Some(TodoItem { text, status })
            })
            .collect(),
    )
}

/// Translate one line of `claude`'s on-disk session `.jsonl` into transcript
/// events — the 7h resume path renders a stored session's history through the
/// same pipeline as live output (PRODUCT §36).
///
/// The session file is a superset of the live stream: it also holds user
/// *turns* (string or text-block content — the live stream only echoes tool
/// results back on `user` events), bookkeeping lines (`mode`, `attachment`,
/// `file-history-snapshot`, …, skipped), meta turns (`isMeta`, skipped), and
/// sub-agent sidechains (`isSidechain`, skipped — their product returns as the
/// spawning Task's tool result).
pub(crate) fn parse_history_line(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    let flag = |key: &str| value.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    if flag("isMeta") || flag("isSidechain") {
        return;
    }
    match value.get("type").and_then(|v| v.as_str()) {
        Some("user") => {
            let content = value.get("message").and_then(|m| m.get("content"));
            let text = match content {
                Some(Value::String(s)) => s.trim().to_owned(),
                Some(Value::Array(blocks)) => blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_owned(),
                _ => String::new(),
            };
            if !text.is_empty() {
                out.push_back(TranscriptEvent::UserMessage(text));
            }
            // Tool results ride on `user` lines in the file exactly like the
            // live stream; reuse that path (a text-only line pushes nothing).
            parse_user_event(value, out);
        }
        Some("assistant") => parse_assistant(value, out),
        // `system`/`result` never appear in the file; bookkeeping types
        // (`mode`, `attachment`, `summary`, …) carry no transcript content.
        _ => {}
    }
}

fn extract_tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|v| v.as_str()).map(str::to_owned))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_result(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    // Token usage + context window land before `Ended` so the panel's context
    // chip is up to date by the time the turn closes.
    if let Some(usage) = parse_usage(value) {
        out.push_back(TranscriptEvent::Usage(usage));
    }
    let is_error = value
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reason = if is_error {
        let message = value
            .get("result")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| "claude reported an error".to_string());
        EndReason::Error(message)
    } else {
        EndReason::Completed
    };
    out.push_back(TranscriptEvent::Ended { reason });
}

/// Extract token usage from a `result` message's `usage` block, plus the
/// context window from `modelUsage[model].contextWindow` (there is one model
/// entry per turn). Returns `None` if the message carries no `usage`.
fn parse_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let context_window = value
        .get("modelUsage")
        .and_then(|m| m.as_object())
        .and_then(|m| m.values().next())
        .and_then(|model| model.get("contextWindow"))
        .and_then(|v| v.as_u64());
    Some(Usage {
        input_tokens: count("input_tokens"),
        cache_read_input_tokens: count("cache_read_input_tokens"),
        cache_creation_input_tokens: count("cache_creation_input_tokens"),
        output_tokens: count("output_tokens"),
        context_window,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init_event() {
        let v: Value = serde_json::from_str(
            r#"{"type":"system","subtype":"init","session_id":"abc","cwd":"/tmp/p","model":"sonnet"}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::SessionInit {
                session_id,
                cwd,
                model,
                ..
            }) => {
                assert_eq!(session_id, "abc");
                assert_eq!(cwd, &PathBuf::from("/tmp/p"));
                assert_eq!(model.as_deref(), Some("sonnet"));
            }
            other => panic!("expected SessionInit, got {other:?}"),
        }
    }

    #[test]
    fn parses_result_usage_and_context_window() {
        let v: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":5102,"cache_read_input_tokens":15818,"cache_creation_input_tokens":5450,"output_tokens":4},"modelUsage":{"claude-fable-5[1m]":{"contextWindow":1000000}}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        // Usage is emitted before Ended so the chip is fresh when the turn closes.
        match out.front() {
            Some(TranscriptEvent::Usage(u)) => {
                assert_eq!(u.context_used(), 5102 + 15818 + 5450);
                assert_eq!(u.context_window, Some(1_000_000));
                assert_eq!(u.output_tokens, 4);
            }
            other => panic!("expected Usage first, got {other:?}"),
        }
        assert!(matches!(out.back(), Some(TranscriptEvent::Ended { .. })));
    }

    #[test]
    fn parses_assistant_text_emits_delta_plus_done() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], TranscriptEvent::AssistantTextDelta { text } if text == "hi"));
        assert!(matches!(out[1], TranscriptEvent::AssistantTextDone));
    }

    #[test]
    fn parses_tool_use_and_then_tool_result() {
        let v1: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"README.md"}}]}}"#,
        )
        .unwrap();
        let v2: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file contents","is_error":false}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v1, &mut out);
        parse_event_into(&v2, &mut out);
        assert_eq!(out.len(), 2);
        assert!(
            matches!(&out[0], TranscriptEvent::ToolCall { id, name, .. } if id == "t1" && name == "Read")
        );
        assert!(
            matches!(&out[1], TranscriptEvent::ToolResult { id, is_error: false, .. } if id == "t1")
        );
    }

    #[test]
    fn tool_result_content_array_concatenates_text_blocks() {
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"is_error":false}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match &out[0] {
            TranscriptEvent::ToolResult { output, .. } => assert_eq!(output.text, "a\nb"),
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn empty_thinking_blocks_emit_no_event() {
        // Live `claude` (2.1.170) emits signature-only thinking blocks with
        // empty text before tool calls; they must not become empty cards.
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"CAIS..."}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(
            out.is_empty(),
            "empty thinking must be skipped, got {out:?}"
        );
    }

    #[test]
    fn subagent_tool_use_carries_parent_id() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","parent_tool_use_id":"task_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"a.rs"}}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::ToolCall { id, parent_id, .. }) => {
                assert_eq!(id, "t2");
                assert_eq!(parent_id.as_deref(), Some("task_1"));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn subagent_text_and_thinking_are_not_main_transcript_content() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","parent_tool_use_id":"task_1","message":{"role":"assistant","content":[{"type":"text","text":"internal"},{"type":"thinking","thinking":"hmm"}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(
            out.is_empty(),
            "sub-agent prose must not render as assistant turns, got {out:?}"
        );
    }

    #[test]
    fn top_level_tool_use_has_no_parent_id() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(matches!(
            out.front(),
            Some(TranscriptEvent::ToolCall {
                parent_id: None,
                ..
            })
        ));
    }

    #[test]
    fn drops_unknown_event_types_quietly() {
        let v: Value = serde_json::from_str(r#"{"type":"some_future_type","foo":1}"#).unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn drops_event_without_type_quietly() {
        let v: Value = serde_json::from_str(r#"{"foo":1}"#).unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn drops_unknown_content_block_inside_assistant() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"future_block","payload":{}}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn top_level_todowrite_routes_to_todos_not_a_card() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"todos":[{"content":"step one","status":"pending"},{"content":"step two","status":"in_progress"},{"content":"done","status":"completed"}]}}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert_eq!(out.len(), 1);
        match out.front() {
            Some(TranscriptEvent::Todos(items)) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].status, TodoStatus::Pending);
                assert_eq!(items[1].status, TodoStatus::InProgress);
                assert_eq!(items[2].status, TodoStatus::Completed);
                assert_eq!(items[0].text, "step one");
            }
            other => panic!("expected Todos, got {other:?}"),
        }
    }

    #[test]
    fn subagent_todowrite_stays_a_nested_tool_card() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","parent_tool_use_id":"task_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"todos":[{"content":"a","status":"pending"}]}}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(
            matches!(
                out.front(),
                Some(TranscriptEvent::ToolCall { name, parent_id: Some(p), .. })
                    if name == "TodoWrite" && p == "task_1"
            ),
            "sub-agent TodoWrite must stay a card, got {out:?}"
        );
    }

    #[test]
    fn malformed_todowrite_falls_back_to_tool_card() {
        // PRODUCT §29: an unfamiliar shape renders as a card, never nothing.
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"unexpected":true}}]}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(matches!(
            out.front(),
            Some(TranscriptEvent::ToolCall { name, .. }) if name == "TodoWrite"
        ));
    }

    #[test]
    fn parse_todos_skips_textless_items_and_defaults_unknown_status() {
        let input: Value = serde_json::from_str(
            r#"{"todos":[{"content":"ok","status":"someday"},{"status":"pending"},{"text":"legacy key","status":"completed"}]}"#,
        )
        .unwrap();
        let todos = parse_todos(&input).expect("array parses");
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].text, "ok");
        assert_eq!(todos[0].status, TodoStatus::Pending);
        assert_eq!(todos[1].text, "legacy key");
        assert_eq!(todos[1].status, TodoStatus::Completed);
    }

    #[test]
    fn history_user_string_line_becomes_user_message() {
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":"fix the bug"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_history_line(&v, &mut out);
        assert!(matches!(
            out.front(),
            Some(TranscriptEvent::UserMessage(m)) if m == "fix the bug"
        ));
    }

    #[test]
    fn history_skips_meta_sidechain_and_bookkeeping_lines() {
        let lines = [
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"meta"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"sidechain prose"}]}}"#,
            r#"{"type":"mode","mode":"default"}"#,
            r#"{"type":"attachment","attachment":{}}"#,
            r#"{"type":"file-history-snapshot","snapshot":{}}"#,
            r#"{"type":"summary","summary":"compacted"}"#,
        ];
        let mut out = VecDeque::new();
        for line in lines {
            parse_history_line(&serde_json::from_str(line).unwrap(), &mut out);
        }
        assert!(out.is_empty(), "expected nothing, got {out:?}");
    }

    #[test]
    fn history_replays_assistant_and_tool_results() {
        let mut out = VecDeque::new();
        parse_history_line(
            &serde_json::from_str(
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.rs"}}]}}"#,
            )
            .unwrap(),
            &mut out,
        );
        parse_history_line(
            &serde_json::from_str(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"data","is_error":false}]}}"#,
            )
            .unwrap(),
            &mut out,
        );
        let kinds: Vec<&'static str> = out
            .iter()
            .map(|e| match e {
                TranscriptEvent::AssistantTextDelta { .. } => "delta",
                TranscriptEvent::AssistantTextDone => "done",
                TranscriptEvent::ToolCall { .. } => "call",
                TranscriptEvent::ToolResult { .. } => "result",
                TranscriptEvent::UserMessage(_) => "user",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["delta", "done", "call", "result"]);
    }

    #[test]
    fn parses_error_result_as_ended_error() {
        let v: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"max turns reached"}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::Ended {
                reason: EndReason::Error(m),
            }) => assert_eq!(m, "max turns reached"),
            other => panic!("expected Ended(Error), got {other:?}"),
        }
    }

    #[test]
    fn parses_success_result_as_ended_completed() {
        let v: Value =
            serde_json::from_str(r#"{"type":"result","subtype":"success","is_error":false}"#)
                .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(matches!(
            out.front(),
            Some(TranscriptEvent::Ended {
                reason: EndReason::Completed
            })
        ));
    }
}
