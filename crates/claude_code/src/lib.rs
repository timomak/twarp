//! `claude_code` — headless protocol + driver for twarp's Claude Code panel
//! (roadmap feature 07).
//!
//! Sub-phase **7b** defines the *contract*: the thin, twarp-native
//! [`TranscriptEvent`] the driver emits and the [`Transcript`] / [`TranscriptItem`]
//! model the panel renders. The subprocess driver and the defensive stream-json
//! parser that *produce* these events land in **7c**; this crate is intentionally
//! headless (no GPUI) so the parser can be unit-tested against golden transcripts
//! and the UI can be view-tested against synthetic events with no live `claude`
//! process (TECH.md §Parallelization).
//!
//! The UI never sees raw `claude` JSON. The 7c driver translates `claude`'s
//! `--output-format stream-json` output into [`TranscriptEvent`]s, and the panel
//! bridge applies them to a [`Transcript`] on the main thread via
//! [`Transcript::apply`]. Keeping the event→model mapping here (and not in the
//! view) is what makes it testable without a window.

pub mod diff;
pub mod driver;
pub mod launch;
pub mod sessions;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

/// A tool's result payload, as surfaced to the UI (PRODUCT §26).
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Full result text (stdout, file contents, match list, …).
    pub text: String,
    /// Optional short summary the card collapses to (line/byte/match count,
    /// exit status). `None` lets the renderer derive one.
    pub summary: Option<String>,
}

/// Status of one `TodoWrite` entry (PRODUCT §37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// One entry in a Claude Code to-do list (PRODUCT §37–§38).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub text: String,
    pub status: TodoStatus,
}

/// One MCP server available to the session, for the read-only MCP viewer
/// (feature 13). Name + status come from the `system`/`init` event; `tools` is
/// derived incrementally from observed `mcp__<server>__<tool>` calls (the init
/// event does not enumerate tools).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerInfo {
    pub name: String,
    /// Raw status string from the init event (`"connected"` / `"failed"` /
    /// `"pending"` / other), or `None` if the event listed the server without a
    /// status.
    pub status: Option<String>,
    /// Tool names (with the `mcp__<server>__` prefix stripped), in first-seen
    /// order. Empty until Claude actually calls one of the server's tools.
    pub tools: Vec<String>,
}

/// Status of a tool-call card (PRODUCT §23: running → completed/failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Completed,
    Failed,
}

/// Why the current turn (or the whole session) ended (PRODUCT §52).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// The turn completed normally.
    Completed,
    /// The user interrupted the turn via Stop (PRODUCT §11).
    Interrupted,
    /// `claude` reported an error (auth / rate-limit / tool failure). Surfaced
    /// verbatim to the user (PRODUCT §55).
    Error(String),
    /// The `claude` process exited unexpectedly mid-turn (PRODUCT §52).
    Exited,
}

/// Token usage + context window for the latest completed turn, parsed from the
/// stream-json `result` message. Surfaced as the composer's context chip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    /// The model's total context window, when `claude` reports it
    /// (`result.modelUsage[model].contextWindow`).
    pub context_window: Option<u64>,
}

impl Usage {
    /// Tokens occupying the context window right now: the whole prompt that was
    /// sent (fresh input + cache read + cache creation). Output tokens are the
    /// reply, not context occupancy.
    pub fn context_used(&self) -> u64 {
        self.input_tokens + self.cache_read_input_tokens + self.cache_creation_input_tokens
    }
}

/// Cost + timing for the turn that just completed, parsed from the stream-json
/// `result` message (PRODUCT §48). Every field is optional: a field the stream
/// omits is left out of the rendered line, never shown as `0`. Session-local
/// display only — twarp meters nothing (PRODUCT §34).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnMetrics {
    /// `result.total_cost_usd`.
    pub total_cost_usd: Option<f64>,
    /// `result.duration_ms` — wall-clock for the whole turn.
    pub duration_ms: Option<u64>,
    /// `result.ttft_ms` — time to first token.
    pub ttft_ms: Option<u64>,
}

impl TurnMetrics {
    /// `true` when no field is present — the caller renders no metrics line
    /// rather than an empty one (PRODUCT §48).
    pub fn is_empty(&self) -> bool {
        self.total_cost_usd.is_none() && self.duration_ms.is_none() && self.ttft_ms.is_none()
    }
}

