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

mod inline_action;
mod tool_cards;

use std::collections::HashMap;
use std::path::PathBuf;

use async_channel::Sender;
use claude_code::driver::{
    interrupt, send_user_message, spawn_session, Child, PermissionMode, SpawnOptions,
    SpawnedSession,
};
use claude_code::{Transcript, TranscriptEvent, TranscriptItem, Usage};
use futures::StreamExt;
use markdown_parser::{parse_markdown, FormattedText, FormattedTextLine};
use pathfinder_color::ColorU;
use warpui::ui_components::button::ButtonVariant;
use warpui::{
    elements::{
        Border, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, DispatchEventResult, Element, EventHandler, Fill, Flex,
        FormattedTextElement, HighlightedHyperlink, HyperlinkUrl, Icon, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, Padding, ParentElement, Radius, ScrollbarWidth, Shrinkable,
    },
    presenter::ChildView,
    text_layout::ClipConfig,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use self::tool_cards::{render_tool_card, ToolCardUi};
use crate::appearance::Appearance;
use crate::editor::{EditorOptions, EditorView, Event as EditorEvent, TextOptions};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent,
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

/// Shell-polish layout constants (the Claude-app frame): a centered reading
/// column, a docked rounded composer, muted context pills, and a zero-state
/// heading.
const CONTENT_MAX_WIDTH: f32 = 760.;
const COMPOSER_MAX_HEIGHT: f32 = 184.;
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
}

impl ClaudeCodeView {
    /// Build the pane view.
    ///
    /// `initial_prompt` is the trailing positional from `claude <prompt>`
    /// (PRODUCT §2): when present it seeds the first user turn. `cwd` is the
    /// terminal's working directory (PRODUCT §4). In 7b there is no driver, so
    /// a non-empty prompt is rendered through the synthetic source.
    pub fn new(
        initial_prompt: Option<String>,
        cwd: Option<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
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

        // PRODUCT §2/§6: `claude <prompt>` starts a live session immediately with
        // the prompt as the first turn; bare `claude` opens idle (no process
        // until the user sends a message). 7c drives the real
        // `claude_code::driver` — there is no synthetic source anymore.
        let mut transcript = Transcript::new();
        let mut streaming = false;
        let first_prompt = initial_prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        if let Some(prompt) = &first_prompt {
            transcript.apply(TranscriptEvent::UserMessage(prompt.clone()));
            streaming = true;
            Self::begin_session(cwd.clone(), Some(prompt.clone()), ctx);
        }

        Self {
            transcript,
            input_editor,
            pane_configuration,
            focus_handle: None,
            cwd,
            scroll_state: ClippedScrollStateHandle::default(),
            child: None,
            message_tx: None,
            streaming,
            submit_button: MouseStateHandle::default(),
            refresh_button: MouseStateHandle::default(),
            stop_button: MouseStateHandle::default(),
            tool_card_ui: HashMap::new(),
        }
    }

