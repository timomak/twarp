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
use async_process::{ChildStdin, ChildStdout};
use futures::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use futures::stream::Stream;
use serde_json::{json, Value};

use crate::{EndReason, ToolOutput, TranscriptEvent};

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
    pub resume_session_id: Option<String>,
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
    if let Some(id) = &opts.resume_session_id {
        cmd.arg("--resume").arg(id);
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

    let events = event_stream_from_stdout(stdout);
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
    buffered: VecDeque<TranscriptEvent>,
}

fn event_stream_from_stdout(stdout: ChildStdout) -> impl Stream<Item = TranscriptEvent> + Send {
    let state = StreamState {
        reader: Some(BufReader::new(stdout)),
        buffered: VecDeque::new(),
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            // Drain the buffer first — a single JSONL line can map to several
            // TranscriptEvents (e.g. an assistant turn with text + tool_use).
            if let Some(evt) = state.buffered.pop_front() {
                return Some((evt, state));
            }
            let reader = state.reader.as_mut()?;
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // EOF — `claude` exited. Surface it as Ended(Exited) once,
                    // then stop the stream so spawn_stream_local fires its
                    // on_done callback.
                    state.reader = None;
                    return Some((
                        TranscriptEvent::Ended {
                            reason: EndReason::Exited,
                        },
                        state,
                    ));
                }
                Ok(_) => {}
                Err(err) => {
                    log::warn!("claude stdout read failed: {err}");
                    state.reader = None;
                    return Some((
                        TranscriptEvent::Ended {
                            reason: EndReason::Exited,
                        },
                        state,
                    ));
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
    out.push_back(TranscriptEvent::SessionInit { session_id, cwd });
}

fn parse_assistant(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    for block in content {
        let Some(ty) = block.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        match ty {
            "text" => {
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
            "thinking" => {
                if let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) {
                    out.push_back(TranscriptEvent::Thinking {
                        text: thinking.to_owned(),
                        duration: None,
                    });
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
                    out.push_back(TranscriptEvent::ToolCall { id, name, input });
                }
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
            Some(TranscriptEvent::SessionInit { session_id, cwd }) => {
                assert_eq!(session_id, "abc");
                assert_eq!(cwd, &PathBuf::from("/tmp/p"));
            }
            other => panic!("expected SessionInit, got {other:?}"),
        }
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
