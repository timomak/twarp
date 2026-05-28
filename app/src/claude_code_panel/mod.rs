//! The Claude Code left-panel (roadmap feature 07).
//!
//! Hosts the *rendering layer* of Warp's Agent Mode (resurrected from the
//! pre-AI-removal commit, reparented onto a thin twarp-side model), driven by
//! the local `claude` CLI. The driver lives in the headless [`claude_code`]
//! crate; this module is the GPUI view that owns its [`Transcript`], spawns
//! sessions, renders the conversation, and pumps events back into the model on
//! the main thread.
//!
//! Architecture (PRODUCT §8–§22, §52–§57):
//!
//! 1. User types into [`EditorView`] and presses Enter → [`Self::submit`]
//!    reads the buffer, spawns a session if one isn't running, enqueues the
//!    user turn on a [`async_channel`] the writer task drains.
//! 2. The writer task ([`ctx.spawn`]) owns the child's stdin and writes each
//!    user turn as JSONL until the sender is dropped.
//! 3. The reader stream (set up via [`ctx.spawn_stream_local`]) parses
//!    `claude`'s stdout one [`TranscriptEvent`] at a time on the main thread
//!    and calls [`Self::apply_event`].
//! 4. Drop on the panel side drops the [`LiveSession`], which kills the child
//!    (the spawn sets `kill_on_drop(true)`, PRODUCT §15) and closes the
//!    sender, which lets the writer task end cleanly.

use std::path::PathBuf;
use std::time::SystemTime;