    /// The pane configuration (tab title) handed to [`PaneView`] by the wrapper.
    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
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
            None => Self::begin_session(self.cwd.clone(), Some(text), ctx),
        }
        ctx.notify();
    }

    /// Spawn a live `claude` session (PRODUCT §6) on the background executor —
    /// `spawn_session` itself spawns the child (tokio), so it must not run on
    /// the foreground. [`Self::on_session_spawned`] wires the result into the
    /// view on the main thread and sends `first_prompt` once stdin is up.
    fn begin_session(
        cwd: Option<PathBuf>,
        first_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let opts = SpawnOptions {
            cwd: cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
            model: None,
            resume_session_id: None,
            // 7c default; the permission-mode selector + interactive prompts are
            // 7g (PRODUCT §24–§26). A turn needing tool permission pauses here
            // until 7g, but text turns stream and complete.
            permission_mode: PermissionMode::Default,
            allowed_tools: Vec::new(),
        };
        ctx.spawn(
            async move { spawn_session(opts) },
            move |view, result, ctx| view.on_session_spawned(result, first_prompt, ctx),
        );
    }

    /// Main-thread wiring once `spawn_session` resolves. Starts two background
    /// tasks — one draining the driver's (tokio) event stream into the
    /// transcript via an `async_channel` + [`ViewContext::spawn_stream_local`]
    /// (keeping tokio I/O off the foreground), one owning stdin and writing
    /// queued user turns — and keeps `child` for Stop / kill-on-drop.
    fn on_session_spawned(
        &mut self,
        result: anyhow::Result<SpawnedSession>,
        first_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let SpawnedSession {
            child,
            stdin,
            mut events,
        } = match result {
            Ok(session) => session,
            Err(err) => {
                // PRODUCT §52/§55: surface the spawn failure verbatim.
                self.streaming = false;
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
        let _ = ctx.spawn_stream_local(event_rx, Self::on_transcript_event, Self::on_stream_done);

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

    /// Apply one driver event to the transcript on the main thread (PRODUCT
    /// §9–§13). An `Ended` event closes the streaming turn.
    fn on_transcript_event(&mut self, event: TranscriptEvent, ctx: &mut ViewContext<Self>) {
        if matches!(event, TranscriptEvent::Ended { .. }) {
            self.streaming = false;
        }
        if let TranscriptEvent::ToolCall { id, .. } = &event {
            // The card's stable mouse handle must exist before the first
            // render so expand/collapse clicks pair across renders.
            self.tool_card_ui.entry(id.clone()).or_default();
        }
        self.transcript.apply(event);
        ctx.notify();
    }

    /// The event stream closed — `claude`'s stdout reached EOF, so the process
    /// is gone (PRODUCT §52). The driver already pushed an `Ended` notice; tear
    /// down the live-session handles so the composer returns to a fresh state.
    fn on_stream_done(&mut self, ctx: &mut ViewContext<Self>) {
        self.streaming = false;
        self.child = None;
        self.message_tx = None;
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
    /// status-derived default (failed cards open showing their error).
    fn toggle_tool_card(&mut self, id: &str, ctx: &mut ViewContext<Self>) {
        let Some(TranscriptItem::Tool {
            status, children, ..
        }) = self.transcript.find_tool(id)
        else {
            return;
        };
        let default = tool_cards::default_expanded(*status, !children.is_empty());
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
        for item in self.transcript.items() {
            column.add_child(self.render_item(item, app));
        }

        ClippedScrollable::vertical(
            self.scroll_state.clone(),
            column.finish(),
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
    fn metadata_chips(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let mut chips = vec![render_pill("Local", appearance)];
        if let Some(model) = self.transcript.model() {
            chips.push(render_pill(&prettify_model(model), appearance));
        }
        if let Some(label) = self.transcript.usage().and_then(format_context) {
            chips.push(render_pill(&label, appearance));
        }
        if let Some(mode) = self.transcript.permission_mode() {
            chips.push(render_pill(&prettify_permission_mode(mode), appearance));
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
            // The transcript area and the docked composer are each centered in a
            // max-width reading column (the Claude-app frame); the canvas around
            // them is the themed background. The composer is pinned to the bottom:
            // the body is the only flexible child of the full-height outer column,
            // so it expands and pushes the composer down — the structure 7c shipped
            // (centering each child rather than the whole stack keeps that pin).
            let body = center_reading_column(self.render_body(app), true);
            let composer = center_reading_column(self.render_input(appearance), false);
            let stack = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1.0, body).finish())
                .with_child(composer)
                .finish();
            Container::new(stack)
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
        }
    }
}

impl BackingView for ClaudeCodeView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &(),
        _ctx: &mut ViewContext<Self>,
    ) {
        // No overflow menu items in 7b.
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
        _app: &AppContext,
    ) -> HeaderContent {
        // PRODUCT §5: title "Claude Code" with the session cwd as a secondary
        // line. Net-new chrome (the Agent-block header was service-coupled;
        // TECH matrix marks it do-NOT-port).
        let cwd = self
            .cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            });
        HeaderContent::Standard(StandardHeader {
            title: PANE_TITLE.to_owned(),
            title_secondary: cwd,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: StandardHeaderOptions::default(),
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

/// Center `child` horizontally in a max-width reading column (the Claude-app
/// frame). `fill_height` stretches the child to the column's full height — used
/// for the transcript area so the docked composer (sized to content) gets pinned
/// to the bottom by the body flexing above it.
fn center_reading_column(child: Box<dyn Element>, fill_height: bool) -> Box<dyn Element> {
    let constrained = ConstrainedBox::new(child)
        .with_max_width(CONTENT_MAX_WIDTH)
        .finish();
    let (inner, main_axis_size) = if fill_height {
        (
            Shrinkable::new(1.0, constrained).finish(),
            MainAxisSize::Max,
        )
    } else {
        (constrained, MainAxisSize::Min)
    };
    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(main_axis_size)
        .with_child(inner)
        .finish()
}

impl ClaudeCodeView {
    /// Bridge dispatch (TECH §The bridge): one arm per [`TranscriptItem`].
    /// User/Assistant render through the ported markdown transcript (7b); Tool
    /// renders through the ported `inline_action` card chrome (7d, PRODUCT
    /// §16–§19). The diff, thinking and todo cards are 7e–7f; those arms render
    /// a minimal themed placeholder rather than crash — the model contract
    /// already carries the variants.
    fn render_item(&self, item: &TranscriptItem, app: &AppContext) -> Box<dyn Element> {
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
            } => render_tool_card(
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
            // 7e–7f bring back the diff/thinking/todo cards (ported from the
            // remaining `view_impl` leaves + feature 05's diff renderer).
            TranscriptItem::Thinking { .. }
            | TranscriptItem::Todos(_)
            | TranscriptItem::Permission { .. } => render_pending_card(item, appearance),
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

/// Placeholder for transcript variants whose rich cards land in 7e–7f.
/// Kept minimal and clearly labelled.
fn render_pending_card(item: &TranscriptItem, appearance: &Appearance) -> Box<dyn Element> {
    let kind = match item {
        TranscriptItem::Thinking { .. } => "Thinking",
        TranscriptItem::Todos(_) => "Task list",
        TranscriptItem::Permission { .. } => "Permission request",
        _ => "Item",
    };
    render_notice(
        &format!("{kind} (rendered in a later sub-phase)"),
        appearance,
    )
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

/// Zero state: a centered heading + a muted one-liner, vertically centered in the
/// reading column above the always-present composer. The "Resume…" entry point
/// (PRODUCT §36) arrives with the session list in 7h.
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
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
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
