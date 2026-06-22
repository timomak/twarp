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

mod composer;
mod diff_cards;
mod inline_action;
mod thinking;
mod todos;
mod tool_cards;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use async_channel::Sender;
use base64::Engine as _;
use claude_code::diff::diff_for_tool;
use claude_code::driver::{
    interrupt, send_user_message, spawn_session, Child, OutgoingImage, OutgoingMessage,
    PermissionMode, SpawnOptions, SpawnedSession,
};
use claude_code::launch::LaunchOptions;
use claude_code::{sessions, Transcript, TranscriptEvent, TranscriptItem, TurnMetrics, Usage};
use futures::StreamExt;
use markdown_parser::{parse_markdown, FormattedText, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_editor::editor::NavigationKey;
use warpui::assets::asset_cache::AssetSource;
use warpui::platform::FilePickerConfiguration;
use warpui::ui_components::button::ButtonVariant;
use warpui::{
    elements::{
        Align, Border, CacheOption, ChildAnchor, Clipped, ClippedScrollStateHandle,
        ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DispatchEventResult, DropShadow, Element, EventDispatchMode, EventHandler, Fill, Flex,
        FormattedTextElement, HighlightedHyperlink, Hoverable, HyperlinkUrl, Icon, Image,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding,
        ParentAnchor, ParentElement, ParentOffsetBounds, Radius, SavePosition, ScrollTarget,
        ScrollToPositionMode, ScrollbarWidth, Shrinkable, Stack,
    },
    platform::Cursor,
    presenter::ChildView,
    text_layout::ClipConfig,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use self::composer::{SuggestionKind, SuggestionQuery};
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
use crate::terminal::{view::Event as TerminalViewEvent, TerminalManager, TerminalView};
use crate::util::path::{resolve_executable, resolve_executable_in_path};
#[cfg(all(feature = "local_fs", feature = "local_tty"))]
use crate::terminal::local_shell::LocalShellState;

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
const COMPOSER_CLEARANCE: f32 = 24.;
/// Height reserved at the bottom of the pane for the floating composer, so the
/// scroll viewport — and therefore the scrollbar track — ends *above* the
/// composer instead of running down behind the message input (the "scroll bar
/// goes beyond the text input" report). Sized to the composer's resting height
/// (one-line input + controls row + padding); a taller composer (multi-line
/// draft, queued messages, suggestions) simply overlaps the transcript as
/// before, but the scrollbar still stops here.
const COMPOSER_RESERVED: f32 = 96.;
/// Position id of the zero-height sentinel pinned to the end of the transcript.
/// Bottom-stick auto-scroll (PRODUCT §14) scrolls this into view to follow
/// streaming output and to open a resumed session at its latest message.
const TRANSCRIPT_BOTTOM_POSITION_ID: &str = "claude_transcript_bottom";
const COMPOSER_CORNER_RADIUS: f32 = 14.;
const MESSAGE_CORNER_RADIUS: f32 = 12.;
const PILL_CORNER_RADIUS: f32 = 6.;
const HEADING_FONT_SIZE: f32 = 20.;

/// The model selector's cycle (PRODUCT §52, 7m). The first entry, "default",
/// maps to passing no `--model` (let `claude` choose); the rest are the
/// `--model` aliases the CLI accepts. Cycling reuses the §25 detach→`--resume`
/// mechanism so the conversation continues under the new model.
const MODEL_CYCLE: &[&str] = &["default", "opus", "sonnet", "haiku"];

/// The effort selector's cycle (PRODUCT §52, 7m). `--effort` is a verified
/// `claude` flag (launch.rs maps it; `--effort max` confirmed on 2.1.173); the
/// first entry, "default", passes no `--effort` (the CLI's own default). Effort
/// is write-only — the headless stream never echoes it back — so the pill shows
/// the selection. A value an older CLI rejects surfaces as a spawn error
/// (PRODUCT §30), never a hang.
const EFFORT_CYCLE: &[&str] = &["default", "low", "medium", "high", "max"];

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
    /// Accept the composer suggestion at this index (PRODUCT §15a, 7j) —
    /// clicked in the suggestions panel; Enter accepts the highlighted one.
    AcceptSuggestion(usize),
    /// Remove an attachment chip (PRODUCT §15b, 7j): the image stays
    /// mentioned in the text (so `claude` can still read it) but is no longer
    /// sent as an inline image block.
    RemoveAttachment(String),
    /// Files were dropped onto the pane (PRODUCT §50, 7l): images become
    /// attachment chips, other files become `@`-mentions.
    DropFiles(Vec<String>),
    /// Open the OS file picker to attach files (PRODUCT §51, 7l) — the
    /// composer's "＋ attach" control.
    AttachFromPicker,
    /// Remove a direct attachment chip (paste / drop / picker) by index
    /// (PRODUCT §49–§51, 7l).
    RemoveDirectAttachment(usize),
    /// Step the model selector to the next model (PRODUCT §52, 7m). Detaches a
    /// live session; the next message resumes the conversation under it.
    CycleModel,
    /// Step the effort selector to the next level (PRODUCT §52, 7m).
    CycleEffort,
    /// Remove the queued (type-ahead) message at this index before it
    /// dispatches (PRODUCT §54, 7m).
    RemoveQueuedMessage(usize),
    /// Approve a plan card (PRODUCT §56, 7n): degrades to switching the
    /// permission mode off `plan` and resuming — no one-click inline accept.
    ApprovePlan,
    /// Select (radio) / toggle (multi-select) an option on an `AskUserQuestion`
    /// card (PRODUCT §1 "questions UI"). `item` is the card's transcript index,
    /// `option` the option index; `multi` chooses toggle vs. replace.
    SelectQuestionOption {
        item: usize,
        option: usize,
        multi: bool,
    },
    /// Submit the chosen answers for the `AskUserQuestion` card at this
    /// transcript index as the next user turn. Headless `claude` auto-dismisses
    /// the tool and ends the turn, so the answer continues the conversation as
    /// an ordinary message rather than a tool_result.
    SubmitQuestionAnswers(usize),
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

/// The embedded raw-CLI terminal (PRODUCT §39, 7i): a real terminal session
/// rendered **inside** this pane in place of the chat — the pane itself never
/// leaves the tree, so toggling can't disturb the layout. Created by the pane
/// group (it owns the session resources) and owned here.
///
/// Field order is load-bearing: dropping `manager` first halts the session's
/// event loop before the view goes away (the same drop-order contract
/// `TerminalPane` documents), and dropping it kills the PTY — which is how
/// leaving raw mode ends the CLI (PRODUCT §42).
struct RawCliSession {
    manager: ModelHandle<Box<dyn TerminalManager>>,
    view: ViewHandle<TerminalView>,
}

/// An image attached directly through paste / drag-drop / file-picker (7l,
/// PRODUCT §49–§51), as opposed to the `@`-mention-derived `pending_images`.
/// The image is encoded up front so all four attachment routes share the one
/// send path (`OutgoingMessage.images`); `thumbnail_path` is `Some` for the
/// file-sourced routes (drop / picker) and `None` for clipboard bytes.
struct DirectAttachment {
    label: String,
    image: OutgoingImage,
    thumbnail_path: Option<PathBuf>,
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
    /// `PATH` captured from the user's interactive login shell, used to resolve
    /// and run `claude` (PRODUCT §4). macOS GUI launches inherit launchd's
    /// minimal `PATH`, which omits the dirs where `claude` lives, so the
    /// process `PATH` alone reports it unavailable. `None` until the async
    /// capture resolves (or when capture is unsupported); availability and the
    /// spawn fall back to the process `PATH` in that case.
    interactive_path: Option<String>,
    scroll_state: ClippedScrollStateHandle,
    /// The live `claude` child, once a session is running. Kept for `interrupt`
    /// (Stop, PRODUCT §11) and to kill the process on drop (PRODUCT §8 —
    /// `spawn_session` sets `kill_on_drop`).
    child: Option<Child>,
    /// Sends user turns to the background task that owns the process stdin
    /// (PRODUCT §16). `None` until a session is running.
    message_tx: Option<Sender<OutgoingMessage>>,
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
    /// The embedded raw-CLI terminal while raw mode is active (PRODUCT §39).
    /// `None` in rendered-chat mode.
    raw_cli: Option<RawCliSession>,
    /// The active composer suggestion query (`/` or `@`), `None` when the
    /// panel is closed (PRODUCT §15a, 7j).
    suggestion_query: Option<SuggestionQuery>,
    /// Filtered suggestions for the active query, capped.
    suggestions: Vec<String>,
    /// The highlighted suggestion (Enter accepts it; Tab/arrows move it).
    suggestion_selected: usize,
    /// Stable per-row mouse handles for the suggestions panel, grown on
    /// demand (the Timeline/shortcut-row pattern).
    suggestion_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// The cwd's mentionable files, walked lazily on the first `@` and kept
    /// for the pane's lifetime (gitignore-aware, capped).
    cwd_files: Option<Vec<String>>,
    /// `@`-mentioned images detected in the draft (PRODUCT §15b) — previewed
    /// as chips and sent as inline image blocks.
    pending_images: Vec<PathBuf>,
    /// Images the user X-ed out of attaching (the mention text remains).
    attachment_optouts: HashSet<PathBuf>,
    /// Stable per-chip mouse handles for attachment removal.
    attachment_chip_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Images attached directly via paste / drag-drop / file picker (PRODUCT
    /// §49–§51, 7l) — not tied to the draft text, so they survive edits until
    /// the message sends. Combined with `pending_images` at send time.
    direct_attachments: Vec<DirectAttachment>,
    /// Stable per-chip mouse handles for the direct-attachment chips.
    direct_chip_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Mouse state for the composer's "＋ attach" file-picker button (§51).
    attach_button: MouseStateHandle,
    /// Type-ahead queue (PRODUCT §53–§54, 7m): messages sent while a turn
    /// streams wait here and dispatch in order as each turn completes.
    message_queue: Vec<OutgoingMessage>,
    /// Stable per-row mouse handles for the removable queue rows (§54).
    queue_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Mouse state for the clickable model-selector pill (PRODUCT §52, 7m).
    model_pill_mouse: MouseStateHandle,
    /// Mouse state for the clickable effort-selector pill (PRODUCT §52, 7m).
    effort_pill_mouse: MouseStateHandle,
    /// Mouse state for a plan card's Approve / Keep-planning controls
    /// (PRODUCT §56, 7n). Shared across cards — a session shows one plan at a
    /// time in practice.
    plan_approve_mouse: MouseStateHandle,
    plan_keep_mouse: MouseStateHandle,
    /// Monotonic session generation. Spawn callbacks and stream pumps carry
    /// the epoch they were started under and are ignored when stale — a
    /// permission-mode restart must not let the old (killed) session's EOF
    /// tear down the new one's handles or spam the transcript.
    session_epoch: u64,
    /// Chosen options for each `AskUserQuestion` card (PRODUCT §1), keyed by the
    /// card's transcript index → the set of selected (flattened) option
    /// indices. A single-select question keeps one entry (radio); a
    /// multi-select toggles.
    question_selected: HashMap<usize, HashSet<usize>>,
    /// Stable mouse handles for the question option rows (keyed by card
    /// transcript index + flattened option index) and the per-card submit
    /// buttons (keyed by card index), created on demand.
    question_option_mouse: std::cell::RefCell<HashMap<(usize, usize), MouseStateHandle>>,
    question_submit_mouse: std::cell::RefCell<HashMap<usize, MouseStateHandle>>,
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
            interactive_path: None,
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
            raw_cli: None,
            suggestion_query: None,
            suggestions: Vec::new(),
            suggestion_selected: 0,
            suggestion_row_mouse: std::cell::RefCell::new(Vec::new()),
            cwd_files: None,
            pending_images: Vec::new(),
            attachment_optouts: HashSet::new(),
            attachment_chip_mouse: std::cell::RefCell::new(Vec::new()),
            direct_attachments: Vec::new(),
            direct_chip_mouse: std::cell::RefCell::new(Vec::new()),
            attach_button: MouseStateHandle::default(),
            message_queue: Vec::new(),
            queue_row_mouse: std::cell::RefCell::new(Vec::new()),
            model_pill_mouse: MouseStateHandle::default(),
            effort_pill_mouse: MouseStateHandle::default(),
            plan_approve_mouse: MouseStateHandle::default(),
            plan_keep_mouse: MouseStateHandle::default(),
            session_epoch: 0,
            question_selected: HashMap::new(),
            question_option_mouse: std::cell::RefCell::new(HashMap::new()),
            question_submit_mouse: std::cell::RefCell::new(HashMap::new()),
        };

        // PRODUCT §4: capture the login-shell PATH up front so availability
        // detection and the spawn see the user's real PATH even under a GUI
        // (launchd-minimal) launch. Resolves asynchronously and re-renders.
        Self::capture_interactive_path(ctx);

        // PRODUCT §36: a resumed pane renders the stored history up front —
        // through the same ingest path as live events so tool/diff/thinking
        // card state exists — and continues live on the next message.
        if let Some(resume) = resume {
            for event in sessions::load_history(&resume.jsonl_path) {
                view.ingest_event(event, ctx);
            }
            view.resume_session_id = Some(resume.session_id);
            // PRODUCT §14: open a resumed session at its latest message, not
            // scrolled to the top of a long history.
            view.scroll_to_bottom();
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
            view.begin_session(Some(OutgoingMessage::text(prompt)), ctx);
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
        // In raw mode the terminal owns the keyboard (PRODUCT §43).
        match &self.raw_cli {
            Some(raw_cli) => ctx.focus(&raw_cli.view),
            None => ctx.focus(&self.input_editor),
        }
    }

    /// Whether the `claude` CLI is resolvable right now (PRODUCT §4) — against
    /// the captured login-shell PATH when available, falling back to the
    /// process PATH (which, under a GUI launch, is launchd-minimal and usually
    /// omits where `claude` lives).
    fn claude_available(&self) -> bool {
        if let Some(path) = &self.interactive_path {
            if resolve_executable_in_path(CLAUDE_BINARY, std::ffi::OsStr::new(path)).is_some() {
                return true;
            }
        }
        resolve_executable(CLAUDE_BINARY).is_some()
    }

    /// Kick off (or refresh) the async capture of the interactive login-shell
    /// PATH, storing it on the view and re-rendering when it resolves. The
    /// underlying capture is cached by `LocalShellState`, so repeated calls
    /// (e.g. the "Check again" button) are cheap. No-op when the local shell
    /// integration isn't compiled in; availability then uses the process PATH.
    #[cfg(all(feature = "local_fs", feature = "local_tty"))]
    fn capture_interactive_path(ctx: &mut ViewContext<Self>) {
        let fut = LocalShellState::handle(ctx)
            .update(ctx, |shell_state, ctx| shell_state.get_interactive_path_env_var(ctx));
        ctx.spawn(fut, |me, path, ctx| {
            if path.is_some() && me.interactive_path != path {
                me.interactive_path = path;
                ctx.notify();
            }
        });
    }

    #[cfg(not(all(feature = "local_fs", feature = "local_tty")))]
    fn capture_interactive_path(_ctx: &mut ViewContext<Self>) {}

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            // PRODUCT §15: Enter sends — unless the suggestions panel is open,
            // in which case it accepts the highlighted suggestion (7j).
            // Shift+Enter is handled by the editor itself (inserts a newline).
            EditorEvent::Enter => {
                if self.suggestion_query.is_some() && !self.suggestions.is_empty() {
                    self.accept_suggestion(self.suggestion_selected, ctx);
                } else {
                    self.submit(ctx);
                }
            }
            // Live-filter the suggestions + attachment chips as the draft
            // changes (PRODUCT §15a–§15b).
            EditorEvent::Edited(_) => self.refresh_composer_intelligence(ctx),
            // PRODUCT §49 (7l): a paste may carry a clipboard image — attach it.
            // The editor has already inserted any plain text, so this only adds
            // image chips (an image-only clipboard inserts no text).
            EditorEvent::Paste => self.attach_pasted_image(ctx),
            EditorEvent::Escape => {
                if self.suggestion_query.take().is_some() {
                    self.suggestions.clear();
                    ctx.notify();
                }
            }
            // Cycle the highlighted suggestion. The editor surfaces these as
            // events when the cursor can't move further (single-line drafts:
            // always); clicking a row works regardless.
            EditorEvent::Navigate(key) if self.suggestion_query.is_some() => {
                let len = self.suggestions.len();
                if len == 0 {
                    return;
                }
                match key {
                    NavigationKey::Down | NavigationKey::Tab => {
                        self.suggestion_selected = (self.suggestion_selected + 1) % len;
                        ctx.notify();
                    }
                    NavigationKey::Up | NavigationKey::ShiftTab => {
                        self.suggestion_selected =
                            self.suggestion_selected.checked_sub(1).unwrap_or(len - 1);
                        ctx.notify();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Recompute the suggestions panel and the attachment chips from the
    /// current draft (PRODUCT §15a–§15b, 7j).
    fn refresh_composer_intelligence(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self
            .input_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx));

        // Attachment chips: @-mentions resolving to images under the cwd.
        let cwd = self
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        self.pending_images = composer::image_mentions(&text, &cwd)
            .into_iter()
            .filter(|path| !self.attachment_optouts.contains(path))
            .collect();

        // Suggestions panel.
        self.suggestion_query = composer::suggestion_query(&text);
        self.suggestions = match &self.suggestion_query {
            Some(query) => {
                let candidates: Vec<String> = match query.kind {
                    SuggestionKind::SlashCommand => {
                        let mut commands: Vec<String> = composer::DEFAULT_SLASH_COMMANDS
                            .iter()
                            .map(|c| c.to_string())
                            .collect();
                        for command in self.transcript.slash_commands() {
                            if !commands.contains(command) {
                                commands.push(command.clone());
                            }
                        }
                        commands.sort();
                        commands
                    }
                    SuggestionKind::FileMention => self
                        .cwd_files
                        .get_or_insert_with(|| composer::list_cwd_files(&cwd))
                        .clone(),
                };
                composer::filter_suggestions(&query.query, &candidates)
            }
            None => Vec::new(),
        };
        self.suggestion_selected = 0;
        ctx.notify();
    }

    /// Accept a suggestion (PRODUCT §15a): replace the queried token in the
    /// draft and keep typing flowing.
    fn accept_suggestion(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let (Some(query), Some(accepted)) =
            (self.suggestion_query.clone(), self.suggestions.get(index))
        else {
            return;
        };
        let accepted = accepted.clone();
        let text = self
            .input_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx));
        let new_text = composer::apply_suggestion(&text, &query, &accepted);
        self.input_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text(&new_text, ctx));
        self.suggestion_query = None;
        self.suggestions.clear();
        self.refresh_composer_intelligence(ctx);
    }

    /// The composer's suggestions panel (PRODUCT §15a): one row per
    /// suggestion, highlighted row tracks the keyboard selection, click
    /// accepts. `None` when no `/` or `@` query is active.
    fn render_suggestions_panel(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let query = self.suggestion_query.as_ref()?;
        if self.suggestions.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let glyph = match query.kind {
            SuggestionKind::SlashCommand => crate::ui_components::icons::Icon::SlashCommands,
            SuggestionKind::FileMention => crate::ui_components::icons::Icon::File,
        };
        let mut rows = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);
        for (index, suggestion) in self.suggestions.iter().enumerate() {
            let row_mouse = {
                let mut states = self.suggestion_row_mouse.borrow_mut();
                while states.len() <= index {
                    states.push(MouseStateHandle::default());
                }
                states[index].clone()
            };
            let selected = index == self.suggestion_selected;
            let icon = ConstrainedBox::new(
                Icon::new(glyph.into(), theme.nonactive_ui_text_color().into_solid()).finish(),
            )
            .with_width(14.)
            .with_height(14.)
            .finish();
            let label = appearance
                .ui_builder()
                .span(suggestion.clone())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.monospace_font_family()),
                    font_size: Some(12.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let mut row_container = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(icon)
                    .with_child(label)
                    .finish(),
            )
            .with_padding_left(8.)
            .with_padding_right(8.)
            .with_padding_top(4.)
            .with_padding_bottom(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if selected {
                row_container = row_container.with_background_color(theme.surface_2().into_solid());
            }
            let row = row_container.finish();
            rows.add_child(
                Hoverable::new(row_mouse, move |_| row)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::AcceptSuggestion(index));
                    })
                    .finish(),
            );
        }
        Some(
            Container::new(rows.finish())
                .with_padding(Padding::uniform(4.))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
        )
    }

    /// The attachment chips row (PRODUCT §15b): a thumbnail + name per
    /// pending image, with an ✕ that drops it back to a plain text mention.
    /// `None` when nothing is attached.
    fn render_attachment_chips(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if self.pending_images.is_empty() && self.direct_attachments.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.);
        for (index, path) in self.pending_images.iter().enumerate() {
            let chip_mouse = {
                let mut states = self.attachment_chip_mouse.borrow_mut();
                while states.len() <= index {
                    states.push(MouseStateHandle::default());
                }
                states[index].clone()
            };
            let thumbnail = ConstrainedBox::new(
                Image::new(
                    AssetSource::LocalFile {
                        path: path.display().to_string(),
                    },
                    CacheOption::BySize,
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
            )
            .with_width(24.)
            .with_height(24.)
            .finish();
            let name = appearance
                .ui_builder()
                .span(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string()),
                )
                .with_style(UiComponentStyles {
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let remove = appearance
                .ui_builder()
                .span("\u{2715}".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let chip = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(thumbnail)
                    .with_child(name)
                    .with_child(remove)
                    .finish(),
            )
            .with_padding_left(6.)
            .with_padding_right(6.)
            .with_padding_top(3.)
            .with_padding_bottom(3.)
            .with_background_color(theme.surface_2().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish();
            let chip_path = path.display().to_string();
            row.add_child(
                Hoverable::new(chip_mouse, move |_| chip)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::RemoveAttachment(
                            chip_path.clone(),
                        ));
                    })
                    .finish(),
            );
        }
        // PRODUCT §49–§51 (7l): directly-attached images (paste / drop /
        // picker). A file-sourced attachment shows a thumbnail; a pasted one
        // (no path) shows an image glyph. The ✕ drops it entirely (unlike a
        // mention chip, there is no underlying text to fall back to).
        for (index, attachment) in self.direct_attachments.iter().enumerate() {
            let chip_mouse = {
                let mut states = self.direct_chip_mouse.borrow_mut();
                while states.len() <= index {
                    states.push(MouseStateHandle::default());
                }
                states[index].clone()
            };
            let preview: Box<dyn Element> = match &attachment.thumbnail_path {
                Some(path) => ConstrainedBox::new(
                    Image::new(
                        AssetSource::LocalFile {
                            path: path.display().to_string(),
                        },
                        CacheOption::BySize,
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
                )
                .with_width(24.)
                .with_height(24.)
                .finish(),
                None => ConstrainedBox::new(
                    Icon::new(
                        crate::ui_components::icons::Icon::Image.into(),
                        theme.nonactive_ui_text_color().into_solid(),
                    )
                    .finish(),
                )
                .with_width(16.)
                .with_height(16.)
                .finish(),
            };
            let name = appearance
                .ui_builder()
                .span(attachment.label.clone())
                .with_style(UiComponentStyles {
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let remove = appearance
                .ui_builder()
                .span("\u{2715}".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let chip = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(preview)
                    .with_child(name)
                    .with_child(remove)
                    .finish(),
            )
            .with_padding_left(6.)
            .with_padding_right(6.)
            .with_padding_top(3.)
            .with_padding_bottom(3.)
            .with_background_color(theme.surface_2().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish();
            row.add_child(
                Hoverable::new(chip_mouse, move |_| chip)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::RemoveDirectAttachment(
                            index,
                        ));
                    })
                    .finish(),
            );
        }
        Some(row.finish())
    }

    /// The queued (type-ahead) messages waiting to dispatch (PRODUCT §54, 7m):
    /// one removable row per queued message, shown above the composer input.
    /// `None` when the queue is empty.
    fn render_message_queue(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if self.message_queue.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.);
        column.add_child(
            appearance
                .ui_builder()
                .span(format!("Queued ({})", self.message_queue.len()))
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(11.),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        for (index, message) in self.message_queue.iter().enumerate() {
            let row_mouse = {
                let mut states = self.queue_row_mouse.borrow_mut();
                while states.len() <= index {
                    states.push(MouseStateHandle::default());
                }
                states[index].clone()
            };
            // One-line, length-bounded preview of the queued text.
            let label = appearance
                .ui_builder()
                .span(queue_preview(&message.text))
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish();
            let remove = appearance
                .ui_builder()
                .span("\u{2715}".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let row = Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(ConstrainedBox::new(label).with_max_width(620.).finish())
                    .with_child(remove)
                    .finish(),
            )
            .with_padding_left(8.)
            .with_padding_right(8.)
            .with_padding_top(4.)
            .with_padding_bottom(4.)
            .with_background_color(theme.surface_2().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish();
            column.add_child(
                Hoverable::new(row_mouse, move |_| row)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::RemoveQueuedMessage(index));
                    })
                    .finish(),
            );
        }
        Some(column.finish())
    }

    /// Encode the draft's surviving image mentions as outgoing image blocks
    /// (PRODUCT §15b). Oversized or unreadable files degrade to the plain
    /// text mention — `claude` reads those itself (§29: never block a send).
    fn encode_pending_images(&self) -> Vec<OutgoingImage> {
        self.pending_images
            .iter()
            .filter_map(|path| read_image_attachment(path))
            .collect()
    }

    /// Every image heading out with the next turn: the surviving `@`-mention
    /// images (7j, re-read now) plus the directly-attached ones (paste / drop /
    /// picker, 7l — already encoded). One list, one send path (PRODUCT §51).
    fn outgoing_images(&self) -> Vec<OutgoingImage> {
        let mut images = self.encode_pending_images();
        images.extend(self.direct_attachments.iter().map(|a| a.image.clone()));
        images
    }

    /// Attach a clipboard image (PRODUCT §49, 7l): triggered by a paste while
    /// the composer is focused. Reads the clipboard, picks the best supported
    /// image, and adds it as a direct-attachment chip. Plain-text pastes are
    /// the editor's own job and are untouched here; an unsupported or oversized
    /// image is dropped silently (§51 degradation, never a crash).
    fn attach_pasted_image(&mut self, ctx: &mut ViewContext<Self>) {
        let content = ctx.clipboard().read();
        let Some(images) = content.images else {
            return;
        };
        let mut added = false;
        for image in images {
            let Some(media_type) = composer::normalize_image_media_type(&image.mime_type) else {
                continue;
            };
            if image.data.len() as u64 > composer::MAX_IMAGE_BYTES {
                log::info!("claude: pasted image exceeds the inline-image cap; skipped");
                continue;
            }
            let label = image
                .filename
                .clone()
                .unwrap_or_else(|| "Pasted image".to_owned());
            self.direct_attachments.push(DirectAttachment {
                label,
                image: OutgoingImage {
                    media_type: media_type.to_owned(),
                    base64_data: base64::engine::general_purpose::STANDARD.encode(&image.data),
                },
                thumbnail_path: None,
            });
            added = true;
            // One image per paste is the common case; the API set is checked in
            // preference order by the clipboard layer, so the first hit is best.
            break;
        }
        if added {
            ctx.notify();
        }
    }

    /// Attach a set of file paths (PRODUCT §50–§51, 7l) — shared by drag-drop
    /// and the file picker. Image files become attachment chips; everything
    /// else is appended to the draft as an `@`-mention (`claude` reads those
    /// itself). Oversized/unreadable images degrade to a mention too.
    fn attach_files(&mut self, paths: Vec<PathBuf>, ctx: &mut ViewContext<Self>) {
        let cwd = self
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let mut mentions: Vec<String> = Vec::new();
        let mut added = false;
        for path in paths {
            match read_image_attachment(&path) {
                Some(image) => {
                    self.direct_attachments.push(DirectAttachment {
                        label: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string()),
                        image,
                        thumbnail_path: Some(path),
                    });
                    added = true;
                }
                // Non-image, oversized, or unreadable → @-mention (§50/§51).
                None => mentions.push(composer::mention_for(&path, &cwd)),
            }
        }
        if !mentions.is_empty() {
            let text = self
                .input_editor
                .read(ctx, |editor, ctx| editor.buffer_text(ctx));
            let new_text = mentions
                .iter()
                .fold(text, |acc, mention| composer::append_mention(&acc, mention));
            self.input_editor
                .update(ctx, |editor, ctx| editor.set_buffer_text(&new_text, ctx));
            self.refresh_composer_intelligence(ctx);
        }
        if added {
            ctx.notify();
        }
    }

    /// Open the OS file picker (PRODUCT §51, 7l) for the composer's "＋ attach"
    /// control. Selected files flow through the same [`Self::attach_files`]
    /// path as a drag-drop: the callback dispatches `DropFiles` back to this
    /// view (the proven cross-thread write-back, mirroring `EditorView`).
    fn open_attach_picker(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let view_id = ctx.view_id();
        ctx.open_file_picker(
            move |result, ctx| {
                if let Ok(paths) = result {
                    if !paths.is_empty() {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            view_id,
                            &ClaudeCodeViewAction::DropFiles(paths),
                        );
                    }
                }
            },
            FilePickerConfiguration::new().allow_multi_select(),
        );
    }

    /// Send the current input as a user turn to the live `claude` session,
    /// spawning the session on the first message if none is running yet
    /// (PRODUCT §6, §16). While a turn streams the message is **queued**
    /// (type-ahead, PRODUCT §53) and dispatched when the turn completes —
    /// the composer input is never disabled.
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self
            .input_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        if text.is_empty() {
            // PRODUCT §15: empty / whitespace-only messages are a no-op.
            return;
        }
        // PRODUCT §15b (7j) / §49–§51 (7l): mention-derived images plus the
        // directly-attached ones (paste / drop / picker) ride along as inline
        // image blocks; the mention text stays for context. Captured now (files
        // read here) so a queued message ships exactly what was attached.
        let message = OutgoingMessage {
            images: self.outgoing_images(),
            text,
        };
        // Clear the composer either way so the user can keep typing (§53).
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.suggestion_query = None;
        self.suggestions.clear();
        self.pending_images.clear();
        self.attachment_optouts.clear();
        self.direct_attachments.clear();

        self.submit_message(message, ctx);
    }

    /// Deliver one already-composed user turn (PRODUCT §16): queue it behind a
    /// streaming turn (type-ahead, §53), write it to a live session's stdin, or
    /// spawn the session on the first message. Shared by the composer
    /// ([`Self::submit`]) and the `AskUserQuestion` answer path (§1).
    fn submit_message(&mut self, message: OutgoingMessage, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            // PRODUCT §53–§54 (7m): a turn is in flight — queue this for
            // automatic dispatch when the turn completes, rather than blocking.
            self.message_queue.push(message);
            ctx.notify();
            return;
        }

        self.transcript
            .apply(TranscriptEvent::UserMessage(message.text.clone()));
        self.streaming = true;
        match &self.message_tx {
            // Session already running — write the turn to its stdin.
            Some(tx) => {
                let _ = tx.try_send(message);
            }
            // First message: spawn the session, forwarding this as turn one.
            None => self.begin_session(Some(message), ctx),
        }
        // PRODUCT §14: a user turn always jumps back to the live bottom.
        self.scroll_to_bottom();
        ctx.notify();
    }

    /// Select (radio) or toggle (multi-select) an option on the
    /// `AskUserQuestion` card at transcript index `item` (PRODUCT §1).
    fn select_question_option(
        &mut self,
        item: usize,
        option: usize,
        multi: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if multi {
            let chosen = self.question_selected.entry(item).or_default();
            if !chosen.insert(option) {
                chosen.remove(&option);
            }
            ctx.notify();
            return;
        }
        // Radio: the selection set spans every question on the card, so clear
        // only the sibling options of the question this option belongs to,
        // leaving other questions' answers intact.
        let siblings: Vec<usize> = {
            let Some(TranscriptItem::Tool { input, .. }) = self.transcript.items().get(item) else {
                return;
            };
            parse_questions(input)
                .into_iter()
                .find(|q| q.options.iter().any(|o| o.flat_index == option))
                .map(|q| q.options.iter().map(|o| o.flat_index).collect())
                .unwrap_or_default()
        };
        let chosen = self.question_selected.entry(item).or_default();
        for sibling in &siblings {
            chosen.remove(sibling);
        }
        chosen.insert(option);
        ctx.notify();
    }

    /// Submit the chosen answers for the `AskUserQuestion` card at `item` as the
    /// next user turn (PRODUCT §1). Headless `claude` auto-dismisses the tool
    /// and ends the turn, so the answer continues the conversation as an
    /// ordinary message; the model reads the question from the transcript above.
    fn submit_question_answers(&mut self, item: usize, ctx: &mut ViewContext<Self>) {
        let parsed = {
            let Some(TranscriptItem::Tool { name, input, .. }) =
                self.transcript.items().get(item)
            else {
                return;
            };
            if name != "AskUserQuestion" {
                return;
            }
            parse_questions(input)
        };
        let Some(selected) = self.question_selected.get(&item) else {
            return;
        };
        let mut lines = Vec::new();
        for question in &parsed {
            let picks: Vec<&str> = question
                .options
                .iter()
                .filter(|o| selected.contains(&o.flat_index))
                .map(|o| o.label.as_str())
                .collect();
            if picks.is_empty() {
                continue;
            }
            let label = if question.header.trim().is_empty() {
                question.question.as_str()
            } else {
                question.header.as_str()
            };
            lines.push(format!("{}: {}", label, picks.join(", ")));
        }
        if lines.is_empty() {
            return;
        }
        // Drop the selection so the card stops offering live controls once the
        // answer is on its way.
        self.question_selected.remove(&item);
        self.submit_message(OutgoingMessage::text(lines.join("\n")), ctx);
    }

    /// Dispatch the next queued message after a turn completes (PRODUCT §53,
    /// 7m). No-op unless the session is idle, still live, and has a queued
    /// message. Each completed turn drains exactly one, so messages dispatch in
    /// order as the conversation advances.
    fn drain_message_queue(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming || self.message_queue.is_empty() {
            return;
        }
        let Some(tx) = self.message_tx.clone() else {
            // Session ended (error/exit): nothing to send onto. Keep the queue
            // so a fresh manual send can still go (the next submit spawns one).
            return;
        };
        let message = self.message_queue.remove(0);
        self.transcript
            .apply(TranscriptEvent::UserMessage(message.text.clone()));
        self.streaming = true;
        let _ = tx.try_send(message);
        ctx.notify();
    }

    /// Step the model selector to the next model (PRODUCT §52, 7m). Disabled
    /// while a turn streams (same rule as §25). Reuses the mode-pill mechanism:
    /// a live session is detached and the next message resumes the same
    /// conversation under the new `--model`.
    fn cycle_model(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            return;
        }
        self.model = Self::advance_cycle(MODEL_CYCLE, self.model.as_deref());
        if self.child.is_some() {
            self.detach_live_session();
        }
        ctx.notify();
    }

    /// Step the effort selector to the next level (PRODUCT §52, 7m). Same
    /// detach→`--resume` mechanism and streaming guard as [`Self::cycle_model`];
    /// the next spawn carries the new `--effort`.
    fn cycle_effort(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming {
            return;
        }
        self.effort = Self::advance_cycle(EFFORT_CYCLE, self.effort.as_deref());
        if self.child.is_some() {
            self.detach_live_session();
        }
        ctx.notify();
    }

    /// Advance a selector cycle to the entry after `current`, mapping the first
    /// entry ("default") to `None` (pass no flag, let `claude` choose).
    fn advance_cycle(cycle: &[&str], current: Option<&str>) -> Option<String> {
        let current = current.unwrap_or(cycle[0]);
        let idx = cycle.iter().position(|m| *m == current).unwrap_or(0);
        let next = cycle[(idx + 1) % cycle.len()];
        (next != cycle[0]).then(|| next.to_owned())
    }

    /// Approve a plan card (PRODUCT §56, 7n). Headless `claude` exposes no
    /// stdio approval channel for `ExitPlanMode` (its tool_result is the §24
    /// wall), so this is **not** a one-click inline accept: it switches the
    /// permission mode off `plan` (to auto-accept edits, so the approved plan
    /// can execute) and resumes via the §25 mode-pill detach path. A no-op
    /// mid-turn; never hangs.
    fn approve_plan(&mut self, ctx: &mut ViewContext<Self>) {
        if self.streaming || self.permission_mode != PermissionMode::Plan {
            return;
        }
        self.permission_mode = PermissionMode::AcceptEdits;
        if self.child.is_some() {
            self.detach_live_session();
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
    fn begin_session(
        &mut self,
        first_prompt: Option<OutgoingMessage>,
        ctx: &mut ViewContext<Self>,
    ) {
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
            path_env: self.interactive_path.clone(),
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
        first_prompt: Option<OutgoingMessage>,
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
        let (message_tx, message_rx) = async_channel::unbounded::<OutgoingMessage>();
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
        // PRODUCT §53 (7m): a turn that completed cleanly (or was interrupted —
        // the session is still alive) dispatches the next queued message.
        let turn_completed = matches!(
            &event,
            TranscriptEvent::Ended {
                reason: claude_code::EndReason::Completed | claude_code::EndReason::Interrupted,
            }
        );
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
        if turn_completed {
            self.drain_message_queue(ctx);
        }
        // PRODUCT §14: follow streaming output to the bottom as it arrives.
        self.scroll_to_bottom();
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
            TranscriptEvent::ThinkingDelta { .. } => {
                // A streaming thinking block (7k): the first delta opens a new
                // item, later deltas accumulate into it. Allocate the card's UI
                // state only when a new item is actually created — i.e. when the
                // last item is not an open thinking block (mirror Transcript's
                // ThinkingDelta rule so the index lines up).
                let opens_new = !matches!(
                    self.transcript.items().last(),
                    Some(TranscriptItem::Thinking { done: false, .. })
                );
                if opens_new {
                    self.thinking_ui
                        .entry(self.transcript.items().len())
                        .or_default();
                }
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

    /// twarp 07 (7i, PRODUCT §39/§42): flip between the rendered chat and the
    /// raw interactive CLI. Entering is disabled while a turn streams (§42 —
    /// same rule as the mode selector); the headless process is detached
    /// first (one driver at a time) and the host pane group hands back a real
    /// terminal session running `claude --resume <session_id>`, which
    /// [`Self::enter_raw_mode`] embeds.
    fn toggle_raw_cli(&mut self, ctx: &mut ViewContext<Self>) {
        if self.raw_cli.is_some() {
            self.exit_raw_mode(ctx);
            return;
        }
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

    /// twarp 07 (7i, PRODUCT §39): embed the raw-CLI terminal the pane group
    /// created. The pane renders it in place of the chat until the user
    /// returns (the floating overlay / the header toggle) or the CLI exits
    /// (§44 — `exec` makes the CLI's exit end the session, which surfaces as
    /// the terminal's `Exited` event).
    pub(crate) fn enter_raw_mode(
        &mut self,
        manager: ModelHandle<Box<dyn TerminalManager>>,
        view: ViewHandle<TerminalView>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
            // The floating "← Claude Code" overlay (PRODUCT §40) or the CLI
            // exiting (`/exit`, a crash, a failed resume — §44).
            TerminalViewEvent::ReturnToClaudePane | TerminalViewEvent::Exited => {
                me.exit_raw_mode(ctx);
            }
            _ => {}
        });
        ctx.focus(&view);
        self.raw_cli = Some(RawCliSession { manager, view });
        ctx.notify();
    }

    /// twarp 07 (7i, PRODUCT §40/§42/§44): leave raw mode — drop the embedded
    /// session (killing the CLI's PTY), re-read the conversation's on-disk
    /// history so raw-mode turns appear in the transcript, and hand focus
    /// back to the composer. The next message resumes the conversation live.
    fn exit_raw_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(raw_cli) = self.raw_cli.take() else {
            return;
        };
        ctx.unsubscribe_to_view(&raw_cli.view);
        drop(raw_cli);
        self.refresh_from_disk(ctx);
        self.resume_session_id = Some(self.session_id.clone());
        ctx.focus(&self.input_editor);
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
        self.scroll_to_bottom();
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

    /// Bottom-stick auto-scroll (PRODUCT §14): bring the transcript's end
    /// marker into view so streaming output stays in sight and a resumed
    /// session opens at its latest message rather than its first. A no-op once
    /// the marker is already visible, so it doesn't yank a view that is already
    /// at the bottom.
    fn scroll_to_bottom(&self) {
        self.scroll_state.scroll_to_position(ScrollTarget {
            position_id: TRANSCRIPT_BOTTOM_POSITION_ID.to_owned(),
            mode: ScrollToPositionMode::FullyIntoView,
        });
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

        // PRODUCT §14: a zero-height marker pinned to the end of the transcript.
        // [`Self::scroll_to_bottom`] scrolls this into view to follow streaming
        // output and to open a resumed session at its latest message.
        column.add_child(
            SavePosition::new(
                ConstrainedBox::new(Container::new(Flex::row().finish()).finish())
                    .with_height(1.)
                    .finish(),
                TRANSCRIPT_BOTTOM_POSITION_ID,
            )
            .finish(),
        );

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

    /// twarp 08d (PRODUCT §13–§16): the bottom gradient fade-out band.
    ///
    /// A full-width region pinned to the bottom of the pane, `COMPOSER_CLEARANCE`
    /// tall (the same gap the scroller reserves below the last message). Its
    /// background is a vertical gradient that runs from fully transparent at the
    /// top of the band to the opaque pane background at the bottom, so transcript
    /// content scrolling up under the floating composer dissolves into the
    /// background instead of ending at a hard horizontal cut (§13).
    ///
    /// `bg` is the pane's live theme background (passed from `theme.background()`),
    /// and the transparent endpoint reuses its RGB with zero alpha — so the fade
    /// never tints toward a hard-coded colour and is invisible-by-design in both
    /// light and dark themes (§16). The band carries no event handlers, so it does
    /// not consume clicks or change scroll extent / hit-testing (§15); the caller
    /// paints it above the scrolled body but below the opaque composer (§14).
    fn render_transcript_fade(bg: ColorU) -> Box<dyn Element> {
        let transparent = ColorU::new(bg.r, bg.g, bg.b, 0);
        // A `MainAxisSize::Max` row expands to the full pane width; the
        // `ConstrainedBox` fixes the band height to the composer clearance.
        ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            )
            .with_background(Fill::Gradient {
                start: vec2f(0., 0.),
                end: vec2f(0., 1.),
                start_color: transparent,
                end_color: bg,
            })
            .finish(),
        )
        .with_height(COMPOSER_CLEARANCE)
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
        // PRODUCT §52 (7m): the model pill is a selector. It shows the active
        // selection (the alias the next spawn passes via `--model`, falling
        // back to what `claude` reported, then a generic label) and clicking it
        // cycles models. Disabled mid-turn (same rule as the permission pill).
        let model_label = self
            .model
            .as_deref()
            .map(prettify_model)
            .or_else(|| self.transcript.model().map(prettify_model))
            .unwrap_or_else(|| "Model".to_owned());
        if self.streaming {
            chips.push(render_pill(&model_label, appearance));
        } else {
            chips.push(render_clickable_pill(
                &model_label,
                self.model_pill_mouse.clone(),
                |ctx| ctx.dispatch_typed_action(ClaudeCodeViewAction::CycleModel),
                appearance,
            ));
        }
        // PRODUCT §52 (7m): the effort selector, write-only (no echo-back), so
        // the pill reflects the current selection. Clickable when idle.
        let effort_label = match self.effort.as_deref() {
            Some(effort) => format!("Effort: {effort}"),
            None => "Effort".to_owned(),
        };
        if self.streaming {
            chips.push(render_pill(&effort_label, appearance));
        } else {
            chips.push(render_clickable_pill(
                &effort_label,
                self.effort_pill_mouse.clone(),
                |ctx| ctx.dispatch_typed_action(ClaudeCodeViewAction::CycleEffort),
                appearance,
            ));
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

        // PRODUCT §51 (7l): the "＋ attach" control opens the OS file picker.
        // A paperclip icon button, grouped with the Send/Stop action on the
        // right.
        let attach = Hoverable::new(self.attach_button.clone(), {
            let glyph =
                Icon::new(crate::ui_components::icons::Icon::Paperclip.into(), muted).finish();
            move |_| {
                ConstrainedBox::new(glyph)
                    .with_width(16.)
                    .with_height(16.)
                    .finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::AttachFromPicker);
        })
        .finish();
        let right = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(10.)
            .with_child(attach)
            .with_child(action)
            .finish();

        let controls = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(left)
            .with_child(right)
            .finish();

        let mut card_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.);
        // PRODUCT §54 (7m): queued type-ahead messages sit at the top of the
        // composer, each removable before it dispatches.
        if let Some(queue) = self.render_message_queue(appearance) {
            card_column.add_child(queue);
        }
        // PRODUCT §15a (7j): the suggestions panel sits above the input,
        // Claude-app style — `/` commands or `@` files, filtered live.
        if let Some(panel) = self.render_suggestions_panel(appearance) {
            card_column.add_child(panel);
        }
        // PRODUCT §15b (7j): attachment chips — @-mentioned images that will
        // ride along as inline image blocks, with previews and an ✕ to drop
        // one back to a plain mention.
        if let Some(chips) = self.render_attachment_chips(appearance) {
            card_column.add_child(chips);
        }
        let card = Container::new(card_column.with_child(editor).with_child(controls).finish())
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
        // In raw mode the embedded terminal owns the keyboard instead (§43).
        if focus_ctx.is_self_focused() {
            self.focus(ctx);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // twarp 07 (7i, PRODUCT §39/§43): raw mode — the pane IS the
        // terminal. Rendered bare (no focus-grab wrapper: the terminal owns
        // every click and keystroke; the way back is its floating overlay,
        // the header toggle, or the CLI exiting). The terminal renders its
        // own floating "← Claude Code" overlay (§40).
        if let Some(raw_cli) = &self.raw_cli {
            return ChildView::new(&raw_cli.view).finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        // PRODUCT §4: the unavailable state replaces the pane body. The pane
        // header (title) is rendered separately by `render_header_content`.
        let contents = if self.claude_available() {
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
                // Reserve the composer's resting height at the bottom so the
                // scroll viewport — and with it the scrollbar track — ends
                // *above* the floating composer instead of running down behind
                // the message input (the "scroll bar goes beyond the text
                // input" report). The composer still floats over this reserved
                // band; the transcript's own COMPOSER_CLEARANCE keeps the last
                // message clear of it.
                Container::new(Align::new(self.render_body(app)).top_left().finish())
                    .with_padding_bottom(COMPOSER_RESERVED)
                    .finish()
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
            // twarp 08d (PRODUCT §13–§16): a bottom gradient fade. Stack
            // children paint in insertion order, so adding this *after* the
            // transcript body and *before* the composer puts the fade above
            // scrolled content but below the opaque composer (§14). The band
            // is a full-width region pinned to the pane bottom over the
            // composer-clearance gap; its vertical gradient runs from
            // transparent (top) to the pane background (bottom), so transcript
            // content sliding under the composer dissolves into the
            // background. The band has no event handlers — it never consumes
            // clicks or alters scroll extent/hit-testing (§15) — and its
            // bottom color is read live from the theme background, so it is
            // invisible-by-design in light and dark themes (§16).
            stack.add_positioned_child(
                Self::render_transcript_fade(theme.background().into_solid()),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
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
            // PRODUCT §50 (7l): files dropped anywhere on the pane body attach —
            // images as chips, others as `@`-mentions.
            .on_drag_and_drop_files(|ctx, _, paths, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::DropFiles(paths.to_vec()));
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
            ClaudeCodeViewAction::Refresh => {
                // Re-capture the login-shell PATH (cheap — cached) in case it
                // wasn't ready on first render, then re-render to re-check
                // availability (PRODUCT §4).
                Self::capture_interactive_path(ctx);
                ctx.notify();
            }
            ClaudeCodeViewAction::Stop => self.stop(ctx),
            ClaudeCodeViewAction::ToggleToolCard(id) => self.toggle_tool_card(id, ctx),
            ClaudeCodeViewAction::ToggleThinking(index) => self.toggle_thinking(*index, ctx),
            ClaudeCodeViewAction::ToggleTodos => {
                self.todos_expanded = !self.todos_expanded;
                ctx.notify();
            }
            ClaudeCodeViewAction::CyclePermissionMode => self.cycle_permission_mode(ctx),
            ClaudeCodeViewAction::AcceptSuggestion(index) => {
                self.accept_suggestion(*index, ctx);
                // A click steals focus from the editor; give it back so
                // typing flows on (PRODUCT §15).
                ctx.focus(&self.input_editor);
            }
            ClaudeCodeViewAction::RemoveAttachment(path) => {
                self.attachment_optouts.insert(PathBuf::from(path));
                self.pending_images
                    .retain(|p| p.display().to_string() != *path);
                ctx.notify();
            }
            ClaudeCodeViewAction::DropFiles(paths) => {
                let paths = paths.iter().map(PathBuf::from).collect();
                self.attach_files(paths, ctx);
            }
            ClaudeCodeViewAction::AttachFromPicker => self.open_attach_picker(ctx),
            ClaudeCodeViewAction::RemoveDirectAttachment(index) => {
                if *index < self.direct_attachments.len() {
                    self.direct_attachments.remove(*index);
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::CycleModel => self.cycle_model(ctx),
            ClaudeCodeViewAction::CycleEffort => self.cycle_effort(ctx),
            ClaudeCodeViewAction::RemoveQueuedMessage(index) => {
                if *index < self.message_queue.len() {
                    self.message_queue.remove(*index);
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::ApprovePlan => self.approve_plan(ctx),
            ClaudeCodeViewAction::SelectQuestionOption {
                item,
                option,
                multi,
            } => self.select_question_option(*item, *option, *multi, ctx),
            ClaudeCodeViewAction::SubmitQuestionAnswers(item) => {
                self.submit_question_answers(*item, ctx)
            }
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
        // PRODUCT §39 (7i): the Raw CLI toggle — embeds the real interactive
        // `claude` (resuming this conversation) in place of the chat, and
        // back. Entering is hidden while a turn streams (§42); in raw mode
        // the button always shows, labeled as the way back. The header lives
        // in the parent PaneView's tree, so the click must dispatch the pane
        // framework's CustomAction (handled by the header view and routed
        // back to `handle_custom_action`), NOT an in-pane
        // ClaudeCodeViewAction — that dies unhandled from header chrome.
        let raw_mode = self.raw_cli.is_some();
        let raw_cli_toggle = (raw_mode || !self.streaming).then(|| {
            let appearance = Appearance::as_ref(app);
            let label = if raw_mode { "Chat UI" } else { "Raw CLI" };
            appearance
                .ui_builder()
                .button(ButtonVariant::Text, self.raw_cli_button.clone())
                .with_text_label(label.to_owned())
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
            // PRODUCT §55–§56 (7n): an `ExitPlanMode` call renders as a themed
            // plan card (its full markdown is in the tool input), not a generic
            // tool card. Falls through to the tool card if the plan text is
            // missing (§29 defensive).
            TranscriptItem::Tool { name, input, .. }
                if name == "ExitPlanMode"
                    && input.get("plan").and_then(|v| v.as_str()).is_some() =>
            {
                let plan = input.get("plan").and_then(|v| v.as_str()).unwrap_or("");
                self.render_plan_card(plan, appearance)
            }
            // PRODUCT §1 (questions UI): an `AskUserQuestion` call renders as an
            // interactive question card (clickable options + a Send button)
            // rather than a generic tool card — headless `claude` auto-dismisses
            // the tool, so the user's pick is sent as the next turn.
            TranscriptItem::Tool { name, input, .. }
                if name == "AskUserQuestion"
                    && input.get("questions").and_then(|v| v.as_array()).is_some() =>
            {
                self.render_question_card(index, input, appearance)
            }
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
            TranscriptItem::Thinking { text, duration, .. } => thinking::render_thinking_card(
                index,
                text,
                *duration,
                self.thinking_ui.get(&index),
                app,
            ),
            TranscriptItem::Metrics(metrics) => render_metrics_line(metrics, appearance),
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

    /// Render an `ExitPlanMode` plan card (PRODUCT §55–§56, 7n): the plan's full
    /// markdown in a themed card, distinct from a generic tool card, with
    /// **Approve** and **Keep planning** affordances.
    ///
    /// Approve is *not* a one-click inline accept — headless `claude` has no
    /// stdio approval channel for plan exit (the §24 wall) — so it switches the
    /// permission mode off `plan` and resumes ([`Self::approve_plan`]). Keep
    /// planning returns focus to the composer so the user refines the plan; the
    /// session stays in plan mode. Both are hidden once the session has left
    /// plan mode (a historical plan card needs no live controls).
    fn render_plan_card(&self, plan: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let surface = theme.surface_2();
        let text_color = theme.main_text_color(surface).into_solid();
        let accent = theme.accent().into_solid();

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(
                ConstrainedBox::new(
                    Icon::new(crate::ui_components::icons::Icon::File.into(), accent).finish(),
                )
                .with_width(15.)
                .with_height(15.)
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .span("Plan".to_owned())
                    .with_style(UiComponentStyles {
                        font_color: Some(text_color),
                        font_size: Some(13.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(10.)
            .with_child(header)
            .with_child(render_markdown_body(plan, text_color, appearance));

        // The live controls only make sense while the session is still in plan
        // mode and idle; otherwise the card is a record of an approved/aborted
        // plan (PRODUCT §56: never hang, never offer a dead control).
        if self.permission_mode == PermissionMode::Plan && !self.streaming {
            let approve = appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.plan_approve_mouse.clone())
                .with_text_label("Approve".to_owned())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ApprovePlan);
                })
                .finish();
            let keep = appearance
                .ui_builder()
                .button(ButtonVariant::Outlined, self.plan_keep_mouse.clone())
                .with_text_label("Keep planning".to_owned())
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::FocusInput);
                })
                .finish();
            column.add_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(10.)
                    .with_child(approve)
                    .with_child(keep)
                    .finish(),
            );
        }

        Container::new(column.finish())
            .with_padding(Padding::uniform(14.))
            .with_margin_top(4.)
            .with_margin_bottom(4.)
            .with_margin_left(TRANSCRIPT_LEFT_MARGIN)
            .with_margin_right(20.)
            .with_background_color(surface.into_solid())
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                MESSAGE_CORNER_RADIUS,
            )))
            .finish()
    }

    /// Render an `AskUserQuestion` call as an interactive question card
    /// (PRODUCT §1). Each question shows its options as selectable rows (radio
    /// for single-select, checkbox for multi-select); a Send button submits the
    /// chosen answers as the next user turn. Controls are live only while the
    /// session is idle (not mid-stream) — a historical card just records the
    /// question (the answer the user already sent renders as the following user
    /// turn).
    fn render_question_card(
        &self,
        index: usize,
        input: &serde_json::Value,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let surface = theme.surface_2();
        let text_color = theme.main_text_color(surface).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = theme.accent().into_solid();
        let questions = parse_questions(input);
        let selected = self.question_selected.get(&index);
        let interactive = !self.streaming;

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(
                ConstrainedBox::new(
                    Icon::new(crate::ui_components::icons::Icon::HelpCircle.into(), accent)
                        .finish(),
                )
                .with_width(15.)
                .with_height(15.)
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .span("Question".to_owned())
                    .with_style(UiComponentStyles {
                        font_color: Some(text_color),
                        font_size: Some(13.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(12.)
            .with_child(header);

        for question in &questions {
            let mut block = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(6.);
            if !question.question.trim().is_empty() {
                block.add_child(
                    appearance
                        .ui_builder()
                        .span(question.question.clone())
                        .with_soft_wrap()
                        .with_style(UiComponentStyles {
                            font_color: Some(text_color),
                            font_size: Some(BODY_FONT_SIZE),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }
            for option in &question.options {
                let is_selected =
                    selected.is_some_and(|chosen| chosen.contains(&option.flat_index));
                let marker = match (question.multi, is_selected) {
                    (true, true) => "\u{2611}",  // ballot box with check
                    (true, false) => "\u{2610}", // ballot box
                    (false, true) => "\u{25C9}", // fisheye (filled radio)
                    (false, false) => "\u{25CB}", // circle
                };
                let mut option_row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(
                        appearance
                            .ui_builder()
                            .span(marker.to_owned())
                            .with_style(UiComponentStyles {
                                font_color: Some(if is_selected { accent } else { muted }),
                                font_size: Some(14.),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    );
                let mut label_col = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(2.)
                    .with_child(
                        appearance
                            .ui_builder()
                            .span(option.label.clone())
                            .with_soft_wrap()
                            .with_style(UiComponentStyles {
                                font_color: Some(text_color),
                                font_size: Some(13.),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    );
                if let Some(description) = &option.description {
                    if !description.trim().is_empty() {
                        label_col.add_child(
                            appearance
                                .ui_builder()
                                .span(description.clone())
                                .with_soft_wrap()
                                .with_style(UiComponentStyles {
                                    font_color: Some(muted),
                                    font_size: Some(11.5),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        );
                    }
                }
                option_row.add_child(label_col.finish());
                let mut row_container = Container::new(option_row.finish())
                    .with_padding_left(8.)
                    .with_padding_right(8.)
                    .with_padding_top(6.)
                    .with_padding_bottom(6.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
                if is_selected {
                    row_container = row_container.with_background_color(theme.surface_1().into_solid());
                }
                let row = row_container.finish();
                if interactive {
                    let mouse = self
                        .question_option_mouse
                        .borrow_mut()
                        .entry((index, option.flat_index))
                        .or_default()
                        .clone();
                    let option_index = option.flat_index;
                    let multi = question.multi;
                    block.add_child(
                        Hoverable::new(mouse, move |_| row)
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    ClaudeCodeViewAction::SelectQuestionOption {
                                        item: index,
                                        option: option_index,
                                        multi,
                                    },
                                );
                            })
                            .finish(),
                    );
                } else {
                    block.add_child(row);
                }
            }
            column.add_child(block.finish());
        }

        // The Send control is live only while idle and only once something is
        // chosen — a mid-stream or unanswered card offers no dead button
        // (PRODUCT §29).
        let has_selection = selected.is_some_and(|chosen| !chosen.is_empty());
        if interactive && has_selection {
            let mouse = self
                .question_submit_mouse
                .borrow_mut()
                .entry(index)
                .or_default()
                .clone();
            column.add_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        appearance
                            .ui_builder()
                            .button(ButtonVariant::Accent, mouse)
                            .with_text_label("Send answer".to_owned())
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    ClaudeCodeViewAction::SubmitQuestionAnswers(index),
                                );
                            })
                            .finish(),
                    )
                    .finish(),
            );
        }

        Container::new(column.finish())
            .with_padding(Padding::uniform(14.))
            .with_margin_top(4.)
            .with_margin_bottom(4.)
            .with_margin_left(TRANSCRIPT_LEFT_MARGIN)
            .with_margin_right(20.)
            .with_background_color(surface.into_solid())
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                MESSAGE_CORNER_RADIUS,
            )))
            .finish()
    }
}

/// One option of a parsed `AskUserQuestion` (PRODUCT §1). `flat_index` is the
/// option's position across *all* the card's questions, so the view's selection
/// set can address every option on the card with a single integer.
struct ParsedQuestionOption {
    flat_index: usize,
    label: String,
    description: Option<String>,
}

/// One parsed question from an `AskUserQuestion` tool call (PRODUCT §1).
struct ParsedQuestion {
    header: String,
    question: String,
    multi: bool,
    options: Vec<ParsedQuestionOption>,
}

/// Parse an `AskUserQuestion` tool input into its questions and options
/// (PRODUCT §1). Defensive like the rest of the ingest path: missing/garbled
/// fields degrade (a label-less option is skipped) rather than panic. The
/// `flat_index` counter runs across every option on the card so the view keys
/// selections by one integer.
fn parse_questions(input: &serde_json::Value) -> Vec<ParsedQuestion> {
    let mut flat_index = 0usize;
    let mut out = Vec::new();
    let Some(questions) = input.get("questions").and_then(|v| v.as_array()) else {
        return out;
    };
    for question in questions {
        let header = question
            .get("header")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let text = question
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let multi = question
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut options = Vec::new();
        if let Some(raw_options) = question.get("options").and_then(|v| v.as_array()) {
            for option in raw_options {
                let label = option
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
                if label.is_empty() {
                    continue;
                }
                let description = option
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                options.push(ParsedQuestionOption {
                    flat_index,
                    label,
                    description,
                });
                flat_index += 1;
            }
        }
        out.push(ParsedQuestion {
            header,
            question: text,
            multi,
            options,
        });
    }
    out
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

/// A one-line, length-bounded preview of a queued message's text (PRODUCT §54).
fn queue_preview(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    const MAX: usize = 80;
    if first_line.chars().count() > MAX {
        let truncated: String = first_line.chars().take(MAX).collect();
        format!("{truncated}…")
    } else {
        first_line.to_owned()
    }
}

/// Read an image file into an outgoing image block (PRODUCT §15b, §50–§51),
/// applying the inline-image size cap. `None` for non-images, oversized, or
/// unreadable files — the caller degrades those to a plain `@`-mention rather
/// than blocking the send (§29).
fn read_image_attachment(path: &Path) -> Option<OutgoingImage> {
    let media_type = composer::image_media_type(path)?;
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > composer::MAX_IMAGE_BYTES {
        log::info!(
            "claude: attachment {} exceeds the inline-image cap; left as a mention",
            path.display()
        );
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(OutgoingImage {
        media_type: media_type.to_owned(),
        base64_data: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

/// Format the per-turn metrics line (PRODUCT §48): cost, wall-clock duration,
/// time-to-first-token. Only fields the `result` carried appear — an absent one
/// is omitted, never shown as `0`. Returns `None` when nothing is present.
fn format_metrics_line(metrics: &TurnMetrics) -> Option<String> {
    // `4200` → `4.2s`; `850` → `850ms`. Sub-second stays in ms for precision.
    fn human_ms(ms: u64) -> String {
        if ms >= 1000 {
            format!("{:.1}s", ms as f64 / 1000.0)
        } else {
            format!("{ms}ms")
        }
    }
    let mut parts = Vec::new();
    if let Some(cost) = metrics.total_cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    if let Some(duration_ms) = metrics.duration_ms {
        parts.push(human_ms(duration_ms));
    }
    if let Some(ttft_ms) = metrics.ttft_ms {
        parts.push(format!("{} to first token", human_ms(ttft_ms)));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// A small, muted per-turn metrics line rendered after a turn's content
/// (PRODUCT §48). Session-local display only — twarp meters nothing (§34).
fn render_metrics_line(metrics: &TurnMetrics, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let Some(text) = format_metrics_line(metrics) else {
        return Container::new(Flex::column().finish()).finish();
    };
    Container::new(
        appearance
            .ui_builder()
            .span(text)
            .with_style(UiComponentStyles {
                font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_padding_top(2.)
    .with_padding_bottom(8.)
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

#[cfg(test)]
mod tests {
    use super::{format_metrics_line, queue_preview};
    use claude_code::TurnMetrics;

    #[test]
    fn queue_preview_takes_first_line_and_caps_length() {
        assert_eq!(queue_preview("hello world"), "hello world");
        assert_eq!(queue_preview("  first\nsecond"), "first");
        let long = "x".repeat(200);
        let preview = queue_preview(&long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), 81); // 80 + ellipsis
    }

    #[test]
    fn metrics_line_omits_absent_fields() {
        // Only duration present — no cost, no ttft segment.
        let line = format_metrics_line(&TurnMetrics {
            total_cost_usd: None,
            duration_ms: Some(4200),
            ttft_ms: None,
        });
        assert_eq!(line.as_deref(), Some("4.2s"));
    }

    #[test]
    fn metrics_line_joins_all_present_fields() {
        let line = format_metrics_line(&TurnMetrics {
            total_cost_usd: Some(0.0123),
            duration_ms: Some(4200),
            ttft_ms: Some(850),
        });
        assert_eq!(
            line.as_deref(),
            Some("$0.0123 · 4.2s · 850ms to first token")
        );
    }

    #[test]
    fn empty_metrics_render_no_line() {
        assert_eq!(format_metrics_line(&TurnMetrics::default()), None);
    }

    #[test]
    fn sub_second_durations_stay_in_milliseconds() {
        let line = format_metrics_line(&TurnMetrics {
            total_cost_usd: None,
            duration_ms: Some(850),
            ttft_ms: None,
        });
        assert_eq!(line.as_deref(), Some("850ms"));
    }
}