use async_channel::Sender;
use claude_code::driver::{self, Child, PermissionMode, SpawnOptions, SpawnedSession};
use claude_code::sessions::{self, StoredSession};
use claude_code::{
    EndReason, TodoItem, TodoStatus, ToolStatus, Transcript, TranscriptEvent, TranscriptItem,
};
use serde_json::Value;
use similar::TextDiff;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::{
    elements::{
        Container, CrossAxisAlignment, Element, Flex, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Shrinkable,
    },
    presenter::ChildView,
    ui_components::components::UiComponent,
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{EditorOptions, EditorView, Event as EditorEvent, TextOptions};
use crate::util::path::resolve_executable;

/// The executable the panel drives. Resolved on `PATH`; its absence is the
/// unavailable state (PRODUCT §6).
const CLAUDE_BINARY: &str = "claude";

/// How many lines of a tool result to show before collapsing (PRODUCT §27).
const TOOL_RESULT_COLLAPSED_LINES: usize = 8;

#[derive(Clone, Debug)]
pub enum ClaudeCodePanelAction {
    /// Submit the input buffer as a user turn (spawns the session on first
    /// submit, PRODUCT §8).
    Submit,
    /// Interrupt the current turn (PRODUCT §11). The session stays alive.
    Stop,
    /// End the live session entirely (PRODUCT §13). Transcript stays visible.
    EndSession,
    /// Switch the permission mode for new sessions (PRODUCT §41). Does not
    /// retroactively affect the currently running session.
    SetPermissionMode(PermissionMode),
    /// Re-check `claude` availability and reload the stored-session list
    /// (PRODUCT §6 "re-checked each time the panel is opened"; §46).
    Refresh,
    /// Resume a stored session by its `claude` session id (PRODUCT §47).
    ResumeSession(String),
    /// Start a fresh session — ends any live session first (PRODUCT §49).
    NewSession,
    /// Toggle whether a tool card's full output is expanded (PRODUCT §27).
    ToggleToolExpanded(usize),
    /// Toggle whether a thinking card is expanded (PRODUCT §34 — collapsed by
    /// default).
    ToggleThinkingExpanded(usize),
}

#[derive(Clone, Default)]
struct MouseStateHandles {
    submit_button: MouseStateHandle,
    stop_button: MouseStateHandle,
    end_session_button: MouseStateHandle,
    permission_mode_button: MouseStateHandle,
    refresh_button: MouseStateHandle,
}

/// A live `claude` session driven by the panel. Dropping it kills the child
/// (the spawn sets `kill_on_drop(true)`) and closes the writer channel, which
/// lets the writer task end cleanly.
struct LiveSession {
    /// Owns the running `claude` process. The field is kept named (not `_`)
    /// because [`driver::interrupt`] borrows it for SIGINT.
    child: Child,
    /// Sender for the writer task. Sending enqueues a user turn; closing it
    /// (drop) signals the writer task to exit.
    msg_tx: Sender<String>,
    /// `claude` session id, once `system/init` arrived. Used by 7h resume.
    #[allow(dead_code)]
    session_id: Option<String>,
}

pub struct ClaudeCodePanelView {
    transcript: Transcript,
    /// The message input. Real editable buffer; Enter submits, Shift+Enter
    /// inserts a newline (PRODUCT §43).
    input_editor: ViewHandle<EditorView>,
    /// Active session, if any. `None` in the zero state.
    session: Option<LiveSession>,
    /// Permission mode the next session will be spawned with (PRODUCT §41).
    permission_mode: PermissionMode,
    /// True while a turn is streaming output (PRODUCT §10).
    streaming: bool,
    /// Indices of expanded tool cards within `transcript.items()`.
    expanded_tools: Vec<usize>,
    /// Indices of expanded thinking cards within `transcript.items()`.
    expanded_thinking: Vec<usize>,
    /// Stored sessions in this panel's cwd (PRODUCT §46). Loaded at view
    /// construction and refreshed every time a session ends.
    stored_sessions: Vec<StoredSession>,
    mouse_state_handles: MouseStateHandles,
}

impl ClaudeCodePanelView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let input_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                autogrow: true,
                soft_wrap: true,
                text: TextOptions::ui_font_size(appearance),
                ..Default::default()
            };
            EditorView::new(options, ctx)
        });
        ctx.subscribe_to_view(&input_editor, Self::handle_editor_event);
        Self {
            transcript: Transcript::new(),
            input_editor,
            session: None,
            // Default chosen so the smoke test doesn't deadlock on prompts:
            // interactive permission prompts are 7g (see TECH §Risks — the
            // wire protocol is undocumented and only best-effort). Users can
            // switch to a prompting mode via the selector when 7g lands.
            permission_mode: PermissionMode::BypassPermissions,
            streaming: false,
            expanded_tools: Vec::new(),
            expanded_thinking: Vec::new(),
            stored_sessions: Self::load_sessions(),
            mouse_state_handles: MouseStateHandles::default(),
        }
    }

    fn load_sessions() -> Vec<StoredSession> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        sessions::list_sessions(&cwd)
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // PRODUCT §43: Enter sends; Shift+Enter is handled by the editor
        // itself (inserts a newline) and does not reach us.
        if matches!(event, EditorEvent::Enter) {
            self.submit(ctx);
        }
    }

    /// Send the current input as a user turn. Spawns the session on the first
    /// submit (PRODUCT §8); subsequent submits enqueue further turns through
    /// the same writer task.
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            // PRODUCT §10: input is disabled while streaming.
            return;
        }
        let text = self
            .input_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        if text.is_empty() {
            // PRODUCT §44: empty/whitespace messages are a no-op.
            return;
        }
        if !Self::claude_available() {
            self.transcript.apply(TranscriptEvent::Ended {
                reason: EndReason::Error(format!(
                    "`{CLAUDE_BINARY}` is not on PATH. Install Claude Code and reopen this panel."
                )),
            });
            ctx.notify();
            return;
        }
        if self.session.is_none() {
            if let Err(err) = self.start_session(None, ctx) {
                log::error!("Failed to start claude session: {err}");
                self.transcript.apply(TranscriptEvent::Ended {
                    reason: EndReason::Error(format!("Failed to start claude session: {err}")),
                });
                ctx.notify();
                return;
            }
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        // Push the user turn into the transcript immediately so the UI reflects
        // it before the model echoes anything back (PRODUCT §16).
        self.transcript
            .apply(TranscriptEvent::UserMessage(text.clone()));
        if session.msg_tx.try_send(text).is_err() {
            // Writer task gone — channel closed. Treat as session ended.
            self.session = None;
            self.streaming = false;
            self.transcript.apply(TranscriptEvent::Ended {
                reason: EndReason::Exited,
            });
        } else {
            self.streaming = true;
        }
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.notify();
    }

    /// Start a new (or resumed) claude session and wire up its writer + reader
    /// tasks. Sets `self.session` on success.
    fn start_session(
        &mut self,
        resume_session_id: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> anyhow::Result<()> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let opts = SpawnOptions {
            cwd,
            model: None,
            resume_session_id,
            permission_mode: self.permission_mode,
            allowed_tools: Vec::new(),
        };
        let SpawnedSession {
            child,
            mut stdin,
            events,
        } = driver::spawn_session(opts)?;

        // Writer task: drains the user-message channel and writes JSONL into
        // claude's stdin. Dropping `msg_tx` (when the session ends) closes the
        // channel; recv returns Err and the task exits, dropping stdin.
        let (msg_tx, msg_rx) = async_channel::unbounded::<String>();
        ctx.spawn(
            async move {
                while let Ok(text) = msg_rx.recv().await {
                    if let Err(err) = driver::send_user_message(&mut stdin, &text).await {
                        log::warn!("claude stdin write failed: {err}");
                        break;
                    }
                }
            },
            |_, _, _| {},
        );

        // Reader stream: spawn_stream_local applies each event on the main
        // thread. The on_done callback runs when the stream finishes (EOF on
        // stdout).
        ctx.spawn_stream_local(events, Self::apply_event, Self::on_events_done);

        self.session = Some(LiveSession {
            child,
            msg_tx,
            session_id: None,
        });
        Ok(())
    }

    fn apply_event(&mut self, event: TranscriptEvent, ctx: &mut ViewContext<Self>) {
        // Cache session_id locally so 7h can resume.
        if let TranscriptEvent::SessionInit { session_id, .. } = &event {
            if let Some(session) = self.session.as_mut() {
                session.session_id = Some(session_id.clone());
            }
        }
        if matches!(&event, TranscriptEvent::Ended { .. }) {
            // PRODUCT §12: when a turn completes, streaming clears.
            self.streaming = false;
        }
        self.transcript.apply(event);
        ctx.notify();
    }

    fn on_events_done(&mut self, ctx: &mut ViewContext<Self>) {
        // EOF on claude's stdout: process is gone. Drop the live session so
        // the writer task ends and a fresh submit will start a new session.
        // Reload the stored-session list so the session we just finished
        // shows up in the zero state's Resume list.
        self.session = None;
        self.streaming = false;
        self.stored_sessions = Self::load_sessions();
        ctx.notify();
    }

    fn stop(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(session) = self.session.as_ref() {
            driver::interrupt(&session.child);
        }
        // Reflect the user-visible state immediately; the streaming flag will
        // also clear when the eventual `result` event arrives (or via EOF).
        self.streaming = false;
        self.transcript.apply(TranscriptEvent::Ended {
            reason: EndReason::Interrupted,
        });
        ctx.notify();
    }

    fn end_session(&mut self, ctx: &mut ViewContext<Self>) {
        // PRODUCT §13: ending terminates the process but keeps the transcript
        // visible until the next submit clears it.
        self.session = None;
        self.streaming = false;
        self.stored_sessions = Self::load_sessions();
        ctx.notify();
    }

    fn resume_session(&mut self, session_id: String, ctx: &mut ViewContext<Self>) {
        // PRODUCT §49: never drive two live processes from one panel.
        self.session = None;
        self.streaming = false;
        // `claude --resume <id>` replays the session's existing history, so we
        // clear our own transcript first to avoid double-rendering it (PRODUCT
        // §47).
        self.transcript = Transcript::new();
        if let Err(err) = self.start_session(Some(session_id), ctx) {
            log::error!("Failed to resume claude session: {err}");
            self.transcript.apply(TranscriptEvent::Ended {
                reason: EndReason::Error(format!("Failed to resume session: {err}")),
            });
        }
        ctx.notify();
    }

    fn new_session(&mut self, ctx: &mut ViewContext<Self>) {
        // PRODUCT §49: end any current session and clear the transcript so the
        // zero state shows again, ready for a fresh submit.
        self.session = None;
        self.streaming = false;
        self.transcript = Transcript::new();
        self.stored_sessions = Self::load_sessions();
        ctx.notify();
    }

    /// Whether the `claude` CLI is resolvable on `PATH` right now (PRODUCT §6).
    fn claude_available() -> bool {
        resolve_executable(CLAUDE_BINARY).is_some()
    }

    fn unavailable_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = appearance
            .ui_builder()
            .span(format!(
                "Claude Code isn't available. The `{CLAUDE_BINARY}` command wasn't found on \
                 your PATH."
            ))
            .with_soft_wrap()
            .build()
            .finish();
        let hint = appearance
            .ui_builder()
            .span(
                "Install Claude Code (https://docs.claude.com/en/docs/claude-code), make sure \
                 `claude` is on your PATH, then re-open this panel.",
            )
            .with_soft_wrap()
            .build()
            .finish();
        let refresh = appearance
            .ui_builder()
            .link(
                "Check again".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(ClaudeCodePanelAction::Refresh);
                })),
                self.mouse_state_handles.refresh_button.clone(),
            )
            .build()
            .finish();
        padded_column(vec![title, hint, refresh])
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.0);

        // Left: a status pill ("Idle", "Streaming…", or "Session: <id-prefix>").
        let status_text = if !Self::claude_available() {
            "Unavailable".to_string()
        } else if self.streaming {
            "Streaming…".to_string()
        } else if let Some(session) = self.session.as_ref() {
            session
                .session_id
                .as_deref()
                .map(|id| format!("Session {}", id_prefix(id)))
                .unwrap_or_else(|| "Live".to_string())
        } else {
            "Idle".to_string()
        };
        row = row.with_child(appearance.ui_builder().span(status_text).build().finish());

        // Right: permission mode selector + (when live) End session link.
        let mut right = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.0);

        right = right.with_child(
            appearance
                .ui_builder()
                .link(
                    format!("Mode: {}", self.permission_mode.label()),
                    None,
                    Some(Box::new(|ctx| {
                        // Cycle to the next mode for a simple selector. A real
                        // dropdown is straightforward to add later; cycling
                        // through the four modes keeps the chrome small.
                        ctx.dispatch_typed_action(ClaudeCodePanelAction::SetPermissionMode(
                            PermissionMode::Default,
                        ));
                    })),
                    self.mouse_state_handles.permission_mode_button.clone(),
                )
                .build()
                .finish(),
        );
        if self.session.is_some() {
            right = right.with_child(
                appearance
                    .ui_builder()
                    .link(
                        "End session".to_owned(),
                        None,
                        Some(Box::new(|ctx| {
                            ctx.dispatch_typed_action(ClaudeCodePanelAction::EndSession);
                        })),
                        self.mouse_state_handles.end_session_button.clone(),
                    )
                    .build()
                    .finish(),
            );
        }
        row = row.with_child(right.finish());
        Container::new(row.finish())
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .with_padding_top(8.0)
            .with_padding_bottom(8.0)
            .finish()
    }

    fn render_input(&self, appearance: &Appearance) -> Box<dyn Element> {
        // The editor itself (autogrow, soft-wrap from new()).
        let input_view = Container::new(ChildView::new(&self.input_editor).finish())
            .with_padding_top(6.0)
            .with_padding_bottom(6.0)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
            .finish();

        // The submit / stop affordance, swapped depending on streaming state.
        let action: Box<dyn Element> = if self.streaming {
            appearance
                .ui_builder()
                .link(
                    "Stop".to_owned(),
                    None,
                    Some(Box::new(|ctx| {
                        ctx.dispatch_typed_action(ClaudeCodePanelAction::Stop);
                    })),
                    self.mouse_state_handles.stop_button.clone(),
                )
                .build()
                .finish()
        } else {
            let label = if self.session.is_some() {
                "Send"
            } else {
                "Start session"
            };
            appearance
                .ui_builder()
                .link(
                    label.to_owned(),
                    None,
                    Some(Box::new(|ctx| {
                        ctx.dispatch_typed_action(ClaudeCodePanelAction::Submit);
                    })),
                    self.mouse_state_handles.submit_button.clone(),
                )
                .build()
                .finish()
        };

        let col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.0)
            .with_child(input_view)
            .with_child(action)
            .finish();
        Container::new(col)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .with_padding_top(6.0)
            .with_padding_bottom(10.0)
            .finish()
    }

    fn render_transcript(&self, appearance: &Appearance) -> Box<dyn Element> {
        if self.transcript.is_empty() {
            return self.render_zero_state(appearance);
        }
        let items = self
            .transcript
            .items()
            .iter()
            .enumerate()
            .map(|(idx, item)| self.render_item(idx, item, appearance));
        let col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(10.0)
            .with_children(items)
            .finish();
        Container::new(col)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .with_padding_top(6.0)
            .with_padding_bottom(10.0)
            .finish()
    }

    fn render_zero_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut children: Vec<Box<dyn Element>> = Vec::new();
        children.push(
            appearance
                .ui_builder()
                .span(
                    "Type a message and start a session — twarp drives the local `claude` CLI \
                     and renders its replies, tool calls, and diffs here. Your existing Claude \
                     Code login is used; twarp adds no account or billing.",
                )
                .with_soft_wrap()
                .build()
                .finish(),
        );
        if !self.stored_sessions.is_empty() {
            children.push(
                appearance
                    .ui_builder()
                    .span("Resume a previous session in this directory:".to_owned())
                    .with_soft_wrap()
                    .build()
                    .finish(),
            );
            // PRODUCT §46: list `claude`'s own stored sessions for the current
            // cwd, most-recent first. Each row resumes via
            // `claude --resume <id>`.
            for session in &self.stored_sessions {
                let id = session.id.clone();
                let label = format!(
                    "▶ {}  — {}",
                    session.title,
                    relative_time(session.timestamp)
                );
                children.push(
                    appearance
                        .ui_builder()
                        .link(
                            label,
                            None,
                            Some(Box::new(move |ctx| {
                                ctx.dispatch_typed_action(ClaudeCodePanelAction::ResumeSession(
                                    id.clone(),
                                ));
                            })),
                            MouseStateHandle::default(),
                        )
                        .build()
                        .finish(),
                );
            }
        }
        padded_column(children)
    }

    fn render_item(
        &self,
        idx: usize,
        item: &TranscriptItem,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match item {
            TranscriptItem::User(text) => render_user_bubble(text, appearance),
            TranscriptItem::Assistant { text, done } => {
                render_assistant_bubble(text, *done, appearance)
            }
            TranscriptItem::Thinking { text, duration } => render_thinking_card(
                idx,
                text,
                *duration,
                self.expanded_thinking.contains(&idx),
                appearance,
            ),
            TranscriptItem::Tool {
                name,
                input,
                status,
                output,
                ..
            } => render_tool_card(
                idx,
                name,
                input,
                *status,
                output.as_ref(),
                self.expanded_tools.contains(&idx),
                appearance,
            ),
            TranscriptItem::Todos(items) => render_todos(items, appearance),
            TranscriptItem::Permission { tool, input, .. } => {
                render_permission_card(tool, input, appearance)
            }
            TranscriptItem::Notice(message) => render_notice(message, appearance),
            TranscriptItem::Error(message) => render_error(message, appearance),
        }
    }
}

