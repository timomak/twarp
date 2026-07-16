//! Subprocess driver for the local `claude` CLI (PRODUCT §8–§22, §52–§57).
//!
//! Spawns `claude -p --input-format stream-json --output-format stream-json
//! --verbose`, parses its JSONL output defensively line-by-line, and emits
//! [`TranscriptEvent`]s — the UI never sees raw `claude` JSON. Used by the
//! Claude Code panel; headless and unit-testable here.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;

use instant::Instant;

use anyhow::{anyhow, Context, Result};
pub use async_process::Child;
use async_process::{ChildStderr, ChildStdin, ChildStdout};
use futures::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use futures::stream::Stream;
use serde_json::{json, Value};

use crate::{
    sessions, EndReason, McpServerInfo, TaskNotification, TaskRunStatus, TodoItem, TodoStatus,
    ToolOutput, TranscriptEvent, TurnMetrics, Usage,
};

pub type DriverFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentProvider {
    Claude,
    Codex,
}

impl AgentProvider {
    pub fn as_persistence_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn from_persistence_str(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    pub fn from_persisted_or_default(value: Option<&str>) -> Self {
        value
            .and_then(Self::from_persistence_str)
            .unwrap_or(Self::Claude)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverCapabilities {
    pub fork: bool,
    pub steering: bool,
    pub thinking: bool,
    pub cost: bool,
    pub usage_tokens: bool,
}

impl DriverCapabilities {
    pub const CLAUDE: Self = Self {
        fork: true,
        steering: true,
        thinking: true,
        cost: true,
        usage_tokens: true,
    };
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    AllowOnce { updated_input: Value },
    AllowAlways { updated_input: Value },
    Deny { message: String },
    Answer(Value),
}

impl Decision {
    pub fn allow_once(updated_input: Value) -> Self {
        Self::AllowOnce { updated_input }
    }

    pub fn allow_always(updated_input: Value) -> Self {
        Self::AllowAlways { updated_input }
    }

    pub fn deny() -> Self {
        Self::Deny {
            message: "The user declined this action.".to_owned(),
        }
    }

    pub fn cancelled() -> Self {
        Self::Answer(json!({ "behavior": "cancelled" }))
    }

    fn into_claude_response(self) -> Value {
        match self {
            Self::AllowOnce { updated_input } | Self::AllowAlways { updated_input } => {
                json!({ "behavior": "allow", "updatedInput": updated_input })
            }
            Self::Deny { message } => json!({ "behavior": "deny", "message": message }),
            Self::Answer(response) => response,
        }
    }
}

pub trait AgentOutputParser: Send {
    fn parse_value(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>);
}

pub trait AgentDriver: Send + Sync {
    fn provider(&self) -> AgentProvider;
    fn capabilities(&self) -> DriverCapabilities;
    fn spawn(&self, opts: SpawnOptions) -> Result<SpawnedSession>;
    fn send_user_message<'a>(
        &self,
        stdin: &'a mut ChildStdin,
        message: &'a OutgoingMessage,
    ) -> DriverFuture<'a, ()>;
    fn interrupt<'a>(&self, stdin: &'a mut ChildStdin, request_id: &'a str)
        -> DriverFuture<'a, ()>;
    fn answer<'a>(
        &self,
        stdin: &'a mut ChildStdin,
        request_id: &'a str,
        decision: Decision,
    ) -> DriverFuture<'a, ()>;
    fn new_parser(&self) -> Box<dyn AgentOutputParser>;
    fn parse_line(
        &self,
        parser: &mut dyn AgentOutputParser,
        line: &str,
        out: &mut VecDeque<TranscriptEvent>,
    ) -> Result<()>;
    fn has_sessions(&self, cwd: &Path) -> bool;
    fn list_sessions(&self, cwd: &Path) -> Vec<sessions::StoredSession>;
    fn load_history(&self, path: &Path) -> Vec<TranscriptEvent>;
    fn fork_session_file(
        &self,
        parent_path: &Path,
        new_session_id: &str,
        keep_user_turns: usize,
        cwd: &Path,
    ) -> std::io::Result<PathBuf>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ClaudeDriver;

pub const CLAUDE_DRIVER: ClaudeDriver = ClaudeDriver;

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

    /// Inverse of [`Self::as_cli_arg`]. Unknown values (the CLI's `auto`/
    /// `dontAsk`, or anything future) return `None` rather than a guess.
    pub fn from_cli_arg(value: &str) -> Option<Self> {
        match value {
            "default" => Some(Self::Default),
            "acceptEdits" => Some(Self::AcceptEdits),
            "plan" => Some(Self::Plan),
            "bypassPermissions" => Some(Self::BypassPermissions),
            _ => None,
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
    /// Inline JSON or a config path passed to `claude --mcp-config`.
    pub mcp_config: Option<String>,
    /// `PATH` to run `claude` under. macOS GUI apps launched via Finder/`open`
    /// inherit launchd's minimal `PATH`, which omits the user's shell dirs
    /// (Homebrew, `~/.local/bin`, version managers) where `claude` usually
    /// lives. The pane captures the login-shell `PATH` and passes it here so
    /// both resolution of the `claude` binary and the child's environment match
    /// what the user gets in a terminal. `None` → inherit the process `PATH`.
    pub path_env: Option<String>,
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

/// Resolve `program` to an absolute path by searching `path_env` (a
/// `PATH`-style string). Returns `None` when `path_env` is `None` or no
/// matching executable file is found, in which case the caller falls back to
/// the bare program name (process-`PATH` lookup).
fn resolve_in_path(program: &str, path_env: Option<&str>) -> Option<PathBuf> {
    let path_env = path_env?;
    std::env::split_paths(path_env)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Spawn `claude` with stream-json IO. PRODUCT §8: the session is one
/// long-lived process driven multi-turn via stdin.
pub fn spawn_session(opts: SpawnOptions) -> Result<SpawnedSession> {
    CLAUDE_DRIVER.spawn(opts)
}

fn spawn_claude_session(opts: SpawnOptions) -> Result<SpawnedSession> {
    // Resolve the `claude` binary against the supplied login-shell PATH. On
    // Unix, program lookup ignores a PATH set via `Command::env` (it searches
    // the parent process's PATH), so we resolve to an absolute path ourselves
    // — otherwise a GUI launch (launchd-minimal PATH) wouldn't find `claude`
    // even though we also set it in the child env below.
    let program = resolve_in_path("claude", opts.path_env.as_deref())
        .unwrap_or_else(|| PathBuf::from("claude"));
    let mut cmd = command::r#async::Command::new(program);
    cmd.arg("-p")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        // 7k: opt into token-level streaming so assistant text, thinking, and
        // tool args arrive as `stream_event` deltas instead of only the
        // consolidated end-of-block message (PRODUCT §45).
        .arg("--include-partial-messages")
        .arg("--permission-mode")
        .arg(opts.permission_mode.as_cli_arg())
        // 7g: route tool-permission decisions over the stream-json control
        // channel (PRODUCT §24). The `stdio` sentinel makes `claude` emit a
        // `can_use_tool` control_request on stdout and wait for our
        // control_response on stdin, instead of auto-denying in headless mode.
        // Verified against `claude` 2.1.195 (the flag is hidden from `--help`
        // but still honoured) — the interactive path the earlier §26 feasibility
        // pass thought was gone. Harmless in the non-prompting modes
        // (`acceptEdits`/`bypassPermissions` simply never raise the request).
        .arg("--permission-prompt-tool")
        .arg("stdio");

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
    if let Some(mcp_config) = &opts.mcp_config {
        cmd.arg("--mcp-config").arg(mcp_config);
    }

    // Run under the login-shell PATH when provided (PRODUCT §4): under `open`
    // the process PATH is launchd-minimal, so without this `claude` — and any
    // tools it shells out to — wouldn't resolve.
    if let Some(path_env) = &opts.path_env {
        cmd.env("PATH", path_env);
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

    let events = event_stream(stdout, stderr, CLAUDE_DRIVER.new_parser());
    Ok(SpawnedSession {
        child,
        stdin,
        events: Box::pin(events),
    })
}

/// Interrupt the in-flight turn over the stream-json control channel — the
/// supported, session-preserving Stop (PRODUCT §11). `claude` acknowledges with
/// a `control_response`, ends the turn with a `result` (subtype
/// `error_during_execution`, `is_error: true`), and **stays alive** for the next
/// turn. This is why Stop must use this and not [`interrupt`]: a SIGINT kills the
/// process, which surfaces as a spurious "session ended unexpectedly" error and
/// wedges the pane until it's reopened. `request_id` is echoed back on the
/// acknowledgement; it only needs to be unique per live session.
pub async fn send_interrupt(stdin: &mut ChildStdin, request_id: &str) -> Result<()> {
    CLAUDE_DRIVER.interrupt(stdin, request_id).await
}

async fn send_claude_interrupt(stdin: &mut ChildStdin, request_id: &str) -> Result<()> {
    let line = json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "interrupt" },
    })
    .to_string();
    stdin
        .write_all(line.as_bytes())
        .await
        .context("write interrupt request to claude stdin")?;
    stdin
        .write_all(b"\n")
        .await
        .context("write newline to claude stdin")?;
    stdin.flush().await.context("flush claude stdin")?;
    Ok(())
}

/// Send a SIGINT to the live `claude` process to interrupt the current turn.
/// Last-resort fallback for platforms / states without a live stdin pump —
/// prefer [`send_interrupt`], which keeps the session alive. Best-effort: Unix
/// only — on other platforms Stop falls back to ending the session via drop.
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

/// An image attached to an outgoing user turn (PRODUCT §15b): already
/// base64-encoded, with its IANA media type (`image/png`, …). Sent as a
/// standard API `image` content block — verified accepted by `claude`'s
/// stream-json input (2.1.175).
#[derive(Clone, Debug)]
pub struct OutgoingImage {
    pub media_type: String,
    pub base64_data: String,
}

/// A user turn heading into the live session: the text plus any attached
/// images.
#[derive(Clone, Debug, Default)]
pub struct OutgoingMessage {
    pub text: String,
    pub images: Vec<OutgoingImage>,
}

impl OutgoingMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }
}

/// Write a user turn into the live session's stdin in the JSONL shape
/// `claude --input-format stream-json` expects (PRODUCT §16). A text-only
/// message sends the plain-string content form; attachments use the
/// content-block array with `image` blocks (PRODUCT §15b).
pub async fn send_user_message(stdin: &mut ChildStdin, message: &OutgoingMessage) -> Result<()> {
    CLAUDE_DRIVER.send_user_message(stdin, message).await
}

async fn send_claude_user_message(stdin: &mut ChildStdin, message: &OutgoingMessage) -> Result<()> {
    let content = if message.images.is_empty() {
        json!(message.text)
    } else {
        let mut blocks = vec![json!({ "type": "text", "text": message.text })];
        blocks.extend(message.images.iter().map(|image| {
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.media_type,
                    "data": image.base64_data,
                },
            })
        }));
        json!(blocks)
    };
    let line = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": content,
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

