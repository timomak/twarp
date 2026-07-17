//! Minimal Codex app-server v2 driver.
//!
//! The pane still speaks only [`crate::TranscriptEvent`]. This module vendors
//! the small JSON-RPC subset needed for 18b and keeps Codex wire JSON out of
//! the UI layer.

pub mod protocol;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use async_process::{ChildStdin, ChildStdout};
use futures::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use futures::stream::Stream;
use serde_json::{json, Value};

use crate::driver::{
    resolve_in_path, AgentOutputParser, Decision, DriverCapabilities, DriverFuture,
    OutgoingMessage, PermissionMode, SpawnOptions, SpawnedSession,
};
use crate::{EndReason, McpServerInfo, TodoItem, TodoStatus, ToolOutput, TranscriptEvent, Usage};

#[derive(Clone, Debug, Default)]
pub struct CodexSessionState {
    thread_id: Arc<Mutex<Option<String>>>,
    turn_id: Arc<Mutex<Option<String>>>,
}

impl CodexSessionState {
    fn set_thread_id(&self, value: String) {
        *self.thread_id.lock().expect("codex thread state poisoned") = Some(value);
    }

    fn thread_id(&self) -> Option<String> {
        self.thread_id
            .lock()
            .expect("codex thread state poisoned")
            .clone()
    }

    fn set_turn_id(&self, value: Option<String>) {
        *self.turn_id.lock().expect("codex turn state poisoned") = value;
    }

    fn turn_id(&self) -> Option<String> {
        self.turn_id
            .lock()
            .expect("codex turn state poisoned")
            .clone()
    }
}

#[derive(Clone, Debug)]
pub struct CodexDriver {
    state: CodexSessionState,
}

impl CodexDriver {
    pub fn new(state: CodexSessionState) -> Self {
        Self { state }
    }

    pub fn capabilities() -> DriverCapabilities {
        DriverCapabilities {
            fork: false,
            steering: false,
            thinking: true,
            cost: false,
            usage_tokens: true,
        }
    }

    pub fn spawn(opts: SpawnOptions) -> Result<SpawnedSession> {
        ensure_minimum_cli_version(opts.path_env.as_deref())?;

        let program = resolve_in_path(protocol::CODEX_PROGRAM, opts.path_env.as_deref())
            .unwrap_or_else(|| PathBuf::from(protocol::CODEX_PROGRAM));
        let mut cmd = command::r#async::Command::new(program);
        cmd.arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&opts.cwd)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path_env) = &opts.path_env {
            cmd.env("PATH", path_env);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn `codex app-server`: {e}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to capture codex stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to capture codex stdout"))?;
        let stderr = child.stderr.take();

        let state = CodexSessionState::default();
        write_startup_requests(&mut stdin, &opts)?;
        if let Some(id) = opts.resume_session_id.clone().or(opts.session_id.clone()) {
            state.set_thread_id(id);
        }

        let events = codex_event_stream(stdout, stderr, state.clone(), opts.cwd.clone());
        Ok(SpawnedSession {
            child,
            stdin,
            events: Box::pin(events),
            codex_state: Some(state),
        })
    }

    pub fn send_user_message<'a>(
        &'a self,
        stdin: &'a mut ChildStdin,
        message: &'a OutgoingMessage,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            let thread_id = self
                .state
                .thread_id()
                .ok_or_else(|| anyhow!("Codex thread is not initialized yet"))?;
            let request = protocol::turn_start_request(
                protocol::next_request_id(),
                &thread_id,
                message,
                None,
                None,
            );
            write_json_line(stdin, &request)
                .await
                .context("write turn/start to codex stdin")
        })
    }

    pub fn interrupt<'a>(&'a self, stdin: &'a mut ChildStdin) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            let thread_id = self
                .state
                .thread_id()
                .ok_or_else(|| anyhow!("Codex thread is not initialized yet"))?;
            let turn_id = self
                .state
                .turn_id()
                .ok_or_else(|| anyhow!("Codex turn is not initialized yet"))?;
            let request =
                protocol::turn_interrupt_request(protocol::next_request_id(), &thread_id, &turn_id);
            write_json_line(stdin, &request)
                .await
                .context("write turn/interrupt to codex stdin")
        })
    }

    pub fn answer<'a>(
        &'a self,
        _stdin: &'a mut ChildStdin,
        _request_id: &'a str,
        _decision: Decision,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    pub fn new_parser(state: CodexSessionState, cwd: PathBuf) -> CodexParser {
        CodexParser {
            state,
            cwd,
            assistant_delta_items: HashSet::new(),
            thinking_delta_items: HashSet::new(),
            tool_output: HashMap::new(),
        }
    }
}