impl View for ClaudeCodePanelView {
    fn ui_name() -> &'static str {
        "ClaudeCodePanelView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // PRODUCT §61: focus the input on entry so typing just works.
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.input_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let available = Self::claude_available();
        // PRODUCT §6: the unavailable state replaces the rest of the panel.
        if !available {
            let mut col = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Max);
            col = col.with_child(self.unavailable_state(appearance));
            return col.finish();
        }
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max);
        col = col.with_child(self.render_header(appearance));
        col = col.with_child(Shrinkable::new(1.0, self.render_transcript(appearance)).finish());
        col = col.with_child(self.render_input(appearance));
        col.finish()
    }
}

impl Entity for ClaudeCodePanelView {
    type Event = ();
}

impl TypedActionView for ClaudeCodePanelView {
    type Action = ClaudeCodePanelAction;

    fn handle_action(&mut self, action: &ClaudeCodePanelAction, ctx: &mut ViewContext<Self>) {
        match action {
            ClaudeCodePanelAction::Submit => self.submit(ctx),
            ClaudeCodePanelAction::Stop => self.stop(ctx),
            ClaudeCodePanelAction::EndSession => self.end_session(ctx),
            ClaudeCodePanelAction::SetPermissionMode(_requested) => {
                // The header link cycles through modes for a low-chrome
                // selector — the explicit mode passed in the action carries
                // no information today; we cycle from current.
                self.permission_mode = next_permission_mode(self.permission_mode);
                ctx.notify();
            }
            ClaudeCodePanelAction::Refresh => {
                self.stored_sessions = Self::load_sessions();
                ctx.notify();
            }
            ClaudeCodePanelAction::ResumeSession(id) => self.resume_session(id.clone(), ctx),
            ClaudeCodePanelAction::NewSession => self.new_session(ctx),
            ClaudeCodePanelAction::ToggleToolExpanded(idx) => {
                toggle_membership(&mut self.expanded_tools, *idx);
                ctx.notify();
            }
            ClaudeCodePanelAction::ToggleThinkingExpanded(idx) => {
                toggle_membership(&mut self.expanded_thinking, *idx);
                ctx.notify();
            }
        }
    }
}

