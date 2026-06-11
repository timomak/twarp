//! The Claude Code **main-content pane** view (roadmap feature 07, sub-phases
//! **7b–7c**).
//!
//! This is the [`BackingView`] hosted by [`ClaudeCodePane`](crate::pane_group::pane::claude_code_pane::ClaudeCodePane):
//! a first-class pane (like an editor or terminal tab) opened by typing
//! `claude` in a terminal. It hosts the *rendering layer* of Warp's Agent Mode
//! — resurrected by **porting** the deleted `ai_assistant` transcript renderer
//! (`render_message` + the markdown-segment splitter
//! `translate_formatted_text_into_markdown_segments` from commit `fea2f7ea`)
//! and reparenting it onto the thin, twarp-native [`claude_code::Transcript`]
//! model. **7c** drives the live [`claude_code::driver`]: `spawn_session` starts
//! the `claude` process, its event stream is pumped onto the [`Transcript`] on
//! the main thread, and user turns are written to its stdin (PRODUCT §6–§14).
//! Stop SIGINTs the turn; dropping the view kills the process.
//!
//! History: PR #69 landed this exact renderer inside a **left sidebar**, which
//! the owner rejected on sight. The re-spec (#70) moved it into this pane and
//! repurposed the sidebar to a read-only session list (7h). The *rendering*
//! carried over unchanged; only the host surface and entry point changed.
//!
//! Why a port and not a rebuild: PR #67 rebuilt the panel from `Flex`/`Container`/
//! `Link` primitives and was abandoned for it (TECH.md §Postmortem). Every
//! card/diff/thinking block must trace to a ported leaf or a reused master
//! renderer. 7b brings back the **markdown transcript** leaf:
//! [`render_markdown_body`] is the port of `render_message`'s markdown body
//! (`FormattedTextElement` for prose, a bordered monospace box for fenced code),
//! fed by [`split_markdown_segments`] (the port of the AI-agnostic splitter).
//! The richer tool/diff/thinking/todo cards land in 7d–7f.
//!
//! Dispatch follows [`GlobalSearchView`](crate::workspace::view::global_search):
//! the view is its own [`TypedActionView`], in-pane clicks dispatch
//! [`ClaudeCodeViewAction`] after an `on_left_mouse_down` focus-grab puts the
//! view in the responder chain. There is **no** `WorkspaceAction` forwarder —
//! that was the #67 symptom-fix and is deleted.

mod diff_cards;
mod inline_action;
mod thinking;
mod todos;
mod tool_cards;

use std::collections::HashMap;
use std::path::PathBuf;

use async_channel::Sender;
use claude_code::diff::diff_for_tool;
use claude_code::driver::{
    interrupt, send_user_message, spawn_session, Child, PermissionMode, SpawnOptions,
    SpawnedSession,
};
use claude_code::launch::LaunchOptions;
use claude_code::{sessions, Transcript, TranscriptEvent, TranscriptItem, Usage};
use futures::StreamExt;
use markdown_parser::{parse_markdown, FormattedText, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warpui::ui_components::button::ButtonVariant;
use warpui::{
    elements::{
        Align, Border, ChildAnchor, Clipped, ClippedScrollStateHandle, ClippedScrollable,
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
        DropShadow, Element, EventDispatchMode, EventHandler, Fill, Flex, FormattedTextElement,
        HighlightedHyperlink, Hoverable, HyperlinkUrl, Icon, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, OffsetPositioning, Padding, ParentAnchor, ParentElement,
        ParentOffsetBounds, Radius, ScrollbarWidth, Shrinkable, Stack,
    },
    platform::Cursor,
    presenter::ChildView,
    text_layout::ClipConfig,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use self::diff_cards::DiffCard;
use self::thinking::ThinkingUi;
use self::tool_cards::{render_tool_card, ToolCardUi};
use crate::appearance::Appearance;
use crate::editor::{EditorOptions, EditorView, Event as EditorEvent, TextOptions};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent, PaneHeaderAction,
};
use crate::util::path::resolve_executable;

/// The executable the pane drives. Resolved on `PATH`; its absence is the
/// unavailable state (PRODUCT §4).
const CLAUDE_BINARY: &str = "claude";

/// Pane title (PRODUCT §5) — drives the tab label via [`PaneConfiguration`].
const PANE_TITLE: &str = "Claude Code";

/// Avatar glyphs for the message rows (the Agent-Mode shape: icon + body).
const USER_ICON_SVG_PATH: &str = "bundled/svg/user.svg";
const ASSISTANT_ICON_SVG_PATH: &str = "bundled/svg/claude.svg";

/// Body / code font sizes. A point past the deleted `ai_assistant::transcript`
/// renderer for the airier, Claude-app reading rhythm (shell-polish pass —
/// PRODUCT §32 visual gate).
const BODY_FONT_SIZE: f32 = 14.;
const CODE_FONT_SIZE: f32 = 12.5;
const TRANSCRIPT_LEFT_MARGIN: f32 = 15.;

/// Shell-polish layout constants (the Claude-app frame): a floating rounded
/// composer, muted context pills, and a zero-state heading. (Per owner
/// feedback on the 7d review: the chat fills the pane, and the composer
/// floats above it at the bottom-center instead of stacking below.)
const COMPOSER_MAX_HEIGHT: f32 = 184.;
/// The floating composer's width cap — it stays a centered card even in a
/// wide pane (the chat behind it is full-width).
const COMPOSER_MAX_WIDTH: f32 = 760.;
/// Bottom padding inside the transcript scroller so the last message can
/// scroll clear of the floating composer.
const COMPOSER_CLEARANCE: f32 = 140.;
const COMPOSER_CORNER_RADIUS: f32 = 14.;
const MESSAGE_CORNER_RADIUS: f32 = 12.;
const PILL_CORNER_RADIUS: f32 = 6.;
const HEADING_FONT_SIZE: f32 = 20.;

/// Events the pane view emits to its host [`ClaudeCodePane`]. 7b only needs
/// `Close` (so the pane-header close button works); 7c adds session-lifecycle
/// events as the live driver lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCodeViewEvent {
    Pane(PaneEvent),
    /// twarp 07 (7i, PRODUCT §39): swap this pane for the raw interactive
    /// `claude` CLI resuming `session_id` — handled by the pane group as a
    /// temporary pane replacement.
    SwapToRawCli {
        session_id: String,
        cwd: Option<PathBuf>,
    },
}