/// The thin, twarp-native event the 7c driver emits and the panel consumes.
///
/// This is the contract both halves of feature 07 meet at: the driver crate
/// produces these, the app-side panel applies them. It deliberately carries no
/// `claude`-specific wire shape — the driver absorbs schema drift behind it.
#[derive(Debug, Clone)]
pub enum TranscriptEvent {
    /// `claude` announced its session id, working directory, and session
    /// metadata (`system`/`init`) — model, permission mode, and fast-mode state,
    /// each optional since older `claude` builds omit them.
    SessionInit {
        session_id: String,
        cwd: PathBuf,
        model: Option<String>,
        permission_mode: Option<String>,
        fast_mode: Option<String>,
        /// The session's available slash commands (built-ins + skills +
        /// plugins), from the `init` message — drives the composer's `/`
        /// suggestions (PRODUCT §15a).
        slash_commands: Vec<String>,
        /// MCP servers the session reported at init (name + status only; tools
        /// are derived from observed calls). Drives the MCP viewer (feature 13).
        mcp_servers: Vec<McpServerInfo>,
    },
    /// A user turn was sent into the session (PRODUCT §16).
    UserMessage(String),
    /// Incremental assistant text, or a whole message if partial streaming is
    /// unavailable (PRODUCT §17).
    AssistantTextDelta { text: String },
    /// The current assistant text block finished.
    AssistantTextDone,
    /// Extended-thinking content, with a duration when known (PRODUCT §34). The
    /// whole-block form: the non-streaming path and 7h history replay emit this
    /// once per finished block. Token streaming uses [`ThinkingDelta`] +
    /// [`ThinkingDone`] instead.
    ///
    /// [`ThinkingDelta`]: TranscriptEvent::ThinkingDelta
    /// [`ThinkingDone`]: TranscriptEvent::ThinkingDone
    Thinking {
        text: String,
        duration: Option<Duration>,
    },
    /// Incremental extended-thinking text (PRODUCT §45) — accumulates into the
    /// open thinking block exactly as [`AssistantTextDelta`] does for prose.
    ///
    /// [`AssistantTextDelta`]: TranscriptEvent::AssistantTextDelta
    ThinkingDelta { text: String },
    /// The open thinking block finished; `duration` is its measured wall-clock
    /// (PRODUCT §47), `None` when the stream carried no timing.
    ThinkingDone { duration: Option<Duration> },
    /// A tool invocation (PRODUCT §23). `input` is the raw tool input; the panel
    /// renders a per-tool summary (7d) or a generic card for unmapped tools.
    /// `parent_id` is set when the call was made by a `Task` sub-agent (the
    /// stream-json `parent_tool_use_id`); the transcript nests it under that
    /// Task's card so the card visually groups the child activity it spawned
    /// (PRODUCT §19).
    ToolCall {
        id: String,
        name: String,
        input: Value,
        parent_id: Option<String>,
    },
    /// A tool's result, matched back to its [`TranscriptEvent::ToolCall`] by id
    /// (PRODUCT §26).
    ToolResult {
        id: String,
        output: ToolOutput,
        is_error: bool,
    },
    /// A `TodoWrite` update. Replaces the live task list in place (PRODUCT §37).
    Todos(Vec<TodoItem>),
    /// `claude` requested permission to use a tool (PRODUCT §24, 7g). `id` is the
    /// control-protocol `request_id` the Allow/Deny answer echoes back
    /// (`driver::send_control_response`); `input` is the proposed tool input,
    /// returned verbatim as `updatedInput` on allow. `tool_use_id` ties the
    /// request to the assistant `tool_use` block it gates — the pane uses it to
    /// hold an `AskUserQuestion` permission open against its inline question card
    /// so the picks ride back as the tool's answers (PRODUCT §1).
    PermissionRequest {
        id: String,
        tool: String,
        input: Value,
        tool_use_id: Option<String>,
    },
    /// `claude` asked the user a question over the control channel (an
    /// `AskUserQuestion` `request_user_dialog`, PRODUCT §24/§1). `id` is the
    /// `request_id` to answer; `payload` is the opaque dialog body (the
    /// `question` kind mirrors the tool's `{questions:[..]}` shape).
    QuestionRequest {
        id: String,
        dialog_kind: String,
        payload: Value,
    },
    /// The current turn (or session) ended (PRODUCT §52).
    Ended { reason: EndReason },
    /// Per-message token usage from an `assistant` message — the live context
    /// occupancy that drives the composer's context chip.
    Usage(Usage),
    /// The model's context window for the turn, from the `result` message's
    /// `modelUsage[model].contextWindow`. Emitted on its own (never folded into
    /// a [`Usage`]) because the `result`'s own `usage` block is the turn
    /// *aggregate* — it sums every API call in the agentic loop, so its cache
    /// reads alone can reach millions of tokens and would wildly overstate the
    /// chip's "context used". Only the window is trustworthy there; the token
    /// counts come from the last per-message `assistant` usage.
    ContextWindow(u64),
    /// Cost + timing for the turn that just completed, from the `result`
    /// message. Rendered as a per-turn metrics line (PRODUCT §48).
    Metrics(TurnMetrics),
}