// ---------- helpers ----------

fn padded_column(children: Vec<Box<dyn Element>>) -> Box<dyn Element> {
    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(10.0);
    for child in children {
        col = col.with_child(child);
    }
    Container::new(col.finish())
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_padding_top(10.0)
        .with_padding_bottom(10.0)
        .finish()
}

fn id_prefix(id: &str) -> &str {
    let cut = id.char_indices().nth(8).map(|(i, _)| i).unwrap_or(id.len());
    &id[..cut]
}

fn next_permission_mode(current: PermissionMode) -> PermissionMode {
    let all = PermissionMode::ALL;
    let idx = all.iter().position(|m| *m == current).unwrap_or(0);
    all[(idx + 1) % all.len()]
}

/// Short, friendly relative-time label for the stored-session list.
fn relative_time(t: SystemTime) -> String {
    match t.elapsed() {
        Ok(d) => {
            let secs = d.as_secs();
            if secs < 60 {
                format!("{secs}s ago")
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        }
        Err(_) => "future?".to_string(),
    }
}

fn toggle_membership(vec: &mut Vec<usize>, value: usize) {
    if let Some(pos) = vec.iter().position(|v| *v == value) {
        vec.remove(pos);
    } else {
        vec.push(value);
    }
}

fn render_user_bubble(text: &str, appearance: &Appearance) -> Box<dyn Element> {
    let body = appearance
        .ui_builder()
        .span(text.to_owned())
        .with_soft_wrap()
        .build()
        .finish();
    Container::new(body)
        .with_padding_top(8.0)
        .with_padding_bottom(8.0)
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
        .finish()
}

fn render_assistant_bubble(text: &str, done: bool, appearance: &Appearance) -> Box<dyn Element> {
    // 7c renders assistant prose as plain text. PRODUCT §18 calls for markdown
    // (feature 03's renderer); the cleanest place to lift that in is here
    // once a shared text→element helper is exposed without dragging the AI
    // blocklist back in. Until then plain wrap renders most replies legibly.
    let prefix = if done { "" } else { "… " };
    let span = appearance
        .ui_builder()
        .span(format!("{prefix}{text}"))
        .with_soft_wrap()
        .build()
        .finish();
    Container::new(span)
        .with_padding_top(6.0)
        .with_padding_bottom(6.0)
        .finish()
}

fn render_thinking_card(
    idx: usize,
    text: &str,
    duration: Option<std::time::Duration>,
    expanded: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let title = match duration {
        Some(d) => format!("Thought for {}s", d.as_secs()),
        None => "Thinking".to_string(),
    };
    let header_label = if expanded {
        format!("{title} ▾")
    } else {
        format!("{title} ▸")
    };
    let header = appearance
        .ui_builder()
        .link(
            header_label,
            None,
            Some(Box::new(move |ctx| {
                ctx.dispatch_typed_action(ClaudeCodePanelAction::ToggleThinkingExpanded(idx));
            })),
            MouseStateHandle::default(),
        )
        .build()
        .finish();
    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.0)
        .with_child(header);
    if expanded {
        col = col.with_child(
            appearance
                .ui_builder()
                .span(text.to_owned())
                .with_soft_wrap()
                .build()
                .finish(),
        );
    }
    Container::new(col.finish())
        .with_padding_top(6.0)
        .with_padding_bottom(6.0)
        .with_padding_left(8.0)
        .with_padding_right(8.0)
        .finish()
}