/// Actions the view handles, dispatched the [`GlobalSearchView`] way (the view
/// is itself the [`TypedActionView`], no `WorkspaceAction` forwarder). 7b keeps
/// this minimal; lifecycle / permissions / resume actions arrive in 7c–7h.
#[derive(Clone, Debug)]
pub enum ClaudeCodeViewAction {
    /// Submit the input buffer as a user turn — spawns the `claude` process on
    /// the first message, then writes each turn to its stdin (PRODUCT §6, §16).
    Submit,
    /// Focus the message input — the `on_left_mouse_down` focus-grab that keeps
    /// the view in the responder chain so in-pane dispatches land (TECH §The
    /// pane; the fix #67 routed around with a forwarder).
    FocusInput,
    /// Open a URL clicked inside assistant markdown (PRODUCT §13 links).
    OpenUrl(HyperlinkUrl),
    /// Re-check `claude` availability — the unavailable state's "Check again"
    /// (PRODUCT §4).
    Refresh,
    /// Interrupt the in-flight turn (SIGINT) without ending the session
    /// (PRODUCT §11). Shown in the composer while streaming.
    Stop,
    /// Expand / collapse the tool card with this tool-use id (PRODUCT §19).
    ToggleToolCard(String),
    /// Expand / collapse the thinking card at this transcript index
    /// (PRODUCT §22).
    ToggleThinking(usize),
    /// Expand / collapse the task list (PRODUCT §23).
    ToggleTodos,
    /// Step the permission mode to the next value (PRODUCT §25). Applies to
    /// the next spawn; a live session is re-attached via `--resume` so the
    /// conversation continues under the new mode.
    CyclePermissionMode,
}

/// Actions dispatched by elements this view renders inside its **pane
/// header** (PRODUCT §39's Raw CLI toggle). Header chrome lives in the
/// parent [`PaneView`]'s tree, so an in-pane [`ClaudeCodeViewAction`] dispatch
/// from there dies unhandled ("dispatched, but no view handled it") — header
/// buttons must route through [`PaneHeaderAction::CustomAction`], which the
/// pane framework forwards back to [`BackingView::handle_custom_action`].
/// Same pattern as `NetworkLogViewCustomAction::Refresh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCodeCustomAction {
    /// Swap this pane for the raw interactive CLI (PRODUCT §39, 7i).
    /// Hidden while a turn streams (§42).
    ToggleRawCli,
}

/// A stored session to reopen (PRODUCT §36, sub-phase 7h): the pane renders
/// the on-disk history up front and continues the conversation live via
/// `claude --resume <session_id>` on the next message.
#[derive(Clone, Debug)]
pub struct ResumeSession {
    pub session_id: String,
    pub jsonl_path: PathBuf,
}

pub struct ClaudeCodeView {
    /// The conversation the pane renders, fed by the live driver's event stream
    /// via [`Self::on_transcript_event`] on the main thread (PRODUCT §9–§13).
    transcript: Transcript,
    /// The message input. Enter submits; Shift+Enter inserts a newline.
    input_editor: ViewHandle<EditorView>,
    /// Pane chrome state (tab title). Owned here, handed to [`PaneView`] by the
    /// [`ClaudeCodePane`] wrapper.
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// Tracks whether the hosting pane is focused (set by the pane framework via
    /// [`BackingView::set_focus_handle`]).
    focus_handle: Option<PaneFocusHandle>,
    /// The working directory of the terminal that opened the pane (PRODUCT §4),
    /// shown in the header. 7c spawns `claude` here.
    cwd: Option<PathBuf>,
    scroll_state: ClippedScrollStateHandle,
    /// The live `claude` child, once a session is running. Kept for `interrupt`
    /// (Stop, PRODUCT §11) and to kill the process on drop (PRODUCT §8 —
    /// `spawn_session` sets `kill_on_drop`).
    child: Option<Child>,
    /// Sends user turns to the background task that owns the process stdin
    /// (PRODUCT §16). `None` until a session is running.
    message_tx: Option<Sender<String>>,
    /// True while `claude` is producing output for the current turn (PRODUCT §9):
    /// the composer shows Stop and sending is disabled until the turn ends.
    streaming: bool,
    /// Stable mouse-state handles kept across renders so a click's
    /// mousedown/mouseup hit the same handle.
    submit_button: MouseStateHandle,
    refresh_button: MouseStateHandle,
    stop_button: MouseStateHandle,
    /// Per-tool-card UI state (stable mouse handle + the user's expand/collapse
    /// choice), keyed by tool-use id. An entry is created when the card's
    /// `ToolCall` event arrives (PRODUCT §16, §19).
    tool_card_ui: HashMap<String, ToolCardUi>,
    /// The feature-05 diff views backing `Edit`/`MultiEdit`/`Write` cards
    /// (PRODUCT §20), keyed by tool-use id. Built when the `ToolCall` event
    /// arrives; render-time only reads.
    diff_cards: HashMap<String, DiffCard>,
    /// Per-thinking-card UI state (PRODUCT §22), keyed by the item's
    /// transcript index — items only ever append, so the index is stable.
    thinking_ui: HashMap<usize, ThinkingUi>,
    /// The task list starts open — it is the content (PRODUCT §23).
    todos_expanded: bool,
    todos_header_mouse: MouseStateHandle,
    /// The selected permission mode (PRODUCT §25). Source of truth for the
    /// composer pill and for every (re)spawn's `--permission-mode`. Seeded
    /// from the launch flags (`--permission-mode` /
    /// `--dangerously-skip-permissions`, e.g. via a user's alias).
    permission_mode: PermissionMode,
    permission_pill_mouse: MouseStateHandle,
    /// `--model` from the launch flags, passed to every spawn.
    model: Option<String>,
    /// `--effort` from the launch flags (e.g. an alias's `--effort max`),
    /// passed to every spawn. Write-only: the headless stream doesn't echo
    /// an effort level back, so there is no chip for it.
    effort: Option<String>,
    /// Resume target for the next spawn (PRODUCT §36; also how a permission-
    /// mode change re-attaches a live conversation, §25). Cleared if a resumed
    /// spawn dies so the next message can start fresh (§37).
    resume_session_id: Option<String>,
    /// The pane's session identity, owned from birth (PRODUCT §41): a fresh
    /// pane generates a UUID and pins it via `--session-id`; a resumed pane
    /// adopts the resumed id. Synced from `init` thereafter. The raw-CLI
    /// toggle and the on-disk history refresh key off this.
    session_id: String,
    /// Mouse state for the header's Raw CLI toggle (PRODUCT §39).
    raw_cli_button: MouseStateHandle,
    /// Monotonic session generation. Spawn callbacks and stream pumps carry
    /// the epoch they were started under and are ignored when stale — a
    /// permission-mode restart must not let the old (killed) session's EOF
    /// tear down the new one's handles or spam the transcript.
    session_epoch: u64,
}