fn write_startup_requests(stdin: &mut ChildStdin, opts: &SpawnOptions) -> Result<()> {
    futures::executor::block_on(async {
        let initialize = protocol::initialize_request(protocol::next_request_id());
        write_json_line(stdin, &initialize).await?;

        let request_id = protocol::next_request_id();
        let request = match &opts.resume_session_id {
            Some(thread_id) => protocol::thread_resume_request(
                request_id,
                thread_id,
                &opts.cwd,
                opts.model.as_deref(),
                permission_mapping(opts.permission_mode).0,
                permission_mapping(opts.permission_mode).1,
            ),
            None => protocol::thread_start_request(
                request_id,
                &opts.cwd,
                opts.model.as_deref(),
                permission_mapping(opts.permission_mode).0,
                permission_mapping(opts.permission_mode).1,
            ),
        };
        write_json_line(stdin, &request).await?;
        stdin.flush().await?;
        Ok::<_, anyhow::Error>(())
    })
}

async fn write_json_line(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let line = serde_json::to_string(value)?;
    stdin.write_all(line.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

fn permission_mapping(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Plan => ("read-only", "on-request"),
        PermissionMode::Default => ("workspace-write", "untrusted"),
        PermissionMode::AcceptEdits => ("workspace-write", "on-request"),
        PermissionMode::BypassPermissions => ("danger-full-access", "never"),
    }
}

fn ensure_minimum_cli_version(path_env: Option<&str>) -> Result<()> {
    let program = resolve_in_path(protocol::CODEX_PROGRAM, path_env)
        .unwrap_or_else(|| PathBuf::from(protocol::CODEX_PROGRAM));
    let output = std::process::Command::new(program)
        .arg("--version")
        .output()
        .map_err(|e| anyhow!("Failed to run `codex --version`: {e}"))?;
    if !output.status.success() {
        return Err(anyhow!("`codex --version` failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_codex_version(&stdout)
        .ok_or_else(|| anyhow!("Could not parse Codex CLI version from `{}`", stdout.trim()))?;
    if compare_versions(&version, protocol::MIN_CODEX_CLI_VERSION) == std::cmp::Ordering::Less {
        return Err(anyhow!(
            "Codex CLI {version} is too old for twarp's app-server integration. Upgrade to codex-cli {} or newer.",
            protocol::MIN_CODEX_CLI_VERSION
        ));
    }
    Ok(())
}

fn parse_codex_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_owned)
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_parts = left.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    let mut right_parts = right.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    for _ in 0..3 {
        match left_parts
            .next()
            .unwrap_or(0)
            .cmp(&right_parts.next().unwrap_or(0))
        {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

fn codex_event_stream(
    stdout: ChildStdout,
    stderr: Option<async_process::ChildStderr>,
    state: CodexSessionState,
    cwd: PathBuf,
) -> impl Stream<Item = TranscriptEvent> + Send {
    let parser = CodexDriver::new_parser(state, cwd);
    futures::stream::unfold(
        (
            BufReader::new(stdout),
            stderr,
            parser,
            VecDeque::new(),
            false,
        ),
        |(mut reader, stderr, mut parser, mut buffered, mut ended)| async move {
            loop {
                if let Some(event) = buffered.pop_front() {
                    return Some((event, (reader, stderr, parser, buffered, ended)));
                }
                if ended {
                    return None;
                }
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        ended = true;
                        let event = codex_end_event(stderr).await;
                        return Some((event, (reader, None, parser, buffered, ended)));
                    }
                    Ok(_) => {}
                    Err(err) => {
                        ended = true;
                        buffered.push_back(TranscriptEvent::Ended {
                            reason: EndReason::Error(format!("Codex stdout read failed: {err}")),
                        });
                        continue;
                    }
                }
                if let Err(err) = parser.parse_line(&line, &mut buffered) {
                    log::warn!("codex: dropped app-server line: {err}");
                }
            }
        },
    )
}

async fn codex_end_event(stderr: Option<async_process::ChildStderr>) -> TranscriptEvent {
    let mut tail = String::new();
    if let Some(stderr) = stderr {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        while reader.read_line(&mut line).await.is_ok_and(|n| n > 0) {
            if tail.len() + line.len() > 4096 {
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

pub struct CodexParser {
    state: CodexSessionState,
    cwd: PathBuf,
    assistant_delta_items: HashSet<String>,
    thinking_delta_items: HashSet<String>,
    tool_output: HashMap<String, String>,
}

impl CodexParser {
    pub fn parse_line(&mut self, line: &str, out: &mut VecDeque<TranscriptEvent>) -> Result<()> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let value: Value = serde_json::from_str(line)?;
        self.parse_value(&value, out);
        Ok(())
    }

    fn parse_response(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        if let Some(error) = value.get("error") {
            out.push_back(TranscriptEvent::Ended {
                reason: EndReason::Error(provider_message(error)),
            });
            return;
        }

        let Some(result) = value.get("result") else {
            return;
        };
        if let Some(thread) = result.get("thread") {
            self.emit_session_init(thread, result, out);
        }
        if let Some(turn_id) = result
            .get("turn")
            .and_then(|turn| string_field(turn, &["id", "turnId"]))
        {
            self.state.set_turn_id(Some(turn_id));
        }
    }

    fn parse_notification(
        &mut self,
        method: &str,
        params: &Value,
        out: &mut VecDeque<TranscriptEvent>,
    ) {
        match method {
            "thread/started" => {
                if let Some(thread) = params.get("thread") {
                    self.emit_session_init(thread, params, out);
                }
            }
            "turn/started" => {
                if let Some(turn_id) = params
                    .get("turn")
                    .and_then(|turn| string_field(turn, &["id", "turnId"]))
                    .or_else(|| string_field(params, &["turnId"]))
                {
                    self.state.set_turn_id(Some(turn_id));
                }
            }
            "turn/completed" => {
                let reason = params
                    .get("turn")
                    .and_then(|turn| turn.get("status"))
                    .and_then(Value::as_str)
                    .map(end_reason_from_status)
                    .unwrap_or(EndReason::Completed);
                self.state.set_turn_id(None);
                out.push_back(TranscriptEvent::Ended { reason });
            }
            "item/started" => {
                if let Some(item) = params.get("item") {
                    self.parse_item_started(item, out);
                }
            }
            "item/completed" => {
                if let Some(item) = params.get("item") {
                    self.parse_item_completed(item, out);
                }
            }
            "item/agentMessage/delta" => {
                if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                    if let Some(item_id) = params.get("itemId").and_then(Value::as_str) {
                        self.assistant_delta_items.insert(item_id.to_owned());
                    }
                    out.push_back(TranscriptEvent::AssistantTextDelta {
                        text: delta.to_owned(),
                    });
                }
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                    if let Some(item_id) = params.get("itemId").and_then(Value::as_str) {
                        self.thinking_delta_items.insert(item_id.to_owned());
                    }
                    out.push_back(TranscriptEvent::ThinkingDelta {
                        text: delta.to_owned(),
                    });
                }
            }
            "item/commandExecution/outputDelta" | "command/exec/outputDelta" => {
                if let Some(item_id) = params.get("itemId").and_then(Value::as_str) {
                    let delta = params
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    self.tool_output
                        .entry(item_id.to_owned())
                        .or_default()
                        .push_str(delta);
                }
            }
            "turn/plan/updated" => {
                if let Some(plan) = params.get("plan").and_then(Value::as_array) {
                    let todos = plan.iter().filter_map(todo_from_plan_item).collect();
                    out.push_back(TranscriptEvent::Todos(todos));
                }
            }
            "thread/tokenUsage/updated" => {
                if let Some(usage) = parse_usage(params) {
                    out.push_back(TranscriptEvent::Usage(usage));
                }
            }
            "error" => {
                out.push_back(TranscriptEvent::Ended {
                    reason: EndReason::Error(provider_message(
                        params.get("error").unwrap_or(params),
                    )),
                });
            }
            _ => {}
        }
    }

    fn emit_session_init(
        &mut self,
        thread: &Value,
        envelope: &Value,
        out: &mut VecDeque<TranscriptEvent>,
    ) {
        let Some(session_id) = string_field(thread, &["id", "threadId", "sessionId"]) else {
            return;
        };
        self.state.set_thread_id(session_id.clone());
        let cwd = string_field(thread, &["cwd"])
            .or_else(|| string_field(envelope, &["cwd"]))
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cwd.clone());
        out.push_back(TranscriptEvent::SessionInit {
            session_id,
            cwd,
            model: string_field(envelope, &["model"]).or_else(|| string_field(thread, &["model"])),
            permission_mode: None,
            fast_mode: None,
            slash_commands: Vec::new(),
            mcp_servers: Vec::<McpServerInfo>::new(),
        });
    }

    fn parse_item_started(&mut self, item: &Value, out: &mut VecDeque<TranscriptEvent>) {
        match item_type(item) {
            Some("commandExecution") => {
                let id = item_id(item);
                let input = json!({
                    "command": item.get("command").and_then(Value::as_str).unwrap_or_default(),
                    "cwd": item.get("cwd").and_then(Value::as_str),
                });
                out.push_back(TranscriptEvent::ToolCall {
                    id,
                    name: "Bash".to_owned(),
                    input,
                    parent_id: None,
                });
            }
            Some("fileChange") => {
                out.push_back(TranscriptEvent::ToolCall {
                    id: item_id(item),
                    name: "Edit".to_owned(),
                    input: item.get("changes").cloned().unwrap_or(Value::Null),
                    parent_id: None,
                });
            }
            Some("mcpToolCall") => {
                let server = item.get("server").and_then(Value::as_str).unwrap_or("mcp");
                let tool = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
                out.push_back(TranscriptEvent::ToolCall {
                    id: item_id(item),
                    name: format!("mcp__{server}__{tool}"),
                    input: item.get("arguments").cloned().unwrap_or(Value::Null),
                    parent_id: None,
                });
            }
            Some("webSearch") => {
                out.push_back(TranscriptEvent::ToolCall {
                    id: item_id(item),
                    name: "WebSearch".to_owned(),
                    input: json!({ "query": item.get("query").and_then(Value::as_str).unwrap_or_default() }),
                    parent_id: None,
                });
            }
            _ => {}
        }
    }

    fn parse_item_completed(&mut self, item: &Value, out: &mut VecDeque<TranscriptEvent>) {
        match item_type(item) {
            Some("agentMessage") => {
                let id = item_id(item);
                if !self.assistant_delta_items.contains(&id) {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            out.push_back(TranscriptEvent::AssistantTextDelta {
                                text: text.to_owned(),
                            });
                        }
                    }
                }
                out.push_back(TranscriptEvent::AssistantTextDone);
            }
            Some("reasoning") => {
                let id = item_id(item);
                if !self.thinking_delta_items.contains(&id) {
                    let text = item
                        .get("summary")
                        .and_then(Value::as_array)
                        .or_else(|| item.get("content").and_then(Value::as_array))
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    if !text.is_empty() {
                        out.push_back(TranscriptEvent::ThinkingDelta { text });
                    }
                }
                out.push_back(TranscriptEvent::ThinkingDone { duration: None });
            }
            Some("commandExecution") => {
                let id = item_id(item);
                let output = item
                    .get("aggregatedOutput")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.tool_output.remove(&id))
                    .unwrap_or_default();
                let exit_code = item.get("exitCode").and_then(Value::as_i64);
                let is_error = exit_code.is_some_and(|code| code != 0)
                    || item.get("status").and_then(Value::as_str) == Some("failed");
                out.push_back(TranscriptEvent::ToolResult {
                    id,
                    output: ToolOutput {
                        text: output,
                        summary: exit_code.map(|code| format!("exit {code}")),
                    },
                    is_error,
                });
            }
            Some("fileChange") => {
                out.push_back(TranscriptEvent::ToolResult {
                    id: item_id(item),
                    output: ToolOutput {
                        text: serde_json::to_string_pretty(
                            item.get("changes").unwrap_or(&Value::Null),
                        )
                        .unwrap_or_default(),
                        summary: Some("file changes".to_owned()),
                    },
                    is_error: item.get("status").and_then(Value::as_str) == Some("failed"),
                });
            }
            Some("mcpToolCall") | Some("webSearch") | Some("dynamicToolCall") => {
                out.push_back(TranscriptEvent::ToolResult {
                    id: item_id(item),
                    output: ToolOutput {
                        text: serde_json::to_string_pretty(item).unwrap_or_default(),
                        summary: item
                            .get("status")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    },
                    is_error: item.get("success").and_then(Value::as_bool) == Some(false),
                });
            }
            _ => {}
        }
    }
}