/// One rendered item in the transcript. The panel owns an ordered `Vec` of
/// these. The rich per-tool cards (7d), diff cards (7e), and thinking/todo
/// styling (7f) are refinements of *how* these items render, not new model
/// shapes — so adding them later does not change this contract.
#[derive(Debug, Clone)]
pub enum TranscriptItem {
    /// A user turn (PRODUCT §16).
    User(String),
    /// Assistant prose. Deltas accumulate into the trailing open `Assistant`
    /// item until [`TranscriptEvent::AssistantTextDone`] closes it (PRODUCT §17).
    Assistant { text: String, done: bool },
    /// A collapsible thinking block (PRODUCT §34). `done` is `false` while
    /// `ThinkingDelta`s are still streaming into it and flips `true` on
    /// `ThinkingDone` (or immediately for the whole-block form); `duration`
    /// fills in at the same moment when the stream carried timing (PRODUCT §47).
    Thinking {
        text: String,
        duration: Option<Duration>,
        done: bool,
    },
    /// A tool-call card, advancing running → completed/failed (PRODUCT §23–§29).
    /// `children` holds the nested activity of a `Task` sub-agent (tool calls
    /// made with this call's id as their `parent_id`), so the Task card groups
    /// what it spawned (PRODUCT §19). Empty for every other tool.
    Tool {
        id: String,
        name: String,
        input: Value,
        status: ToolStatus,
        output: Option<ToolOutput>,
        children: Vec<TranscriptItem>,
    },
    /// The in-place task list (PRODUCT §37).
    Todos(Vec<TodoItem>),
    /// An in-transcript permission prompt (PRODUCT §24). `decision` is `None`
    /// while pending, `Some(true/false)` once Allow/Deny is clicked.
    Permission {
        id: String,
        tool: String,
        input: Value,
        decision: Option<bool>,
    },
    /// An in-transcript question prompt from `claude`'s control channel — an
    /// `AskUserQuestion` `request_user_dialog` (PRODUCT §24/§1). `id` is the
    /// `request_id` to answer; `payload` is the opaque dialog body. `answered`
    /// flips once the user submits, so the card stops offering live controls.
    Question {
        id: String,
        dialog_kind: String,
        payload: Value,
        answered: bool,
    },
    /// A per-turn metrics line (cost / duration / time-to-first-token), pushed
    /// when a turn completes (PRODUCT §48).
    Metrics(TurnMetrics),
    /// An out-of-band notice (turn interrupted, session ended).
    Notice(String),
    /// An error surfaced verbatim from `claude` (PRODUCT §55).
    Error(String),
}