impl ClaudeCodeView {
    /// Build the pane view.
    ///
    /// `launch` is the parsed `claude [flags] [prompt]` invocation (PRODUCT
    /// §2): recognized flags — including alias-injected ones like
    /// `--dangerously-skip-permissions` / `--effort max` — seed the session's
    /// spawn options, and the positional seeds the first user turn. `cwd` is
    /// the terminal's working directory (PRODUCT §4). `resume` reopens a
    /// stored session (PRODUCT §36): its on-disk history renders immediately
    /// and the next message continues it live via `claude --resume` (a
    /// `claude --resume <id>` typed in a terminal arrives through `launch`
    /// and gets the same treatment).
    pub fn new(
        launch: LaunchOptions,
        cwd: Option<PathBuf>,
        resume: Option<ResumeSession>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let LaunchOptions {
            prompt,
            permission_mode,
            model,
            effort,
            resume_session_id,
        } = launch;

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
        input_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Message Claude Code…", ctx);
        });

        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(PANE_TITLE));

        // `claude --resume <id>` typed at the prompt: derive the session's
        // on-disk file so the pane pre-renders its history exactly like the
        // sidebar's resume (PRODUCT §36). `load_history` is best-effort, so a
        // wrong derivation just renders nothing and `claude --resume` itself
        // remains the source of truth.
        let resume = resume.or_else(|| {
            let session_id = resume_session_id?;
            let dir = cwd
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .and_then(|cwd| sessions::sessions_dir(&cwd))?;
            Some(ResumeSession {
                jsonl_path: dir.join(format!("{session_id}.jsonl")),
                session_id,
            })
        });

        // PRODUCT §41: every pane-born session has its identity from birth —
        // a resumed pane adopts the resumed id, a fresh pane mints one and
        // pins it at spawn via `--session-id`. The raw-CLI toggle, the
        // permission-mode restart, and the history refresh all key off it.
        let session_id = resume
            .as_ref()
            .map(|resume| resume.session_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut view = Self {
            transcript: Transcript::new(),
            input_editor,
            pane_configuration,
            focus_handle: None,
            cwd,
            scroll_state: ClippedScrollStateHandle::default(),
            child: None,
            message_tx: None,
            streaming: false,
            submit_button: MouseStateHandle::default(),
            refresh_button: MouseStateHandle::default(),
            stop_button: MouseStateHandle::default(),
            tool_card_ui: HashMap::new(),
            diff_cards: HashMap::new(),
            thinking_ui: HashMap::new(),
            todos_expanded: true,
            todos_header_mouse: MouseStateHandle::default(),
            permission_mode: permission_mode.unwrap_or(PermissionMode::Default),
            permission_pill_mouse: MouseStateHandle::default(),
            model,
            effort,
            resume_session_id: None,
            session_id,
            raw_cli_button: MouseStateHandle::default(),
            session_epoch: 0,
        };

        // PRODUCT §36: a resumed pane renders the stored history up front —
        // through the same ingest path as live events so tool/diff/thinking
        // card state exists — and continues live on the next message.
        if let Some(resume) = resume {
            for event in sessions::load_history(&resume.jsonl_path) {
                view.ingest_event(event, ctx);
            }
            view.resume_session_id = Some(resume.session_id);
        }

        // PRODUCT §2/§6: `claude <prompt>` starts a live session immediately
        // with the prompt as the first turn; bare `claude` opens idle (no
        // process until the user sends a message).
        let first_prompt = prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        if let Some(prompt) = first_prompt {
            view.transcript
                .apply(TranscriptEvent::UserMessage(prompt.clone()));
            view.streaming = true;
            view.begin_session(Some(prompt), ctx);
        }

        view
    }

    /// The pane configuration (tab title) handed to [`PaneView`] by the wrapper.
    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// The working directory of the terminal that opened the pane (PRODUCT §4).
    /// Exposed so the pane group can treat the pane like a terminal session for
    /// directory context: new splits/tabs inherit it, and it roots the Open
    /// Changes / file tree panels (owner feedback on the 7d review).
    pub fn cwd(&self) -> Option<&PathBuf> {
        self.cwd.as_ref()
    }

    /// Focus the message input (PRODUCT §34: keyboard-first).
    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.input_editor);
    }

    /// Whether the `claude` CLI is resolvable on `PATH` right now (PRODUCT §4).
    fn claude_available() -> bool {
        resolve_executable(CLAUDE_BINARY).is_some()
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // PRODUCT §15: Enter sends; Shift+Enter is handled by the editor itself
        // (inserts a newline) and does not reach us.
        if matches!(event, EditorEvent::Enter) {
            self.submit(ctx);
        }
    }

    /// Send the current input as a user turn to the live `claude` session,
    /// spawning the session on the first message if none is running yet
    /// (PRODUCT §6, §16). A no-op while a turn is streaming (PRODUCT §9).
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            // PRODUCT §9: sending is disabled until the current turn completes.
            return;
        }
        let text = self
            .input_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        if text.is_empty() {
            // PRODUCT §15: empty / whitespace-only messages are a no-op.
            return;
        }
        self.transcript
            .apply(TranscriptEvent::UserMessage(text.clone()));
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.streaming = true;
        match &self.message_tx {
            // Session already running — write the turn to its stdin.
            Some(tx) => {
                let _ = tx.try_send(text);
            }
            // First message: spawn the session, forwarding this as turn one.
            None => self.begin_session(Some(text), ctx),
        }
        ctx.notify();
    }

    /// Spawn a live `claude` session (PRODUCT §6) on the background executor —
    /// `spawn_session` itself spawns the child (tokio), so it must not run on
    /// the foreground. [`Self::on_session_spawned`] wires the result into the
    /// view on the main thread and sends `first_prompt` once stdin is up.
    ///
    /// The spawn carries the selected permission mode (PRODUCT §25) and, when
    /// set, the resume target (PRODUCT §36) — both read off `self` at the
    /// moment of spawn.
    fn begin_session(&mut self, first_prompt: Option<String>, ctx: &mut ViewContext<Self>) {
        self.session_epoch += 1;
        let epoch = self.session_epoch;
        let opts = SpawnOptions {
            cwd: self
                .cwd
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
            model: self.model.clone(),
            effort: self.effort.clone(),
            resume_session_id: self.resume_session_id.clone(),
            // A fresh session spawns under the pane's own id (PRODUCT §41);
            // a resume continues the id it targets.
            session_id: self
                .resume_session_id
                .is_none()
                .then(|| self.session_id.clone()),
            permission_mode: self.permission_mode,
            allowed_tools: Vec::new(),
        };
        ctx.spawn(
            async move { spawn_session(opts) },
            move |view, result, ctx| view.on_session_spawned(epoch, result, first_prompt, ctx),
        );
    }

    /// Main-thread wiring once `spawn_session` resolves. Starts two background
    /// tasks — one draining the driver's (tokio) event stream into the
    /// transcript via an `async_channel` + [`ViewContext::spawn_stream_local`]
    /// (keeping tokio I/O off the foreground), one owning stdin and writing
    /// queued user turns — and keeps `child` for Stop / kill-on-drop.
    ///
    /// Everything is guarded by `epoch`: if the view moved on to a newer
    /// session (permission-mode restart, §25) the stale spawn is dropped on
    /// the floor — dropping `child` kills it — and its stream never touches
    /// the transcript.
    fn on_session_spawned(
        &mut self,
        epoch: u64,
        result: anyhow::Result<SpawnedSession>,
        first_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if epoch != self.session_epoch {
            return;
        }
        let SpawnedSession {
            child,
            stdin,
            mut events,
        } = match result {
            Ok(session) => session,
            Err(err) => {
                // PRODUCT §28/§30: surface the spawn failure verbatim.
                self.streaming = false;
                // A failed resume must not wedge the pane on the dead id —
                // the next message starts fresh (PRODUCT §37).
                self.resume_session_id = None;
                self.transcript.apply(TranscriptEvent::Ended {
                    reason: claude_code::EndReason::Error(format!(
                        "Couldn't start `claude`: {err}"
                    )),
                });
                ctx.notify();
                return;
            }
        };

        // Drain the (tokio) event stream on the background executor; forward each
        // event over a runtime-agnostic channel that the foreground pump applies.
        let (event_tx, event_rx) = async_channel::unbounded::<TranscriptEvent>();
        ctx.background_executor()
            .spawn(async move {
                while let Some(event) = events.next().await {
                    if event_tx.send(event).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
        let _ = ctx.spawn_stream_local(
            event_rx,
            move |view: &mut Self, event, ctx| view.on_transcript_event(epoch, event, ctx),
            move |view: &mut Self, ctx| view.on_stream_done(epoch, ctx),
        );

        // Own stdin in a background task; the view queues user turns onto it.
        let (message_tx, message_rx) = async_channel::unbounded::<String>();
        ctx.background_executor()
            .spawn(async move {
                let mut stdin = stdin;
                while let Ok(message) = message_rx.recv().await {
                    if send_user_message(&mut stdin, &message).await.is_err() {
                        break;
                    }
                }
            })
            .detach();

        self.child = Some(child);
        self.message_tx = Some(message_tx);

        // Send the first turn now that stdin is wired (PRODUCT §6).
        if let (Some(prompt), Some(tx)) = (first_prompt, &self.message_tx) {
            let _ = tx.try_send(prompt);
        }
        ctx.notify();
    }

    /// Apply one driver event on the main thread (PRODUCT §9–§13), dropping
    /// events from a superseded session. An `Ended` event closes the
    /// streaming turn.
    fn on_transcript_event(
        &mut self,
        epoch: u64,
        event: TranscriptEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if epoch != self.session_epoch {
            return;
        }
        if matches!(event, TranscriptEvent::Ended { .. }) {
            self.streaming = false;
        }
        if let TranscriptEvent::Ended {
            reason: claude_code::EndReason::Error(_) | claude_code::EndReason::Exited,
        } = &event
        {
            // The process died (a failed `--resume` lands here too, with its
            // stderr surfaced verbatim): clear the resume target so the next
            // message starts fresh instead of re-failing (PRODUCT §37).
            self.resume_session_id = None;
        }
        self.ingest_event(event, ctx);
        ctx.notify();
    }

    /// Apply one event to the transcript and keep the per-card UI state in
    /// step. Shared by the live pump and by 7h history replay — both need the
    /// card mouse handles and diff views to exist before the first render.
    fn ingest_event(&mut self, event: TranscriptEvent, ctx: &mut ViewContext<Self>) {
        match &event {
            TranscriptEvent::SessionInit { session_id, .. } => {
                // Stay in lockstep with what `claude` actually reports —
                // normally the id the pane pinned via `--session-id` or
                // resumed (PRODUCT §41).
                self.session_id = session_id.clone();
            }
            TranscriptEvent::ToolCall {
                id, name, input, ..
            } => {
                // The card's stable mouse handle must exist before the first
                // render so expand/collapse clicks pair across renders.
                self.tool_card_ui.entry(id.clone()).or_default();
                // 7e: an edit tool renders as a feature-05 diff card — build
                // its views now (views cannot be created at render time).
                if !self.diff_cards.contains_key(id) {
                    if let Some(tool_diff) = diff_for_tool(name, input) {
                        let card = diff_cards::build_diff_card(tool_diff, ctx);
                        self.diff_cards.insert(id.clone(), card);
                    }
                }
            }
            TranscriptEvent::Thinking { .. } => {
                // The new item lands at the end of the transcript; key its UI
                // state by that index (items only append, §22).
                self.thinking_ui
                    .entry(self.transcript.items().len())
                    .or_default();
            }
            _ => {}
        }
        self.transcript.apply(event);
    }

    /// The event stream closed — `claude`'s stdout reached EOF, so the process
    /// is gone (PRODUCT §28). The driver already pushed an `Ended` notice; tear
    /// down the live-session handles so the composer returns to a fresh state.
    /// Stale streams (superseded sessions) are ignored.
    fn on_stream_done(&mut self, epoch: u64, ctx: &mut ViewContext<Self>) {
        if epoch != self.session_epoch {
            return;
        }
        self.streaming = false;
        self.child = None;
        self.message_tx = None;
        ctx.notify();
    }

    /// Step to the next permission mode (PRODUCT §25): Ask → Accept edits →
    /// Plan → Bypass → Ask. The mode applies to the next spawn; a live
    /// session is detached (killed) and its id kept as the resume target, so
    /// the next message continues the same conversation under the new mode —
    /// the only mode-change channel `claude`'s documented flags offer.
    fn cycle_permission_mode(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            // §25 applies between turns; restarting mid-turn would kill the
            // in-flight work.
            return;
        }
        self.permission_mode = match self.permission_mode {
            PermissionMode::Default => PermissionMode::AcceptEdits,
            PermissionMode::AcceptEdits => PermissionMode::Plan,
            PermissionMode::Plan => PermissionMode::BypassPermissions,
            PermissionMode::BypassPermissions => PermissionMode::Default,
        };
        if self.child.is_some() {
            // Detach the live process; the next message resumes the
            // conversation under the new mode. The pane owns its session id
            // from birth (§41), so there is no "id not announced yet" window.
            self.detach_live_session();
        }
        ctx.notify();
    }

    /// Kill the live `claude` process (if any) and keep the conversation as
    /// the resume target for the next spawn. The epoch bump makes the killed
    /// session's EOF/events stale so they can't spam the transcript or wipe
    /// the next session's handles.
    fn detach_live_session(&mut self) {
        self.resume_session_id = Some(self.session_id.clone());
        self.session_epoch += 1;
        self.child = None; // kill_on_drop
        self.message_tx = None;
        self.streaming = false;
    }

    /// twarp 07 (7i, PRODUCT §39/§42): hand this conversation to the raw
    /// interactive CLI. Disabled while a turn streams (§42 — same rule as the
    /// mode selector); otherwise the headless process is detached first (one
    /// driver at a time) and the host pane swaps to a terminal running
    /// `claude --resume <session_id>`.
    fn toggle_raw_cli(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            return;
        }
        self.detach_live_session();
        ctx.emit(ClaudeCodeViewEvent::SwapToRawCli {
            session_id: self.session_id.clone(),
            cwd: self.cwd.clone(),
        });
        ctx.notify();
    }

    /// twarp 07 (7i, PRODUCT §40): returning from raw-CLI mode — re-read the
    /// session's on-disk history so turns produced in the raw CLI appear in
    /// the transcript, and keep the conversation as the resume target so the
    /// next message continues it live. Best-effort like every store read: a
    /// missing file just renders what the pane already had.
    pub fn refresh_from_disk(&mut self, ctx: &mut ViewContext<Self>) {
        let dir = self
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .and_then(|cwd| sessions::sessions_dir(&cwd));
        let Some(dir) = dir else {
            return;
        };
        let jsonl_path = dir.join(format!("{}.jsonl", self.session_id));
        let history = sessions::load_history(&jsonl_path);
        if history.is_empty() {
            // Nothing (re)readable on disk — keep the rendered transcript
            // rather than blanking it (PRODUCT §29 defensive).
            return;
        }
        self.transcript.clear();
        self.tool_card_ui.clear();
        self.diff_cards.clear();
        self.thinking_ui.clear();
        for event in history {
            self.ingest_event(event, ctx);
        }
        self.resume_session_id = Some(self.session_id.clone());
        self.streaming = false;
        ctx.notify();
    }

    /// Expand / collapse the thinking card at `index` (PRODUCT §22).
    fn toggle_thinking(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let ui = self.thinking_ui.entry(index).or_default();
        ui.expanded = !ui.expanded;
        ctx.notify();
    }

    /// Stop the current turn (PRODUCT §11): SIGINT the live process. The session
    /// stays alive; `claude` emits `Ended { Interrupted }`, which clears the
    /// streaming state via [`Self::on_transcript_event`].
    fn stop(&mut self, _ctx: &mut ViewContext<Self>) {
        if let Some(child) = &self.child {
            interrupt(child);
        }
    }

    /// Flip a tool card between collapsed and expanded (PRODUCT §19). The
    /// effective state before the click is the user's prior choice, or the
    /// status-derived default (failed cards open showing their error; diff
    /// cards open showing their diff, §20).
    fn toggle_tool_card(&mut self, id: &str, ctx: &mut ViewContext<Self>) {
        let Some(TranscriptItem::Tool {
            status, children, ..
        }) = self.transcript.find_tool(id)
        else {
            return;
        };
        let default = if self.diff_cards.contains_key(id) {
            diff_cards::default_expanded()
        } else {
            tool_cards::default_expanded(*status, !children.is_empty())
        };
        if let Some(ui) = self.tool_card_ui.get_mut(id) {
            let effective = ui.expanded_override.unwrap_or(default);
            ui.expanded_override = Some(!effective);
            ctx.notify();
        }
    }

    fn render_body(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        if self.transcript.is_empty() {
            // Zero state when no session has produced anything.
            return render_zero_state(appearance);
        }
        self.render_transcript(app)
    }

    /// Render the transcript into a [`UniformList`] (PRODUCT §14) wrapped in a
    /// vertical [`Scrollable`], mirroring [`GlobalSearchView::render_results`].
    /// Bottom-stick auto-scroll (PRODUCT §14) layers on in 7c when content
    /// actually streams; 7b's synthetic transcript is static.
    fn render_transcript(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // A chat transcript has variable-height items (a one-line user turn vs a
        // multi-paragraph assistant reply), so a `UniformList` — which clips
        // every row to a single measured height — truncated multi-line replies
        // to one line. Render each item at its natural height in a column;
        // a variable-height virtualized list can return if very large sessions
        // need it (PRODUCT §14), but correctness comes first.
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);
        for (index, item) in self.transcript.items().iter().enumerate() {
            column.add_child(self.render_item(index, item, app));
        }

        // The composer floats over the bottom of the pane; this clearance is
        // inside the scroll content so the newest message can scroll out from
        // underneath it (PRODUCT §15).
        let content = Container::new(column.finish())
            .with_padding_bottom(COMPOSER_CLEARANCE)
            .finish();

        ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish()
    }

    /// The composer's context chips, built from the live session metadata the
    /// driver parses out of `claude`'s stream-json: a Local indicator (the pane
    /// always drives the local CLI — there is no remote session to report), the
    /// model, the context-window usage, the permission mode, and fast-mode when
    /// on. Effort is intentionally absent: the headless stream-json doesn't
    /// report an effort level (only `fast_mode_state`), unlike the interactive
    /// TUI's status line.
    ///
    /// The permission pill is the §25 selector: it shows the view's selected
    /// mode (the value every spawn passes via `--permission-mode`) and a click
    /// steps to the next mode. Disabled while a turn streams (§25 applies
    /// between turns).
    fn metadata_chips(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let mut chips = vec![render_pill("Local", appearance)];
        if let Some(model) = self.transcript.model() {
            chips.push(render_pill(&prettify_model(model), appearance));
        }
        if let Some(label) = self.transcript.usage().and_then(format_context) {
            chips.push(render_pill(&label, appearance));
        }
        let mode_label = prettify_permission_mode(self.permission_mode.as_cli_arg());
        if self.streaming {
            chips.push(render_pill(&mode_label, appearance));
        } else {
            chips.push(render_clickable_pill(
                &mode_label,
                self.permission_pill_mouse.clone(),
                |ctx| ctx.dispatch_typed_action(ClaudeCodeViewAction::CyclePermissionMode),
                appearance,
            ));
        }
        if self.transcript.fast_mode() == Some("on") {
            chips.push(render_pill("Fast mode", appearance));
        }
        chips
    }

    /// The docked composer (PRODUCT §15): a rounded, bordered card holding the
    /// message input above a controls row — muted context pills on the left, the
    /// Send / Stop action on the right — pinned to the bottom of the reading
    /// column, Claude-app style. Mirrors `GlobalSearchView`'s bordered query box.
    fn render_input(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();

        // The growable message input, capped so a long draft scrolls inside the
        // composer instead of shoving the transcript off-screen (PRODUCT §15).
        // Size to the editor's own (autogrowing) height — NOT `Shrinkable`: a flex
        // child in the `MainAxisSize::Min` card column below gets an unbounded
        // main-axis constraint (the card is measured for its natural height) and
        // panics the flex. `CrossAxisAlignment::Stretch` on the card gives it the
        // full width instead.
        let editor =
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.input_editor).finish()).finish())
                .with_max_height(COMPOSER_MAX_HEIGHT)
                .finish();

        // PRODUCT §9–§11: the controls row carries the session context chips
        // (model / context usage / permission mode); while a turn streams it also
        // shows a "Working…" cue, and the action becomes Stop (SIGINT).
        let left = {
            let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            if self.streaming {
                row.add_child(
                    Container::new(
                        appearance
                            .ui_builder()
                            .span("Working…".to_owned())
                            .with_style(UiComponentStyles {
                                font_color: Some(muted),
                                font_size: Some(12.),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                );
            }
            for chip in self.metadata_chips(appearance) {
                row.add_child(chip);
            }
            row.finish()
        };

        let action: Box<dyn Element> = if self.streaming {
            appearance
                .ui_builder()
                .button(ButtonVariant::Outlined, self.stop_button.clone())
                .with_text_label("Stop".to_owned())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::Stop);
                })
                .finish()
        } else {
            let label = if self.transcript.is_empty() {
                "Start session"
            } else {
                "Send"
            };
            appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.submit_button.clone())
                .with_text_label(label.to_owned())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::Submit);
                })
                .finish()
        };

        let controls = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(left)
            .with_child(action)
            .finish();

        let card = Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(8.)
                .with_child(editor)
                .with_child(controls)
                .finish(),
        )
        .with_padding(Padding::uniform(10.))
        .with_background_color(theme.surface_1().into_solid())
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            COMPOSER_CORNER_RADIUS,
        )))
        // The composer floats over the transcript; the shadow separates the
        // layers (same treatment as the input-suggestions detail panel).
        .with_drop_shadow(DropShadow::default())
        .finish();

        Container::new(card)
            .with_padding_top(6.)
            .with_padding_bottom(12.)
            .finish()
    }

    fn render_unavailable_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        // PRODUCT §4: replaces the pane body; names the missing binary, gives an
        // install hint, no input affordances.
        let title = appearance
            .ui_builder()
            .span(format!(
                "Claude Code isn't available. The `{CLAUDE_BINARY}` command wasn't found on your \
                 PATH."
            ))
            .with_soft_wrap()
            .build()
            .finish();
        let hint = appearance
            .ui_builder()
            .span(
                "Install Claude Code (https://docs.claude.com/en/docs/claude-code), make sure \
                 `claude` is on your PATH, then re-open this pane."
                    .to_owned(),
            )
            .with_soft_wrap()
            .build()
            .finish();
        let check = appearance
            .ui_builder()
            .link(
                "Check again".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::Refresh);
                })),
                self.refresh_button.clone(),
            )
            .build()
            .finish();
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(10.)
                .with_child(title)
                .with_child(hint)
                .with_child(check)
                .finish(),
        )
        .with_uniform_padding(15.)
        .finish()
    }
}

