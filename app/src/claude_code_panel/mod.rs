//! The Claude Code left-panel (roadmap feature 07, sub-phase **7b**).
//!
//! Hosts the *rendering layer* of Warp's Agent Mode — resurrected by **porting**
//! the deleted `ai_assistant` transcript renderer (`render_message` + the
//! markdown-segment splitter `translate_formatted_text_into_markdown_segments`
//! from commit `fea2f7ea`) and reparenting it onto the thin, twarp-native
//! [`claude_code::Transcript`] model. The headless driver lives in the
//! [`claude_code`] crate; **7b feeds the transcript from a synthetic event
//! source** (no live `claude` process) so the ported renderer is testable
//! end-to-end with no subprocess. 7c swaps the synthetic source for the real
//! [`claude_code::driver`].
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
//! the panel is its own [`TypedActionView`], in-panel clicks dispatch
//! [`ClaudeCodePanelAction`] after an `on_left_mouse_down` focus-grab puts the
//! panel in the responder chain. There is **no** `WorkspaceAction` forwarder —
//! that was the #67 symptom-fix and is deleted.

use std::ops::Range;

use claude_code::{Transcript, TranscriptEvent, TranscriptItem};
use markdown_parser::{parse_markdown, FormattedText, FormattedTextLine};
use pathfinder_color::ColorU;
use warpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
        Element, EventHandler, Fill, Flex, FormattedTextElement, HighlightedHyperlink,
        HyperlinkUrl, Icon, MainAxisSize, MouseStateHandle, ParentElement, Radius,
        ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable, UniformList,
        UniformListState,
    },
    presenter::ChildView,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{EditorOptions, EditorView, Event as EditorEvent, TextOptions};
use crate::util::path::resolve_executable;

/// The executable the panel drives. Resolved on `PATH`; its absence is the
/// unavailable state (PRODUCT §6).
const CLAUDE_BINARY: &str = "claude";

/// Avatar glyphs for the message rows (the Agent-Mode shape: icon + body).
const USER_ICON_SVG_PATH: &str = "bundled/svg/user.svg";
const ASSISTANT_ICON_SVG_PATH: &str = "bundled/svg/agentmode.svg";

/// Body / code font sizes, matching the deleted `ai_assistant::transcript`
/// renderer so the ported transcript keeps Agent Mode's proportions.
const BODY_FONT_SIZE: f32 = 13.;
const CODE_FONT_SIZE: f32 = 12.;
const PANEL_LEFT_MARGIN: f32 = 15.;

/// Actions the panel handles, dispatched the [`GlobalSearchView`] way (the panel
/// is itself the [`TypedActionView`], no `WorkspaceAction` forwarder). 7b keeps
/// this minimal; lifecycle / permissions / resume actions arrive in 7c–7h.
#[derive(Clone, Debug)]
pub enum ClaudeCodePanelAction {
    /// Submit the input buffer as a user turn. In 7b this drives the synthetic
    /// event source; 7c spawns the real `claude` process (PRODUCT §8).
    Submit,
    /// Focus the message input — the `on_left_mouse_down` focus-grab that keeps
    /// the panel in the responder chain so in-panel dispatches land (TECH §The
    /// panel; the fix #67 routed around with a forwarder).
    FocusInput,
    /// Open a URL clicked inside assistant markdown (PRODUCT §18 links).
    OpenUrl(HyperlinkUrl),
    /// Re-check `claude` availability — the unavailable state's "Check again"
    /// (PRODUCT §6: "re-checked each time the panel is opened").
    Refresh,
}