/// Answer a `control_request` `claude` raised over the stream-json control
/// channel (7g, PRODUCT §24) — a tool-permission `can_use_tool` or an
/// `AskUserQuestion` `request_user_dialog`. `response` is the request-specific
/// decision payload (e.g. `{"behavior":"allow","updatedInput":..}` for a
/// permission, `{"behavior":"cancelled"}` to release a dialog); we wrap it in
/// the `control_response` envelope `claude` matches back to its pending request
/// by `request_id`. Best-effort: a late answer (past `claude`'s park deadline)
/// is ignored on its side, never an error on ours (PRODUCT §26: never hang).
pub async fn send_control_response(
    stdin: &mut ChildStdin,
    request_id: &str,
    decision: Decision,
) -> Result<()> {
    CLAUDE_DRIVER.answer(stdin, request_id, decision).await
}

async fn send_claude_control_response(
    stdin: &mut ChildStdin,
    request_id: &str,
    decision: Decision,
) -> Result<()> {
    let response = decision.into_claude_response();
    let line = json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
    })
    .to_string();
    stdin
        .write_all(line.as_bytes())
        .await
        .context("write control response to claude stdin")?;
    stdin
        .write_all(b"\n")
        .await
        .context("write newline to claude stdin")?;
    stdin.flush().await.context("flush claude stdin")?;
    Ok(())
}

impl AgentDriver for ClaudeDriver {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Claude
    }

    fn capabilities(&self) -> DriverCapabilities {
        DriverCapabilities::CLAUDE
    }

    fn spawn(&self, opts: SpawnOptions) -> Result<SpawnedSession> {
        spawn_claude_session(opts)
    }

    fn send_user_message<'a>(
        &self,
        stdin: &'a mut ChildStdin,
        message: &'a OutgoingMessage,
    ) -> DriverFuture<'a, ()> {
        Box::pin(send_claude_user_message(stdin, message))
    }

    fn interrupt<'a>(
        &self,
        stdin: &'a mut ChildStdin,
        request_id: &'a str,
    ) -> DriverFuture<'a, ()> {
        Box::pin(send_claude_interrupt(stdin, request_id))
    }

    fn answer<'a>(
        &self,
        stdin: &'a mut ChildStdin,
        request_id: &'a str,
        decision: Decision,
    ) -> DriverFuture<'a, ()> {
        Box::pin(send_claude_control_response(stdin, request_id, decision))
    }

    fn new_parser(&self) -> Box<dyn AgentOutputParser> {
        Box::<Parser>::default()
    }

    fn parse_line(
        &self,
        parser: &mut dyn AgentOutputParser,
        line: &str,
        out: &mut VecDeque<TranscriptEvent>,
    ) -> Result<()> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let value: Value = serde_json::from_str(line)?;
        parser.parse_value(&value, out);
        Ok(())
    }

    fn has_sessions(&self, cwd: &Path) -> bool {
        sessions::has_sessions(cwd)
    }

    fn list_sessions(&self, cwd: &Path) -> Vec<sessions::StoredSession> {
        sessions::list_sessions(cwd)
    }

    fn load_history(&self, path: &Path) -> Vec<TranscriptEvent> {
        sessions::load_history(path)
    }

    fn fork_session_file(
        &self,
        parent_path: &Path,
        new_session_id: &str,
        keep_user_turns: usize,
        cwd: &Path,
    ) -> std::io::Result<PathBuf> {
        sessions::fork_session_file(parent_path, new_session_id, keep_user_turns, cwd)
    }
}