impl Entity for ClaudeCodeView {
    type Event = ClaudeCodeViewEvent;
}

impl View for ClaudeCodeView {
    fn ui_name() -> &'static str {
        "ClaudeCodeView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // PRODUCT §34: focus the input on entry so typing just works. Focusing a
        // child keeps the view itself in the responder chain, so in-pane
        // `ClaudeCodeViewAction` dispatches reach `handle_action` below.
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.input_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        // PRODUCT §4: the unavailable state replaces the pane body. The pane
        // header (title) is rendered separately by `render_header_content`.
        let contents = if Self::claude_available() {
            // Owner feedback on the 7d review: the chat fills the pane and the
            // composer FLOATS above it (z-axis) at the bottom-center, instead
            // of stacking below in a flex column. `Align` is load-bearing for
            // width: it reports the full incoming constraint, so the body
            // spans the pane even where a flex child would have been measured
            // loose and shrunk to its content (the bug behind the "chat is not
            // full width" report — the zero state rendered left-hugging at
            // content width).
            let body: Box<dyn Element> = if self.transcript.is_empty() {
                // Centered zero state over the full pane.
                Align::new(self.render_body(app)).finish()
            } else {
                Align::new(self.render_body(app)).top_left().finish()
            };
            // The floating composer: a positioned stack child anchored to the
            // pane's bottom-center, capped at a reading width, shrunk (never
            // moved) if the pane is narrower. The transcript scrolls
            // underneath; its scroller keeps COMPOSER_CLEARANCE of bottom
            // padding so the newest message can scroll clear of it.
            let composer = ConstrainedBox::new(self.render_input(appearance))
                .with_max_width(COMPOSER_MAX_WIDTH)
                .finish();
            // Waterfall so a click on the floating composer is consumed there
            // and never also reaches the transcript beneath it.
            let mut stack = Stack::new().with_event_dispatch_mode(EventDispatchMode::Waterfall);
            stack.add_child(body);
            stack.add_positioned_child(
                composer,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
            Container::new(stack.finish())
                .with_background_color(theme.background().into_solid())
                .with_padding_left(16.)
                .with_padding_right(16.)
                .with_padding_top(8.)
                .finish()
        } else {
            self.render_unavailable_state(appearance)
        };

        // The focus-grab (TECH §The pane): any click on empty pane chrome
        // focuses the input, guaranteeing the view is the dispatch target for
        // subsequent in-pane actions — what #67 worked around with a forwarder.
        EventHandler::new(contents)
            .on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::FocusInput);
                DispatchEventResult::StopPropagation
            })
            .finish()
    }
}

