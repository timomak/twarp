//! The Claude Code left-panel (roadmap feature 07).
//!
//! Sub-phase **7b** delivers the *scaffold*: the panel registers as a left-panel
//! tab, opens, and renders its zero state (PRODUCT §5) or unavailable state
//! (PRODUCT §6). It owns the [`Transcript`] model (the contract defined in the
//! headless `claude_code` crate) and a placeholder transcript renderer for the
//! `§16–§22` streaming surface, but it spawns **no** `claude` process — merely
//! showing the panel never starts a session (PRODUCT §7).
//!
//! The live session (spawning `claude`, applying [`claude_code::TranscriptEvent`]s
//! to the transcript, streaming, Stop) lands in **7c**; the rich tool/diff/
//! thinking/todo cards in **7d–7f**; permissions + a real editable input in
//! **7g**; session list + resume in **7h**. This module is the host they grow
//! into, so it is a proper child `View` (the Warp Drive panel pattern) rather
//! than inline left-panel rendering.

use claude_code::{Transcript, TranscriptItem};
use warp_core::ui::theme::color::internal_colors;
use warpui::{
    elements::{
        Container, CrossAxisAlignment, Element, Flex, MainAxisSize, MouseStateHandle, ParentElement,
    },
    ui_components::components::UiComponent,
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::util::path::resolve_executable;

/// The executable the panel drives. Resolved on `PATH`; its absence is the
/// unavailable state (PRODUCT §6).
const CLAUDE_BINARY: &str = "claude";

#[derive(Clone, Debug)]
pub enum ClaudeCodePanelAction {
    /// The zero-state "Start session" affordance (PRODUCT §8).
    ///
    /// 7b placeholder: spawning the `claude` driver and replacing the zero
    /// state with the live conversation is 7c. Wiring the action now keeps the
    /// affordance live and the dispatch plumbing in place; for 7b it is a
    /// deliberate no-op so that *merely interacting with the scaffold spawns no
    /// subprocess* (PRODUCT §7).
    StartSession,
}

#[derive(Clone, Default)]
struct MouseStateHandles {
    start_session_button: MouseStateHandle,
}

pub struct ClaudeCodePanelView {
    /// The ordered conversation this panel renders. Empty until 7c feeds it a
    /// live session's events; 7b only ever shows the zero/unavailable state.
    transcript: Transcript,
    mouse_state_handles: MouseStateHandles,
}

impl ClaudeCodePanelView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            transcript: Transcript::new(),
            mouse_state_handles: MouseStateHandles::default(),
        }
    }

    /// Whether the `claude` CLI is resolvable on `PATH` right now. Checked at
    /// render time so it is "re-checked each time the panel is opened"
    /// (PRODUCT §6) without a cached-staleness window.
    fn claude_available() -> bool {
        resolve_executable(CLAUDE_BINARY).is_some()
    }

    /// PRODUCT §6: `claude` is not installed / not on `PATH`. Names the missing
    /// binary, gives a one-line install hint, and shows **no** input affordances.
    fn render_unavailable_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.0);
        col = col.with_child(
            appearance
                .ui_builder()
                .span(format!(
                    "Claude Code isn't available. The `{CLAUDE_BINARY}` command wasn't found on \
                     your PATH."
                ))
                .with_soft_wrap()
                .build()
                .finish(),
        );
        col = col.with_child(
            appearance
                .ui_builder()
                .span(
                    "Install Claude Code and make sure `claude` is on your PATH, then reopen this \
                     panel.",
                )
                .with_soft_wrap()
                .build()
                .finish(),
        );
        Container::new(col.finish())
            .with_padding_top(10.0)
            .with_padding_bottom(10.0)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .finish()
    }

    /// PRODUCT §5: no session has ever started. Shows a short explanation, a
    /// (single-line) message input, and a "Start session" affordance.
    ///
    /// The input is a styled, non-editable placeholder in 7b — real editing,
    /// Enter-to-send, and Shift+Enter multi-line are 7c/7g (PRODUCT §43). The
    /// "Resume…" entry point (PRODUCT §5/§46, shown when prior sessions exist
    /// for the cwd) lands with the session store in 7h.
    fn render_zero_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let explanation = appearance
            .ui_builder()
            .span(
                "Run Claude Code inside twarp. Type a message and start a session — twarp drives \
                 the local `claude` CLI and renders its replies, tool calls, and diffs here. Your \
                 existing Claude Code login is used; twarp adds no account or billing.",
            )
            .with_soft_wrap()
            .build()
            .finish();

        // 7b input placeholder. A backgrounded box reads as an input; it becomes
        // a real editable field in 7c/7g.
        let input = Container::new(
            appearance
                .ui_builder()
                .span("Message Claude Code…")
                .with_soft_wrap()
                .build()
                .finish(),
        )
        .with_padding_top(8.0)
        .with_padding_bottom(8.0)
        .with_padding_left(10.0)
        .with_padding_right(10.0)
        .with_background_color(internal_colors::fg_overlay_3(appearance.theme()).into())
        .finish();

        let start_session = appearance
            .ui_builder()
            .link(
                "Start session".to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(ClaudeCodePanelAction::StartSession);
                })),
                self.mouse_state_handles.start_session_button.clone(),
            )
            .build()
            .finish();

        let col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(10.0)
            .with_child(explanation)
            .with_child(input)
            .with_child(start_session)
            .finish();

        Container::new(col)
            .with_padding_top(10.0)
            .with_padding_bottom(10.0)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .finish()
    }

    /// Placeholder transcript renderer — the `§16–§22` scaffold.
    ///
    /// Never reached in 7b (the transcript is always empty until 7c spawns a
    /// session), but the exhaustive match documents the rendering contract for
    /// every [`TranscriptItem`] the model can hold. 7c upgrades this to a
    /// bottom-stick `UniformList`; 7d–7g replace these plain lines with the
    /// rich tool/diff/thinking/todo/permission cards.
    fn render_transcript(&self, appearance: &Appearance) -> Box<dyn Element> {
        let lines = self.transcript.items().iter().map(|item| {
            let text = match item {
                TranscriptItem::User(text) => format!("You: {text}"),
                TranscriptItem::Assistant { text, .. } => text.clone(),
                TranscriptItem::Thinking { .. } => "Thinking…".to_owned(),
                TranscriptItem::Tool { name, .. } => format!("Tool: {name}"),
                TranscriptItem::Todos(_) => "To-dos".to_owned(),
                TranscriptItem::Permission { tool, .. } => format!("Permission requested: {tool}"),
                TranscriptItem::Notice(text) => text.clone(),
                TranscriptItem::Error(text) => format!("Error: {text}"),
            };
            appearance
                .ui_builder()
                .span(text)
                .with_soft_wrap()
                .build()
                .finish()
        });

        let col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.0)
            .with_children(lines)
            .finish();

        Container::new(col)
            .with_padding_top(10.0)
            .with_padding_bottom(10.0)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .finish()
    }
}

impl View for ClaudeCodePanelView {
    fn ui_name() -> &'static str {
        "ClaudeCodePanelView"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {
        // 7b: no internal editable input to delegate focus to yet — the view
        // itself holds focus, which is enough for keyboard reachability
        // (PRODUCT §61). 7g focuses the real message input here.
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        if !Self::claude_available() {
            return self.render_unavailable_state(appearance);
        }
        if self.transcript.is_empty() {
            return self.render_zero_state(appearance);
        }
        self.render_transcript(appearance)
    }
}

impl Entity for ClaudeCodePanelView {
    type Event = ();
}

impl TypedActionView for ClaudeCodePanelView {
    type Action = ClaudeCodePanelAction;

    fn handle_action(&mut self, action: &ClaudeCodePanelAction, _ctx: &mut ViewContext<Self>) {
        match action {
            // 7b no-op: 7c spawns the `claude` driver here, sends the typed
            // message as the first turn, and replaces the zero state with the
            // live conversation (PRODUCT §8).
            ClaudeCodePanelAction::StartSession => {}
        }
    }
}