struct StreamState {
    reader: Option<BufReader<ChildStdout>>,
    stderr: Option<ChildStderr>,
    buffered: VecDeque<TranscriptEvent>,
    /// Holds the cross-line streaming state (open content blocks, the
    /// done-marker flag) the partial-message path needs (7k).
    parser: Box<dyn AgentOutputParser>,
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
    parser: Box<dyn AgentOutputParser>,
) -> impl Stream<Item = TranscriptEvent> + Send {
    let state = StreamState {
        reader: Some(BufReader::new(stdout)),
        stderr,
        buffered: VecDeque::new(),
        parser,
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
            state.parser.parse_value(&value, &mut state.buffered);
        }
    })
}

/// An assistant content block that is mid-stream under
/// `--include-partial-messages`, keyed by its content-block `index`. Tracks just
/// enough to route each delta and finalize the block (7k, PRODUCT §45).
enum OpenBlock {
    /// A streaming `text` block. `suppressed` when it belongs to a `Task`
    /// sub-agent (sub-agent prose is internal monologue, PRODUCT §19).
    Text { suppressed: bool },
    /// A streaming `thinking` block. `start` stamps the wall-clock at
    /// `content_block_start` so the duration is the measured streamed span
    /// (PRODUCT §47); `wrote` guards against finalizing a signature-only block.
    Thinking {
        start: Instant,
        wrote: bool,
        suppressed: bool,
    },
    /// A streaming `tool_use` block: `input` arrives as `input_json_delta`
    /// fragments accumulated here and parsed once at `content_block_stop`.
    Tool {
        id: String,
        name: String,
        json: String,
        parent_id: Option<String>,
    },
}

/// Stateful stream-json parser. Most event types are stateless, but the
/// partial-message path (7k) needs cross-line state: the open content blocks and
/// a flag marking that the current message streamed (so its consolidated
/// `assistant` event is treated as a pure done-marker, PRODUCT §46).
#[derive(Default)]
pub(crate) struct Parser {
    /// Open content blocks of the message currently streaming, by index.
    blocks: HashMap<u64, OpenBlock>,
    /// `true` once the current message emitted `message_start` — its content has
    /// streamed incrementally, so the consolidated `assistant` is a done-marker.
    streamed: bool,
}

impl Parser {
    /// Translate one parsed stream-json value into zero or more
    /// [`TranscriptEvent`]s. Defensive: unknown event types and missing optional
    /// fields are tolerated (PRODUCT §53).
    pub(crate) fn parse(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        let Some(ty) = value.get("type").and_then(|v| v.as_str()) else {
            log::warn!("claude: event without `type` field, dropped");
            return;
        };
        match ty {
            "system" => parse_system(value, out),
            "assistant" => self.parse_assistant(value, out),
            "user" => parse_user_event(value, out),
            "result" => parse_result(value, out),
            "stream_event" => self.parse_stream_event(value, out),
            "control_request" => parse_control_request(value, out),
            other => log::debug!("claude: ignoring unknown event type `{other}`"),
        }
    }