impl TypedActionView for ClaudeCodeView {
    type Action = ClaudeCodeViewAction;

    fn handle_action(&mut self, action: &ClaudeCodeViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            ClaudeCodeViewAction::Submit => self.submit(ctx),
            ClaudeCodeViewAction::FocusInput => ctx.focus(&self.input_editor),
            ClaudeCodeViewAction::OpenUrl(url) => ctx.open_url(&url.url),
            // PRODUCT §4: render re-checks availability, so a notify suffices.
            ClaudeCodeViewAction::Refresh => ctx.notify(),
            ClaudeCodeViewAction::Stop => self.stop(ctx),
            ClaudeCodeViewAction::ToggleToolCard(id) => self.toggle_tool_card(id, ctx),
            ClaudeCodeViewAction::ToggleThinking(index) => self.toggle_thinking(*index, ctx),
            ClaudeCodeViewAction::ToggleTodos => {
                self.todos_expanded = !self.todos_expanded;
                ctx.notify();
            }
            ClaudeCodeViewAction::CyclePermissionMode => self.cycle_permission_mode(ctx),
        }
    }
}

impl BackingView for ClaudeCodeView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ClaudeCodeCustomAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &(),
        _ctx: &mut ViewContext<Self>,
    ) {
        // No overflow menu items in 7b.
    }

    /// Header-button actions, routed back here by the pane framework
    /// (PRODUCT §39; see [`ClaudeCodeCustomAction`]).
    fn handle_custom_action(
        &mut self,
        custom_action: &Self::CustomAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match custom_action {
            ClaudeCodeCustomAction::ToggleRawCli => self.toggle_raw_cli(ctx),
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // 7c also tears down the live `claude` process here (PRODUCT §8); 7b has
        // no driver, so closing the pane just drops the synthetic transcript.
        ctx.emit(ClaudeCodeViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> HeaderContent {
        // PRODUCT §5: title "Claude Code" with the session cwd as a secondary
        // line. Net-new chrome (the Agent-block header was service-coupled;
        // TECH matrix marks it do-NOT-port). The header renders title and
        // secondary back-to-back, so the separator lives in the string.
        let cwd = self
            .cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            })
            .map(|cwd| format!(" — {cwd}"));
        // PRODUCT §39 (7i): the Raw CLI toggle — swaps the pane for the real
        // interactive `claude` resuming this conversation. Hidden while a
        // turn streams (§42); the header re-renders when streaming flips.
        // The header lives in the parent PaneView's tree, so the click must
        // dispatch the pane framework's CustomAction (handled by the header
        // view and routed back to `handle_custom_action`), NOT an in-pane
        // ClaudeCodeViewAction — that dies unhandled from header chrome.
        let raw_cli_toggle = (!self.streaming).then(|| {
            let appearance = Appearance::as_ref(app);
            appearance
                .ui_builder()
                .button(ButtonVariant::Text, self.raw_cli_button.clone())
                .with_text_label("Raw CLI".to_owned())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action::<PaneHeaderAction<(), ClaudeCodeCustomAction>>(
                        PaneHeaderAction::CustomAction(ClaudeCodeCustomAction::ToggleRawCli),
                    );
                })
                .finish()
        });
        HeaderContent::Standard(StandardHeader {
            title: PANE_TITLE.to_owned(),
            title_secondary: cwd,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: raw_cli_toggle,
            options: StandardHeaderOptions::default(),
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

impl ClaudeCodeView {
    /// Bridge dispatch (TECH §The bridge): one arm per [`TranscriptItem`].
    /// User/Assistant render through the ported markdown transcript (7b); Tool
    /// renders through the ported `inline_action` card chrome (7d, PRODUCT
    /// §16–§19) — or, for `Edit`/`MultiEdit`/`Write`, the feature-05 diff card
    /// (7e, §20–§21); Thinking/Todos render the 7f cards (§22–§23).
    fn render_item(
        &self,
        index: usize,
        item: &TranscriptItem,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        match item {
            TranscriptItem::User(text) => {
                render_message_row(true, USER_ICON_SVG_PATH, text, appearance)
            }
            TranscriptItem::Assistant { text, .. } => {
                render_message_row(false, ASSISTANT_ICON_SVG_PATH, text, appearance)
            }
            TranscriptItem::Notice(message) => render_notice(message, appearance),
            TranscriptItem::Error(message) => render_error(message, appearance),
            TranscriptItem::Tool {
                id,
                name,
                input,
                status,
                output,
                children,
            } => match self.diff_cards.get(id) {
                Some(card) => diff_cards::render_diff_card(
                    id,
                    *status,
                    output.as_ref(),
                    card,
                    &self.tool_card_ui,
                    app,
                ),
                None => render_tool_card(
                    id,
                    name,
                    input,
                    *status,
                    output.as_ref(),
                    children,
                    &self.tool_card_ui,
                    false,
                    app,
                ),
            },
            TranscriptItem::Thinking { text, duration } => thinking::render_thinking_card(
                index,
                text,
                *duration,
                self.thinking_ui.get(&index),
                app,
            ),
            TranscriptItem::Todos(items) => todos::render_todos(
                items,
                self.todos_expanded,
                self.todos_header_mouse.clone(),
                app,
            ),
            // No interactive permission wire channel exists on the pinned
            // `claude` (2.1.x dropped `--permission-prompt-tool`); the driver
            // never emits these today. If a future CLI brings them back, the
            // request surfaces as a themed notice rather than nothing
            // (PRODUCT §26 degradation: never crash, never hang).
            TranscriptItem::Permission { tool, .. } => render_notice(
                &format!(
                    "Claude requested permission to use {tool}. Pick a permission mode below and \
                     re-send to proceed."
                ),
                appearance,
            ),
        }
    }
}

/// Port of `ai_assistant::transcript::render_message`: an avatar glyph + a
/// markdown body. The shell-polish pass drops the heavy full-width alternating
/// tint for the Claude-app shape — the assistant reply flows on the canvas, the
/// user turn sits in a subtle rounded bubble — so the turns stay visually
/// distinct without striping the column (PRODUCT §12, §32).
fn render_message_row(
    is_user: bool,
    icon_svg: &'static str,
    text: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // Contrast the text against whatever surface the row actually sits on: the
    // user bubble (`surface_2`) or the bare canvas (`background`).
    let surface = if is_user {
        theme.surface_2()
    } else {
        theme.background()
    };
    let text_color = theme.main_text_color(surface).into_solid();
    let icon = ConstrainedBox::new(Icon::new(icon_svg, text_color).finish())
        .with_height(16.)
        .with_width(16.)
        .finish();

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Container::new(icon)
                .with_margin_right(12.)
                .with_margin_top(2.)
                .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Container::new(render_markdown_body(text, text_color, appearance)).finish(),
            )
            .finish(),
        );

    let mut container = Container::new(row.finish())
        .with_padding(Padding::uniform(14.))
        .with_margin_top(4.)
        .with_margin_bottom(4.);
    if is_user {
        container = container
            .with_background_color(theme.surface_2().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                MESSAGE_CORNER_RADIUS,
            )));
    }
    container.finish()
}

