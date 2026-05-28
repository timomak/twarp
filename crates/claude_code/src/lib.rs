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

pub mod driver;
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

/// The thin, twarp-native event the 7c driver emits and the panel consumes.
///
/// This is the contract both halves of feature 07 meet at: the driver crate
/// produces these, the app-side panel applies them. It deliberately carries no
/// `claude`-specific wire shape — the driver absorbs schema drift behind it.
#[derive(Debug, Clone)]
pub enum TranscriptEvent {
    /// `claude` announced its session id and working directory (`system`/`init`).
    SessionInit { session_id: String, cwd: PathBuf },
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
    ToolCall {
        id: String,
        name: String,
        input: Value,
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
    Tool {
        id: String,
        name: String,
        input: Value,
        status: ToolStatus,
        output: Option<ToolOutput>,
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
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &[TranscriptItem] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Reset the transcript (e.g. starting a brand-new session).
    pub fn clear(&mut self) {
        self.items.clear();
        self.session_id = None;
    }

    /// Apply one driver event to the model.
    ///
    /// This is the single mutation point the 7c bridge calls. Keeping it
    /// headless makes the event→model mapping — delta accumulation, in-place
    /// todo updates, tool-result matching — unit-testable without GPUI.
    pub fn apply(&mut self, event: TranscriptEvent) {
        match event {
            TranscriptEvent::SessionInit { session_id, .. } => {
                self.session_id = Some(session_id);
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
            TranscriptEvent::ToolCall { id, name, input } => {
                self.items.push(TranscriptItem::Tool {
                    id,
                    name,
                    input,
                    status: ToolStatus::Running,
                    output: None,
                });
            }
            TranscriptEvent::ToolResult {
                id,
                output,
                is_error,
            } => {
                // Attach the result to the most recent matching running card.
                let idx = self.items.iter().rposition(
                    |item| matches!(item, TranscriptItem::Tool { id: tid, .. } if *tid == id),
                );
                if let Some(idx) = idx {
                    if let TranscriptItem::Tool {
                        status,
                        output: slot,
                        ..
                    } = &mut self.items[idx]
                    {
                        *status = if is_error {
                            ToolStatus::Failed
                        } else {
                            ToolStatus::Completed
                        };
                        *slot = Some(output);
                    }
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
    fn session_init_records_id() {
        let mut t = Transcript::new();
        t.apply(TranscriptEvent::SessionInit {
            session_id: "abc-123".to_string(),
            cwd: PathBuf::from("/tmp/project"),
        });
        assert_eq!(t.session_id(), Some("abc-123"));
        assert!(t.is_empty(), "session init alone renders nothing");
    }
}