impl AgentOutputParser for CodexParser {
    fn parse_value(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        if value.get("id").is_some() {
            self.parse_response(value, out);
            return;
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return;
        };
        let params = value.get("params").unwrap_or(&Value::Null);
        self.parse_notification(method, params, out);
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

fn item_type(item: &Value) -> Option<&str> {
    item.get("type").and_then(Value::as_str)
}

fn item_id(item: &Value) -> String {
    item.get("id")
        .and_then(Value::as_str)
        .unwrap_or("codex-item")
        .to_owned()
}

fn end_reason_from_status(status: &str) -> EndReason {
    match status {
        "completed" => EndReason::Completed,
        "interrupted" => EndReason::Interrupted,
        "failed" | "errored" => EndReason::Error("Codex turn failed".to_owned()),
        other => EndReason::Error(format!("Codex turn ended with status `{other}`")),
    }
}

fn provider_message(value: &Value) -> String {
    value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            serde_json::to_string(value).unwrap_or_else(|_| "Codex error".to_owned())
        })
}

fn todo_from_plan_item(value: &Value) -> Option<TodoItem> {
    let text = value.get("text").and_then(Value::as_str)?.to_owned();
    let status = match value.get("status").and_then(Value::as_str) {
        Some("completed") => TodoStatus::Completed,
        Some("in_progress" | "inProgress") => TodoStatus::InProgress,
        _ => TodoStatus::Pending,
    };
    Some(TodoItem { text, status })
}