fn render_tool_card(
    idx: usize,
    name: &str,
    input: &Value,
    status: ToolStatus,
    output: Option<&claude_code::ToolOutput>,
    expanded: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let summary = tool_input_summary(name, input);
    let status_label = match status {
        ToolStatus::Running => "running…",
        ToolStatus::Completed => "ok",
        ToolStatus::Failed => "failed",
    };
    let header_text = format!("{name} · {status_label}  {summary}");
    let header = appearance
        .ui_builder()
        .span(header_text)
        .with_soft_wrap()
        .build()
        .finish();

    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.0)
        .with_child(header);

    // 7e diff cards: for file-mutating tools, render a unified diff body
    // synthesized from the tool input.
    if let Some(diff) = diff_for_tool(name, input) {
        col = col.with_child(render_diff_body(&diff, appearance));
    }

    if let Some(output) = output {
        let body_text = if expanded || line_count(&output.text) <= TOOL_RESULT_COLLAPSED_LINES {
            output.text.clone()
        } else {
            let head: String = output
                .text
                .lines()
                .take(TOOL_RESULT_COLLAPSED_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "{head}\n… ({} more lines — click to expand)",
                line_count(&output.text).saturating_sub(TOOL_RESULT_COLLAPSED_LINES)
            )
        };
        let body = appearance
            .ui_builder()
            .span(body_text)
            .with_soft_wrap()
            .build()
            .finish();
        col = col.with_child(
            Container::new(body)
                .with_padding_top(4.0)
                .with_padding_left(8.0)
                .finish(),
        );
        if line_count(&output.text) > TOOL_RESULT_COLLAPSED_LINES {
            let toggle_label = if expanded { "Collapse" } else { "Expand" };
            col = col.with_child(
                appearance
                    .ui_builder()
                    .link(
                        toggle_label.to_owned(),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(ClaudeCodePanelAction::ToggleToolExpanded(
                                idx,
                            ));
                        })),
                        MouseStateHandle::default(),
                    )
                    .build()
                    .finish(),
            );
        }
    }

    Container::new(col.finish())
        .with_padding_top(8.0)
        .with_padding_bottom(8.0)
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
        .finish()
}