    /// Consume a `stream_event` line (PRODUCT §45): the partial-message deltas
    /// `--include-partial-messages` emits. Routes each delta to its open block
    /// by content-block index and emits incremental [`TranscriptEvent`]s.
    fn parse_stream_event(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        // Each line carries its own `parent_tool_use_id`; a sub-agent's prose is
        // suppressed exactly as in the consolidated path (PRODUCT §19).
        let parent_id = value
            .get("parent_tool_use_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let Some(event) = value.get("event") else {
            return;
        };
        let Some(ety) = event.get("type").and_then(|v| v.as_str()) else {
            return;
        };
        let index = || event.get("index").and_then(|v| v.as_u64());
        match ety {
            "message_start" => {
                // A new assistant message: mark it streamed and drop any stale
                // open blocks (defensive — they should already be closed).
                self.streamed = true;
                self.blocks.clear();
            }
            "content_block_start" => {
                let Some(idx) = index() else { return };
                // Mark the message streamed here too, not only at `message_start`:
                // the consolidated `assistant` event suppresses its (already
                // streamed) text only when `streamed` is set, and an opened
                // content block is the real proof that content streamed. If a
                // `message_start` is ever missing/dropped, the deltas still
                // render (the block was inserted) and the consolidated event
                // would otherwise re-append the same text — a duplicate of the
                // last message (PRODUCT §46).
                self.streamed = true;
                let cb = event.get("content_block");
                let suppressed = parent_id.is_some();
                match cb.and_then(|c| c.get("type")).and_then(|v| v.as_str()) {
                    Some("text") => {
                        self.blocks.insert(idx, OpenBlock::Text { suppressed });
                    }
                    Some("thinking") => {
                        self.blocks.insert(
                            idx,
                            OpenBlock::Thinking {
                                start: Instant::now(),
                                wrote: false,
                                suppressed,
                            },
                        );
                    }
                    Some("tool_use") => {
                        let cb = cb.expect("matched above");
                        let id = cb
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_owned();
                        let name = cb
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_owned();
                        self.blocks.insert(
                            idx,
                            OpenBlock::Tool {
                                id,
                                name,
                                json: String::new(),
                                parent_id: parent_id.clone(),
                            },
                        );
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let Some(idx) = index() else { return };
                let Some(delta) = event.get("delta") else {
                    return;
                };
                let dty = delta.get("type").and_then(|v| v.as_str());
                match self.blocks.get_mut(&idx) {
                    Some(OpenBlock::Text { suppressed }) if dty == Some("text_delta") => {
                        if !*suppressed {
                            if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                if !text.is_empty() {
                                    out.push_back(TranscriptEvent::AssistantTextDelta {
                                        text: text.to_owned(),
                                    });
                                }
                            }
                        }
                    }
                    Some(OpenBlock::Thinking {
                        wrote, suppressed, ..
                    }) if dty == Some("thinking_delta") => {
                        if !*suppressed {
                            if let Some(text) = delta.get("thinking").and_then(|v| v.as_str()) {
                                if !text.is_empty() {
                                    *wrote = true;
                                    out.push_back(TranscriptEvent::ThinkingDelta {
                                        text: text.to_owned(),
                                    });
                                }
                            }
                        }
                    }
                    Some(OpenBlock::Tool { json, .. }) if dty == Some("input_json_delta") => {
                        if let Some(fragment) = delta.get("partial_json").and_then(|v| v.as_str()) {
                            json.push_str(fragment);
                        }
                    }
                    // Unknown delta type or no matching open block — skip (§53).
                    _ => {}
                }
            }
            "content_block_stop" => {
                let Some(idx) = index() else { return };
                self.finish_block(idx, out);
            }
            // `message_delta` / `message_stop` / `ping` carry no transcript
            // content; the consolidated `assistant` event is the done-marker.
            _ => {}
        }
    }

    /// Finalize one open content block at `content_block_stop`.
    fn finish_block(&mut self, idx: u64, out: &mut VecDeque<TranscriptEvent>) {
        match self.blocks.remove(&idx) {
            Some(OpenBlock::Text { suppressed }) => {
                if !suppressed {
                    out.push_back(TranscriptEvent::AssistantTextDone);
                }
            }
            Some(OpenBlock::Thinking {
                start,
                wrote,
                suppressed,
            }) => {
                // A signature-only block (no thinking_delta) opened no item, so
                // there is nothing to finalize (PRODUCT §22).
                if !suppressed && wrote {
                    out.push_back(TranscriptEvent::ThinkingDone {
                        duration: Some(start.elapsed()),
                    });
                }
            }
            Some(OpenBlock::Tool {
                id,
                name,
                json,
                parent_id,
            }) => {
                let input = serde_json::from_str(&json).unwrap_or(Value::Null);
                emit_tool_call(id, name, input, parent_id, out);
            }
            None => {}
        }
    }

    fn parse_assistant(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        // A streamed message's consolidated `assistant` event is a pure
        // done-marker (PRODUCT §46): its text/thinking/tools already arrived as
        // deltas. Close any block a missing `content_block_stop` left open, then
        // reset for the next message and emit nothing else.
        if self.streamed {
            let mut open: Vec<u64> = self.blocks.keys().copied().collect();
            open.sort_unstable();
            for idx in open {
                self.finish_block(idx, out);
            }
            self.streamed = false;
            // The text/thinking/tools arrived as deltas, but the consolidated
            // event still carries this message's token usage — surface it (the
            // free `parse_assistant` below is skipped on this path).
            emit_message_usage(value, out);
            return;
        }
        parse_assistant(value, out);
    }
}

impl AgentOutputParser for Parser {
    fn parse_value(&mut self, value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        self.parse(value, out);
    }
}

/// Emit a tool call, routing a top-level `TodoWrite` to the in-place task list
/// instead of a card (PRODUCT §17, §23). Shared by the consolidated path and the
/// streaming `content_block_stop` finalizer so both honor the same routing. A
/// sub-agent's `TodoWrite` (`parent_id` set) stays a nested card; malformed
/// input falls back to a card so something always renders (PRODUCT §29).
fn emit_tool_call(
    id: String,
    name: String,
    input: Value,
    parent_id: Option<String>,
    out: &mut VecDeque<TranscriptEvent>,
) {
    if id.is_empty() || name.is_empty() {
        return;
    }
    if name == "TodoWrite" && parent_id.is_none() {
        if let Some(todos) = parse_todos(&input) {
            out.push_back(TranscriptEvent::Todos(todos));
            return;
        }
    }
    out.push_back(TranscriptEvent::ToolCall {
        id,
        name,
        input,
        parent_id,
    });
}

/// Parse a `control_request` `claude` raised over the stream-json control
/// channel (7g). Two subtypes surface as transcript events the pane can answer
/// (`driver::send_control_response`):
///
/// - `can_use_tool` — a tool-permission prompt (PRODUCT §24). Carries the
///   `tool_name` and the proposed `input`; the pane renders Allow/Deny and
///   echoes `input` back as `updatedInput` on allow.
/// - `request_user_dialog` — an `AskUserQuestion` (and kin). The `payload` is
///   opaque per `dialog_kind`; we hand it through verbatim for the pane to
///   render (the `question` kind mirrors the tool's `{questions:[..]}` input).
///
/// Both carry the `request_id` the control_response must echo. Unknown subtypes
/// are ignored — `claude` settles them with its own park deadline (PRODUCT §26).
fn parse_control_request(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    let Some(request_id) = value.get("request_id").and_then(|v| v.as_str()) else {
        log::warn!("claude: control_request without request_id, dropped");
        return;
    };
    let Some(request) = value.get("request") else {
        return;
    };
    match request.get("subtype").and_then(|v| v.as_str()) {
        Some("can_use_tool") => {
            let tool = request
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let input = request.get("input").cloned().unwrap_or(Value::Null);
            // The `tool_use_id` ties this permission to the assistant `tool_use`
            // block it gates. The pane needs it to attach a held `AskUserQuestion`
            // permission to its inline question card (PRODUCT §1), so answers ride
            // back on this same `can_use_tool` rather than being auto-dismissed.
            let tool_use_id = request
                .get("tool_use_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            out.push_back(TranscriptEvent::PermissionRequest {
                id: request_id.to_owned(),
                tool,
                input,
                tool_use_id,
            });
        }
        Some("request_user_dialog") => {
            let dialog_kind = request
                .get("dialog_kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let payload = request.get("payload").cloned().unwrap_or(Value::Null);
            out.push_back(TranscriptEvent::QuestionRequest {
                id: request_id.to_owned(),
                dialog_kind,
                payload,
            });
        }
        other => {
            log::debug!("claude: ignoring control_request subtype {other:?}");
        }
    }
}

fn parse_system(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    match value.get("subtype").and_then(|v| v.as_str()) {
        Some("init") => {}
        // A background task reached a terminal state. This is the *only*
        // completion signal a `run_in_background` Bash gets from current
        // `claude` builds (they never poll via `BashOutput`) — dropping it
        // left the background-scripts panel showing "running" forever.
        Some("task_notification") => {
            if let Some(notification) = parse_task_notification_event(value) {
                out.push_back(TranscriptEvent::TaskNotification(notification));
            }
            return;
        }
        _ => return,
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
    // The session's available slash commands (built-ins + skills + plugins),
    // straight from `claude` — drives the composer's `/` suggestions
    // (PRODUCT §15a). Order is claude's; the UI fuzzy-filters.
    let slash_commands = value
        .get("slash_commands")
        .and_then(|v| v.as_array())
        .map(|commands| {
            commands
                .iter()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    // MCP servers the session can reach (feature 13). The init event lists
    // `{ name, status }` per server; tools are not enumerated here (the panel
    // derives them from observed calls). Parse defensively — the field may be
    // absent (older CLI), empty, or carry a server without a status.
    let mcp_servers = value
        .get("mcp_servers")
        .and_then(|v| v.as_array())
        .map(|servers| {
            servers
                .iter()
                .filter_map(|s| {
                    let name = s.get("name")?.as_str()?.to_owned();
                    let status = s.get("status").and_then(|v| v.as_str()).map(str::to_owned);
                    Some(McpServerInfo {
                        name,
                        status,
                        tools: Vec::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.push_back(TranscriptEvent::SessionInit {
        session_id,
        cwd,
        model: str_field("model"),
        permission_mode: str_field("permissionMode"),
        fast_mode: str_field("fast_mode_state"),
        slash_commands,
        mcp_servers,
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
    emit_message_usage(value, out);
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
                // Shared routing: a top-level `TodoWrite` becomes the in-place
                // task list, everything else a tool card (PRODUCT §17, §23, §29).
                emit_tool_call(id, name, input, parent_id.clone(), out);
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

/// The wire → [`TaskRunStatus`] mapping shared by the live system event and
/// the stored `<task-notification>` line. An unknown status yields `None` —
/// the notification is dropped rather than guessed (PRODUCT §53), leaving the
/// script at "running".
fn parse_task_status(status: &str) -> Option<TaskRunStatus> {
    match status {
        "completed" => Some(TaskRunStatus::Completed),
        "failed" => Some(TaskRunStatus::Failed),
        // The notification says `stopped`; the sibling `task_updated` patch
        // says `killed` for the same transition — accept both.
        "stopped" | "killed" => Some(TaskRunStatus::Stopped),
        _ => None,
    }
}

/// Parse a live `system`/`task_notification` event. `None` when the required
/// `task_id`/`status` fields are missing or the status is unfamiliar.
fn parse_task_notification_event(value: &Value) -> Option<TaskNotification> {
    let str_field = |key: &str| value.get(key).and_then(|v| v.as_str());
    Some(TaskNotification {
        task_id: str_field("task_id")?.to_owned(),
        tool_use_id: str_field("tool_use_id").map(str::to_owned),
        status: parse_task_status(str_field("status")?)?,
        summary: str_field("summary").map(str::to_owned),
    })
}

/// Parse a stored-history `<task-notification>` user line — the on-disk twin
/// of the live `task_notification` system event (the harness enqueues the
/// notification as a user turn, so that's how the session file records it).
/// `None` when the text isn't such an envelope or lacks the required tags.
fn parse_task_notification_text(text: &str) -> Option<TaskNotification> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("<task-notification>") {
        return None;
    }
    Some(TaskNotification {
        task_id: tag_content(trimmed, "task-id")?.trim().to_owned(),
        tool_use_id: tag_content(trimmed, "tool-use-id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        status: parse_task_status(tag_content(trimmed, "status")?.trim())?,
        summary: tag_content(trimmed, "summary")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    })
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
/// Map a stored user turn back to what the user typed. Slash-command turns
/// are persisted as envelope tags (`<command-message>`, `<command-name>`,
/// `<command-args>`) rather than the literal command line, and local commands
/// additionally echo a `<local-command-stdout>` line. Returns the
/// reconstructed command line (`/fleet run it`), `None` for stdout echoes
/// (they aren't something the user said), or the text unchanged when it isn't
/// a command envelope.
pub(crate) fn display_user_text(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with("<task-notification>") {
        // Harness bookkeeping (a background task's completion), not something
        // the user said — even when the envelope is too malformed to parse
        // into a TaskNotification, never render it as a user bubble.
        return None;
    }
    if trimmed.starts_with("<command-") {
        if let Some(name) = tag_content(trimmed, "command-name") {
            let name = name.trim();
            let args = tag_content(trimmed, "command-args").unwrap_or("").trim();
            let line = if args.is_empty() {
                name.to_owned()
            } else {
                format!("{name} {args}")
            };
            return (!line.is_empty()).then_some(line);
        }
    }
    if trimmed.starts_with("<local-command-stdout>") {
        return None;
    }
    Some(text.to_owned())
}

/// The body of the first `<tag>…</tag>` in `text`, `None` when absent.
fn tag_content<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

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
                // A background task's completion is enqueued as a user turn,
                // so the session file stores it as a `<task-notification>`
                // envelope — replay it as the notification it is (the live
                // stream's `system`/`task_notification`), never as user prose
                // (`display_user_text` drops the envelope regardless).
                if let Some(notification) = parse_task_notification_text(&text) {
                    out.push_back(TranscriptEvent::TaskNotification(notification));
                } else if text.starts_with("[Request interrupted by user") {
                    // Stop stores this marker as a `user` line, but it isn't a
                    // user turn: live panes render the interruption as the
                    // `Ended` notice, and `fork_session_file` counts user turns
                    // through this parser — both need it classified the same.
                    out.push_back(TranscriptEvent::Ended {
                        reason: EndReason::Interrupted,
                    });
                } else if let Some(display) = display_user_text(&text) {
                    out.push_back(TranscriptEvent::UserMessage(display));
                }
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
    // Only the context window lands here — the `result`'s own `usage` block is
    // the turn aggregate (it sums every API call in the loop, so its cache reads
    // can reach millions) and would massively overstate the chip's "context
    // used". The live token counts come from per-message `assistant` usage; this
    // just stamps the window onto them before `Ended` closes the turn.
    if let Some(window) = parse_context_window(value) {
        out.push_back(TranscriptEvent::ContextWindow(window));
    }
    // Per-turn cost/timing line (PRODUCT §48), also before `Ended` so it renders
    // as the turn's last item. Omitted entirely when the result carries none.
    let metrics = parse_turn_metrics(value);
    if !metrics.is_empty() {
        out.push_back(TranscriptEvent::Metrics(metrics));
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

/// Extract the per-turn cost/timing line from a `result` message (PRODUCT §48).
/// Every field is independently optional — an absent one stays `None` and is
/// omitted from the rendered line, never shown as `0`.
fn parse_turn_metrics(value: &Value) -> TurnMetrics {
    TurnMetrics {
        total_cost_usd: value.get("total_cost_usd").and_then(|v| v.as_f64()),
        duration_ms: value.get("duration_ms").and_then(|v| v.as_u64()),
        ttft_ms: value.get("ttft_ms").and_then(|v| v.as_u64()),
    }
}

/// Surface the per-message token usage from an `assistant` event. The running
/// context + output counts live in `message.usage` (the end-of-turn `result`
/// re-reports the final figure with the context window); surfacing it here lets
/// the streaming status show a live token count instead of waiting for the turn
/// to close. Only the main thread counts — a sub-agent (`parent_tool_use_id`
/// set) has its own context window. There is no `modelUsage` mid-turn, so the
/// window is `None`; the reducer keeps the last-known window across the turn.
fn emit_message_usage(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
    let is_subagent = value
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .is_some();
    if is_subagent {
        return;
    }
    if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
        out.push_back(TranscriptEvent::Usage(usage_from_obj(usage, None)));
    }
}

/// Build a [`Usage`] from a wire `usage` object (the four token counts). The
/// context window is supplied separately because it lives in a sibling
/// `modelUsage` block on the `result` message and is absent on the per-message
/// `assistant` usage.
fn usage_from_obj(usage: &Value, context_window: Option<u64>) -> Usage {
    let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    Usage {
        input_tokens: count("input_tokens"),
        cache_read_input_tokens: count("cache_read_input_tokens"),
        cache_creation_input_tokens: count("cache_creation_input_tokens"),
        output_tokens: count("output_tokens"),
        context_window,
    }
}

/// Extract the context window from a `result` message's
/// `modelUsage[model].contextWindow` (there is one model entry per turn).
/// Returns `None` when the result carries no model-usage block. The result's
/// sibling `usage` block is deliberately ignored — it is the turn aggregate,
/// not the current context occupancy (see [`TranscriptEvent::ContextWindow`]).
fn parse_context_window(value: &Value) -> Option<u64> {
    value
        .get("modelUsage")
        .and_then(|m| m.as_object())
        .and_then(|m| m.values().next())
        .and_then(|model| model.get("contextWindow"))
        .and_then(|v| v.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Transcript, TranscriptItem};

    /// Stateless test entry point: most tests feed one self-contained line and
    /// never exercise the cross-line streaming state, so a fresh [`Parser`] per
    /// call matches the old free-function semantics.
    fn parse_event_into(value: &Value, out: &mut VecDeque<TranscriptEvent>) {
        Parser::default().parse(value, out);
    }

    #[test]
    fn provider_persistence_defaults_unknown_values_to_claude() {
        assert_eq!(
            AgentProvider::from_persisted_or_default(Some("claude")),
            AgentProvider::Claude
        );
        assert_eq!(
            AgentProvider::from_persisted_or_default(Some("codex")),
            AgentProvider::Codex
        );
        assert_eq!(
            AgentProvider::from_persisted_or_default(None),
            AgentProvider::Claude
        );
        assert_eq!(
            AgentProvider::from_persisted_or_default(Some("future-provider")),
            AgentProvider::Claude
        );
    }

    #[test]
    fn decision_serializes_to_current_claude_control_response_payloads() {
        let input = json!({ "command": "cargo test" });

        assert_eq!(
            Decision::allow_once(input.clone()).into_claude_response(),
            json!({ "behavior": "allow", "updatedInput": input })
        );
        assert_eq!(
            Decision::allow_always(json!({ "command": "cargo test" })).into_claude_response(),
            json!({ "behavior": "allow", "updatedInput": { "command": "cargo test" } })
        );
        assert_eq!(
            Decision::deny().into_claude_response(),
            json!({ "behavior": "deny", "message": "The user declined this action." })
        );
        assert_eq!(
            Decision::cancelled().into_claude_response(),
            json!({ "behavior": "cancelled" })
        );
    }

    #[test]
    fn golden_claude_transcript_replay_matches_stable_snapshot() {
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"sess-1","cwd":"/tmp/project","model":"sonnet","permissionMode":"default"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"I'll inspect it."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":10,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4}}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"Cargo.toml\n","is_error":false}]}}"#,
            r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"cargo test"},"tool_use_id":"toolu_2"}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.01,"duration_ms":1200,"ttft_ms":300,"modelUsage":{"sonnet":{"contextWindow":200000}}}"#,
        ];
        let driver = ClaudeDriver;
        let mut parser = driver.new_parser();
        let mut events = VecDeque::new();
        for line in lines {
            driver
                .parse_line(parser.as_mut(), line, &mut events)
                .unwrap();
        }

        let mut transcript = Transcript::new();
        for event in events {
            transcript.apply(event);
        }

        assert_eq!(transcript.session_id(), Some("sess-1"));
        assert_eq!(
            transcript.usage(),
            Some(Usage {
                input_tokens: 10,
                cache_read_input_tokens: 2,
                cache_creation_input_tokens: 3,
                output_tokens: 4,
                context_window: Some(200_000),
            })
        );

        let snapshot: Vec<String> = transcript
            .items()
            .iter()
            .map(|item| match item {
                TranscriptItem::Assistant { text, done } => {
                    format!("assistant:{done}:{text}")
                }
                TranscriptItem::Tool {
                    id,
                    name,
                    input,
                    status,
                    output,
                    children,
                } => format!(
                    "tool:{id}:{name}:{status:?}:{input}:{}:{}",
                    output.as_ref().map(|o| o.text.as_str()).unwrap_or(""),
                    children.len()
                ),
                TranscriptItem::Permission {
                    id,
                    tool,
                    input,
                    decision,
                } => format!("permission:{id}:{tool}:{input}:{decision:?}"),
                TranscriptItem::Metrics(metrics) => format!(
                    "metrics:{:?}:{:?}:{:?}",
                    metrics.total_cost_usd, metrics.duration_ms, metrics.ttft_ms
                ),
                other => format!("unexpected:{other:?}"),
            })
            .collect();

        assert_eq!(
            snapshot,
            vec![
                "assistant:true:I'll inspect it.".to_owned(),
                "tool:toolu_1:Bash:Completed:{\"command\":\"ls\"}:Cargo.toml\n:0".to_owned(),
                "permission:req-1:Bash:{\"command\":\"cargo test\"}:None".to_owned(),
                "metrics:Some(0.01):Some(1200):Some(300)".to_owned(),
            ]
        );
    }

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
    fn parses_init_mcp_servers() {
        let v: Value = serde_json::from_str(
            r#"{"type":"system","subtype":"init","session_id":"abc","cwd":"/tmp/p",
                "mcp_servers":[
                    {"name":"github","status":"connected"},
                    {"name":"figma","status":"failed"},
                    {"name":"bare"}
                ]}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::SessionInit { mcp_servers, .. }) => {
                assert_eq!(mcp_servers.len(), 3);
                assert_eq!(mcp_servers[0].name, "github");
                assert_eq!(mcp_servers[0].status.as_deref(), Some("connected"));
                assert!(mcp_servers[0].tools.is_empty());
                assert_eq!(mcp_servers[1].status.as_deref(), Some("failed"));
                assert_eq!(mcp_servers[2].name, "bare");
                assert_eq!(mcp_servers[2].status, None, "missing status tolerated");
            }
            other => panic!("expected SessionInit, got {other:?}"),
        }
    }

    #[test]
    fn parses_init_without_mcp_servers_field() {
        let v: Value = serde_json::from_str(
            r#"{"type":"system","subtype":"init","session_id":"abc","cwd":"/tmp/p"}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::SessionInit { mcp_servers, .. }) => {
                assert!(mcp_servers.is_empty(), "absent field → empty, not a panic");
            }
            other => panic!("expected SessionInit, got {other:?}"),
        }
    }

    #[test]
    fn parses_task_notification_system_event() {
        // The live completion signal for a `run_in_background` Bash — verbatim
        // shape (minus ids) from a captured claude 2.x stream.
        let v: Value = serde_json::from_str(
            r#"{"type":"system","subtype":"task_notification","task_id":"b0nv3gmmz",
                "tool_use_id":"toolu_014j","status":"completed",
                "output_file":"/tmp/tasks/b0nv3gmmz.output",
                "summary":"Background command \"build\" completed (exit code 0)"}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::TaskNotification(n)) => {
                assert_eq!(n.task_id, "b0nv3gmmz");
                assert_eq!(n.tool_use_id.as_deref(), Some("toolu_014j"));
                assert_eq!(n.status, TaskRunStatus::Completed);
                assert!(n.summary.as_deref().unwrap().contains("exit code 0"));
            }
            other => panic!("expected TaskNotification, got {other:?}"),
        }
    }

    #[test]
    fn task_notification_statuses_map_failed_and_stopped() {
        for (wire, expected) in [
            ("failed", TaskRunStatus::Failed),
            ("stopped", TaskRunStatus::Stopped),
            ("killed", TaskRunStatus::Stopped),
        ] {
            let v: Value = serde_json::from_str(&format!(
                r#"{{"type":"system","subtype":"task_notification","task_id":"t1","status":"{wire}"}}"#,
            ))
            .unwrap();
            let mut out = VecDeque::new();
            parse_event_into(&v, &mut out);
            match out.front() {
                Some(TranscriptEvent::TaskNotification(n)) => assert_eq!(n.status, expected),
                other => panic!("expected TaskNotification for `{wire}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn task_notification_with_unknown_status_is_dropped() {
        let v: Value = serde_json::from_str(
            r#"{"type":"system","subtype":"task_notification","task_id":"t1","status":"paused"}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(out.is_empty(), "unfamiliar status dropped, not guessed");
    }

    #[test]
    fn history_task_notification_line_replays_as_notification_not_user_turn() {
        // The session file stores the notification as a `user` line carrying a
        // `<task-notification>` envelope — verbatim shape from claude's jsonl.
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":"<task-notification>\n<task-id>b0nv3gmmz</task-id>\n<tool-use-id>toolu_014j</tool-use-id>\n<output-file>/tmp/tasks/b0nv3gmmz.output</output-file>\n<status>completed</status>\n<summary>Background command \"build\" completed (exit code 0)</summary>\n</task-notification>"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_history_line(&v, &mut out);
        assert_eq!(out.len(), 1);
        match out.front() {
            Some(TranscriptEvent::TaskNotification(n)) => {
                assert_eq!(n.task_id, "b0nv3gmmz");
                assert_eq!(n.tool_use_id.as_deref(), Some("toolu_014j"));
                assert_eq!(n.status, TaskRunStatus::Completed);
            }
            other => panic!("expected TaskNotification, got {other:?}"),
        }
    }

    #[test]
    fn malformed_task_notification_never_renders_as_user_bubble() {
        // Even when the envelope can't be parsed (no task-id), it's harness
        // bookkeeping — a resumed pane must not show the raw XML as a prompt.
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":"<task-notification>garbled</task-notification>"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_history_line(&v, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn parses_can_use_tool_as_permission_request() {
        let v: Value = serde_json::from_str(
            r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/tmp/x.txt","content":"hi"},"tool_use_id":"toolu_42"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::PermissionRequest {
                id,
                tool,
                input,
                tool_use_id,
            }) => {
                assert_eq!(id, "req-1");
                assert_eq!(tool, "Write");
                assert_eq!(
                    input.get("file_path").and_then(|v| v.as_str()),
                    Some("/tmp/x.txt")
                );
                assert_eq!(tool_use_id.as_deref(), Some("toolu_42"));
            }
            other => panic!("expected PermissionRequest, got {other:?}"),
        }
    }

    #[test]
    fn parses_request_user_dialog_as_question_request() {
        let v: Value = serde_json::from_str(
            r#"{"type":"control_request","request_id":"req-9","request":{"subtype":"request_user_dialog","dialog_kind":"question","payload":{"questions":[{"header":"Indent","question":"Tabs or spaces?","options":["Tabs","Spaces"]}]}}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        match out.front() {
            Some(TranscriptEvent::QuestionRequest {
                id,
                dialog_kind,
                payload,
            }) => {
                assert_eq!(id, "req-9");
                assert_eq!(dialog_kind, "question");
                assert!(payload
                    .get("questions")
                    .and_then(|v| v.as_array())
                    .is_some());
            }
            other => panic!("expected QuestionRequest, got {other:?}"),
        }
    }

    #[test]
    fn ignores_unknown_control_request_subtype() {
        // An unrecognized control_request must not crash or emit — `claude`
        // settles it with its own park deadline (PRODUCT §26).
        let v: Value = serde_json::from_str(
            r#"{"type":"control_request","request_id":"x","request":{"subtype":"oauth_token_refresh"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn result_emits_only_context_window_not_aggregate_usage() {
        // The result's `usage` is the turn aggregate (cache reads sum across the
        // whole agentic loop); only the context window is taken from it, so the
        // chip isn't overstated by the cumulative figure.
        let v: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":5102,"cache_read_input_tokens":8500000,"cache_creation_input_tokens":5450,"output_tokens":4},"modelUsage":{"claude-fable-5[1m]":{"contextWindow":200000}}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        // ContextWindow is emitted before Ended so the chip is fresh on close.
        match out.front() {
            Some(TranscriptEvent::ContextWindow(window)) => assert_eq!(*window, 200_000),
            other => panic!("expected ContextWindow first, got {other:?}"),
        }
        // The aggregate token counts are never surfaced as a Usage event.
        assert!(!out.iter().any(|e| matches!(e, TranscriptEvent::Usage(_))));
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
    fn assistant_message_emits_live_usage() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":5000,"cache_read_input_tokens":50000,"cache_creation_input_tokens":3700,"output_tokens":12}}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        // Usage rides ahead of the text so the live token count is fresh.
        match out.front() {
            Some(TranscriptEvent::Usage(u)) => {
                assert_eq!(u.context_used(), 5000 + 50000 + 3700);
                assert_eq!(u.output_tokens, 12);
                // No `modelUsage` mid-turn — the window fills in at `result`.
                assert_eq!(u.context_window, None);
            }
            other => panic!("expected Usage first, got {other:?}"),
        }
        assert!(out
            .iter()
            .any(|e| matches!(e, TranscriptEvent::AssistantTextDelta { text } if text == "hi")));
    }

    #[test]
    fn subagent_assistant_message_emits_no_usage() {
        // A `Task` sub-agent has its own context window; its tokens must not
        // overwrite the main thread's live count.
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","parent_tool_use_id":"task_1","message":{"role":"assistant","content":[{"type":"text","text":"sub"}],"usage":{"input_tokens":1,"output_tokens":2}}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_event_into(&v, &mut out);
        assert!(!out.iter().any(|e| matches!(e, TranscriptEvent::Usage(_))));
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
    fn history_command_envelope_becomes_typed_command_line() {
        // Both stored orderings (message-first from the CLI, name-first with
        // indented follow-up lines from the desktop app).
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":"<command-message>fleet</command-message>\n<command-name>/fleet</command-name>\n<command-args>run it and monitor</command-args>"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_history_line(&v, &mut out);
        assert!(matches!(
            out.front(),
            Some(TranscriptEvent::UserMessage(m)) if m == "/fleet run it and monitor"
        ));

        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>\n            <command-message>model</command-message>"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_history_line(&v, &mut out);
        assert!(matches!(
            out.front(),
            Some(TranscriptEvent::UserMessage(m)) if m == "/model"
        ));
    }

    #[test]
    fn history_interrupt_marker_is_the_interrupted_notice_not_a_user_turn() {
        // Stop stores "[Request interrupted by user]" as a `user` line. It must
        // replay as the interruption (like the live `Ended`), never as a user
        // turn — `fork_session_file` counts turns through this parser and an
        // extra one shifts the fork boundary a whole turn early.
        for content in [
            r#""[Request interrupted by user]""#,
            r#"[{"type":"text","text":"[Request interrupted by user for tool use]"}]"#,
        ] {
            let v: Value = serde_json::from_str(&format!(
                r#"{{"type":"user","message":{{"role":"user","content":{content}}}}}"#
            ))
            .unwrap();
            let mut out = VecDeque::new();
            parse_history_line(&v, &mut out);
            assert!(
                matches!(
                    out.front(),
                    Some(TranscriptEvent::Ended {
                        reason: EndReason::Interrupted
                    })
                ),
                "expected interrupted end, got {out:?}"
            );
            assert!(!out
                .iter()
                .any(|e| matches!(e, TranscriptEvent::UserMessage(_))));
        }
    }

    #[test]
    fn history_skips_local_command_stdout_echo() {
        let v: Value = serde_json::from_str(
            r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to Opus</local-command-stdout>"}}"#,
        )
        .unwrap();
        let mut out = VecDeque::new();
        parse_history_line(&v, &mut out);
        assert!(out.is_empty(), "expected nothing, got {out:?}");
    }

    #[test]
    fn display_user_text_leaves_plain_and_tag_quoting_text_alone() {
        assert_eq!(
            display_user_text("fix the bug").as_deref(),
            Some("fix the bug")
        );
        // Tags mentioned mid-message (e.g. the user quoting a transcript)
        // aren't a command envelope — only leading tags are.
        let quoting = "why does it show <command-name>/fleet</command-name>?";
        assert_eq!(display_user_text(quoting).as_deref(), Some(quoting));
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

    // --- 7k: token streaming via `--include-partial-messages` ---------------

    /// Feed a sequence of stream-json lines through one [`Parser`] (streaming
    /// needs cross-line state) and collect every emitted event.
    fn stream(lines: &[&str]) -> Vec<TranscriptEvent> {
        let mut parser = Parser::default();
        let mut out = VecDeque::new();
        for line in lines {
            parser.parse(&serde_json::from_str(line).unwrap(), &mut out);
        }
        out.into()
    }

    #[test]
    fn text_deltas_stream_then_consolidated_assistant_is_a_done_marker() {
        // Two text_deltas, a content_block_stop, then the consolidated
        // `assistant` must NOT re-append the text (PRODUCT §46).
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]}}"#,
        ]);
        let texts: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                TranscriptEvent::AssistantTextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Hel", "lo"], "text streams once, not twice");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, TranscriptEvent::AssistantTextDone))
                .count(),
            1
        );
    }

    #[test]
    fn streamed_text_without_message_start_still_suppresses_consolidated() {
        // If a `message_start` is missing but content blocks still stream, the
        // text must render exactly once — the consolidated `assistant` event
        // must not re-append it (the "duplicate of the last message" bug). The
        // opened content block is enough to mark the message streamed.
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]}}"#,
        ]);
        let texts: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                TranscriptEvent::AssistantTextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Hello"], "text streams once, not twice");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, TranscriptEvent::AssistantTextDone))
                .count(),
            1
        );
    }

    #[test]
    fn non_streamed_assistant_still_renders_from_consolidated_event() {
        // A turn that emits no partial deltas (no message_start) renders via the
        // consolidated event exactly as before (PRODUCT §46).
        let events = stream(&[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
        ]);
        assert!(matches!(&events[0], TranscriptEvent::AssistantTextDelta { text } if text == "hi"));
        assert!(matches!(events[1], TranscriptEvent::AssistantTextDone));
    }

    #[test]
    fn thinking_deltas_stream_and_finish_with_a_measured_duration() {
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
        ]);
        assert!(
            matches!(&events[0], TranscriptEvent::ThinkingDelta { text } if text == "Let me think")
        );
        assert!(matches!(
            events[1],
            TranscriptEvent::ThinkingDone { duration: Some(_) }
        ));
    }

    #[test]
    fn signature_only_thinking_block_emits_nothing() {
        // A thinking block with no thinking_delta (signature only) opens no item
        // and finalizes nothing (PRODUCT §22).
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"CAIS"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
        ]);
        assert!(events.is_empty(), "expected nothing, got {events:?}");
    }

    #[test]
    fn streamed_tool_use_args_accumulate_and_emit_one_call_at_stop() {
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"Read","input":{}}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"a.rs\"}"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.rs"}}]}}"#,
        ]);
        let calls: Vec<&TranscriptEvent> = events
            .iter()
            .filter(|e| matches!(e, TranscriptEvent::ToolCall { .. }))
            .collect();
        assert_eq!(
            calls.len(),
            1,
            "exactly one card, not a streamed + consolidated dup"
        );
        match calls[0] {
            TranscriptEvent::ToolCall {
                id, name, input, ..
            } => {
                assert_eq!(id, "t1");
                assert_eq!(name, "Read");
                assert_eq!(input["file_path"], "a.rs");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn streamed_top_level_todowrite_routes_to_todos() {
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"TodoWrite","input":{}}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"todos\":[{\"content\":\"step\",\"status\":\"pending\"}]}"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
        ]);
        assert!(
            matches!(&events[0], TranscriptEvent::Todos(items) if items.len() == 1),
            "streamed TodoWrite must route to Todos, got {events:?}"
        );
    }

    #[test]
    fn subagent_streamed_text_is_suppressed() {
        // A sub-agent's streamed prose (parent_tool_use_id set) is internal
        // monologue and must not surface as main-transcript text (PRODUCT §19).
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":"task_1","event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":"task_1","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":"task_1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"internal"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":"task_1","event":{"type":"content_block_stop","index":0}}"#,
        ]);
        assert!(
            events.is_empty(),
            "sub-agent prose must be suppressed, got {events:?}"
        );
    }

    #[test]
    fn result_emits_per_turn_metrics_before_ended() {
        let events = stream(&[
            r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0123,"duration_ms":4200,"ttft_ms":850}"#,
        ]);
        let metrics = events.iter().find_map(|e| match e {
            TranscriptEvent::Metrics(m) => Some(*m),
            _ => None,
        });
        let m = metrics.expect("metrics emitted");
        assert_eq!(m.total_cost_usd, Some(0.0123));
        assert_eq!(m.duration_ms, Some(4200));
        assert_eq!(m.ttft_ms, Some(850));
        // Metrics precede Ended so they render as the turn's last content item.
        let metrics_pos = events
            .iter()
            .position(|e| matches!(e, TranscriptEvent::Metrics(_)))
            .unwrap();
        let ended_pos = events
            .iter()
            .position(|e| matches!(e, TranscriptEvent::Ended { .. }))
            .unwrap();
        assert!(metrics_pos < ended_pos);
    }

    #[test]
    fn result_without_metric_fields_emits_no_metrics_event() {
        let events = stream(&[r#"{"type":"result","subtype":"success","is_error":false}"#]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, TranscriptEvent::Metrics(_))),
            "absent fields render no line, got {events:?}"
        );
    }

    #[test]
    fn unknown_stream_event_delta_type_is_skipped() {
        let events = stream(&[
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"role":"assistant"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":0,"delta":{"type":"future_delta","data":"?"}}}"#,
            r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_stop","index":0}}"#,
        ]);
        // Only the block's done marker survives; the unknown delta is dropped.
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TranscriptEvent::AssistantTextDone));
    }
}