pub struct ClaudeCodePanelView {
    /// The conversation the panel renders. 7b populates it from a synthetic
    /// source on submit; the live driver fills it in 7c.
    transcript: Transcript,
    /// The message input. Enter submits; Shift+Enter inserts a newline (the
    /// full §43 semantics are validated in 7g, but the editor gives both now).
    input_editor: ViewHandle<EditorView>,
    /// Weak self-handle so the [`UniformList`] `build_items` closure can render
    /// rows on demand (the [`GlobalSearchView`] virtualized-list pattern).
    handle: WeakViewHandle<Self>,
    /// Virtualized transcript list state (PRODUCT §21 — large sessions stay
    /// responsive because only visible rows render).
    uniform_list_state: UniformListState,
    scroll_state: ScrollStateHandle,
    /// Stable mouse-state handles kept across renders so a click's
    /// mousedown/mouseup hit the same handle.
    submit_button: MouseStateHandle,
    refresh_button: MouseStateHandle,
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
        input_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Message Claude Code…", ctx);
        });
        Self {
            transcript: Transcript::new(),
            input_editor,
            handle: ctx.handle(),
            uniform_list_state: UniformListState::new(),
            scroll_state: ScrollStateHandle::default(),
            submit_button: MouseStateHandle::default(),
            refresh_button: MouseStateHandle::default(),
        }
    }

    /// Whether the `claude` CLI is resolvable on `PATH` right now (PRODUCT §6).
    fn claude_available() -> bool {
        resolve_executable(CLAUDE_BINARY).is_some()
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // PRODUCT §43: Enter sends; Shift+Enter is handled by the editor itself
        // (inserts a newline) and does not reach us.
        if matches!(event, EditorEvent::Enter) {
            self.submit(ctx);
        }
    }

    /// Send the current input as a user turn.
    ///
    /// 7b has **no driver**: instead of spawning `claude`, it applies a
    /// representative sequence of [`TranscriptEvent`]s (the synthetic source) so
    /// the ported renderer is exercised end-to-end. 7c replaces
    /// [`synthetic_reply`] with the real `claude_code::driver` event stream and
    /// the `apply_event` pump (TECH §Re-derived sub-phase plan).
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self
            .input_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        if text.is_empty() {
            // PRODUCT §44: empty / whitespace-only messages are a no-op.
            return;
        }
        self.transcript
            .apply(TranscriptEvent::UserMessage(text.clone()));
        for event in synthetic_reply(&text) {
            self.transcript.apply(event);
        }
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.notify();
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        // PRODUCT §4: the header shows the session cwd. Net-new chrome (the
        // Agent-block header was service-coupled; TECH matrix marks it
        // do-NOT-port). 7b shows the current dir; 7c shows the live session cwd.
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "—".to_owned());
        let title = appearance
            .ui_builder()
            .span("Claude Code".to_owned())
            .build()
            .finish();
        let dir = appearance
            .ui_builder()
            .span(cwd)
            .with_soft_wrap()
            .build()
            .finish();
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(2.)
                .with_child(title)
                .with_child(dir)
                .finish(),
        )
        .with_padding_left(10.)
        .with_padding_right(10.)
        .with_padding_top(8.)
        .with_padding_bottom(8.)
        .finish()
    }

    fn render_body(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        if self.transcript.is_empty() {
            // PRODUCT §5: zero state when no session has produced anything.
            return render_zero_state(appearance);
        }
        self.render_transcript(app)
    }

    /// Render the transcript into a [`UniformList`] (PRODUCT §21) wrapped in a
    /// vertical [`Scrollable`], mirroring [`GlobalSearchView::render_results`].
    /// Bottom-stick auto-scroll (PRODUCT §22) layers on in 7c when content
    /// actually streams; 7b's synthetic transcript is static.
    fn render_transcript(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let handle = self.handle.clone();
        let item_count = self.transcript.items().len();

        let build_items = move |range: Range<usize>, app: &AppContext| {
            let view_handle = handle
                .upgrade(app)
                .expect("ClaudeCodePanelView handle should be upgradeable");
            let view = view_handle.as_ref(app);
            let appearance = Appearance::as_ref(app);
            (range.start..range.end)
                .map(|idx| render_item(&view.transcript.items()[idx], appearance))
                .collect::<Vec<_>>()
                .into_iter()
        };

        let list = UniformList::new(self.uniform_list_state.clone(), item_count, build_items);
        Scrollable::vertical(
            self.scroll_state.clone(),
            list.finish_scrollable(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish()
    }

    fn render_input(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let input_view = Container::new(ChildView::new(&self.input_editor).finish())
            .with_padding_top(6.)
            .with_padding_bottom(6.)
            .with_padding_left(10.)
            .with_padding_right(10.)
            .with_background_color(theme.surface_3().into_solid())
            .finish();

        // PRODUCT §5: "Start session" in the zero state, "Send" once a
        // conversation exists. Both dispatch `Submit` (synthetic in 7b).
        let label = if self.transcript.is_empty() {
            "Start session"
        } else {
            "Send"
        };
        let action = appearance
            .ui_builder()
            .link(
                label.to_owned(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(ClaudeCodePanelAction::Submit);
                })),
                self.submit_button.clone(),
            )
            .build()
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(8.)
                .with_child(input_view)
                .with_child(action)
                .finish(),
        )
        .with_padding_left(10.)
        .with_padding_right(10.)
        .with_padding_top(6.)
        .with_padding_bottom(10.)
        .finish()
    }

    fn render_unavailable_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        // PRODUCT §6: replaces the whole panel; names the missing binary, gives
        // an install hint, no input affordances.
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
                 `claude` is on your PATH, then re-open this panel."
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
                    ctx.dispatch_typed_action(ClaudeCodePanelAction::Refresh);
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