/// The ordered conversation the panel renders.
///
/// The 7c bridge feeds it [`TranscriptEvent`]s on the main thread via
/// [`apply`](Transcript::apply). 7b owns this model; the panel starts with an
/// empty transcript (the zero state) and never mutates it until a live session
/// exists in 7c.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    items: Vec<TranscriptItem>,
    /// The `claude` session id, once known. Used by 7h resume.
    session_id: Option<String>,
    /// Session metadata from the `init` message, surfaced as composer chips.
    model: Option<String>,
    permission_mode: Option<String>,
    fast_mode: Option<String>,
    /// Slash commands `claude` reported at init (PRODUCT §15a).
    slash_commands: Vec<String>,
    /// MCP servers available to this session (feature 13). Name + status from
    /// the init event; per-server `tools` derived from observed tool calls.
    mcp_servers: Vec<McpServerInfo>,
    /// Latest turn's token usage + context window (from `result`).
    usage: Option<Usage>,
    /// Bumped on every mutation (`apply` / `clear`). Lets renderers cheaply
    /// invalidate derived views (e.g. the background-scripts list) without
    /// diffing the items — equal revision ⇒ identical `items`.
    revision: u64,
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &[TranscriptItem] {
        &self.items
    }

    /// A monotonic counter bumped on every mutation. Two reads with the same
    /// revision are guaranteed to see identical `items`, so a renderer can use
    /// it as a cache key for anything derived purely from the transcript.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Find a tool card by id, searching nested Task children too. Used by the
    /// panel to resolve a card's current state (e.g. on expand/collapse).
    pub fn find_tool(&self, id: &str) -> Option<&TranscriptItem> {
        find_tool_ref(&self.items, id)
    }

    /// Record a permission decision (7g, PRODUCT §24): flip the matching
    /// pending `Permission` card to `Some(allow)` and return its proposed tool
    /// `input` (echoed back as `updatedInput` on allow). `None` if the id is
    /// unknown or already answered — the caller then sends no control_response.
    pub fn answer_permission(&mut self, id: &str, allow: bool) -> Option<Value> {
        for item in self.items.iter_mut().rev() {
            if let TranscriptItem::Permission {
                id: pid,
                input,
                decision,
                ..
            } = item
            {
                if pid == id && decision.is_none() {
                    *decision = Some(allow);
                    self.revision = self.revision.wrapping_add(1);
                    return Some(input.clone());
                }
            }
        }
        None
    }

    /// Mark a question card answered (7g, PRODUCT §24/§1) and return its dialog
    /// `payload` so the caller can build the answer. `None` if the id is unknown
    /// or already answered.
    pub fn answer_question(&mut self, id: &str) -> Option<Value> {
        for item in self.items.iter_mut().rev() {
            if let TranscriptItem::Question {
                id: qid,
                payload,
                answered,
                ..
            } = item
            {
                if qid == id && !*answered {
                    *answered = true;
                    self.revision = self.revision.wrapping_add(1);
                    return Some(payload.clone());
                }
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The model `claude` reported at session init (e.g. `claude-fable-5[1m]`).
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The active permission mode (`default` / `acceptEdits` / `plan` /
    /// `bypassPermissions`) from session init.
    pub fn permission_mode(&self) -> Option<&str> {
        self.permission_mode.as_deref()
    }

    /// Fast-mode state (`on` / `off`) from session init — the only effort-ish
    /// signal the headless stream-json exposes.
    pub fn fast_mode(&self) -> Option<&str> {
        self.fast_mode.as_deref()
    }

    /// Slash commands `claude` reported at init (PRODUCT §15a). Empty until
    /// the first session init.
    pub fn slash_commands(&self) -> &[String] {
        &self.slash_commands
    }

    /// MCP servers available to this session (feature 13), in init order.
    /// Empty until the first session init. Per-server `tools` fill in as Claude
    /// calls each server's tools during the session.
    pub fn mcp_servers(&self) -> &[McpServerInfo] {
        &self.mcp_servers
    }

    /// Latest turn's token usage + context window, once a turn has completed.
    pub fn usage(&self) -> Option<Usage> {
        self.usage
    }

    /// Reset the transcript (e.g. starting a brand-new session).
    pub fn clear(&mut self) {
        self.items.clear();
        self.session_id = None;
        self.model = None;
        self.permission_mode = None;
        self.fast_mode = None;
        self.slash_commands.clear();
        self.mcp_servers.clear();
        self.usage = None;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Record a tool call against the MCP viewer's per-server tool sets
    /// (feature 13). Non-MCP tools (no `mcp__` prefix) are ignored. The server
    /// is found-or-inserted (inserted with `status: None` if the init event
    /// never listed it — defensive), and the tool is appended once, preserving
    /// first-seen order. Mirrors `tool_cards::tool_display_name`'s split so the
    /// server/tool parsing stays consistent.
    fn record_mcp_tool(&mut self, tool_name: &str) {
        let Some(rest) = tool_name.strip_prefix("mcp__") else {
            return;
        };
        let Some((server, tool)) = rest.split_once("__") else {
            return;
        };
        if tool.is_empty() {
            return;
        }
        let entry = match self.mcp_servers.iter_mut().find(|s| s.name == server) {
            Some(entry) => entry,
            None => {
                self.mcp_servers.push(McpServerInfo {
                    name: server.to_owned(),
                    status: None,
                    tools: Vec::new(),
                });
                self.mcp_servers.last_mut().expect("just pushed")
            }
        };
        if !entry.tools.iter().any(|t| t == tool) {
            entry.tools.push(tool.to_owned());
        }
    }

    /// Apply one driver event to the model.
    ///
    /// This is the single mutation point the 7c bridge calls. Keeping it
    /// headless makes the event→model mapping — delta accumulation, in-place
    /// todo updates, tool-result matching — unit-testable without GPUI.
    pub fn apply(&mut self, event: TranscriptEvent) {
        self.revision = self.revision.wrapping_add(1);
        match event {
            TranscriptEvent::SessionInit {
                session_id,
                model,
                permission_mode,
                fast_mode,
                slash_commands,
                mut mcp_servers,
                ..
            } => {
                self.session_id = Some(session_id);
                self.model = model;
                self.permission_mode = permission_mode;
                self.fast_mode = fast_mode;
                if !slash_commands.is_empty() {
                    self.slash_commands = slash_commands;
                }
                // On resume the init event replays before the tool history
                // re-streams, so overwrite the server list (to pick up fresh
                // statuses) but carry over any tools we'd already derived for a
                // same-named server — otherwise the resume would blank them.
                for server in &mut mcp_servers {
                    if let Some(prev) = self.mcp_servers.iter().find(|s| s.name == server.name) {
                        if server.tools.is_empty() {
                            server.tools = prev.tools.clone();
                        }
                    }
                }
                self.mcp_servers = mcp_servers;
            }
            TranscriptEvent::Usage(usage) => {
                // Mid-turn `assistant` usage carries no context window (that
                // arrives only on the end-of-turn `result`). Keep the last-known
                // window so the context chip's fill doesn't blink off and back
                // on as live counts stream in.
                let context_window = usage
                    .context_window
                    .or_else(|| self.usage.and_then(|u| u.context_window));
                self.usage = Some(Usage {
                    context_window,
                    ..usage
                });
            }
            TranscriptEvent::ContextWindow(window) => match &mut self.usage {
                // Stamp the window onto the live per-message counts; never adopt
                // the `result`'s aggregate token figures (see the variant docs).
                Some(usage) => usage.context_window = Some(window),
                None => {
                    self.usage = Some(Usage {
                        context_window: Some(window),
                        ..Usage::default()
                    })
                }
            },
            TranscriptEvent::Metrics(metrics) => {
                // A turn's cost/timing line renders inline after its content
                // (PRODUCT §48); an all-empty metric set renders nothing.
                if !metrics.is_empty() {
                    self.items.push(TranscriptItem::Metrics(metrics));
                }
            }
            TranscriptEvent::UserMessage(text) => {
                self.items.push(TranscriptItem::User(text));
            }
            TranscriptEvent::AssistantTextDelta { text } => match self.items.last_mut() {
                // Accumulate into the open assistant block (incremental streaming).
                Some(TranscriptItem::Assistant {
                    text: existing,
                    done: false,
                }) => existing.push_str(&text),
                _ => self
                    .items
                    .push(TranscriptItem::Assistant { text, done: false }),
            },
            TranscriptEvent::AssistantTextDone => {
                if let Some(TranscriptItem::Assistant { done, .. }) = self.items.last_mut() {
                    *done = true;
                }
            }
            TranscriptEvent::Thinking { text, duration } => {
                // Whole-block form (non-streaming path, history replay): a
                // finished block, pushed done.
                self.items.push(TranscriptItem::Thinking {
                    text,
                    duration,
                    done: true,
                });
            }
            TranscriptEvent::ThinkingDelta { text } => match self.items.last_mut() {
                // Accumulate into the open thinking block (incremental, §45).
                Some(TranscriptItem::Thinking {
                    text: existing,
                    done: false,
                    ..
                }) => existing.push_str(&text),
                _ => self.items.push(TranscriptItem::Thinking {
                    text,
                    duration: None,
                    done: false,
                }),
            },
            TranscriptEvent::ThinkingDone { duration } => {
                if let Some(TranscriptItem::Thinking {
                    duration: slot,
                    done,
                    ..
                }) = self.items.last_mut()
                {
                    *done = true;
                    *slot = duration;
                }
            }
            TranscriptEvent::ToolCall {
                id,
                name,
                input,
                parent_id,
            } => {
                // Bucket MCP tool calls under their server for the viewer
                // (feature 13). Done before the idempotency guard so a server
                // listed at init that only ever has a replayed call still picks
                // up its tool; dedup inside the helper makes repeats a no-op.
                self.record_mcp_tool(&name);
                // Idempotent on id: resuming a session first renders the stored
                // history (`load_history`), then `claude --resume` replays the
                // same turns on stdout, re-emitting each `tool_use` with the
                // same id. Without this guard each tool would render twice — two
                // cards sharing one expand state, the replayed `tool_result`
                // landing on the duplicate (see `find_tool_mut`'s most-recent
                // semantics). Refresh the existing card in place instead.
                if let Some(TranscriptItem::Tool {
                    name: slot_name,
                    input: slot_input,
                    ..
                }) = find_tool_mut(&mut self.items, &id)
                {
                    *slot_name = name;
                    *slot_input = input;
                    return;
                }
                let card = TranscriptItem::Tool {
                    id,
                    name,
                    input,
                    status: ToolStatus::Running,
                    output: None,
                    children: Vec::new(),
                };
                // A sub-agent's call nests under its spawning Task card
                // (PRODUCT §19). If the parent isn't in the transcript (e.g.
                // resumed mid-task), degrade to a top-level card rather than
                // dropping the activity.
                let parent = parent_id.and_then(|pid| find_tool_mut(&mut self.items, &pid));
                match parent {
                    Some(TranscriptItem::Tool { children, .. }) => children.push(card),
                    _ => self.items.push(card),
                }
            }
            TranscriptEvent::ToolResult {
                id,
                output,
                is_error,
            } => {
                // Attach the result to the matching card, wherever it lives —
                // top-level or nested under a Task (PRODUCT §19).
                if let Some(TranscriptItem::Tool {
                    status,
                    output: slot,
                    ..
                }) = find_tool_mut(&mut self.items, &id)
                {
                    *status = if is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Completed
                    };
                    *slot = Some(output);
                }
            }
            TranscriptEvent::Todos(todos) => {
                // In-place update: revise the existing list rather than append
                // a duplicate each turn (PRODUCT §37).
                let idx = self
                    .items
                    .iter()
                    .rposition(|item| matches!(item, TranscriptItem::Todos(_)));
                if let Some(idx) = idx {
                    if let TranscriptItem::Todos(existing) = &mut self.items[idx] {
                        *existing = todos;
                    }
                } else {
                    self.items.push(TranscriptItem::Todos(todos));
                }
            }
            TranscriptEvent::PermissionRequest {
                id,
                tool,
                input,
                tool_use_id: _,
            } => {
                self.items.push(TranscriptItem::Permission {
                    id,
                    tool,
                    input,
                    decision: None,
                });
            }
            TranscriptEvent::QuestionRequest {
                id,
                dialog_kind,
                payload,
            } => {
                self.items.push(TranscriptItem::Question {
                    id,
                    dialog_kind,
                    payload,
                    answered: false,
                });
            }
            TranscriptEvent::Ended { reason } => match reason {
                EndReason::Completed => {}
                EndReason::Interrupted => {
                    self.items
                        .push(TranscriptItem::Notice("Interrupted.".to_string()));
                }
                EndReason::Error(message) => {
                    self.items.push(TranscriptItem::Error(message));
                }
                EndReason::Exited => self.items.push(TranscriptItem::Notice(
                    "The Claude Code session ended unexpectedly.".to_string(),
                )),
            },
        }
    }
}

/// Depth-first, most-recent-first search for a tool card by id, descending into
/// Task children. `ToolCall` application is idempotent on id, so a live
/// transcript holds at most one card per id; most-recent-first is just a
/// belt-and-braces ordering.
fn find_tool_mut<'a>(items: &'a mut [TranscriptItem], id: &str) -> Option<&'a mut TranscriptItem> {
    for item in items.iter_mut().rev() {
        let is_match = matches!(&*item, TranscriptItem::Tool { id: tid, .. } if tid == id);
        if is_match {
            return Some(item);
        }
        if let TranscriptItem::Tool { children, .. } = item {
            if let Some(found) = find_tool_mut(children, id) {
                return Some(found);
            }
        }
    }
    None
}

fn find_tool_ref<'a>(items: &'a [TranscriptItem], id: &str) -> Option<&'a TranscriptItem> {
    for item in items.iter().rev() {
        if let TranscriptItem::Tool {
            id: tid, children, ..
        } = item
        {
            if tid == id {
                return Some(item);
            }
            if let Some(found) = find_tool_ref(children, id) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(text: &str) -> TranscriptEvent {
        TranscriptEvent::AssistantTextDelta {
            text: text.to_string(),
        }
    }

    #[test]
    fn mid_turn_usage_keeps_last_known_context_window() {
        let mut t = Transcript::new();
        // End-of-turn result establishes the window.
        t.apply(TranscriptEvent::Usage(Usage {
            input_tokens: 1000,
            output_tokens: 50,
            context_window: Some(1_000_000),
            ..Usage::default()
        }));
        // Next turn's live `assistant` usage carries no window — it must not
        // blink the chip's fill off.
        t.apply(TranscriptEvent::Usage(Usage {
            input_tokens: 2000,
            output_tokens: 5,
            context_window: None,
            ..Usage::default()
        }));
        let usage = t.usage().expect("usage stored");
        assert_eq!(usage.input_tokens, 2000);
        assert_eq!(usage.context_window, Some(1_000_000));
    }

    #[test]
    fn context_window_stamps_window_without_clobbering_live_counts() {
        let mut t = Transcript::new();
        // Live per-message usage sets the real context occupancy.
        t.apply(TranscriptEvent::Usage(Usage {
            input_tokens: 5102,
            cache_read_input_tokens: 180_000,
            output_tokens: 12,
            context_window: None,
            ..Usage::default()
        }));
        // The result's window arrives separately and must only stamp the window
        // — the aggregate token counts it carried are never adopted here.
        t.apply(TranscriptEvent::ContextWindow(200_000));
        let usage = t.usage().expect("usage stored");
        assert_eq!(usage.context_used(), 5102 + 180_000);
        assert_eq!(usage.context_window, Some(200_000));
    }

    #[test]
    fn context_window_seeds_usage_when_none_yet() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::ContextWindow(200_000));
        let usage = t.usage().expect("usage seeded");
        assert_eq!(usage.context_used(), 0);
        assert_eq!(usage.context_window, Some(200_000));
    }

    #[test]
    fn assistant_deltas_accumulate_into_one_block() {
        let mut t = Transcript::new();
        t.apply(delta("Hel"));
        t.apply(delta("lo, "));
        t.apply(delta("world"));
        assert_eq!(t.items().len(), 1);
        match &t.items()[0] {
            TranscriptItem::Assistant { text, done } => {
                assert_eq!(text, "Hello, world");
                assert!(!done);
            }
            other => panic!("expected open assistant block, got {other:?}"),
        }
    }

    #[test]
    fn assistant_done_closes_block_so_next_delta_starts_new_one() {
        let mut t = Transcript::new();
        t.apply(delta("first"));
        t.apply(TranscriptEvent::AssistantTextDone);
        t.apply(delta("second"));
        assert_eq!(t.items().len(), 2);
        assert!(matches!(
            &t.items()[0],
            TranscriptItem::Assistant { done: true, .. }
        ));
    }

    #[test]
    fn tool_result_attaches_to_matching_call() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::ToolCall {
            id: "call_1".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({ "file_path": "README.md" }),
            parent_id: None,
        });
        t.apply(TranscriptEvent::ToolResult {
            id: "call_1".to_string(),
            output: ToolOutput {
                text: "contents".to_string(),
                summary: Some("42 lines".to_string()),
            },
            is_error: false,
        });
        match &t.items()[0] {
            TranscriptItem::Tool { status, output, .. } => {
                assert_eq!(*status, ToolStatus::Completed);
                assert!(output.is_some());
            }
            other => panic!("expected tool card, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_tool_call_id_refreshes_in_place() {
        // Resume replays the stored turns after `load_history` already rendered
        // them, re-emitting each `tool_use` with the same id. The second call
        // must refresh the existing card, not push a duplicate (else the tool
        // renders twice and the replayed result lands on the dup).
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::ToolCall {
            id: "call_1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": "ls" }),
            parent_id: None,
        });
        t.apply(TranscriptEvent::ToolResult {
            id: "call_1".to_string(),
            output: ToolOutput {
                text: "a.txt".to_string(),
                summary: None,
            },
            is_error: false,
        });
        // Replay of the same call (no result re-applied this round).
        t.apply(TranscriptEvent::ToolCall {
            id: "call_1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": "ls" }),
            parent_id: None,
        });
        assert_eq!(t.items().len(), 1, "duplicate id must not add a card");
        match &t.items()[0] {
            TranscriptItem::Tool { status, output, .. } => {
                // The earlier result and completed status survive the refresh.
                assert_eq!(*status, ToolStatus::Completed);
                assert!(output.is_some(), "in-place refresh must keep the output");
            }
            other => panic!("expected tool card, got {other:?}"),
        }
    }

    fn tool_call(id: &str, name: &str, parent_id: Option<&str>) -> TranscriptEvent {
        TranscriptEvent::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            input: Value::Null,
            parent_id: parent_id.map(str::to_owned),
        }
    }

    #[test]
    fn subagent_calls_nest_under_their_task_card() {
        let mut t = Transcript::new();
        t.apply(tool_call("task_1", "Task", None));
        t.apply(tool_call("child_1", "Read", Some("task_1")));
        t.apply(tool_call("child_2", "Bash", Some("task_1")));
        assert_eq!(t.items().len(), 1, "children must not appear top-level");
        match &t.items()[0] {
            TranscriptItem::Tool { children, .. } => {
                assert_eq!(children.len(), 2);
                assert!(
                    matches!(&children[0], TranscriptItem::Tool { name, .. } if name == "Read")
                );
            }
            other => panic!("expected Task card, got {other:?}"),
        }
    }

    #[test]
    fn nested_child_result_resolves_inside_task_card() {
        let mut t = Transcript::new();
        t.apply(tool_call("task_1", "Task", None));
        t.apply(tool_call("child_1", "Read", Some("task_1")));
        t.apply(TranscriptEvent::ToolResult {
            id: "child_1".to_string(),
            output: ToolOutput {
                text: "contents".to_string(),
                summary: None,
            },
            is_error: false,
        });
        let child = t.find_tool("child_1").expect("child found via find_tool");
        match child {
            TranscriptItem::Tool { status, .. } => assert_eq!(*status, ToolStatus::Completed),
            other => panic!("expected tool card, got {other:?}"),
        }
        // The Task card itself is still running.
        match t.find_tool("task_1") {
            Some(TranscriptItem::Tool { status, .. }) => assert_eq!(*status, ToolStatus::Running),
            other => panic!("expected Task card, got {other:?}"),
        }
    }

    #[test]
    fn orphaned_parent_id_degrades_to_top_level_card() {
        let mut t = Transcript::new();
        t.apply(tool_call("child_1", "Read", Some("missing_task")));
        assert_eq!(t.items().len(), 1);
        assert!(matches!(
            &t.items()[0],
            TranscriptItem::Tool { name, .. } if name == "Read"
        ));
    }

    #[test]
    fn todos_update_in_place_instead_of_stacking() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::Todos(vec![TodoItem {
            text: "step one".to_string(),
            status: TodoStatus::Pending,
        }]));
        t.apply(TranscriptEvent::Todos(vec![TodoItem {
            text: "step one".to_string(),
            status: TodoStatus::Completed,
        }]));
        let todo_items = t
            .items()
            .iter()
            .filter(|i| matches!(i, TranscriptItem::Todos(_)))
            .count();
        assert_eq!(todo_items, 1, "todo list must update in place, not stack");
        match &t.items()[0] {
            TranscriptItem::Todos(items) => assert_eq!(items[0].status, TodoStatus::Completed),
            other => panic!("expected todos, got {other:?}"),
        }
    }

    #[test]
    fn permission_request_then_answer_records_decision_and_returns_input() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::PermissionRequest {
            id: "req-1".to_string(),
            tool: "Bash".to_string(),
            input: serde_json::json!({ "command": "echo hi" }),
            tool_use_id: Some("toolu_1".to_string()),
        });
        // The proposed input comes back so the caller can echo it as updatedInput.
        let input = t
            .answer_permission("req-1", true)
            .expect("known pending id");
        assert_eq!(
            input.get("command").and_then(|v| v.as_str()),
            Some("echo hi")
        );
        match &t.items()[0] {
            TranscriptItem::Permission { decision, .. } => assert_eq!(*decision, Some(true)),
            other => panic!("expected Permission, got {other:?}"),
        }
        // Answering twice is a no-op (the control_response was already sent).
        assert!(t.answer_permission("req-1", false).is_none());
    }

    #[test]
    fn question_request_then_answer_marks_answered_and_returns_payload() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::QuestionRequest {
            id: "req-2".to_string(),
            dialog_kind: "question".to_string(),
            payload: serde_json::json!({ "questions": [] }),
        });
        let payload = t.answer_question("req-2").expect("known pending id");
        assert!(payload.get("questions").is_some());
        match &t.items()[0] {
            TranscriptItem::Question { answered, .. } => assert!(*answered),
            other => panic!("expected Question, got {other:?}"),
        }
        assert!(t.answer_question("req-2").is_none());
    }

    #[test]
    fn error_end_surfaces_verbatim_error_item() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::Ended {
            reason: EndReason::Error("usage limit reached".to_string()),
        });
        assert!(matches!(&t.items()[0], TranscriptItem::Error(m) if m == "usage limit reached"));
    }

    #[test]
    fn session_init_records_id_and_metadata() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::SessionInit {
            session_id: "abc-123".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            model: Some("claude-fable-5[1m]".to_string()),
            permission_mode: Some("default".to_string()),
            fast_mode: Some("off".to_string()),
            slash_commands: vec!["compact".to_string()],
            mcp_servers: vec![],
        });
        assert_eq!(t.session_id(), Some("abc-123"));
        assert_eq!(t.model(), Some("claude-fable-5[1m]"));
        assert_eq!(t.permission_mode(), Some("default"));
        assert_eq!(t.fast_mode(), Some("off"));
        assert_eq!(t.slash_commands(), ["compact".to_string()]);
        assert!(t.is_empty(), "session init alone renders nothing");
    }

    /// A `SessionInit` carrying the given MCP servers (name, raw status), with
    /// the other metadata fields left at sensible defaults — for the feature-13
    /// viewer tests.
    fn session_init_with_mcp(servers: &[(&str, Option<&str>)]) -> TranscriptEvent {
        TranscriptEvent::SessionInit {
            session_id: "s".to_string(),
            cwd: PathBuf::from("/tmp"),
            model: None,
            permission_mode: None,
            fast_mode: None,
            slash_commands: vec![],
            mcp_servers: servers
                .iter()
                .map(|(name, status)| McpServerInfo {
                    name: (*name).to_string(),
                    status: status.map(str::to_owned),
                    tools: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn session_init_records_mcp_servers_with_status() {
        let mut t = Transcript::new();
        t.apply(session_init_with_mcp(&[
            ("github", Some("connected")),
            ("figma", Some("failed")),
            ("linear", None),
        ]));
        let servers = t.mcp_servers();
        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].name, "github");
        assert_eq!(servers[0].status.as_deref(), Some("connected"));
        assert!(servers[0].tools.is_empty(), "init enumerates no tools");
        assert_eq!(servers[1].status.as_deref(), Some("failed"));
        assert_eq!(servers[2].status, None);
    }

    #[test]
    fn mcp_tool_calls_bucket_under_their_server() {
        let mut t = Transcript::new();
        t.apply(session_init_with_mcp(&[("github", Some("connected"))]));
        t.apply(tool_call("c1", "mcp__github__create_issue", None));
        t.apply(tool_call("c2", "mcp__github__list_issues", None));
        // A non-MCP tool must not touch the server inventory.
        t.apply(tool_call("c3", "Read", None));
        let github = t
            .mcp_servers()
            .iter()
            .find(|s| s.name == "github")
            .expect("github server present");
        assert_eq!(
            github.tools,
            vec!["create_issue".to_string(), "list_issues".to_string()],
            "tools bucket under server, prefix stripped, first-seen order"
        );
        assert_eq!(t.mcp_servers().len(), 1, "Read did not add a server");
    }

    #[test]
    fn repeated_mcp_tool_calls_dedup() {
        let mut t = Transcript::new();
        t.apply(session_init_with_mcp(&[("github", Some("connected"))]));
        t.apply(tool_call("c1", "mcp__github__create_issue", None));
        t.apply(tool_call("c2", "mcp__github__create_issue", None));
        let github = &t.mcp_servers()[0];
        assert_eq!(github.tools, vec!["create_issue".to_string()]);
    }

    #[test]
    fn mcp_tool_for_unlisted_server_is_inserted_defensively() {
        let mut t = Transcript::new();
        // No init listing `surprise`; a call to its tool still records it.
        t.apply(tool_call("c1", "mcp__surprise__do_thing", None));
        let server = &t.mcp_servers()[0];
        assert_eq!(server.name, "surprise");
        assert_eq!(server.status, None);
        assert_eq!(server.tools, vec!["do_thing".to_string()]);
    }

    #[test]
    fn resumed_init_preserves_derived_tools_and_refreshes_status() {
        let mut t = Transcript::new();
        t.apply(session_init_with_mcp(&[("github", Some("pending"))]));
        t.apply(tool_call("c1", "mcp__github__create_issue", None));
        // Resume replays the init (now connected) before the tool history
        // re-streams: status must refresh, derived tools must survive.
        t.apply(session_init_with_mcp(&[("github", Some("connected"))]));
        let github = &t.mcp_servers()[0];
        assert_eq!(github.status.as_deref(), Some("connected"));
        assert_eq!(
            github.tools,
            vec!["create_issue".to_string()],
            "resume must not blank already-derived tools"
        );
    }

    #[test]
    fn thinking_deltas_accumulate_then_done_stamps_duration() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::ThinkingDelta {
            text: "Let me ".to_string(),
        });
        t.apply(TranscriptEvent::ThinkingDelta {
            text: "reason".to_string(),
        });
        // Still open: one item, not done, no duration yet.
        assert_eq!(t.items().len(), 1);
        assert!(matches!(
            &t.items()[0],
            TranscriptItem::Thinking {
                done: false,
                duration: None,
                ..
            }
        ));
        t.apply(TranscriptEvent::ThinkingDone {
            duration: Some(Duration::from_secs(3)),
        });
        match &t.items()[0] {
            TranscriptItem::Thinking {
                text,
                duration,
                done,
            } => {
                assert_eq!(text, "Let me reason");
                assert!(*done);
                assert_eq!(*duration, Some(Duration::from_secs(3)));
            }
            other => panic!("expected finished thinking block, got {other:?}"),
        }
    }

    #[test]
    fn whole_block_thinking_event_pushes_done() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::Thinking {
            text: "done block".to_string(),
            duration: None,
        });
        assert!(matches!(
            &t.items()[0],
            TranscriptItem::Thinking { done: true, .. }
        ));
        // A following delta opens a fresh block rather than reopening this one.
        t.apply(TranscriptEvent::ThinkingDelta {
            text: "next".to_string(),
        });
        assert_eq!(t.items().len(), 2);
    }

    #[test]
    fn metrics_event_pushes_line_and_skips_empty() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::Metrics(TurnMetrics::default()));
        assert!(t.is_empty(), "an all-empty metrics set renders nothing");
        t.apply(TranscriptEvent::Metrics(TurnMetrics {
            total_cost_usd: Some(0.01),
            duration_ms: Some(1200),
            ttft_ms: None,
        }));
        assert!(matches!(
            &t.items()[0],
            TranscriptItem::Metrics(m) if m.total_cost_usd == Some(0.01)
        ));
    }

    #[test]
    fn usage_event_records_latest() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::Usage(Usage {
            input_tokens: 5102,
            cache_read_input_tokens: 15818,
            cache_creation_input_tokens: 5450,
            output_tokens: 4,
            context_window: Some(1_000_000),
        }));
        let u = t.usage().expect("usage recorded");
        assert_eq!(u.context_used(), 5102 + 15818 + 5450);
        assert_eq!(u.context_window, Some(1_000_000));
    }
}