fn render_todos(items: &[TodoItem], appearance: &Appearance) -> Box<dyn Element> {
    let header = appearance
        .ui_builder()
        .span("To-do".to_owned())
        .build()
        .finish();
    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.0)
        .with_child(header);
    for item in items {
        let marker = match item.status {
            TodoStatus::Pending => "•",
            TodoStatus::InProgress => "→",
            TodoStatus::Completed => "✓",
        };
        let display = match item.status {
            TodoStatus::Completed => format!("{marker} ~~{}~~", item.text),
            _ => format!("{marker} {}", item.text),
        };
        col = col.with_child(
            appearance
                .ui_builder()
                .span(display)
                .with_soft_wrap()
                .build()
                .finish(),
        );
    }
    Container::new(col.finish())
        .with_padding_top(8.0)
        .with_padding_bottom(8.0)
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
        .finish()
}

fn render_permission_card(tool: &str, input: &Value, appearance: &Appearance) -> Box<dyn Element> {
    // 7g surface for interactive prompts (PRODUCT §39). The wire protocol is
    // undocumented (TECH §Risks), so this card is informational for now: it
    // surfaces the request, and the mode selector + --allowedTools at spawn
    // remain the robust permission path.
    let body = appearance
        .ui_builder()
        .span(format!(
            "Claude requested permission for `{tool}`: {}",
            tool_input_summary(tool, input)
        ))
        .with_soft_wrap()
        .build()
        .finish();
    Container::new(body)
        .with_padding_top(8.0)
        .with_padding_bottom(8.0)
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
        .finish()
}