/// Port of `render_message`'s markdown body: render each [`MarkdownSegment`] —
/// prose via [`FormattedTextElement`] (feature 03's stack), fenced code via a
/// bordered monospace box. PRODUCT §13 (markdown), §29 (defensive: fall back to
/// plain wrapped text if the markdown does not parse).
fn render_markdown_body(
    text: &str,
    text_color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let inline_code_bg = theme.surface_3().into_solid();

    let Ok(formatted) = parse_markdown(text) else {
        return appearance
            .ui_builder()
            .wrappable_text(text.to_owned(), true)
            .with_style(UiComponentStyles {
                font_size: Some(BODY_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish();
    };

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min);
    for segment in split_markdown_segments(formatted) {
        let child = match segment {
            MarkdownSegment::Prose(formatted_text) => FormattedTextElement::new(
                formatted_text,
                BODY_FONT_SIZE,
                appearance.ui_font_family(),
                appearance.monospace_font_family(),
                text_color,
                HighlightedHyperlink::default(),
            )
            .with_inline_code_properties(
                Some(theme.nonactive_ui_text_color().into()),
                Some(inline_code_bg),
            )
            .register_default_click_handlers(|url, ctx, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenUrl(url));
            })
            .finish(),
            MarkdownSegment::Code(code) => render_code_block(&code.code, appearance),
        };
        column.add_child(child);
    }
    column.finish()
}