fn parse_usage(value: &Value) -> Option<Usage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("tokenUsage"))
        .or_else(|| value.get("tokens"))?;
    Some(Usage {
        input_tokens: usage
            .get("inputTokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .get("outputTokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_read_input_tokens: usage
            .get("cachedInputTokens")
            .or_else(|| usage.get("cache_read_input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_creation_input_tokens: usage
            .get("cacheCreationInputTokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        context_window: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(lines: &[&str]) -> Vec<TranscriptEvent> {
        let state = CodexSessionState::default();
        let mut parser = CodexDriver::new_parser(state, PathBuf::from("/repo"));
        let mut out = VecDeque::new();
        for line in lines {
            let value: Value = serde_json::from_str(line).unwrap();
            parser.parse_value(&value, &mut out);
        }
        out.into_iter().collect()
    }

    #[test]
    fn replay_maps_thread_streaming_reasoning_command_usage_and_done() {
        let events = parse(&[
            r#"{"method":"thread/started","params":{"thread":{"id":"thread-1","cwd":"/repo"},"model":"gpt-5"}}"#,
            r#"{"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"running","items":[]}}}"#,
            r#"{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"r1","summaryIndex":0,"delta":"thinking"}}"#,
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"a1","delta":"hello"}}"#,
            r#"{"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"cmd-1","type":"commandExecution","command":"ls","cwd":"/repo","status":"inProgress","commandActions":[]}}}"#,
            r#"{"method":"item/commandExecution/outputDelta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"cmd-1","delta":"file\n"}}"#,
            r#"{"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"cmd-1","type":"commandExecution","command":"ls","cwd":"/repo","status":"completed","commandActions":[],"aggregatedOutput":"file\n","exitCode":0}}}"#,
            r#"{"method":"thread/tokenUsage/updated","params":{"threadId":"thread-1","turnId":"turn-1","usage":{"inputTokens":3,"outputTokens":5,"cachedInputTokens":2}}}"#,
            r#"{"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"a1","type":"agentMessage","text":"hello"}}}"#,
            r#"{"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed","items":[]}}}"#,
        ]);
        assert!(
            matches!(&events[0], TranscriptEvent::SessionInit { session_id, .. } if session_id == "thread-1")
        );
        assert!(events.iter().any(
            |event| matches!(event, TranscriptEvent::ThinkingDelta { text } if text == "thinking")
        ));
        assert!(events.iter().any(
            |event| matches!(event, TranscriptEvent::AssistantTextDelta { text } if text == "hello")
        ));
        assert!(events.iter().any(|event| matches!(event, TranscriptEvent::ToolCall { id, name, .. } if id == "cmd-1" && name == "Bash")));
        assert!(events.iter().any(|event| matches!(event, TranscriptEvent::ToolResult { id, output, is_error: false } if id == "cmd-1" && output.text == "file\n")));
        assert!(events.iter().any(|event| matches!(event, TranscriptEvent::Usage(usage) if usage.input_tokens == 3 && usage.output_tokens == 5)));
        assert!(matches!(
            events.last(),
            Some(TranscriptEvent::Ended {
                reason: EndReason::Completed
            })
        ));
    }

    #[test]
    fn replay_maps_file_changes_plan_unknown_and_error() {
        let events = parse(&[
            r#"{"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"edit-1","type":"fileChange","status":"inProgress","changes":[{"path":"a.rs","kind":{"type":"update"},"diff":"@@ x"}]}}}"#,
            r#"{"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"edit-1","type":"fileChange","status":"completed","changes":[{"path":"a.rs","kind":{"type":"update"},"diff":"@@ x"}]}}}"#,
            r#"{"method":"turn/plan/updated","params":{"threadId":"thread-1","turnId":"turn-1","plan":[{"text":"one","status":"completed"},{"text":"two","status":"inProgress"}]}}"#,
            r#"{"method":"error","params":{"threadId":"thread-1","turnId":"turn-1","error":{"message":"boom"}}}"#,
        ]);
        assert!(events.iter().any(|event| matches!(event, TranscriptEvent::ToolCall { id, name, .. } if id == "edit-1" && name == "Edit")));
        assert!(events.iter().any(|event| matches!(event, TranscriptEvent::Todos(items) if items.len() == 2 && items[0].status == TodoStatus::Completed)));
        assert!(
            matches!(events.last(), Some(TranscriptEvent::Ended { reason: EndReason::Error(message) }) if message == "boom")
        );
    }

    #[test]
    fn interrupt_request_uses_tracked_turn_id() {
        let request = protocol::turn_interrupt_request("req-1".to_owned(), "thread-1", "turn-1");
        assert_eq!(request["method"], "turn/interrupt");
        assert_eq!(request["params"]["threadId"], "thread-1");
        assert_eq!(request["params"]["turnId"], "turn-1");
    }

    #[test]
    fn version_check_compares_semver_prefix() {
        assert_eq!(
            parse_codex_version("codex-cli 0.135.0").as_deref(),
            Some("0.135.0")
        );
        assert_eq!(
            compare_versions("0.135.0", protocol::MIN_CODEX_CLI_VERSION),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_versions("0.134.9", protocol::MIN_CODEX_CLI_VERSION),
            std::cmp::Ordering::Less
        );
    }
}