impl Entity for ClaudeCodePanelView {
    type Event = ();
}

impl View for ClaudeCodePanelView {
    fn ui_name() -> &'static str {
        "ClaudeCodePanelView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // PRODUCT §61: focus the input on entry so typing just works. Focusing a
        // child keeps the panel itself in the responder chain, so in-panel
        // `ClaudeCodePanelAction` dispatches reach `handle_action` below.
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.input_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        // PRODUCT §6: the unavailable state replaces everything.
        if !Self::claude_available() {
            return self.render_unavailable_state(appearance);
        }
        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_header(appearance))
            .with_child(Shrinkable::new(1.0, self.render_body(app)).finish())
            .with_child(self.render_input(appearance))
            .finish();

        // The focus-grab (TECH §The panel): any click on empty panel chrome
        // focuses the input, guaranteeing the panel is the dispatch target for
        // subsequent in-panel actions — what #67 worked around with a forwarder.
        EventHandler::new(body)
            .on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(ClaudeCodePanelAction::FocusInput);
                DispatchEventResult::StopPropagation
            })
            .finish()
    }
}

impl TypedActionView for ClaudeCodePanelView {
    type Action = ClaudeCodePanelAction;

    fn handle_action(&mut self, action: &ClaudeCodePanelAction, ctx: &mut ViewContext<Self>) {
        match action {
            ClaudeCodePanelAction::Submit => self.submit(ctx),
            ClaudeCodePanelAction::FocusInput => ctx.focus(&self.input_editor),
            ClaudeCodePanelAction::OpenUrl(url) => ctx.open_url(&url.url),
            // PRODUCT §6: render re-checks availability, so a notify suffices.
            ClaudeCodePanelAction::Refresh => ctx.notify(),
        }
    }
}

// ---------- transcript rendering (the ported leaf) ----------

/// Bridge dispatch (TECH §The bridge): one arm per [`TranscriptItem`]. 7b
/// renders the markdown transcript (User / Assistant). The rich tool, diff,
/// thinking and todo cards are 7d–7f; the 7b synthetic source emits none of
/// them, so those arms render a minimal themed placeholder rather than crash —
/// the model contract already carries the variants.
fn render_item(item: &TranscriptItem, appearance: &Appearance) -> Box<dyn Element> {
    match item {
        TranscriptItem::User(text) => render_message_row(
            USER_ICON_SVG_PATH,
            text,
            appearance.theme().surface_1().into_solid(),
            appearance,
        ),
        TranscriptItem::Assistant { text, .. } => render_message_row(
            ASSISTANT_ICON_SVG_PATH,
            text,
            appearance.theme().surface_2().into_solid(),
            appearance,
        ),
        TranscriptItem::Notice(message) => render_notice(message, appearance),
        TranscriptItem::Error(message) => render_error(message, appearance),
        // 7d–7f bring back the rich tool/diff/thinking/todo cards (ported from
        // the `inline_action` chrome + feature 05's diff renderer). Not reached
        // by 7b's synthetic source.
        TranscriptItem::Thinking { .. }
        | TranscriptItem::Tool { .. }
        | TranscriptItem::Todos(_)
        | TranscriptItem::Permission { .. } => render_pending_card(item, appearance),
    }
}