/// Port of `ai_assistant::transcript`'s code-block branch, minus the Warp-AI
/// affordances (paste-in-terminal / save-as-workflow / per-block selection)
/// that don't apply to a read-only Claude Code transcript: a bordered,
/// rounded, monospace box. The copy affordance is a later refinement.
fn render_code_block(code: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        appearance
            .ui_builder()
            .wrappable_text(code.to_owned(), true)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.monospace_font_family()),
                font_size: Some(CODE_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_uniform_padding(12.)
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
    .with_margin_top(10.)
    .with_margin_bottom(10.)
    .finish()
}

/// Out-of-band notice (turn interrupted / session ended) — a subtle themed row.
fn render_notice(message: &str, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        appearance
            .ui_builder()
            .span(message.to_owned())
            .with_soft_wrap()
            .build()
            .finish(),
    )
    .with_padding_top(6.)
    .with_padding_bottom(6.)
    .with_padding_left(TRANSCRIPT_LEFT_MARGIN)
    .with_padding_right(20.)
    .finish()
}

/// Error surfaced verbatim from `claude` (PRODUCT §30) on a themed surface. The
/// copy affordance lands with the live driver in 7c.
fn render_error(message: &str, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        appearance
            .ui_builder()
            .span(format!("Error: {message}"))
            .with_soft_wrap()
            .build()
            .finish(),
    )
    .with_background_color(appearance.theme().surface_2().into_solid())
    .with_padding_top(8.)
    .with_padding_bottom(8.)
    .with_padding_left(TRANSCRIPT_LEFT_MARGIN)
    .with_padding_right(20.)
    .finish()
}