fn render_notice(message: &str, appearance: &Appearance) -> Box<dyn Element> {
    let body = appearance
        .ui_builder()
        .span(message.to_owned())
        .with_soft_wrap()
        .build()
        .finish();
    Container::new(body)
        .with_padding_top(6.0)
        .with_padding_bottom(6.0)
        .finish()
}

fn render_error(message: &str, appearance: &Appearance) -> Box<dyn Element> {
    // PRODUCT §55: auth/billing/limit errors surface verbatim. Render as a
    // distinct card so the user can copy the message.
    let body = appearance
        .ui_builder()
        .span(format!("Error: {message}"))
        .with_soft_wrap()
        .build()
        .finish();
    Container::new(body)
        .with_padding_top(8.0)
        .with_padding_bottom(8.0)
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
        .finish()
}

fn render_diff_body(unified_diff: &str, appearance: &Appearance) -> Box<dyn Element> {
    // 7e: simple +/- tinted lines, mirroring feature 05's hunk treatment.
    // (The Open Changes panel renders against an Editor with a full diff
    // model; replicating that here would pull in the code-review editor
    // wiring. Plain spans per line keep the visual shape consistent.)
    let theme = appearance.theme();
    let plus_bg = internal_colors::fg_overlay_3(theme);
    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min);
    for line in unified_diff.lines() {
        let _is_add = line.starts_with('+') && !line.starts_with("+++");
        let _is_del = line.starts_with('-') && !line.starts_with("---");
        // The themed background highlights are kept subtle and uniform; a
        // future refinement could differentiate add/delete tints once the
        // theme exposes diff colors directly.
        let span = appearance
            .ui_builder()
            .span(line.to_owned())
            .build()
            .finish();
        col = col.with_child(span);
    }
    Container::new(col.finish())
        .with_padding_top(4.0)
        .with_padding_bottom(4.0)
        .with_padding_left(8.0)
        .with_padding_right(8.0)
        .with_background_color(plus_bg.into())
        .finish()
}