/// Port of `ai_assistant::transcript::render_message`: an icon + a markdown body
/// on a themed surface. User and assistant rows differ only by glyph and
/// background, so the message turns are visually distinct (PRODUCT §16).
fn render_message_row(
    icon_svg: &'static str,
    text: &str,
    background: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_color = theme.main_text_color(background.into()).into_solid();
    let icon = ConstrainedBox::new(Icon::new(icon_svg, text_color).finish())
        .with_height(16.)
        .with_width(16.)
        .finish();

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(
            Container::new(icon)
                .with_margin_right(12.)
                .with_margin_top(3.)
                .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Container::new(render_markdown_body(text, text_color, appearance)).finish(),
            )
            .finish(),
        );

    Container::new(row.finish())
        .with_background_color(background)
        .with_padding_left(PANEL_LEFT_MARGIN)
        .with_padding_top(16.)
        .with_padding_bottom(16.)
        .with_padding_right(20.)
        .finish()
}

/// Port of `render_message`'s markdown body: render each [`MarkdownSegment`] —
/// prose via [`FormattedTextElement`] (feature 03's stack), fenced code via a
/// bordered monospace box. PRODUCT §18 (markdown), §53 (defensive: fall back to
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
                ctx.dispatch_typed_action(ClaudeCodePanelAction::OpenUrl(url));
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
    .with_padding_left(PANEL_LEFT_MARGIN)
    .with_padding_right(20.)
    .finish()
}

/// Error surfaced verbatim from `claude` (PRODUCT §55) on a themed surface. The
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
    .with_padding_left(PANEL_LEFT_MARGIN)
    .with_padding_right(20.)
    .finish()
}

/// Placeholder for transcript variants whose rich cards land in 7d–7f. Kept
/// minimal and clearly labelled; never reached by 7b's synthetic source.
fn render_pending_card(item: &TranscriptItem, appearance: &Appearance) -> Box<dyn Element> {
    let kind = match item {
        TranscriptItem::Thinking { .. } => "Thinking",
        TranscriptItem::Tool { .. } => "Tool call",
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

/// The 7b **synthetic event source**: stands in for the live driver so the
/// ported renderer can be exercised end-to-end with no `claude` process. Returns
/// a representative markdown assistant reply (heading, list, inline code, a
/// fenced code block, a link) that demonstrates the Agent-Mode transcript shape
/// — the 7b acceptance gate. 7c deletes this and feeds real
/// [`claude_code::driver`] events instead (TECH §Re-derived sub-phase plan).
fn synthetic_reply(user_text: &str) -> Vec<TranscriptEvent> {
    let markdown = format!(
        "Here's how I'd approach **{user_text}**.\n\n\
         ## Plan\n\
         - Inspect the project layout with `ls` and `cat`\n\
         - Make the change in the relevant module\n\
         - Re-run the tests with `cargo test`\n\n\
         Example command:\n\n\
         ```bash\n\
         cargo test -p claude_code\n\
         ```\n\n\
         See the [Claude Code docs](https://docs.claude.com/en/docs/claude-code) for more. \
         This reply is the ported Agent-Mode transcript renderer (sub-phase 7b); a live \
         `claude` session replaces it in 7c."
    );
    vec![
        TranscriptEvent::AssistantTextDelta { text: markdown },
        TranscriptEvent::AssistantTextDone,
    ]
}

/// Zero state (PRODUCT §5): a short explanation. The single-line message input
/// and "Start session" affordance live in the always-present input row below.
/// The "Resume…" entry point (PRODUCT §46) arrives with the session list in 7h.
fn render_zero_state(appearance: &Appearance) -> Box<dyn Element> {
    let explanation = appearance
        .ui_builder()
        .span(
            "Type a message below and start a session — twarp drives the local `claude` CLI and \
             renders its replies, tool calls, and diffs here. Your existing Claude Code login is \
             used; twarp adds no account or billing."
                .to_owned(),
        )
        .with_soft_wrap()
        .build()
        .finish();
    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(10.)
            .with_child(explanation)
            .finish(),
    )
    .with_uniform_padding(15.)
    .finish()
}
