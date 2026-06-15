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
    },
    /// A user turn was sent into the session (PRODUCT §16).
    UserMessage(String),
    /// Incremental assistant text, or a whole message if partial streaming is
    /// unavailable (PRODUCT §17).
    AssistantTextDelta { text: String },
    /// The current assistant text block finished.
    AssistantTextDone,
    /// Extended-thinking content, with a duration when known (PRODUCT §34).
    Thinking {
        text: String,
        duration: Option<Duration>,
    },
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
    /// `claude` requested permission to use a tool (PRODUCT §39; see TECH §Risks
    /// — this wire channel is the highest-risk, version-gated part of 7g).
    PermissionRequest {
        id: String,
        tool: String,
        input: Value,
    },
    /// The current turn (or session) ended (PRODUCT §52).
    Ended { reason: EndReason },
    /// Token usage + context window for the turn that just completed, from the
    /// `result` message. Drives the composer's context chip.
    Usage(Usage),
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
    /// A collapsible thinking block (PRODUCT §34).
    Thinking {
        text: String,
        duration: Option<Duration>,
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
    /// An in-transcript permission prompt (PRODUCT §39). `decision` is `None`
    /// while pending, `Some(true/false)` once answered.
    Permission {
        id: String,
        tool: String,
        input: Value,
        decision: Option<bool>,
    },
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
    /// Latest turn's token usage + context window (from `result`).
    usage: Option<Usage>,
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &[TranscriptItem] {
        &self.items
    }

    /// Find a tool card by id, searching nested Task children too. Used by the
    /// panel to resolve a card's current state (e.g. on expand/collapse).
    pub fn find_tool(&self, id: &str) -> Option<&TranscriptItem> {
        find_tool_ref(&self.items, id)
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
        self.usage = None;
    }

    /// Apply one driver event to the model.
    ///
    /// This is the single mutation point the 7c bridge calls. Keeping it
    /// headless makes the event→model mapping — delta accumulation, in-place
    /// todo updates, tool-result matching — unit-testable without GPUI.
    pub fn apply(&mut self, event: TranscriptEvent) {
        match event {
            TranscriptEvent::SessionInit {
                session_id,
                model,
                permission_mode,
                fast_mode,
                slash_commands,
                ..
            } => {
                self.session_id = Some(session_id);
                self.model = model;
                self.permission_mode = permission_mode;
                self.fast_mode = fast_mode;
                if !slash_commands.is_empty() {
                    self.slash_commands = slash_commands;
                }
            }
            TranscriptEvent::Usage(usage) => {
                self.usage = Some(usage);
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
                self.items.push(TranscriptItem::Thinking { text, duration });
            }
            TranscriptEvent::ToolCall {
                id,
                name,
                input,
                parent_id,
            } => {
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
            TranscriptEvent::PermissionRequest { id, tool, input } => {
                self.items.push(TranscriptItem::Permission {
                    id,
                    tool,
                    input,
                    decision: None,
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
/// Task children. Most-recent-first preserves the old `rposition` semantics
/// when (pathologically) two calls share an id.
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
        });
        assert_eq!(t.session_id(), Some("abc-123"));
        assert_eq!(t.model(), Some("claude-fable-5[1m]"));
        assert_eq!(t.permission_mode(), Some("default"));
        assert_eq!(t.fast_mode(), Some("off"));
        assert_eq!(t.slash_commands(), ["compact".to_string()]);
        assert!(t.is_empty(), "session init alone renders nothing");
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