/// A contiguous run of a message's markdown: prose (rendered via
/// [`FormattedTextElement`]) or a fenced code block (rendered specially).
///
/// Ported and simplified from `ai_assistant::utils::MarkdownSegment`: the
/// Claude Code transcript is read-only, so the per-code-block selection index
/// and Warp-AI action mouse handles are dropped — only the code string is kept.
enum MarkdownSegment {
    Prose(FormattedText),
    Code(markdown_parser::CodeBlockText),
}

/// Port of `ai_assistant::utils::translate_formatted_text_into_markdown_segments`
/// (AI-agnostic): split a parsed [`FormattedText`] into code-block vs contiguous
/// non-code runs so code blocks can render in their own box.
fn split_markdown_segments(formatted: FormattedText) -> Vec<MarkdownSegment> {
    let mut segments = Vec::new();
    let mut running_prose: Vec<FormattedTextLine> = Vec::new();

    for line in formatted.lines {
        match line {
            FormattedTextLine::CodeBlock(mut code) => {
                if !running_prose.is_empty() {
                    segments.push(MarkdownSegment::Prose(FormattedText::new_trimmed(
                        std::mem::take(&mut running_prose),
                    )));
                }
                code.code = code.code.trim().to_string();
                segments.push(MarkdownSegment::Code(code));
            }
            other => running_prose.push(other),
        }
    }
    if !running_prose.is_empty() {
        segments.push(MarkdownSegment::Prose(FormattedText::new_trimmed(
            running_prose,
        )));
    }
    segments
}

/// A muted, rounded context pill for the composer's controls row (the Claude-app
/// cwd / "Local" chips). Non-interactive — purely informational, so it carries no
/// mouse handlers.
fn render_pill(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        appearance
            .ui_builder()
            .span(label.to_owned())
            .with_style(UiComponentStyles {
                font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_padding_left(8.)
    .with_padding_right(8.)
    .with_padding_top(3.)
    .with_padding_bottom(3.)
    .with_margin_right(6.)
    .with_background_color(theme.surface_2().into_solid())
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
    .finish()
}

/// The §25 permission-mode selector pill: the muted pill chrome with hover +
/// pointer affordance; a click dispatches the cycle action. The label carries
/// a chevron-ish suffix so it reads as a control, not a static chip.
fn render_clickable_pill(
    label: &str,
    mouse_state: MouseStateHandle,
    on_click: impl Fn(&mut warpui::EventContext) + 'static,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label = format!("{label} ▾");
    let pill = Container::new(
        appearance
            .ui_builder()
            .span(label)
            .with_style(UiComponentStyles {
                font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_padding_left(8.)
    .with_padding_right(8.)
    .with_padding_top(3.)
    .with_padding_bottom(3.)
    .with_background_color(theme.surface_2().into_solid())
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
    .finish();
    Container::new(
        Hoverable::new(mouse_state, move |_| pill)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| on_click(ctx))
            .finish(),
    )
    .with_margin_right(6.)
    .finish()
}

/// Shorten a `claude` model id for the chip: drop the `claude-` prefix the CLI
/// prepends (`claude-fable-5[1m]` → `fable-5[1m]`).
fn prettify_model(model: &str) -> String {
    model.strip_prefix("claude-").unwrap_or(model).to_owned()
}

/// Friendly label for a `--permission-mode` value (the `init` message's
/// `permissionMode`). Unknown modes pass through verbatim.
fn prettify_permission_mode(mode: &str) -> String {
    match mode {
        "default" => "Ask".to_owned(),
        "acceptEdits" => "Accept edits".to_owned(),
        "plan" => "Plan".to_owned(),
        "bypassPermissions" => "Bypass".to_owned(),
        other => other.to_owned(),
    }
}

/// The context chip label — `used / window`, e.g. `26K / 1M`. `None` until
/// `claude` reports a context window (the first turn's `result`).
fn format_context(usage: Usage) -> Option<String> {
    let window = usage.context_window?;
    Some(format!(
        "{} / {}",
        format_token_count(usage.context_used()),
        format_token_count(window),
    ))
}

/// Compact token count: `938` → `938`, `26370` → `26K`, `1000000` → `1M`.
fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if (m - m.round()).abs() < 0.05 {
            format!("{}M", m.round() as u64)
        } else {
            format!("{m:.1}M")
        }
    } else if n >= 1_000 {
        format!("{}K", (n as f64 / 1_000.0).round() as u64)
    } else {
        n.to_string()
    }
}

/// Zero state: a centered heading + a muted one-liner. Sized to content — the
/// caller centers it over the full pane via [`Align`]. The "Resume…" entry
/// point (PRODUCT §36) arrives with the session list in 7h.
fn render_zero_state(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let heading = appearance
        .ui_builder()
        .span("Start a Claude Code session".to_owned())
        .with_style(UiComponentStyles {
            font_size: Some(HEADING_FONT_SIZE),
            font_color: Some(theme.main_text_color(theme.background()).into_solid()),
            ..Default::default()
        })
        .build()
        .finish();
    let explanation = appearance
        .ui_builder()
        .span(
            "Type a message below — twarp drives the local `claude` CLI and renders its replies, \
             tool calls, and diffs here. Your existing Claude Code login is used; twarp adds no \
             account or billing."
                .to_owned(),
        )
        .with_soft_wrap()
        .with_style(UiComponentStyles {
            font_color: Some(theme.nonactive_ui_text_color().into_solid()),
            font_size: Some(13.),
            ..Default::default()
        })
        .build()
        .finish();
    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(12.)
            .with_child(heading)
            .with_child(
                ConstrainedBox::new(explanation)
                    .with_max_width(460.)
                    .finish(),
            )
            .finish(),
    )
    .with_uniform_padding(24.)
    .finish()
}