/// PRODUCT §24: per-tool one-line summary of the key input.
fn tool_input_summary(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let path = s("file_path");
            if path.is_empty() {
                "(no path)".to_owned()
            } else {
                path.to_owned()
            }
        }
        "Bash" => {
            let cmd = s("command");
            let desc = s("description");
            if !desc.is_empty() {
                format!("{cmd}  — {desc}")
            } else {
                cmd.to_owned()
            }
        }
        "BashOutput" | "KillShell" => {
            let id = s("shell_id");
            format!("shell {id}")
        }
        "Grep" => {
            let pattern = s("pattern");
            let path = s("path");
            if path.is_empty() {
                format!("/{pattern}/")
            } else {
                format!("/{pattern}/ in {path}")
            }
        }
        "Glob" => {
            let pattern = s("pattern");
            let path = s("path");
            if path.is_empty() {
                pattern.to_owned()
            } else {
                format!("{pattern} in {path}")
            }
        }
        "WebFetch" => s("url").to_owned(),
        "WebSearch" => s("query").to_owned(),
        "Task" => s("description").to_owned(),
        "TodoWrite" => "(see To-do list)".to_owned(),
        "ExitPlanMode" => "exit plan".to_owned(),
        other if other.starts_with("mcp__") => format!("(MCP) {}", short_value(input)),
        _ => short_value(input),
    }
}

fn short_value(v: &Value) -> String {
    let mut s = serde_json::to_string(v).unwrap_or_default();
    const MAX: usize = 80;
    if s.len() > MAX {
        s.truncate(MAX);
        s.push('…');
    }
    s
}

fn line_count(s: &str) -> usize {
    s.lines().count()
}

/// PRODUCT §30–§33: synthesize a unified diff for file-mutating tools so 7e's
/// diff cards render the change visually.
fn diff_for_tool(name: &str, input: &Value) -> Option<String> {
    let path = input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let label_old = if path.is_empty() {
        "before".to_owned()
    } else {
        format!("a/{path}")
    };
    let label_new = if path.is_empty() {
        "after".to_owned()
    } else {
        format!("b/{path}")
    };
    match name {
        "Edit" => {
            let old = input.get("old_string").and_then(|v| v.as_str())?;
            let new = input.get("new_string").and_then(|v| v.as_str())?;
            Some(unified_diff(old, new, &label_old, &label_new))
        }
        "MultiEdit" => {
            let edits = input.get("edits").and_then(|v| v.as_array())?;
            let mut parts = Vec::new();
            for (i, edit) in edits.iter().enumerate() {
                let old = edit
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new = edit
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let label_old_i = format!("{label_old} (edit {})", i + 1);
                let label_new_i = format!("{label_new} (edit {})", i + 1);
                parts.push(unified_diff(old, new, &label_old_i, &label_new_i));
            }
            Some(parts.join("\n"))
        }
        "Write" => {
            let content = input.get("content").and_then(|v| v.as_str())?;
            Some(unified_diff("", content, &label_old, &label_new))
        }
        _ => None,
    }
}

fn unified_diff(old: &str, new: &str, label_old: &str, label_new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(label_old, label_new)
        .missing_newline_hint(false)
        .to_string()
}
