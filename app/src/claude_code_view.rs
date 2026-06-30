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

mod background_scripts;
mod composer;
mod diff_cards;
mod inline_action;
mod repo_context;
mod thinking;
mod todos;
mod tool_cards;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::Sender;
use base64::Engine as _;
use claude_code::diff::diff_for_tool;
use claude_code::driver::{
    interrupt, send_control_response, send_interrupt, send_user_message, spawn_session, Child,
    OutgoingImage,
    OutgoingMessage, PermissionMode, SpawnOptions, SpawnedSession,
};
use claude_code::launch::LaunchOptions;
use claude_code::{sessions, Transcript, TranscriptEvent, TranscriptItem, TurnMetrics, Usage};
use futures::StreamExt;
use markdown_parser::{
    parse_markdown_with_gfm_tables, FormattedTable, FormattedText, FormattedTextInline,
    FormattedTextLine, TableAlignment,
};
use parking_lot::RwLock;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_editor::editor::NavigationKey;
use warpui::assets::asset_cache::AssetSource;
use warpui::clipboard::ClipboardContent;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::platform::FilePickerConfiguration;
use warpui::r#async::Timer;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::slider::SliderStateHandle;
use warpui::{
    elements::{
        Align, Border, CacheOption, ChildAnchor, Clipped, ClippedScrollStateHandle,
        ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
        DispatchEventResult, DropShadow, Element, EventDispatchMode, EventHandler, Expanded, Fill,
        Flex, FormattedTextElement, HighlightedHyperlink, Hoverable, HyperlinkUrl, Icon, Image,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding,
        ParentAnchor, ParentElement, ParentOffsetBounds, PositionedElementAnchor,
        PositionedElementOffsetBounds, PulsingIcon, PulsingIconStateHandle, Radius, SavePosition,
        ScrollTarget, ScrollToPositionMode, ScrollbarWidth, SelectableArea, SelectionHandle,
        Shrinkable, Stack,
    },
    platform::Cursor,
    presenter::ChildView,
    text_layout::ClipConfig,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use self::background_scripts::{BackgroundScript, BackgroundScriptState};
use self::composer::{SuggestionKind, SuggestionQuery};
use self::diff_cards::DiffCard;
use self::repo_context::{CiState, RepoContext};
use self::thinking::ThinkingUi;
use self::tool_cards::{render_tool_card, ToolCardUi};
use crate::appearance::Appearance;
use crate::claude_code_session_defaults::ClaudeSessionDefaultsModel;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent, PaneHeaderAction,
};
#[cfg(all(feature = "local_fs", feature = "local_tty"))]
use crate::terminal::local_shell::LocalShellState;
use crate::terminal::{view::Event as TerminalViewEvent, TerminalManager, TerminalView};
use crate::util::path::{resolve_executable, resolve_executable_in_path};
use crate::workspace::{WorkspaceAction, WorkspaceRegistry};

/// The executable the pane drives. Resolved on `PATH`; its absence is the
/// unavailable state (PRODUCT §4).
const CLAUDE_BINARY: &str = "claude";

/// Pane title (PRODUCT §5) — drives the tab label via [`PaneConfiguration`].
const PANE_TITLE: &str = "Claude Code";

/// Avatar glyphs for the message rows (the Agent-Mode shape: icon + body).
const USER_ICON_SVG_PATH: &str = "bundled/svg/user.svg";
const ASSISTANT_ICON_SVG_PATH: &str = "bundled/svg/claude.svg";
/// Branch glyph for the hover "Fork" affordance below an assistant response.
const FORK_ICON_SVG_PATH: &str = "bundled/svg/git-branch-02.svg";
/// Down-chevron for the floating "scroll to bottom" button (shown above the
/// composer's right edge while the transcript is scrolled up off the bottom).
const SCROLL_TO_BOTTOM_ICON_SVG_PATH: &str = "bundled/svg/chevron-down.svg";

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
/// Width cap for the floating background-scripts panel (twarp): a compact status
/// card pinned to the pane's top-right, narrow enough to clear the centered
/// reading column behind it.
const BACKGROUND_PANEL_MAX_WIDTH: f32 = 360.;
/// Width cap for a composer pill's dropdown popover (permission / model /
/// effort / context / branch / CI / PR). Keeps the menu a compact card anchored
/// to its pill rather than stretching to the pane edge (issue #1).
const COMPOSER_MENU_MAX_WIDTH: f32 = 320.;
/// Height of the clearance spacer between the last message and the end-of-
/// transcript sentinel, so the newest message scrolls fully clear of the
/// floating composer (which floats over the bottom of the pane) rather than
/// tucking behind it. Sized generously past the composer's resting height.
const COMPOSER_CLEARANCE: f32 = 200.;
/// Height of the bottom gradient fade band (§13). Runs from transparent at its
/// top to the opaque pane background at the bottom, tall enough that transcript
/// content scrolling toward the floating composer dissolves into the background
/// well above the composer's top edge instead of meeting a hard cut.
const TRANSCRIPT_FADE_HEIGHT: f32 = 150.;
/// Horizontal gutter inside the transcript scroller. Lives *inside* the
/// scrollable so the overlay scrollbar can hug the pane's right edge while the
/// prose keeps its breathing room.
const TRANSCRIPT_GUTTER: f32 = 16.;
/// Position id of the zero-height sentinel pinned to the end of the transcript.
/// Bottom-stick auto-scroll (PRODUCT §14) scrolls this into view to follow
/// streaming output and to open a resumed session at its latest message.
const TRANSCRIPT_BOTTOM_POSITION_ID: &str = "claude_transcript_bottom";
const COMPOSER_CORNER_RADIUS: f32 = 14.;
/// Slack (px) for the streaming follow-to-bottom check. While following, the
/// view sits exactly at the bottom; this only absorbs sub-pixel/line-height
/// rounding so a genuine upward scroll (tens of px) reliably pauses the follow.
const AUTOSCROLL_STICK_SLACK: f32 = 16.;
const MESSAGE_CORNER_RADIUS: f32 = 12.;
/// Cap the outgoing (user) iMessage bubble so long messages wrap into a column
/// hugging the right edge instead of stretching across the whole transcript.
const USER_BUBBLE_MAX_WIDTH: f32 = 520.;
/// Side length of a sent-image preview thumbnail (#8): a fixed square so
/// attachments sit as uniform tiles above the message bubble.
const SENT_IMAGE_SIZE: f32 = 120.;
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
        /// The resolved `claude` executable, an absolute path whenever it can
        /// be found on the captured login-shell PATH (PRODUCT §43). Resolved
        /// here (the view holds `interactive_path`) rather than in the pane,
        /// which only sees the launchd-minimal process PATH under a GUI launch
        /// and would fall back to a bare `claude` — a bare token gets eaten by
        /// the `claude`-at-submit trigger, so the CLI never starts and the pane
        /// shows an empty terminal (the release-only "Raw CLI does nothing").
        claude_binary: String,
        /// The alias-derived default flags (`--effort` / `--model` /
        /// `--permission-mode`) to re-apply on the raw-CLI command line. Raw
        /// mode launches the binary by absolute path (PRODUCT §43), which
        /// sidesteps the shell alias that supplied those defaults — so without
        /// passing them explicitly the CLI falls back to its own config
        /// defaults (e.g. xhigh effort instead of the alias's `--effort
        /// medium`), and the two modes disagree on an empty chat. Pre-joined
        /// and shell-quoted by the view, which owns the parsed flags.
        flags: String,
    },
    /// twarp: fork this conversation into a new pane (PRODUCT "Fork
    /// conversation"; Claude's `--fork-session`). The view has already written
    /// the truncated branch file under `resume.session_id`; the pane group
    /// opens it as a resumed session in a fresh split, inheriting the parent
    /// pane's `launch` settings (model / effort / permission mode) and `cwd`.
    ForkSession {
        resume: ResumeSession,
        launch: LaunchOptions,
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
    /// twarp: jump the transcript to its latest message — the floating
    /// "scroll to bottom" button shown above the composer when the user has
    /// scrolled up off the bottom.
    ScrollToBottom,
    /// Expand / collapse the tool card with this tool-use id (PRODUCT §19).
    ToggleToolCard(String),
    /// Expand / collapse the thinking card at this transcript index
    /// (PRODUCT §22).
    ToggleThinking(usize),
    /// Expand / collapse the task list (PRODUCT §23).
    ToggleTodos,
    /// Pick a permission mode from the dropdown (PRODUCT §25, #2). Applies to
    /// the next spawn; a live session is re-attached via `--resume` so the
    /// conversation continues under the new mode.
    SetPermissionMode(PermissionMode),
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
    /// The OS is dragging file(s) over (or away from) the pane (7l): toggles the
    /// composer's drag-sensing highlight. `true` only while the cursor is inside
    /// the pane bounds.
    SetDragActive(bool),
    /// Open the OS file picker to attach files (PRODUCT §51, 7l) — the
    /// composer's "＋ attach" control.
    AttachFromPicker,
    /// Remove a direct attachment chip (paste / drop / picker) by index
    /// (PRODUCT §49–§51, 7l).
    RemoveDirectAttachment(usize),
    /// twarp: open a sent-image preview (#8) in the OS default image app —
    /// a click on the thumbnail above a user turn reveals the full image.
    OpenSentImage(String),
    /// Open / close one of the composer dropdowns (model / effort / context /
    /// branch / CI / PR), #13. Re-dispatching the open menu closes it.
    ToggleComposerMenu(ComposerMenu),
    /// Close whatever composer dropdown is open (#13). Dispatched by the menu's
    /// click-outside [`Dismiss`] scrim.
    CloseComposerMenu,
    /// twarp: copy a string (branch name, PR URL, …) to the clipboard and close
    /// the open menu.
    CopyToClipboard(String),
    /// twarp: open the current branch on GitHub (`…/tree/<branch>`).
    OpenBranchInGitHub,
    /// twarp: check out a different local branch from the branch menu, then
    /// refresh the context bar. Runs `git checkout <name>` in the cwd.
    CheckoutBranch(String),
    /// twarp: open a PR for the current branch (`gh pr create --web`) when none
    /// exists yet — the "Create PR" pill.
    CreatePr,
    /// twarp: toggle the "start new chats in a worktree" preference (#11). When
    /// on, the first turn spawns its session in a fresh worktree at `../<name>`.
    ToggleWorktree,
    /// Pick a model from the dropdown (#13; `None` = let `claude` choose).
    /// Detaches a live session so the next message resumes under it.
    SetModel(Option<String>),
    /// Set the effort from the slider (#13; `None` = the CLI's default).
    SetEffort(Option<String>),
    /// Remove the queued (type-ahead) message at this index before it
    /// dispatches (PRODUCT §54, 7m).
    RemoveQueuedMessage(usize),
    /// Toggle the full-text expansion of the queued message at this index
    /// (clicking a queue row reveals the whole prompt, not just the preview).
    ToggleQueuedMessage(usize),
    /// Dispatch the queued message at this index immediately rather than
    /// waiting for the in-flight turn to drain it: while streaming it jumps to
    /// the front of the queue (sent next); when idle it ships right away.
    SendQueuedMessageNow(usize),
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
    /// Answer a `can_use_tool` permission prompt over the control channel (7g,
    /// PRODUCT §24): `request_id` is the control-protocol id; `allow` proceeds,
    /// otherwise the action is denied and the session continues.
    AnswerPermission { request_id: String, allow: bool },
    /// Submit the chosen answers for a control-channel question card (the
    /// `request_user_dialog` path, 7g/§24) at this transcript index. Releases the
    /// dialog and sends the picks as the next turn (PRODUCT §26: never hang).
    SubmitQuestionDialog(usize),
    /// twarp: resume the recent session at this index in the zero-state
    /// "Welcome back" panel — loads its stored history into this pane and
    /// re-attaches it, the same as a sidebar resume but in place.
    ResumeRecentSession(usize),
    /// twarp: fork the conversation at the assistant response with this
    /// transcript index ("Fork conversation" — Claude's `--fork-session`).
    /// Branches the session up to that turn into a new pane to the right,
    /// leaving this one untouched. Shown on hover below a response.
    ForkConversation(usize),
    /// twarp: expand / collapse the floating background-scripts panel — the
    /// pill listing this chat's `run_in_background` Bash launches.
    ToggleBackgroundPanel,
    /// twarp: expand / collapse one background-script row's captured output,
    /// keyed by the launching tool-use id.
    ToggleBackgroundScript(String),
    /// twarp: expand / collapse one server's tool list in the MCP viewer popover
    /// (feature 13). Only one server is expanded at a time; re-dispatching the
    /// open server collapses it.
    ToggleMcpServer(String),
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
    /// twarp: expand / collapse the floating background-scripts panel from the
    /// header's icon button (left of the Chat UI / Raw CLI toggle). The button
    /// lives in the parent `PaneView`'s header tree, so it routes through the
    /// pane framework rather than dispatching the in-pane
    /// [`ClaudeCodeViewAction::ToggleBackgroundPanel`] directly.
    ToggleBackgroundPanel,
}

/// Which composer dropdown / popover is open (#13). The model picker, the
/// effort slider, and the context-usage breakdown each open above the input;
/// only one is open at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposerMenu {
    Permission,
    Model,
    Effort,
    Context,
    /// The branch pill's menu: copy name, open on GitHub, switch branch (#11).
    Branch,
    /// The CI pill's menu: one row per status check (#11).
    Ci,
    /// The PR pill's menu when a PR exists: open / copy URL (#11).
    Pr,
    /// The MCP pill's menu: read-only list of the session's MCP servers and
    /// their tools (feature 13).
    Mcp,
}

impl ComposerMenu {
    /// The `SavePosition` id of this menu's trigger pill, so the floating
    /// overlay can anchor to it.
    fn anchor_id(self) -> &'static str {
        match self {
            ComposerMenu::Permission => "claude_pill_permission",
            ComposerMenu::Model => "claude_pill_model",
            ComposerMenu::Effort => "claude_pill_effort",
            ComposerMenu::Context => "claude_pill_context",
            ComposerMenu::Branch => "claude_pill_branch",
            ComposerMenu::Ci => "claude_pill_ci",
            ComposerMenu::Pr => "claude_pill_pr",
            ComposerMenu::Mcp => "claude_pill_mcp",
        }
    }

    /// Whether this menu's trigger pill lives in the context bar *above* the
    /// input (branch / CI / PR) — those menus drop *downward*; the bottom
    /// control pills (permission / model / effort / context) open *upward*.
    fn opens_downward(self) -> bool {
        matches!(
            self,
            ComposerMenu::Branch | ComposerMenu::Ci | ComposerMenu::Pr
        )
    }
}

/// A stored session to reopen (PRODUCT §36, sub-phase 7h): the pane renders
/// the on-disk history up front and continues the conversation live via
/// `claude --resume <session_id>` on the next message.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// One thing to write to the live `claude` process's stdin. The background
/// writer task owns stdin exclusively, so user turns (7c) and control-protocol
/// answers (7g — permission Allow/Deny, question replies, PRODUCT §24) share the
/// one channel instead of racing two writers.
enum StdinCommand {
    /// A user turn (`send_user_message`).
    Turn(OutgoingMessage),
    /// An answer to a `control_request` (`send_control_response`): the
    /// `request_id` to echo and the decision payload.
    Control {
        request_id: String,
        response: serde_json::Value,
    },
    /// Interrupt the in-flight turn (`send_interrupt`) — the session-preserving
    /// Stop (PRODUCT §11). `request_id` is echoed on the acknowledgement.
    Interrupt { request_id: String },
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
    /// The window this pane lives in (#10/#11), so render can look up the
    /// active tab's colour to theme the UI.
    window_id: WindowId,
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
    /// Drag-to-select state for the transcript (PRODUCT §13: the prose is
    /// read-only, but the user still needs to highlight + copy it). The
    /// [`SelectableArea`] wrapping the transcript drives the gesture; the
    /// selected text is mirrored into `transcript_selection` so a Copy
    /// (Cmd+C with an empty composer, surfaced as [`EditorEvent::Copy`]) can
    /// write it to the clipboard. `set_selectable(true)` on the text elements
    /// alone is inert without this coordinator.
    selection_handle: SelectionHandle,
    transcript_selection: Arc<RwLock<Option<String>>>,
    /// The live `claude` child, once a session is running. Kept for `interrupt`
    /// (Stop, PRODUCT §11) and to kill the process on drop (PRODUCT §8 —
    /// `spawn_session` sets `kill_on_drop`).
    child: Option<Child>,
    /// Sends user turns and control-protocol answers to the background task that
    /// owns the process stdin (PRODUCT §16, §24). `None` until a session is
    /// running.
    message_tx: Option<Sender<StdinCommand>>,
    /// True while `claude` is producing output for the current turn (PRODUCT §9):
    /// the composer shows Stop and sending is disabled until the turn ends.
    streaming: bool,
    /// True between a user Stop and the turn's terminal event (PRODUCT §11). The
    /// interrupt makes `claude` end the turn with an error `result`, but the
    /// session stays alive — this flag re-labels that terminal event as a clean
    /// `Interrupted` so it shows the "Interrupted." notice instead of a spurious
    /// error and keeps the session resumable.
    interrupt_pending: bool,
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
    /// Mouse state for the header's Raw CLI / Chat UI section toggle (PRODUCT
    /// §39, #7). One handle per segment.
    raw_cli_button: MouseStateHandle,
    chat_ui_button: MouseStateHandle,
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
    /// The `/` slash-command catalogue (names + descriptions), scanned lazily
    /// on the first `/` from the on-disk skill dirs so skills + their
    /// descriptions show before the session's first `init` (issues #2/#3).
    slash_command_index: Option<composer::SlashCommandIndex>,
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
    /// twarp: mouse state for the floating "scroll to bottom" button, shown
    /// above the composer's right edge whenever the transcript is scrolled up
    /// off the bottom (a no-op affordance once the view follows the latest
    /// message).
    scroll_to_bottom_button: MouseStateHandle,
    /// Type-ahead queue (PRODUCT §53–§54, 7m): messages sent while a turn
    /// streams wait here and dispatch in order as each turn completes.
    message_queue: Vec<OutgoingMessage>,
    /// Stable per-row mouse handles for the expand-on-click queue rows (§54).
    queue_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Per-row mouse handles for the queue rows' "✕ remove" buttons (§54).
    queue_remove_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Per-row mouse handles for the queue rows' "send now" buttons (§54).
    queue_send_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Indices of queue rows currently expanded to their full prompt text.
    /// Cleared whenever the queue is mutated so it never points at a stale row.
    queue_expanded: std::collections::HashSet<usize>,
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
    /// Stable mouse handles for the permission card's Allow / Deny buttons (7g,
    /// PRODUCT §24), keyed by the control-protocol `request_id`, created on
    /// demand so the two buttons keep distinct hover state across renders.
    permission_button_mouse: std::cell::RefCell<HashMap<(String, bool), MouseStateHandle>>,
    /// Animation state for the composer's shimmering "Working…" indicator (#9):
    /// the Claude-app-style loading shimmer shown while a turn streams. Persisted
    /// across renders so the shimmer keeps a continuous phase; the element
    /// self-schedules repaints while it's on screen.
    working_shimmer: ShimmeringTextStateHandle,
    /// Animation phase for the Claude glyph beside the streaming "Working…"
    /// status: a gentle opacity pulse signalling the turn is live. Persisted
    /// across renders so the pulse keeps a continuous clock; the element
    /// self-schedules repaints while it's on screen.
    working_icon_pulse: PulsingIconStateHandle,
    /// Folder / branch / diff / PR / CI shown in the composer context bar (#11).
    /// `None` until the first async `git`/`gh` probe resolves; refreshed on open
    /// and after each turn.
    repo_context: Option<RepoContext>,
    /// The open composer dropdown / popover (#13), or `None`.
    composer_menu: Option<ComposerMenu>,
    /// `true` while the OS is dragging file(s) over the pane (PRODUCT §50, 7l):
    /// the composer lights up with an accent border + "Drop to attach" hint so
    /// the drop target is obvious before release. Cleared on drop or drag-exit.
    drag_active: bool,
    /// Slider state for the effort picker (#13). Persisted so the thumb keeps
    /// its position across renders.
    effort_slider: SliderStateHandle,
    /// Mouse state for the clickable context-usage chip that opens the context
    /// breakdown popover (#13).
    context_button: MouseStateHandle,
    /// Pooled mouse handles for the model-picker rows (#13).
    model_menu_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Pooled mouse handles for the permission-picker rows (#2).
    permission_menu_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Mouse state for the MCP viewer pill (feature 13).
    mcp_pill_mouse: MouseStateHandle,
    /// Pooled mouse handles for the MCP popover's server rows (feature 13).
    mcp_menu_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Which MCP server row is expanded in the viewer popover, if any
    /// (feature 13). One at a time; ephemeral UI state, not persisted.
    mcp_expanded_server: Option<String>,
    /// When the current streaming turn started (#7), for the live elapsed in
    /// the status line below the last message. `None` when idle.
    turn_started: Option<Instant>,
    /// Preview image paths for sent user turns (#8), keyed by the user item's
    /// transcript index — rendered above that message's bubble. Only covers
    /// images sent this session (resumed history has no previews).
    sent_images: HashMap<usize, Vec<PathBuf>>,
    /// Pooled mouse handles for the clickable sent-image thumbnails (#8),
    /// keyed by path string so hover state survives across renders.
    sent_image_mouse: std::cell::RefCell<HashMap<String, MouseStateHandle>>,
    /// The tab-derived accent (#10) and its faint wash (#11), cached at the top
    /// of each `render` so the deep render tree and free helper functions can
    /// theme to the tab colour without threading `app` everywhere.
    render_accent: std::cell::Cell<ColorU>,
    render_wash: std::cell::Cell<ColorU>,
    /// twarp: recent stored sessions for this pane's cwd, listed in the
    /// zero-state "Welcome back" panel so the empty pane is a launchpad — pick
    /// up a recent session or start fresh by typing. Captured once in `new`
    /// (the zero state only shows before the first turn) and dropped from view
    /// the moment the transcript has content.
    recent_sessions: Vec<sessions::StoredSession>,
    /// Pooled mouse handles for the zero-state recent-session rows.
    recent_session_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Pooled mouse handles, per transcript index, for the hover "Fork"
    /// affordance below assistant responses: `fork_row_mouse` senses the hover
    /// over the response block, `fork_button_mouse` drives the button itself.
    fork_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    fork_button_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Mouse handles for the context-bar pills (#11): branch / CI / PR / the
    /// Create-PR button / the worktree toggle.
    branch_pill_mouse: MouseStateHandle,
    ci_pill_mouse: MouseStateHandle,
    pr_pill_mouse: MouseStateHandle,
    worktree_toggle_mouse: MouseStateHandle,
    /// Pooled mouse handles for the branch / CI menu rows (#11).
    branch_menu_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    ci_menu_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// twarp (#11): when on, the first turn of a not-yet-started session spawns
    /// `claude` in a fresh git worktree at `../<branch>` instead of the cwd, so
    /// the work happens on an isolated branch. A per-pane toggle, reset only by
    /// the user; ignored once a session is live (`message_tx` set).
    use_worktree: bool,
    /// twarp: shell-style draft history. Every submitted message text is pushed
    /// here (oldest first); Up/Down in the composer recall older / newer entries
    /// (like a terminal). `history_cursor` is the position being browsed (`None`
    /// = the live draft, not in history); `history_draft` stashes the in-progress
    /// text when navigation starts so Down past the newest entry restores it.
    message_history: Vec<String>,
    history_cursor: Option<usize>,
    history_draft: String,
    /// twarp: whether the floating background-scripts panel is expanded. The
    /// panel lists this chat's `run_in_background` Bash launches (derived from
    /// the transcript each render via [`background_scripts::collect`]); collapsed
    /// by default to a compact pill so it never crowds the conversation.
    background_scripts_expanded: bool,
    /// Per-script output disclosure, keyed by the launching tool-use id. A
    /// script id is present iff the user expanded that row's captured output.
    background_expanded_rows: HashSet<String>,
    /// Stable per-row mouse handles for the background-script rows, keyed by
    /// launch id and created on demand (the `sent_image_mouse` pattern) so hover
    /// state survives across renders even though the rows are derived.
    background_row_mouse: std::cell::RefCell<HashMap<String, MouseStateHandle>>,
    /// Mouse state for the panel's header (the collapse/expand toggle).
    background_panel_mouse: MouseStateHandle,
    /// Mouse state for the header's background-scripts icon button (left of the
    /// Chat UI / Raw CLI toggle) that opens the floating panel.
    background_button_mouse: MouseStateHandle,
    /// Memoized [`background_scripts::collect`] output, keyed by the transcript
    /// revision it was derived from. The list is recomputed (walking the whole
    /// transcript, descending into Task children) by both the header button and
    /// the floating panel; without this it ran twice per render. `Rc` so the
    /// two readers share one allocation; invalidated whenever the transcript's
    /// revision moves on.
    background_scripts_memo:
        std::cell::RefCell<Option<(u64, std::rc::Rc<Vec<background_scripts::BackgroundScript>>)>>,
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

        // twarp 07: a fresh pane inherits the PREVIOUS session's settings. Any
        // setting the invocation didn't pin (typed flags, or the alias during
        // the first-run bootstrap) falls back to the persisted last-used store;
        // the effective settings are then recorded back as the new last-used so
        // the next pane — and a crash-restored pane — inherits them in turn.
        let stored = ClaudeSessionDefaultsModel::as_ref(ctx).get().cloned();
        let model = model.or_else(|| stored.as_ref().and_then(|s| s.model.clone()));
        let effort = effort.or_else(|| stored.as_ref().and_then(|s| s.effort.clone()));
        let permission_mode = permission_mode
            .or_else(|| stored.as_ref().and_then(|s| s.permission_mode))
            .unwrap_or(PermissionMode::Default);
        ClaudeSessionDefaultsModel::handle(ctx).update(ctx, |defaults, ctx| {
            defaults.record(model.clone(), effort.clone(), permission_mode, ctx);
        });

        let input_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                autogrow: true,
                soft_wrap: true,
                text: TextOptions::ui_font_size(appearance),
                // Surface Tab / Shift+Tab and the arrow keys to the view so the
                // composer can accept a `/` or `@` suggestion with Tab and cycle
                // the list / recall draft history with the arrows. `AtBoundary`
                // keeps multi-line editing intact — the arrows only propagate
                // once the cursor can't move further within the draft. This is
                // the menu-closed default; `sync_navigation_propagation` flips it
                // to `Always` while a suggestion menu is open so the arrows cycle
                // the menu even mid-draft.
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::AtBoundary,
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
            message_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            transcript: Transcript::new(),
            window_id: ctx.window_id(),
            input_editor,
            pane_configuration,
            focus_handle: None,
            cwd,
            interactive_path: None,
            scroll_state: ClippedScrollStateHandle::default(),
            selection_handle: Default::default(),
            transcript_selection: Default::default(),
            child: None,
            message_tx: None,
            streaming: false,
            interrupt_pending: false,
            submit_button: MouseStateHandle::default(),
            refresh_button: MouseStateHandle::default(),
            stop_button: MouseStateHandle::default(),
            tool_card_ui: HashMap::new(),
            diff_cards: HashMap::new(),
            thinking_ui: HashMap::new(),
            todos_expanded: true,
            todos_header_mouse: MouseStateHandle::default(),
            permission_mode,
            permission_pill_mouse: MouseStateHandle::default(),
            model,
            effort,
            resume_session_id: None,
            session_id,
            raw_cli_button: MouseStateHandle::default(),
            chat_ui_button: MouseStateHandle::default(),
            raw_cli: None,
            suggestion_query: None,
            suggestions: Vec::new(),
            suggestion_selected: 0,
            suggestion_row_mouse: std::cell::RefCell::new(Vec::new()),
            cwd_files: None,
            slash_command_index: None,
            pending_images: Vec::new(),
            attachment_optouts: HashSet::new(),
            attachment_chip_mouse: std::cell::RefCell::new(Vec::new()),
            direct_attachments: Vec::new(),
            direct_chip_mouse: std::cell::RefCell::new(Vec::new()),
            attach_button: MouseStateHandle::default(),
            scroll_to_bottom_button: MouseStateHandle::default(),
            message_queue: Vec::new(),
            queue_row_mouse: std::cell::RefCell::new(Vec::new()),
            queue_remove_mouse: std::cell::RefCell::new(Vec::new()),
            queue_send_mouse: std::cell::RefCell::new(Vec::new()),
            queue_expanded: std::collections::HashSet::new(),
            model_pill_mouse: MouseStateHandle::default(),
            effort_pill_mouse: MouseStateHandle::default(),
            plan_approve_mouse: MouseStateHandle::default(),
            plan_keep_mouse: MouseStateHandle::default(),
            session_epoch: 0,
            question_selected: HashMap::new(),
            question_option_mouse: std::cell::RefCell::new(HashMap::new()),
            question_submit_mouse: std::cell::RefCell::new(HashMap::new()),
            permission_button_mouse: std::cell::RefCell::new(HashMap::new()),
            working_shimmer: ShimmeringTextStateHandle::new(),
            working_icon_pulse: PulsingIconStateHandle::new(),
            repo_context: None,
            composer_menu: None,
            drag_active: false,
            effort_slider: SliderStateHandle::default(),
            context_button: MouseStateHandle::default(),
            model_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            permission_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            mcp_pill_mouse: MouseStateHandle::default(),
            mcp_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            mcp_expanded_server: None,
            turn_started: None,
            sent_images: HashMap::new(),
            sent_image_mouse: std::cell::RefCell::new(HashMap::new()),
            render_accent: std::cell::Cell::new(
                Appearance::as_ref(ctx).theme().accent().into_solid(),
            ),
            render_wash: std::cell::Cell::new(ColorU::new(0, 0, 0, 0)),
            recent_sessions: Vec::new(),
            recent_session_mouse: std::cell::RefCell::new(Vec::new()),
            fork_row_mouse: std::cell::RefCell::new(Vec::new()),
            fork_button_mouse: std::cell::RefCell::new(Vec::new()),
            branch_pill_mouse: MouseStateHandle::default(),
            ci_pill_mouse: MouseStateHandle::default(),
            pr_pill_mouse: MouseStateHandle::default(),
            worktree_toggle_mouse: MouseStateHandle::default(),
            branch_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            ci_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            use_worktree: false,
            background_scripts_expanded: false,
            background_expanded_rows: HashSet::new(),
            background_row_mouse: std::cell::RefCell::new(HashMap::new()),
            background_panel_mouse: MouseStateHandle::default(),
            background_button_mouse: MouseStateHandle::default(),
            background_scripts_memo: std::cell::RefCell::new(None),
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
            let started = Instant::now();
            view.streaming = true;
            view.turn_started = Some(started);
            view.schedule_elapsed_tick(started, ctx);
            view.begin_session(Some(OutgoingMessage::text(prompt)), ctx);
        }

        // twarp: a bare `claude` opens to the zero state — load the cwd's recent
        // sessions so the empty pane doubles as a launchpad (pick one up, or type
        // to start fresh). Skipped when the pane already has content (a resumed
        // pane or `claude <prompt>`), where the transcript replaces the panel.
        if view.transcript.is_empty() {
            view.recent_sessions = view
                .cwd
                .as_deref()
                .map(sessions::list_sessions)
                .unwrap_or_default();
        }

        // #7: name the tab from the first user message (resumed history or the
        // `claude <prompt>` first turn); stays "Claude Code" for a bare `claude`
        // until the user sends something.
        view.update_pane_title(ctx);

        // #11: populate the composer context bar (folder / branch / diff / PR /
        // CI) for the pane's directory.
        view.refresh_repo_context(ctx);

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

    /// The Claude session id this view is bound to. Every pane has one from
    /// birth (a resumed pane adopts the resumed id, a fresh pane mints a UUID),
    /// so this is safe to match on to detect a session already open in a pane.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The embedded raw-CLI terminal view while raw mode is active (#14).
    /// The host [`ClaudeCodePane`] checks this directly for focus: the terminal
    /// is created by the pane group, not as a structural child of this view, so
    /// the usual layout-ancestor focus check on the pane can miss it — which
    /// left Cmd+W targeting whatever pane was focused before. Checking the
    /// terminal handle itself is layout-timing-independent.
    pub(crate) fn raw_cli_view(&self) -> Option<ViewHandle<TerminalView>> {
        self.raw_cli.as_ref().map(|raw| raw.view.clone())
    }

    /// The message input editor (#13). The host pane checks it directly for
    /// focus: in chat mode the editor holds keyboard focus, and like the raw-CLI
    /// terminal its focus can be missed by the pane's layout-ancestor check
    /// right after the pane opens (before the first layout). Checking the editor
    /// handle itself is layout-timing-independent, so Cmd+W / maximize target
    /// this pane while the chat is focused.
    pub(crate) fn input_editor_view(&self) -> ViewHandle<EditorView> {
        self.input_editor.clone()
    }

    /// The active tab's colour (#10/#11) resolved to a `ColorU`, or `None` when
    /// the tab has no colour. Looked up via the window's workspace.
    fn tab_accent(&self, app: &AppContext) -> Option<ColorU> {
        let workspace = WorkspaceRegistry::as_ref(app).get(self.window_id, app)?;
        let identifier = workspace.as_ref(app).active_tab_color()?;
        let theme = Appearance::as_ref(app).theme();
        Some(ColorU::from(
            identifier.to_tab_color(&theme.terminal_colors().normal),
        ))
    }

    /// The pane's primary/accent colour (#10): the active tab's colour when set,
    /// otherwise the theme accent. Used for the primary button, the Claude
    /// glyph, links, progress, and selection highlights.
    fn accent(&self, app: &AppContext) -> ColorU {
        self.tab_accent(app)
            .unwrap_or_else(|| Appearance::as_ref(app).theme().accent().into_solid())
    }

    /// A faint wash of the accent (#11) for the "gray" highlight fills — pill
    /// backgrounds, the context bar, selected rows — so they read as tinted by
    /// the tab colour rather than neutral gray.
    fn accent_wash(&self, app: &AppContext) -> ColorU {
        let accent = self.accent(app);
        ColorU::new(accent.r, accent.g, accent.b, 52)
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
        let fut = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_path_env_var(ctx)
        });
        ctx.spawn(fut, |me, path, ctx| {
            if path.is_some() && me.interactive_path != path {
                me.interactive_path = path;
                // The constructor's first `refresh_repo_context` ran before this
                // PATH was available, so its `git`/`gh` probe used the fallback
                // login-shell PATH — which on some setups lacks `git` (homebrew /
                // Xcode location), leaving every git-derived field empty (only the
                // cwd-derived folder pill resolved). Re-run the probe now that the
                // richer interactive PATH is known, so the branch pill and the
                // worktree toggle (both gated on a resolved branch) appear on a
                // fresh chat.
                me.refresh_repo_context(ctx);
                ctx.notify();
            }
        });
    }

    #[cfg(not(all(feature = "local_fs", feature = "local_tty")))]
    fn capture_interactive_path(_ctx: &mut ViewContext<Self>) {}

    /// Refresh the composer context bar (#11): run `git`/`gh` in the user's
    /// login shell (so they resolve and the right repo/PR are visible) and store
    /// the parsed folder / branch / diff / PR / CI. Best-effort and off the main
    /// thread — a missing repo, absent `gh`, or a slow network call just leaves
    /// the bar partial or unchanged. Called on open and after each turn (the
    /// agent may have edited files, committed, or pushed).
    #[cfg(all(feature = "local_fs", feature = "local_tty"))]
    fn refresh_repo_context(&self, ctx: &mut ViewContext<Self>) {
        use crate::terminal::local_shell::execute_command;
        let Some(cwd) = self.cwd.clone().or_else(|| std::env::current_dir().ok()) else {
            return;
        };
        let folder = repo_context::folder_name(&cwd);
        let command = repo_context::build_command(&cwd);
        let shell_state = LocalShellState::as_ref(ctx);
        let Some(shell) = shell_state.local_shell_info() else {
            return;
        };
        let shell_type = shell.get_shell_type();
        let shell_path = shell.get_shell_path().clone();
        let path_env = self
            .interactive_path
            .clone()
            .or_else(|| shell_state.login_shell_path_env());
        let fut = async move {
            execute_command(shell_type, shell_path, path_env, &command)
                .await
                .ok()
        };
        ctx.spawn(fut, move |me, output, ctx| {
            let context = match output {
                Some(out) => repo_context::parse(&out, folder),
                None => RepoContext {
                    folder,
                    ..Default::default()
                },
            };
            me.repo_context = Some(context);
            ctx.notify();
        });
    }

    #[cfg(not(all(feature = "local_fs", feature = "local_tty")))]
    fn refresh_repo_context(&self, _ctx: &mut ViewContext<Self>) {}

    /// Run a one-off `git`/`gh` command in the user's login shell from the cwd,
    /// then refresh the context bar (#11). Best-effort: any failure leaves the
    /// bar unchanged. Shared by the branch-switch and Create-PR menu actions.
    #[cfg(all(feature = "local_fs", feature = "local_tty"))]
    fn run_repo_command(&self, command: String, ctx: &mut ViewContext<Self>) {
        use crate::terminal::local_shell::execute_command;
        let Some(cwd) = self.cwd.clone().or_else(|| std::env::current_dir().ok()) else {
            return;
        };
        let dir = cwd.to_string_lossy().replace('\'', r"'\''");
        let command = format!("cd '{dir}' 2>/dev/null || exit 0\n{command}\n");
        let shell_state = LocalShellState::as_ref(ctx);
        let Some(shell) = shell_state.local_shell_info() else {
            return;
        };
        let shell_type = shell.get_shell_type();
        let shell_path = shell.get_shell_path().clone();
        let path_env = self
            .interactive_path
            .clone()
            .or_else(|| shell_state.login_shell_path_env());
        let fut = async move {
            let _ = execute_command(shell_type, shell_path, path_env, &command).await;
        };
        ctx.spawn(fut, move |me, _output, ctx| {
            me.refresh_repo_context(ctx);
        });
    }

    #[cfg(not(all(feature = "local_fs", feature = "local_tty")))]
    fn run_repo_command(&self, _command: String, _ctx: &mut ViewContext<Self>) {}

    /// Check out another local branch from the branch menu (#11), then refresh
    /// the bar so it reflects the new branch / diff / PR.
    fn checkout_branch(&mut self, branch: String, ctx: &mut ViewContext<Self>) {
        self.composer_menu = None;
        let escaped = branch.replace('\'', r"'\''");
        self.run_repo_command(format!("git checkout '{escaped}' 2>/dev/null"), ctx);
        ctx.notify();
    }

    /// Open a PR for the current branch via `gh pr create --web` (#11) — the
    /// browser create page, so the user fills in the title/body. Refreshes the
    /// bar afterward so the new `PR #n` pill appears once it exists.
    fn create_pr(&mut self, ctx: &mut ViewContext<Self>) {
        self.composer_menu = None;
        self.run_repo_command("gh pr create --web 2>/dev/null".to_owned(), ctx);
        ctx.notify();
    }

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
            // PRODUCT §13: Cmd+C with an empty composer surfaces here (the editor
            // emits `Copy` instead of writing an empty clipboard). Copy the
            // current transcript selection if there is one. When the composer is
            // non-empty the editor copies its own selection and never emits this.
            EditorEvent::Copy => {
                if let Some(text) = self.transcript_selection.read().clone() {
                    if !text.is_empty() {
                        ctx.clipboard().write(ClipboardContent::plain_text(text));
                    }
                }
            }
            EditorEvent::Escape => {
                if self.suggestion_query.take().is_some() {
                    self.suggestions.clear();
                    self.sync_navigation_propagation(ctx);
                    ctx.notify();
                }
            }
            // Cycle the highlighted suggestion. While a menu is open the editor
            // is flipped to `Always` propagation (see
            // `sync_navigation_propagation`), so the arrows reach us on every
            // keypress — even mid-draft in a multi-line message; clicking a row
            // works regardless.
            EditorEvent::Navigate(key) if self.suggestion_query.is_some() => {
                let len = self.suggestions.len();
                if len == 0 {
                    return;
                }
                match key {
                    // #9: Tab accepts the highlighted suggestion (like the
                    // `claude` CLI); arrows still move the selection.
                    NavigationKey::Tab => self.accept_suggestion(self.suggestion_selected, ctx),
                    NavigationKey::Down => {
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
            // twarp: with no suggestions open, Up/Down walk the draft history
            // like a terminal. The editor only surfaces these when the cursor
            // can't move further (single-line drafts: always; multi-line: at the
            // top/bottom edge), so multi-line editing still works normally.
            EditorEvent::Navigate(NavigationKey::Up) => self.recall_history(true, ctx),
            EditorEvent::Navigate(NavigationKey::Down) => self.recall_history(false, ctx),
            _ => {}
        }
    }

    /// twarp: step through the submitted-message history in the composer. `older`
    /// recalls the previous (older) entry; `!older` moves toward newer entries
    /// and, past the newest, restores the draft that was in the box when
    /// browsing began. A no-op when there is no history.
    fn recall_history(&mut self, older: bool, ctx: &mut ViewContext<Self>) {
        if self.message_history.is_empty() {
            return;
        }
        let last = self.message_history.len() - 1;
        let next = match (older, self.history_cursor) {
            // Entering history from the live draft: stash it, recall the newest.
            (true, None) => {
                self.history_draft = self
                    .input_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx));
                Some(last)
            }
            // Older: move toward index 0, clamping at the oldest entry.
            (true, Some(i)) => Some(i.saturating_sub(1)),
            // Newer past the newest entry: leave history, restore the draft.
            (false, Some(i)) if i >= last => None,
            // Newer: move toward the newest entry.
            (false, Some(i)) => Some(i + 1),
            // Already on the live draft and pressing Down: nothing to do.
            (false, None) => return,
        };
        let text = match next {
            Some(i) => self.message_history[i].clone(),
            None => std::mem::take(&mut self.history_draft),
        };
        self.history_cursor = next;
        self.input_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&text, ctx);
            // Park the cursor at the edge we'd continue walking from, so
            // repeated presses keep stepping through history even when an entry
            // wraps to several visual rows. The editor only surfaces Up/Down to
            // us at a visual-row boundary (`AtBoundary`), and `set_buffer_text`
            // leaves the cursor at the very end: fine for Down (already on the
            // last row → next Down steps newer), but for Up we must move to the
            // start (first row) or the next Up would just walk up within the
            // recalled text instead of reaching the older entry.
            if older {
                editor.move_to_buffer_start(ctx);
            }
        });
        ctx.notify();
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
                        // The catalogue (built-ins + on-disk skills with their
                        // descriptions) is scanned once and kept; a running
                        // session's `init` list folds in any names we can't see
                        // on disk (plugins / MCP commands).
                        let index = self
                            .slash_command_index
                            .get_or_insert_with(|| composer::build_slash_command_index(&cwd));
                        index.merge_names(
                            self.transcript.slash_commands().iter().map(String::as_str),
                        );
                        index.names.clone()
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
        self.sync_navigation_propagation(ctx);
        ctx.notify();
    }

    /// twarp: keep the composer editor's vertical-arrow behaviour in sync with
    /// the suggestions panel. While a `/` or `@` menu is open the arrows must
    /// always cycle the menu — even mid-draft in a multi-line message, where
    /// the cursor isn't at a top/bottom edge — so we flip the editor to
    /// `Always`. With the menu closed we restore `AtBoundary`: multi-line
    /// editing keeps the arrows in the buffer, and Up/Down only walk the
    /// submitted-message history once the cursor reaches the top/bottom edge.
    fn sync_navigation_propagation(&mut self, ctx: &mut ViewContext<Self>) {
        let menu_open = self.suggestion_query.is_some() && !self.suggestions.is_empty();
        let mode = if menu_open {
            PropagateAndNoOpNavigationKeys::Always
        } else {
            PropagateAndNoOpNavigationKeys::AtBoundary
        };
        self.input_editor.update(ctx, |editor, _ctx| {
            editor.set_propagate_vertical_navigation_keys(mode);
        });
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
            // For `/` commands, show the skill's description beneath the name
            // (issue #3) when we discovered one on disk.
            let description = (query.kind == SuggestionKind::SlashCommand)
                .then(|| {
                    self.slash_command_index
                        .as_ref()
                        .and_then(|index| index.descriptions.get(suggestion))
                })
                .flatten();
            let label_block: Box<dyn Element> = if let Some(description) = description {
                let desc = appearance
                    .ui_builder()
                    .span(truncate_description(description))
                    .with_style(UiComponentStyles {
                        font_size: Some(11.),
                        font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(1.)
                    .with_child(label)
                    .with_child(desc)
                    .finish()
            } else {
                label
            };
            let mut row_container = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(icon)
                    .with_child(label_block)
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
        let row_handle = |handles: &std::cell::RefCell<Vec<MouseStateHandle>>, index: usize| {
            let mut states = handles.borrow_mut();
            while states.len() <= index {
                states.push(MouseStateHandle::default());
            }
            states[index].clone()
        };
        for (index, message) in self.message_queue.iter().enumerate() {
            let row_mouse = row_handle(&self.queue_row_mouse, index);
            let send_mouse = row_handle(&self.queue_send_mouse, index);
            let remove_mouse = row_handle(&self.queue_remove_mouse, index);
            let expanded = self.queue_expanded.contains(&index);
            // Collapsed: a one-line, length-bounded preview. Expanded (click the
            // row): the whole prompt, wrapped to the row's full width.
            let text = if expanded {
                message.text.trim().to_owned()
            } else {
                queue_preview(&message.text)
            };
            let label = appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish();
            // The label fills the row's width and toggles the full-text
            // expansion on click; only the buttons remove or send.
            let label_click = Hoverable::new(row_mouse, move |_| label)
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleQueuedMessage(index));
                })
                .finish();
            // ↑ send-now, sits to the left of the ✕ remove control.
            let send_glyph = appearance
                .ui_builder()
                .span("\u{2191}".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(12.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let send = Hoverable::new(send_mouse, move |_| send_glyph)
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::SendQueuedMessageNow(index));
                })
                .finish();
            let remove_glyph = appearance
                .ui_builder()
                .span("\u{2715}".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let remove = Hoverable::new(remove_mouse, move |_| remove_glyph)
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::RemoveQueuedMessage(index));
                })
                .finish();
            let row = Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(Expanded::new(1., label_click).finish())
                    .with_child(send)
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
            column.add_child(row);
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
            // twarp (#8): persist the clipboard bytes to a temp file so the
            // paste previews as a real thumbnail in the composer and the chat,
            // and can be re-opened in the OS default image app on click. A
            // write failure degrades to `None` — the paste still sends inline.
            let thumbnail_path = persist_pasted_image(&image.data, media_type);
            self.direct_attachments.push(DirectAttachment {
                label,
                image: OutgoingImage {
                    media_type: media_type.to_owned(),
                    base64_data: base64::engine::general_purpose::STANDARD.encode(&image.data),
                },
                thumbnail_path,
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
        // twarp: record the turn for Up/Down recall (skip an immediate
        // duplicate of the last entry, like a shell). Reset the browse cursor
        // so the next Up starts from the newest message again.
        if self.message_history.last() != Some(&text) {
            self.message_history.push(text.clone());
        }
        self.history_cursor = None;
        self.history_draft.clear();
        let message = OutgoingMessage {
            images: self.outgoing_images(),
            text,
        };
        // #8: the on-disk paths of the images riding along, to preview above the
        // user bubble (pasted-only images have no path and are omitted).
        let mut previews: Vec<PathBuf> = self.pending_images.clone();
        previews.extend(
            self.direct_attachments
                .iter()
                .filter_map(|attachment| attachment.thumbnail_path.clone()),
        );
        // Clear the composer either way so the user can keep typing (§53).
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.suggestion_query = None;
        self.suggestions.clear();
        self.pending_images.clear();
        self.attachment_optouts.clear();
        self.direct_attachments.clear();

        self.submit_message(message, previews, ctx);
    }

    /// Deliver one already-composed user turn (PRODUCT §16): queue it behind a
    /// streaming turn (type-ahead, §53), write it to a live session's stdin, or
    /// spawn the session on the first message. Shared by the composer
    /// ([`Self::submit`]) and the `AskUserQuestion` answer path (§1).
    fn submit_message(
        &mut self,
        message: OutgoingMessage,
        previews: Vec<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.streaming {
            // PRODUCT §53–§54 (7m): a turn is in flight — queue this for
            // automatic dispatch when the turn completes, rather than blocking.
            // (Type-ahead image previews aren't carried through the queue.)
            self.message_queue.push(message);
            ctx.notify();
            return;
        }

        self.transcript
            .apply(TranscriptEvent::UserMessage(message.text.clone()));
        // #8: attach the previews to the just-pushed user item (it's last).
        if !previews.is_empty() {
            let index = self.transcript.items().len().saturating_sub(1);
            self.sent_images.insert(index, previews);
        }
        let started = Instant::now();
        self.streaming = true;
        self.turn_started = Some(started);
        self.schedule_elapsed_tick(started, ctx);
        match &self.message_tx {
            // Session already running — write the turn to its stdin.
            Some(tx) => {
                let _ = tx.try_send(StdinCommand::Turn(message));
            }
            // First message: spawn the session, forwarding this as turn one.
            None => self.begin_session(Some(message), ctx),
        }
        // #7: the first user turn names the tab.
        self.update_pane_title(ctx);
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
            let Some(source) = question_source(self.transcript.items().get(item)) else {
                return;
            };
            parse_questions(source)
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
            let Some(TranscriptItem::Tool { name, input, .. }) = self.transcript.items().get(item)
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
        self.submit_message(OutgoingMessage::text(lines.join("\n")), Vec::new(), ctx);
    }

    /// Answer a `can_use_tool` permission prompt over the control channel (7g,
    /// PRODUCT §24). Flips the card to its decided state and writes the
    /// `control_response` `claude` is waiting on: Allow echoes the proposed
    /// `input` back as `updatedInput`; Deny rejects with a short reason and the
    /// session continues. A no-op (no response sent) if the request was already
    /// answered or the session is gone — never hangs (PRODUCT §26).
    fn answer_permission(&mut self, request_id: &str, allow: bool, ctx: &mut ViewContext<Self>) {
        let Some(input) = self.transcript.answer_permission(request_id, allow) else {
            return;
        };
        let response = if allow {
            serde_json::json!({ "behavior": "allow", "updatedInput": input })
        } else {
            serde_json::json!({ "behavior": "deny", "message": "The user declined this action." })
        };
        if let Some(tx) = &self.message_tx {
            let _ = tx.try_send(StdinCommand::Control {
                request_id: request_id.to_owned(),
                response,
            });
        }
        ctx.notify();
    }

    /// Submit the chosen answers for a control-channel question card (the
    /// `request_user_dialog` path, 7g/§24) at transcript index `item`. The exact
    /// `completed`-answer wire shape for the `question` dialog kind isn't pinned
    /// down against the headless CLI, so we take the safe route the §26
    /// degradation already sanctions: **cancel** the dialog (releasing the
    /// turn — never a hang) and resend the picks as the next user turn, exactly
    /// like the tool-card question path. The model reads the answer from the
    /// transcript above.
    fn submit_question_dialog(&mut self, item: usize, ctx: &mut ViewContext<Self>) {
        let (request_id, payload) = {
            let Some(TranscriptItem::Question {
                id,
                payload,
                answered,
                ..
            }) = self.transcript.items().get(item)
            else {
                return;
            };
            if *answered {
                return;
            }
            (id.clone(), payload.clone())
        };
        let parsed = parse_questions(&payload);
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
        // Mark the card answered (stops its live controls) and release the
        // dialog so `claude` continues, then resend the picks as a turn.
        self.transcript.answer_question(&request_id);
        self.question_selected.remove(&item);
        if let Some(tx) = &self.message_tx {
            let _ = tx.try_send(StdinCommand::Control {
                request_id,
                response: serde_json::json!({ "behavior": "cancelled" }),
            });
        }
        self.submit_message(OutgoingMessage::text(lines.join("\n")), Vec::new(), ctx);
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
        self.queue_expanded.clear();
        self.transcript
            .apply(TranscriptEvent::UserMessage(message.text.clone()));
        let started = Instant::now();
        self.streaming = true;
        self.turn_started = Some(started);
        self.schedule_elapsed_tick(started, ctx);
        let _ = tx.try_send(StdinCommand::Turn(message));
        ctx.notify();
    }

    /// Dispatch a specific queued message now rather than waiting for the
    /// in-flight turn to drain it (PRODUCT §54). While a turn streams the
    /// message jumps to the front of the queue so it's sent next; when the
    /// session is idle it ships immediately through the normal send path.
    fn send_queued_now(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index >= self.message_queue.len() {
            return;
        }
        let message = self.message_queue.remove(index);
        self.queue_expanded.clear();
        if self.streaming {
            self.message_queue.insert(0, message);
            ctx.notify();
        } else {
            // Type-ahead image previews aren't carried through the queue.
            self.submit_message(message, Vec::new(), ctx);
        }
    }

    /// Open the given composer dropdown / popover, or close it if it's already
    /// the open one (#13). The model and effort pickers only open while idle
    /// (changing model/effort mid-turn would tear down the live turn, §25); the
    /// read-only context popover opens any time.
    fn toggle_composer_menu(&mut self, menu: ComposerMenu, ctx: &mut ViewContext<Self>) {
        // Model / effort / permission tear down or re-attach the live turn, so
        // they're idle-only; the read-only / git menus (context, branch, CI, PR)
        // open any time.
        let idle_only = matches!(
            menu,
            ComposerMenu::Model | ComposerMenu::Effort | ComposerMenu::Permission
        );
        if self.streaming && idle_only {
            return;
        }
        self.composer_menu = if self.composer_menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        ctx.notify();
    }

    /// Pick a model from the dropdown (#13). No-op (just closes the menu) when
    /// the choice is unchanged or a turn is streaming; otherwise detaches a
    /// live session so the next message resumes under the new `--model` (§25).
    /// twarp 07: mirror the pane's current settings into the global last-used
    /// store so the next Claude pane (and a crash-restored one) inherits them.
    fn persist_session_defaults(&self, ctx: &mut ViewContext<Self>) {
        let (model, effort, permission_mode) = (
            self.model.clone(),
            self.effort.clone(),
            self.permission_mode,
        );
        ClaudeSessionDefaultsModel::handle(ctx).update(ctx, |defaults, ctx| {
            defaults.record(model, effort, permission_mode, ctx);
        });
    }

    fn set_model(&mut self, model: Option<String>, ctx: &mut ViewContext<Self>) {
        self.composer_menu = None;
        if self.streaming || self.model == model {
            ctx.notify();
            return;
        }
        self.model = model;
        self.persist_session_defaults(ctx);
        if self.child.is_some() {
            self.detach_live_session();
        }
        ctx.notify();
    }

    /// Set the effort from the slider (#13). Same detach→`--resume` mechanism
    /// and streaming guard as [`Self::set_model`]; idempotent so the slider's
    /// continuous `on_change` doesn't restart the session on every pixel.
    fn set_effort(&mut self, effort: Option<String>, ctx: &mut ViewContext<Self>) {
        if self.streaming || self.effort == effort {
            return;
        }
        self.effort = effort;
        self.persist_session_defaults(ctx);
        if self.child.is_some() {
            self.detach_live_session();
        }
        ctx.notify();
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
        self.persist_session_defaults(ctx);
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
    /// Create a fresh git worktree for this session at `../<name>` on a new
    /// branch (#11, the worktree toggle). Returns the worktree path on success;
    /// `None` (and no change) if the cwd isn't a repo or `git` fails. Blocking,
    /// but a local `git worktree add` is sub-second and only runs on the user's
    /// explicit toggle + first send.
    #[cfg(all(feature = "local_fs", feature = "local_tty"))]
    fn create_worktree(&self) -> Option<PathBuf> {
        let cwd = self.cwd.clone().or_else(|| std::env::current_dir().ok())?;
        let parent = cwd.parent()?;
        // A readable, collision-resistant name: `<folder>-<short session id>`.
        let folder = repo_context::folder_name(&cwd).unwrap_or_else(|| "work".to_owned());
        let short: String = self
            .session_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect();
        let name = format!("{folder}-{short}");
        let target = parent.join(&name);
        if target.exists() {
            return None;
        }
        let mut command = std::process::Command::new("git");
        command
            .current_dir(&cwd)
            .args(["worktree", "add", "-b", &name])
            .arg(&target);
        if let Some(path_env) = &self.interactive_path {
            command.env("PATH", path_env);
        }
        match command.output() {
            Ok(output) if output.status.success() => Some(target),
            _ => None,
        }
    }

    #[cfg(not(all(feature = "local_fs", feature = "local_tty")))]
    fn create_worktree(&self) -> Option<PathBuf> {
        None
    }

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
        // twarp (#11): when the worktree toggle is on, the first turn of a fresh
        // session runs in a new git worktree at `../<name>` on an isolated
        // branch, so the agent's work doesn't touch the original checkout. Only
        // for a fresh (non-resumed) session; falls back to the cwd on any error.
        if self.use_worktree && self.resume_session_id.is_none() {
            if let Some(worktree) = self.create_worktree() {
                self.cwd = Some(worktree);
                self.use_worktree = false;
                self.refresh_repo_context(ctx);
                // The pane's cwd just moved to the worktree — tell the host so
                // the workspace re-detects this pane's repo and the code-review /
                // Open Changes panel follows it to the worktree (otherwise the
                // panel stays pinned to the original checkout).
                ctx.emit(ClaudeCodeViewEvent::Pane(PaneEvent::RepoChanged));
            }
        }
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
            mcp_config: {
                #[cfg(not(target_family = "wasm"))]
                {
                    crate::browser_mcp::BrowserMcpBridge::as_ref(ctx).mcp_config_json()
                }
                #[cfg(target_family = "wasm")]
                {
                    None
                }
            },
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

        // Own stdin in a background task; the view queues user turns and
        // control-protocol answers onto it (one writer, no races, §24).
        let (message_tx, message_rx) = async_channel::unbounded::<StdinCommand>();
        ctx.background_executor()
            .spawn(async move {
                let mut stdin = stdin;
                while let Ok(command) = message_rx.recv().await {
                    let wrote = match command {
                        StdinCommand::Turn(message) => {
                            send_user_message(&mut stdin, &message).await
                        }
                        StdinCommand::Control {
                            request_id,
                            response,
                        } => send_control_response(&mut stdin, &request_id, response).await,
                        StdinCommand::Interrupt { request_id } => {
                            send_interrupt(&mut stdin, &request_id).await
                        }
                    };
                    if wrote.is_err() {
                        break;
                    }
                }
            })
            .detach();

        self.child = Some(child);
        self.message_tx = Some(message_tx);

        // Send the first turn now that stdin is wired (PRODUCT §6).
        if let (Some(prompt), Some(tx)) = (first_prompt, &self.message_tx) {
            let _ = tx.try_send(StdinCommand::Turn(prompt));
        }
        ctx.notify();
    }

    /// Apply one driver event on the main thread (PRODUCT §9–§13), dropping
    /// events from a superseded session. An `Ended` event closes the
    /// streaming turn.
    fn on_transcript_event(
        &mut self,
        epoch: u64,
        mut event: TranscriptEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if epoch != self.session_epoch {
            return;
        }
        // PRODUCT §1: `AskUserQuestion` must never sit behind a `can_use_tool`
        // permission prompt. claude blocks the whole turn waiting for that
        // answer, but the inline Question card (the §1 tool-card path) is
        // non-interactive mid-stream — so the user has no way to pick an option
        // and the only live control is a confusing "Allow AskUserQuestion"
        // prompt. (Observed: a turn wedged ~2h with a dead question card until
        // the user clicked the permission, which resolved as "user did not
        // answer".) Auto-allow it: the tool returns immediately, the turn ends,
        // and the live Question card lets the user answer as the next turn.
        if let TranscriptEvent::PermissionRequest { id, tool, input } = &event {
            if should_auto_allow_permission(tool) {
                if let Some(tx) = &self.message_tx {
                    let _ = tx.try_send(StdinCommand::Control {
                        request_id: id.clone(),
                        response: serde_json::json!({
                            "behavior": "allow",
                            "updatedInput": input,
                        }),
                    });
                }
                return;
            }
        }
        // PRODUCT §11: a user-requested Stop makes `claude` end the turn with an
        // error `result` (subtype `error_during_execution`) while the session
        // stays alive. Re-label that terminal event as a clean interrupt so it
        // shows the "Interrupted." notice — not a scary error — and stays
        // resumable (the Error/Exited arm below would otherwise drop the resume
        // id and wedge the pane).
        if self.interrupt_pending {
            if let TranscriptEvent::Ended { reason } = &event {
                if matches!(
                    reason,
                    claude_code::EndReason::Error(_) | claude_code::EndReason::Exited
                ) {
                    event = TranscriptEvent::Ended {
                        reason: claude_code::EndReason::Interrupted,
                    };
                }
                self.interrupt_pending = false;
            }
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
        let ended = matches!(event, TranscriptEvent::Ended { .. });
        self.ingest_event(event, ctx);
        if turn_completed {
            self.drain_message_queue(ctx);
        }
        if ended {
            // #11: the turn may have edited files, committed, or pushed —
            // refresh the diff / branch / PR / CI bar.
            self.refresh_repo_context(ctx);
        }
        // PRODUCT §14: follow streaming output to the bottom as it arrives —
        // but only while the user is still pinned to the bottom. If they've
        // scrolled up to read earlier output mid-turn, leave their position
        // untouched so scrolling stays smooth instead of yanking them back down.
        if self.scroll_state.is_at_bottom(AUTOSCROLL_STICK_SLACK) {
            self.scroll_to_bottom();
        }
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
        self.interrupt_pending = false;
        self.child = None;
        self.message_tx = None;
        ctx.notify();
    }

    /// Set the permission mode from the dropdown (PRODUCT §25, #2). The mode
    /// applies to the next spawn; a live session is detached (killed) and its id
    /// kept as the resume target, so the next message continues the same
    /// conversation under the new mode — the only mode-change channel `claude`'s
    /// documented flags offer. Closes the menu; idempotent / a no-op mid-turn
    /// (§25 applies between turns).
    fn set_permission_mode(&mut self, mode: PermissionMode, ctx: &mut ViewContext<Self>) {
        self.composer_menu = None;
        if self.streaming || self.permission_mode == mode {
            ctx.notify();
            return;
        }
        self.permission_mode = mode;
        self.persist_session_defaults(ctx);
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
        self.interrupt_pending = false;
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
            claude_binary: self.resolve_claude_binary(),
            flags: self.raw_cli_flags(),
        });
        ctx.notify();
    }

    /// The alias-derived defaults to re-apply when launching the raw CLI by
    /// absolute path (which bypasses the shell alias). Mirrors the flags the
    /// headless spawn passes (`SpawnOptions` → `driver::spawn_session`), so the
    /// chat UI and the raw CLI agree on effort / model / permission mode on an
    /// empty chat rather than the CLI silently falling back to its own config
    /// defaults. Values are single-quoted so a model id never splits a token.
    fn raw_cli_flags(&self) -> String {
        let mut flags = Vec::new();
        if let Some(effort) = &self.effort {
            flags.push(format!("--effort '{effort}'"));
        }
        if let Some(model) = &self.model {
            flags.push(format!("--model '{model}'"));
        }
        // Always set: the chat UI always spawns with an explicit
        // `--permission-mode` (`driver::spawn_session`), so passing the view's
        // current mode keeps the two modes in lockstep — e.g. an alias's
        // `--dangerously-skip-permissions` (→ bypassPermissions) carries over.
        flags.push(format!(
            "--permission-mode {}",
            self.permission_mode.as_cli_arg()
        ));
        flags.join(" ")
    }

    /// Resolve the `claude` executable to launch in raw mode. Prefers an
    /// absolute path off the captured login-shell PATH (mirrors
    /// [`Self::claude_available`]); falls back to the process PATH, then to the
    /// bare command. An absolute path is required so the raw-CLI command isn't
    /// intercepted by the `claude`-at-submit trigger (PRODUCT §43) — under a
    /// GUI launch the process PATH is launchd-minimal and omits where `claude`
    /// lives, so a bare fallback would be eaten by the trigger and never run.
    fn resolve_claude_binary(&self) -> String {
        if let Some(path) = &self.interactive_path {
            if let Some(resolved) =
                resolve_executable_in_path(CLAUDE_BINARY, std::ffi::OsStr::new(path))
            {
                return resolved.display().to_string();
            }
        }
        resolve_executable(CLAUDE_BINARY)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| CLAUDE_BINARY.to_owned())
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

    /// Stop the current turn (PRODUCT §11): send an `interrupt` control request
    /// over stdin. The session stays alive; `claude` ends the turn with an error
    /// `result`, which [`Self::on_transcript_event`] re-labels as
    /// `Ended { Interrupted }` (via `interrupt_pending`) — clearing the streaming
    /// state and keeping the session resumable. Falls back to a SIGINT only when
    /// there's no live stdin pump (e.g. the process is mid-spawn).
    fn stop(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.streaming {
            return;
        }
        if let Some(tx) = &self.message_tx {
            self.interrupt_pending = true;
            let request_id = format!("interrupt-{}", self.session_epoch);
            let _ = tx.try_send(StdinCommand::Interrupt { request_id });
            ctx.notify();
        } else if let Some(child) = &self.child {
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

    /// twarp: the floating circular "scroll to bottom" button. Returned only
    /// when the transcript has content and is scrolled up off the bottom (see
    /// `render_input`); a click jumps back to the latest message. A bare
    /// down-chevron in a shadowed circle, anchored above the composer's right
    /// edge so it reads as belonging to the conversation, not the input.
    fn render_scroll_to_bottom_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon = ConstrainedBox::new(
            Icon::new(
                SCROLL_TO_BOTTOM_ICON_SVG_PATH,
                theme.active_ui_text_color().into_solid(),
            )
            .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();
        let sized = ConstrainedBox::new(Align::new(icon).finish())
            .with_width(28.)
            .with_height(28.)
            .finish();
        let circle = Container::new(sized)
            .with_background_color(theme.surface_1().into_solid())
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
            .with_drop_shadow(DropShadow::default())
            .finish();
        Hoverable::new(self.scroll_to_bottom_button.clone(), move |_| circle)
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::ScrollToBottom);
            })
            .finish()
    }

    /// Label the pane's tab with the conversation's first user message (#7) —
    /// a wall of identical "Claude Code" tabs is impossible to tell apart once a
    /// few are open. Keeps the generic title until a user turn exists.
    /// `set_title` no-ops when unchanged, so calling this on each new turn is
    /// cheap.
    fn update_pane_title(&self, ctx: &mut ViewContext<Self>) {
        let title = self
            .transcript
            .items()
            .iter()
            .find_map(|item| match item {
                TranscriptItem::User(text) => Some(pane_tab_title(text)),
                _ => None,
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| PANE_TITLE.to_owned());
        self.pane_configuration
            .update(ctx, |config, ctx| config.set_title(title, ctx));
    }

    /// Drive the "Working · <elapsed>" counter (#7) on a steady 1 s cadence.
    ///
    /// The shimmer self-schedules ~30 fps repaints, but those only re-*shade*
    /// the already-baked label string — they never re-run
    /// [`Self::render_streaming_status`], so `turn_started.elapsed()` is not
    /// recomputed by them. Absent this timer the label refreshes only when a
    /// stream event lands and calls `notify()`, and those arrive in irregular
    /// bursts (deltas, then long gaps during tool calls / model thinking), so
    /// the seconds jump instead of ticking. A self-re-arming 1 s `notify()`
    /// re-renders the status line on a regular beat. Stale chains die on their
    /// own: when the turn ends `streaming` is false, and when a new turn starts
    /// its fresh `turn_started` `Instant` no longer matches the captured one.
    fn schedule_elapsed_tick(&self, started: Instant, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(Duration::from_secs(1)).await;
            },
            move |me, _, ctx| {
                if me.streaming && me.turn_started == Some(started) {
                    ctx.notify();
                    me.schedule_elapsed_tick(started, ctx);
                }
            },
        );
    }

    /// The live streaming status line (#7), shown below the last message while
    /// a turn is in flight: a Claude glyph + a shimmering "Working · <elapsed>"
    /// label. The shimmer animates the gradient; [`Self::schedule_elapsed_tick`]
    /// re-renders this line once a second so the elapsed counter ticks smoothly.
    /// (Live token counts aren't in the headless stream, so the stat is the
    /// elapsed time.)
    fn render_streaming_status(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let bright = theme.main_text_color(theme.background()).into_solid();
        let accent = self.render_accent.get();
        // Format: "<elapsed> · <tokens> tokens · Working…", matching the
        // interactive CLI's status line. The token segment is dropped until a
        // live count is known (the first `assistant` message reports usage);
        // the elapsed clock stays compact (e.g. `1m 21s`) as turns run long.
        let label = match self.turn_started {
            Some(started) => {
                let mut label = thinking::format_compact_elapsed(started.elapsed());
                if let Some(used) = self
                    .transcript
                    .usage()
                    .map(|usage| usage.context_used())
                    .filter(|used| *used > 0)
                {
                    label.push_str(&format!(" · {} tokens", fmt_tokens(used)));
                }
                label.push_str(" · Working…");
                label
            }
            None => "Working…".to_owned(),
        };
        let icon = ConstrainedBox::new(
            PulsingIcon::new(
                ASSISTANT_ICON_SVG_PATH,
                accent,
                self.working_icon_pulse.clone(),
            )
            .finish(),
        )
        .with_width(14.)
        .with_height(14.)
        .finish();
        let shimmer = ShimmeringTextElement::new(
            label,
            appearance.ui_font_family(),
            13.,
            muted,
            bright,
            ShimmerConfig::default(),
            self.working_shimmer.clone(),
        )
        .finish();
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(8.)
                .with_child(icon)
                .with_child(shimmer)
                .finish(),
        )
        .with_margin_left(TRANSCRIPT_LEFT_MARGIN)
        .with_margin_top(8.)
        .with_margin_bottom(8.)
        .finish()
    }

    /// twarp: resume a recent session from the zero-state panel, in place.
    /// Mirrors the resume path in [`Self::new`] (load stored history through the
    /// same ingest path, adopt the session id, re-attach on the next message)
    /// without spawning a new pane — the empty pane becomes the resumed one.
    fn resume_recent_session(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        // Only meaningful from the zero state; a live/streaming pane has no panel.
        if self.streaming || !self.transcript.is_empty() {
            return;
        }
        let Some(session) = self.recent_sessions.get(index).cloned() else {
            return;
        };

        // Render the stored history up front (PRODUCT §36), then key the pane's
        // identity off the resumed id so `--resume`, the raw-CLI toggle, and the
        // history refresh all target the right session.
        for event in sessions::load_history(&session.jsonl_path) {
            self.ingest_event(event, ctx);
        }
        self.resume_session_id = Some(session.id.clone());
        self.session_id = session.id;
        // The panel is gone the moment the transcript has content.
        self.recent_sessions = Vec::new();
        self.recent_session_mouse.borrow_mut().clear();

        // Open at the latest message, name the tab from the history, and refresh
        // the composer context bar for the resumed conversation.
        self.scroll_to_bottom();
        self.update_pane_title(ctx);
        self.refresh_repo_context(ctx);
        ctx.focus(&self.input_editor);
        ctx.notify();
    }

    /// twarp: fork the conversation at the assistant response at transcript
    /// `index` ("Fork conversation"; Claude's `--fork-session`). Truncates the
    /// session's on-disk jsonl after that turn into a new branch file, then asks
    /// the pane group to open it as a resumed session in a split — this pane and
    /// its session are left untouched.
    fn fork_conversation(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index >= self.transcript.items().len() {
            return;
        }
        // The live conversation is always stored under the pane's own session id
        // (`--session-id` for a fresh pane, `--resume <id>` for a resumed one,
        // where `session_id == resume id`). Forking needs that file on disk.
        let Some(cwd) = self.cwd.clone().or_else(|| std::env::current_dir().ok()) else {
            return;
        };
        let Some(parent_path) = sessions::session_file(&cwd, &self.session_id) else {
            return;
        };
        if !parent_path.exists() {
            // Nothing persisted yet (first turn still streaming) — nothing to fork.
            return;
        }

        // Keep every user turn up to and including the one this response belongs
        // to. `User` items map 1:1 to the `UserMessage` events `fork_session_file`
        // counts, so the boundary the user sees and the file cut agree.
        let keep_user_turns = self.transcript.items()[..=index]
            .iter()
            .filter(|item| matches!(item, TranscriptItem::User(_)))
            .count();
        if keep_user_turns == 0 {
            return;
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        let jsonl_path =
            match sessions::fork_session_file(&parent_path, &new_id, keep_user_turns, &cwd) {
                Ok(path) => path,
                Err(err) => {
                    log::warn!("claude: fork conversation failed: {err}");
                    return;
                }
            };

        // The branch inherits this pane's effective settings (model / effort /
        // permission mode); `prompt`/`resume_session_id` stay empty because the
        // `ResumeSession` itself is the resume target.
        let launch = LaunchOptions {
            prompt: None,
            permission_mode: Some(self.permission_mode),
            model: self.model.clone(),
            effort: self.effort.clone(),
            resume_session_id: None,
        };
        ctx.emit(ClaudeCodeViewEvent::ForkSession {
            resume: ResumeSession {
                session_id: new_id,
                jsonl_path,
            },
            launch,
            cwd: Some(cwd),
        });
    }

    /// twarp: an assistant response plus its hover "Fork" affordance. The reply
    /// renders exactly as before; on hover a small branch button slides in below
    /// it, dispatching [`ClaudeCodeViewAction::ForkConversation`] for this index.
    fn render_assistant_response(
        &self,
        index: usize,
        text: &str,
        is_reply_end: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let row = render_message_row(
            false,
            ASSISTANT_ICON_SVG_PATH,
            text,
            self.render_accent.get(),
            appearance,
        );
        // Fork is offered only at the end of a reply (the last assistant
        // message of a turn) — not on intermediate assistant messages that sit
        // between tool calls within the same turn.
        if !is_reply_end {
            return row;
        }
        // The affordance is always laid out below the row; hovering only
        // toggles its visibility (transparent colours ↔ painted). Both states
        // have identical layout, so the row's hover bounds never change — that
        // stable region is what kills the old show/hide flicker (adding the
        // button moved the cursor outside last frame's bounds, hiding it
        // again) and avoids the layout jump from inserting/removing it.
        let fork_visible = self.render_fork_affordance(index, true, app);
        let fork_hidden = self.render_fork_affordance(index, false, app);
        let row_mouse = pooled_mouse_state(&self.fork_row_mouse, index);
        Hoverable::new(row_mouse, move |state| {
            let fork = if state.is_hovered() {
                fork_visible
            } else {
                fork_hidden
            };
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(row)
                .with_child(fork)
                .finish()
        })
        .finish()
    }

    /// Whether the assistant message at `index` is the last one of its turn —
    /// no later assistant message appears before the next user turn (or the end
    /// of the transcript). Drives where the Fork affordance is offered.
    fn is_reply_end(&self, index: usize) -> bool {
        for later in self.transcript.items().iter().skip(index + 1) {
            match later {
                TranscriptItem::User(_) => return true,
                TranscriptItem::Assistant { .. } => return false,
                _ => {}
            }
        }
        true
    }

    /// twarp: the "⑂ Fork" button shown under a response on hover. Aligned under
    /// the message prose (past the avatar gutter) and clickable on its own — the
    /// surrounding response stays plain text. When `visible` is false the same
    /// element is laid out with transparent colours and no click/cursor, so it
    /// reserves its space silently (see [`Self::render_assistant_response`]).
    fn render_fork_affordance(
        &self,
        index: usize,
        visible: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        // Transparent when hidden — keeps identical layout to the visible state.
        let label_color = if visible {
            theme.nonactive_ui_text_color().into_solid()
        } else {
            ColorU::new(0, 0, 0, 0)
        };
        let pill_bg = if visible {
            self.accent_wash(app)
        } else {
            ColorU::new(0, 0, 0, 0)
        };
        let button_mouse = pooled_mouse_state(&self.fork_button_mouse, index);

        let icon = ConstrainedBox::new(Icon::new(FORK_ICON_SVG_PATH, label_color).finish())
            .with_width(12.)
            .with_height(12.)
            .finish();
        let label = appearance
            .ui_builder()
            .span("Fork")
            .with_style(UiComponentStyles {
                font_color: Some(label_color),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish();
        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(5.)
            .with_child(icon)
            .with_child(label)
            .finish();
        let pill = Container::new(content)
            .with_padding_left(8.)
            .with_padding_right(8.)
            .with_padding_top(3.)
            .with_padding_bottom(3.)
            .with_background_color(pill_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
            .finish();
        let button = Hoverable::new(button_mouse, move |_| pill);
        // Only the painted state is interactive — the transparent placeholder
        // must not catch clicks or show the pointer cursor in the empty gap.
        let button = if visible {
            button
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ForkConversation(index));
                })
        } else {
            button
        };
        // Indent under the prose (avatar gutter ≈ 14 padding + 16 icon + 12
        // margin) so it reads as belonging to the message above it.
        Container::new(button.finish())
            .with_margin_left(42.)
            .with_margin_bottom(6.)
            .finish()
    }

    fn render_body(&self, app: &AppContext) -> Box<dyn Element> {
        if self.transcript.is_empty() {
            // Zero state when no session has produced anything — a "Welcome
            // back" launchpad listing this directory's recent sessions.
            return self.render_zero_state(app);
        }
        self.render_transcript(app)
    }

    /// twarp: the zero state — a "Welcome back" launchpad. An accent-tinted
    /// Claude glyph + heading, then either the directory's recent sessions
    /// (click to resume in place) or a first-run explanation when there are
    /// none. Width-capped to the composer's reading column so the two align;
    /// the caller top-centers it over the pane (the composer floats below).
    fn render_zero_state(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let accent = self.render_accent.get();
        let muted = theme.nonactive_ui_text_color().into_solid();

        // Header: the Claude glyph beside a "Welcome back" heading.
        let glyph = ConstrainedBox::new(Icon::new(ASSISTANT_ICON_SVG_PATH, accent).finish())
            .with_width(28.)
            .with_height(28.)
            .finish();
        let heading = appearance
            .ui_builder()
            .span("Welcome back".to_owned())
            .with_style(UiComponentStyles {
                font_size: Some(HEADING_FONT_SIZE),
                font_color: Some(theme.main_text_color(theme.background()).into_solid()),
                ..Default::default()
            })
            .build()
            .finish();
        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(12.)
            .with_child(glyph)
            .with_child(heading)
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(16.)
            .with_child(header);

        if self.recent_sessions.is_empty() {
            // First-run: no stored sessions for this directory yet.
            let explanation = appearance
                .ui_builder()
                .span(
                    "Type a message below — twarp drives the local `claude` CLI and renders its \
                     replies, tool calls, and diffs here. Your existing Claude Code login is used; \
                     twarp adds no account or billing."
                        .to_owned(),
                )
                .with_soft_wrap()
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(13.),
                    ..Default::default()
                })
                .build()
                .finish();
            column.add_child(explanation);
        } else {
            // A "Sessions" section listing this directory's recent sessions,
            // capped so a long history doesn't run under the floating composer.
            const MAX_ROWS: usize = 6;
            let section = appearance
                .ui_builder()
                .span("Sessions".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(11.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let mut rows = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(2.);
            for (idx, session) in self.recent_sessions.iter().enumerate().take(MAX_ROWS) {
                rows.add_child(self.render_recent_session_row(idx, session, app));
            }
            column.add_child(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(8.)
                    .with_child(section)
                    .with_child(rows.finish())
                    .finish(),
            );
        }

        Container::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(COMPOSER_MAX_WIDTH)
                .finish(),
        )
        .with_padding_left(24.)
        .with_padding_right(24.)
        .with_padding_top(64.)
        .finish()
    }

    /// One recent-session row in the zero state: title on the left, relative
    /// time + chevron on the right, hover highlight, click to resume in place.
    /// Mirrors the sidebar's session row (left_panel) but laid out as a full
    /// list item like the Claude desktop "Sessions" rows.
    fn render_recent_session_row(
        &self,
        idx: usize,
        session: &sessions::StoredSession,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();

        let timestamp = chrono::DateTime::<chrono::Utc>::from(session.timestamp);
        let when = crate::util::time_format::format_approx_duration_from_now_utc(timestamp);

        let title = appearance
            .ui_builder()
            .span(session.title.clone())
            .with_style(UiComponentStyles {
                font_color: Some(theme.main_text_color(theme.background()).into_solid()),
                font_size: Some(13.),
                ..Default::default()
            })
            .build()
            .finish();
        let when_label = appearance
            .ui_builder()
            .span(when)
            .with_style(UiComponentStyles {
                font_color: Some(muted),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish();
        let chevron = ConstrainedBox::new(
            Icon::new(
                crate::ui_components::icons::Icon::ChevronRight.into(),
                muted,
            )
            .finish(),
        )
        .with_width(14.)
        .with_height(14.)
        .finish();

        // Title on the left; time + chevron hug the right edge. SpaceBetween
        // pushes the two groups apart across the full row width; a long title
        // shrinks rather than shoving the right group off the edge.
        let right = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(when_label)
            .with_child(chevron)
            .finish();
        let inner = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_spacing(12.)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(right)
            .finish();

        let row_mouse_state = {
            let mut states = self.recent_session_mouse.borrow_mut();
            while states.len() <= idx {
                states.push(MouseStateHandle::default());
            }
            states[idx].clone()
        };
        let highlight = self.accent_wash(app);
        Hoverable::new(row_mouse_state, move |state| {
            let mut body = Container::new(inner)
                .with_padding_left(12.)
                .with_padding_right(12.)
                .with_padding_top(10.)
                .with_padding_bottom(10.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
            if state.is_hovered() {
                body = body.with_background_color(highlight);
            }
            body.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ResumeRecentSession(idx));
        })
        .finish()
    }

    /// twarp: the per-chat background-scripts list, derived from the transcript
    /// via [`background_scripts::collect`] and memoized on the transcript's
    /// revision. Both the header button and the floating panel read it each
    /// render; the memo keeps that to one walk of the transcript per mutation
    /// instead of two per render.
    fn background_scripts(&self) -> std::rc::Rc<Vec<background_scripts::BackgroundScript>> {
        let revision = self.transcript.revision();
        if let Some((rev, scripts)) = self.background_scripts_memo.borrow().as_ref() {
            if *rev == revision {
                return scripts.clone();
            }
        }
        let scripts = std::rc::Rc::new(background_scripts::collect(self.transcript.items()));
        *self.background_scripts_memo.borrow_mut() = Some((revision, scripts.clone()));
        scripts
    }

    /// twarp: the header's **background-scripts icon button** — a terminal
    /// glyph sitting to the left of the Chat UI / Raw CLI toggle that opens the
    /// floating [`render_background_panel`](Self::render_background_panel)
    /// menu. While any of this chat's `run_in_background` Bash launches are
    /// still running, a small accent notification bubble overlays the glyph's
    /// top-right corner with the active count.
    ///
    /// Returns `None` (so the header shows no button) when this chat launched
    /// no background scripts. The button lives in the parent `PaneView`'s
    /// header tree, so its click routes through
    /// [`ClaudeCodeCustomAction::ToggleBackgroundPanel`] rather than dispatching
    /// the in-pane action directly.
    fn render_background_button(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let scripts = self.background_scripts();
        if scripts.is_empty() {
            return None;
        }
        let active = scripts.iter().filter(|s| s.state.is_active()).count();

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let accent = self.accent(app);
        let wash = self.accent_wash(app);
        let expanded = self.background_scripts_expanded;

        let glyph = ConstrainedBox::new(
            Icon::new(crate::ui_components::icons::Icon::Terminal.into(), accent).finish(),
        )
        .with_width(15.)
        .with_height(15.)
        .finish();

        // Compose the glyph with the floating notification bubble. The badge is
        // anchored to the glyph's top-right corner, nudged out so it reads as an
        // overlay rather than part of the icon.
        let mut stack = Stack::new();
        stack.add_child(
            Container::new(glyph)
                .with_padding(Padding::uniform(1.))
                .finish(),
        );
        if active > 0 {
            let label = appearance
                .ui_builder()
                .span(active.to_string())
                .with_style(UiComponentStyles {
                    font_color: Some(ColorU::white()),
                    font_size: Some(9.),
                    ..Default::default()
                })
                .build()
                .finish();
            let sized = ConstrainedBox::new(Align::new(label).finish())
                .with_min_width(13.)
                .with_min_height(13.)
                .finish();
            let badge = Container::new(sized)
                .with_padding_left(2.)
                .with_padding_right(2.)
                .with_background_color(accent)
                .with_border(Border::all(1.).with_border_fill(theme.background()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(7.)))
                .finish();
            stack.add_positioned_child(
                badge,
                OffsetPositioning::offset_from_parent(
                    vec2f(5., -5.),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::TopRight,
                    ChildAnchor::Center,
                ),
            );
        }

        let inner = stack.finish();
        let button = Hoverable::new(self.background_button_mouse.clone(), move |state| {
            let mut body = Container::new(inner)
                .with_padding(Padding::uniform(4.))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
            // Highlight while the panel is open (or hovered) so the button reads
            // as the panel's toggle, mirroring the active segment of the toggle.
            if expanded || state.is_hovered() {
                body = body.with_background_color(wash);
            }
            body.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action::<PaneHeaderAction<(), ClaudeCodeCustomAction>>(
                PaneHeaderAction::CustomAction(ClaudeCodeCustomAction::ToggleBackgroundPanel),
            );
        })
        .finish();
        Some(button)
    }

    /// twarp: the floating **background-scripts** panel — a per-chat status
    /// widget for the `run_in_background` Bash commands Claude launched in this
    /// session (a dev server, a watcher, a long build). Derived fresh from the
    /// transcript each render via [`background_scripts::collect`] — twarp keeps no
    /// separate model — so it stays in lock-step with the conversation and needs
    /// no teardown. Read-only: twarp can observe a background script (command,
    /// state, captured output) but can't start, poll, or kill it; those are
    /// Claude's tool calls.
    ///
    /// Opened from the header's
    /// [`render_background_button`](Self::render_background_button); returns
    /// `None` (floats nothing) unless the panel is expanded and this chat
    /// launched at least one background script. When shown it is the open menu:
    /// a header row (click to close) over one row per script, each expanding
    /// again to its captured output.
    fn render_background_panel(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        // twarp: the panel is now opened from the header's background-scripts
        // icon button (left of the Chat UI / Raw CLI toggle). It only floats
        // while expanded; collapsed, the header button is the sole affordance.
        if !self.background_scripts_expanded {
            return None;
        }
        let scripts = self.background_scripts();
        if scripts.is_empty() {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let accent = self.render_accent.get();
        let wash = self.render_wash.get();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let main = theme.main_text_color(theme.background()).into_solid();

        let active = scripts.iter().filter(|s| s.state.is_active()).count();
        let expanded = self.background_scripts_expanded;

        // Header: terminal glyph + title + a muted count, then a chevron whose
        // direction tracks the expand state. The whole row toggles the panel.
        let glyph = ConstrainedBox::new(
            Icon::new(crate::ui_components::icons::Icon::Terminal.into(), accent).finish(),
        )
        .with_width(15.)
        .with_height(15.)
        .finish();
        let title = appearance
            .ui_builder()
            .span("Background scripts".to_owned())
            .with_style(UiComponentStyles {
                font_color: Some(main),
                font_size: Some(12.5),
                ..Default::default()
            })
            .build()
            .finish();
        let count_text = if active > 0 {
            format!("{} · {active} running", scripts.len())
        } else {
            format!(
                "{} script{}",
                scripts.len(),
                if scripts.len() == 1 { "" } else { "s" }
            )
        };
        let count = appearance
            .ui_builder()
            .span(count_text)
            .with_style(UiComponentStyles {
                font_color: Some(muted),
                font_size: Some(11.),
                ..Default::default()
            })
            .build()
            .finish();
        let chevron_icon = if expanded {
            crate::ui_components::icons::Icon::ChevronUp
        } else {
            crate::ui_components::icons::Icon::ChevronDown
        };
        let chevron = ConstrainedBox::new(Icon::new(chevron_icon.into(), muted).finish())
            .with_width(14.)
            .with_height(14.)
            .finish();
        let header_inner = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_child(glyph)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(count)
            .with_child(chevron)
            .finish();
        let header = Hoverable::new(self.background_panel_mouse.clone(), move |state| {
            let mut body = Container::new(header_inner)
                .with_padding_left(12.)
                .with_padding_right(12.)
                .with_padding_top(9.)
                .with_padding_bottom(9.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)));
            if state.is_hovered() {
                body = body.with_background_color(wash);
            }
            body.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleBackgroundPanel);
        })
        .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(header);

        if expanded {
            for script in scripts.iter() {
                column.add_child(self.render_background_row(script, app));
            }
        }

        // The card: a surface that floats above the transcript, shadowed like the
        // composer menus so it reads as an overlay.
        let card = Container::new(column.finish())
            .with_background_color(theme.surface_1().into_solid())
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .with_drop_shadow(DropShadow::default())
            .finish();
        Some(
            ConstrainedBox::new(card)
                .with_max_width(BACKGROUND_PANEL_MAX_WIDTH)
                .finish(),
        )
    }

    /// One background-script row in the expanded panel: a status glyph + the
    /// command + its state label, expanding on click into the captured output.
    fn render_background_row(
        &self,
        script: &BackgroundScript,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let main = theme.main_text_color(theme.background()).into_solid();
        let wash = self.render_wash.get();

        let status_icon: Box<dyn Element> = match script.state {
            BackgroundScriptState::Running => inline_action::running_icon(appearance).finish(),
            BackgroundScriptState::Finished => inline_action::green_check_icon(appearance).finish(),
            BackgroundScriptState::LaunchFailed => inline_action::red_x_icon(appearance).finish(),
            BackgroundScriptState::Killed => {
                Icon::new(crate::ui_components::icons::Icon::Stop.into(), muted).finish()
            }
        };
        let status_icon = ConstrainedBox::new(status_icon)
            .with_width(13.)
            .with_height(13.)
            .finish();

        let command = tool_cards::format_command_text(&script.command);
        let command_text = warpui::elements::Text::new_inline(
            command,
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 1.,
        )
        .with_color(main.into())
        .with_selectable(false)
        .finish();
        let state_label = appearance
            .ui_builder()
            .span(script.state.label().to_owned())
            .with_style(UiComponentStyles {
                font_color: Some(muted),
                font_size: Some(10.5),
                ..Default::default()
            })
            .build()
            .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_child(status_icon)
            .with_child(Shrinkable::new(1., Clipped::new(command_text).finish()).finish())
            .with_child(state_label)
            .finish();

        let row_mouse = {
            let mut states = self.background_row_mouse.borrow_mut();
            states
                .entry(script.id.clone())
                .or_insert_with(MouseStateHandle::default)
                .clone()
        };
        let id = script.id.clone();
        let header = Hoverable::new(row_mouse, move |state| {
            let mut body = Container::new(row)
                .with_padding_left(12.)
                .with_padding_right(12.)
                .with_padding_top(7.)
                .with_padding_bottom(7.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
            if state.is_hovered() {
                body = body.with_background_color(wash);
            }
            body.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleBackgroundScript(id.clone()));
        })
        .finish();

        // The captured output, revealed on click. Capped the same way tool-card
        // results are so a chatty watcher can't stall layout.
        let output_open =
            self.background_expanded_rows.contains(&script.id) && !script.output.trim().is_empty();
        if !output_open {
            return header;
        }
        let (shown, hidden) = tool_cards::truncate_output(script.output.trim());
        let mut body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(header);
        body.add_child(
            Container::new(
                inline_action::render_requested_action_body_text(
                    std::borrow::Cow::Owned(shown),
                    appearance.monospace_font_family(),
                    app,
                )
                .finish(),
            )
            .with_padding_left(28.)
            .with_padding_right(12.)
            .with_padding_bottom(6.)
            .finish(),
        );
        if hidden > 0 {
            body.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(format!("… {hidden} more lines"))
                        .with_style(UiComponentStyles {
                            font_color: Some(muted),
                            font_size: Some(10.5),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding_left(28.)
                .with_padding_bottom(6.)
                .finish(),
            );
        }
        body.finish()
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

        // #7: a live status line below the last message while a turn streams —
        // an animated label + elapsed, replacing the composer's "Working…".
        if self.streaming {
            column.add_child(self.render_streaming_status(app));
        }

        // Clearance spacer between the last message and the end marker. It must
        // sit *above* the sentinel: `scroll_to_bottom` aligns the sentinel to the
        // viewport's bottom edge, so this spacer is what lifts the last message
        // clear of the floating composer (trailing padding *below* the sentinel
        // would just be scrolled out of view, behind the composer).
        column.add_child(
            ConstrainedBox::new(Container::new(Flex::row().finish()).finish())
                .with_height(COMPOSER_CLEARANCE)
                .finish(),
        );

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
            .with_padding_left(TRANSCRIPT_GUTTER)
            .with_padding_right(TRANSCRIPT_GUTTER)
            .finish();

        // PRODUCT §13: make the transcript prose highlightable. `SelectableArea`
        // is the coordinator that turns the per-element `set_selectable(true)`
        // into an actual drag-to-select gesture (it tracks the drag and paints
        // the selection). It must wrap the scrollable from the *outside*, not sit
        // inside it: outside, the `SelectableArea` keeps the viewport's fixed
        // origin/size (so the mouse-down hit-test in viewport coordinates works),
        // and the scrollable forwards `get_selection`/`expand_selection` down to
        // its scroll-translated children (`ClippedScrollable::as_selectable_element`),
        // which carry their real painted bounds. An earlier arrangement nested the
        // `SelectableArea` *inside* the `ClippedScrollable`; that compiled but
        // never selected at runtime — this is the proven outside-wrapping pattern
        // (see `warpui` `table-sample` and `NewScrollable`). The selected text is
        // mirrored into `transcript_selection`; Copy is handled in
        // `handle_editor_event` (an empty composer surfaces Cmd+C as
        // `EditorEvent::Copy`).
        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let selection = self.transcript_selection.clone();
        SelectableArea::new(
            self.selection_handle.clone(),
            move |args, ctx, _| {
                // A mouse-down on the transcript is consumed by this
                // `SelectableArea` (its `dispatch_event` returns "handled"),
                // so it never bubbles to the pane's outer focus-grab or the
                // pane group's `Activate` handler — without this, clicking
                // transcript text wouldn't select the pane (only the composer
                // did). Re-fire the focus-grab on the initiating click (no
                // selection yet) so a click anywhere in the transcript makes
                // this the active pane, matching the composer. See `FocusInput`.
                //
                // twarp: gate on an *actual selection change*. `SelectableArea`
                // invokes this handler on EVERY `LeftMouseDown`/`LeftMouseUp` in
                // the window — including clicks outside this transcript (see
                // `selectable_area.rs` `dispatch_event`). Dispatching `FocusInput`
                // on every `selection.is_none()` fire therefore stole application
                // focus back to this pane's composer whenever the user clicked
                // anything else (e.g. the left-panel file search), making those
                // inputs impossible to type into. An out-of-pane click leaves the
                // selection unchanged (typically `None` -> `None`), so skipping
                // unchanged fires keeps focus where the user put it while still
                // grabbing focus when a real in-transcript selection is cleared.
                if *selection.read() == args.selection {
                    return;
                }
                if args.selection.is_none() {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::FocusInput);
                }
                *selection.write() = args.selection;
            },
            scrollable,
        )
        .finish()
    }

    /// twarp 08d (PRODUCT §13–§16): the bottom gradient fade-out band.
    ///
    /// A full-width region pinned to the bottom of the pane, `TRANSCRIPT_FADE_HEIGHT`
    /// tall (tall enough to span the floating composer plus a soft zone above it).
    /// Its background is a vertical gradient that runs from fully transparent at the
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
            Container::new(Flex::row().with_main_axis_size(MainAxisSize::Max).finish())
                .with_background(Fill::Gradient {
                    start: vec2f(0., 0.),
                    end: vec2f(0., 1.),
                    start_color: transparent,
                    end_color: bg,
                })
                .finish(),
        )
        .with_height(TRANSCRIPT_FADE_HEIGHT)
        .finish()
    }

    /// The permission-mode selector (§25, #2): clicking opens a dropdown of the
    /// four modes, like the model picker. Static while a turn streams (the mode
    /// applies between turns).
    fn render_permission_control(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = prettify_permission_mode(self.permission_mode.as_cli_arg());
        if self.streaming {
            render_pill(&label, self.render_wash.get(), appearance)
        } else {
            render_clickable_pill(
                &label,
                self.permission_pill_mouse.clone(),
                |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                        ComposerMenu::Permission,
                    ))
                },
                self.render_wash.get(),
                ComposerMenu::Permission.anchor_id(),
                appearance,
            )
        }
    }

    /// The context-usage chip (#13): the live context-window fill, clickable to
    /// open the context breakdown popover. Read-only, so it opens even
    /// mid-turn.
    fn render_context_control(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = self
            .transcript
            .usage()
            .and_then(format_context)
            .unwrap_or_else(|| "Context".to_owned());
        render_clickable_pill(
            &label,
            self.context_button.clone(),
            |ctx| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                    ComposerMenu::Context,
                ))
            },
            self.render_wash.get(),
            ComposerMenu::Context.anchor_id(),
            appearance,
        )
    }

    /// The model chip (#13): the active model; clicking opens the model
    /// dropdown. Static while a turn streams (changing model restarts the
    /// session, §25).
    fn render_model_control(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = self
            .model
            .as_deref()
            .map(prettify_model)
            .or_else(|| self.transcript.model().map(prettify_model))
            .unwrap_or_else(|| "Model".to_owned());
        if self.streaming {
            render_pill(&label, self.render_wash.get(), appearance)
        } else {
            render_clickable_pill(
                &label,
                self.model_pill_mouse.clone(),
                |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                        ComposerMenu::Model,
                    ))
                },
                self.render_wash.get(),
                ComposerMenu::Model.anchor_id(),
                appearance,
            )
        }
    }

    /// The effort chip (#13): the selected effort; clicking opens the effort
    /// slider. Static while a turn streams.
    fn render_effort_control(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = match self.effort.as_deref() {
            Some(effort) => format!("Effort: {effort}"),
            None => "Effort".to_owned(),
        };
        if self.streaming {
            render_pill(&label, self.render_wash.get(), appearance)
        } else {
            render_clickable_pill(
                &label,
                self.effort_pill_mouse.clone(),
                |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                        ComposerMenu::Effort,
                    ))
                },
                self.render_wash.get(),
                ComposerMenu::Effort.anchor_id(),
                appearance,
            )
        }
    }

    /// The MCP viewer pill (feature 13): `MCP · N` where N is the count of
    /// servers the session reported at init. Read-only, so — unlike the
    /// permission / model / effort pills — it stays clickable mid-stream (like
    /// the context chip).
    fn render_mcp_control(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = format!("MCP · {}", self.transcript.mcp_servers().len());
        render_clickable_pill(
            &label,
            self.mcp_pill_mouse.clone(),
            |ctx| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                    ComposerMenu::Mcp,
                ))
            },
            self.render_wash.get(),
            ComposerMenu::Mcp.anchor_id(),
            appearance,
        )
    }

    /// The MCP viewer popover (feature 13): a read-only list of the session's
    /// MCP servers, each with a status indicator and tool count; clicking a row
    /// expands its (incrementally-derived) tool list. Mirrors the permission /
    /// CI menus' chrome. Empty state shows the CLI hint.
    fn render_mcp_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();
        // Reuse the CI menu's canonical status colours so the two popovers read
        // consistently (theme has no dedicated success/error accessor here).
        let green = ColorU::new(0, 142, 65, 255);
        let red = ColorU::new(188, 54, 42, 255);

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.)
            .with_child(menu_header("MCP servers", muted, appearance));

        let servers = self.transcript.mcp_servers();
        if servers.is_empty() {
            // Empty state (PRODUCT): the feature stays discoverable, and the row
            // points at the CLI as the place to add servers.
            column.add_child(context_segment(
                appearance,
                "No MCP servers connected.".to_owned(),
                muted,
            ));
            column.add_child(context_segment(
                appearance,
                "Configure with the claude CLI (claude mcp add).".to_owned(),
                muted,
            ));
            return column.finish();
        }

        let mut rows = self.mcp_menu_row_mouse.borrow_mut();
        for (index, server) in servers.iter().enumerate() {
            let mouse = pool_mouse(&mut rows, index);
            let expanded = self.mcp_expanded_server.as_deref() == Some(server.name.as_str());

            // Status indicator: connected → green, failed → red, anything else
            // (pending / unknown / a server seen only via a tool call) → muted.
            // A server listed at init without a status counts as connected
            // (it was reported), shown muted (PRODUCT "best-effort").
            let (status_color, status_label) = match server.status.as_deref() {
                Some("connected") => (green, "connected".to_owned()),
                Some("failed") => (red, "failed".to_owned()),
                Some("pending") => (muted, "pending".to_owned()),
                Some(other) => (muted, other.to_owned()),
                None => (muted, "connected".to_owned()),
            };
            let count_label = match server.tools.len() {
                0 => "tools unknown".to_owned(),
                1 => "1 tool".to_owned(),
                n => format!("{n} tools"),
            };

            let meta = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(context_segment(
                    appearance,
                    format!("\u{25CF} {status_label}"),
                    status_color,
                ))
                .with_child(context_segment(appearance, count_label, muted))
                .finish();
            let header_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(context_segment(
                    appearance,
                    server.name.clone(),
                    if expanded { accent } else { text_color },
                ))
                .with_child(meta)
                .finish();

            let mut row = Container::new(header_row)
                .with_padding_left(8.)
                .with_padding_right(8.)
                .with_padding_top(5.)
                .with_padding_bottom(5.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if expanded {
                row = row.with_background_color(theme.surface_1().into_solid());
            }
            let row = row.finish();
            let server_name = server.name.clone();
            column.add_child(
                Hoverable::new(mouse, move |_| row)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleMcpServer(
                            server_name.clone(),
                        ));
                    })
                    .finish(),
            );

            // Expanded: the bare tool names (prefix already stripped at parse
            // time), indented and muted, in first-seen order.
            if expanded {
                for tool in &server.tools {
                    column.add_child(
                        Container::new(context_segment(appearance, tool.clone(), muted))
                            .with_padding_left(20.)
                            .with_padding_top(2.)
                            .with_padding_bottom(2.)
                            .finish(),
                    );
                }
            }
        }
        column.finish()
    }

    /// The open composer dropdown / popover (#13), wrapped in a bordered card
    /// that sits above the input. `None` when nothing is open.
    fn render_composer_menu(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let menu = self.composer_menu?;
        let theme = appearance.theme();
        let content = match menu {
            ComposerMenu::Permission => self.render_permission_menu(appearance),
            ComposerMenu::Model => self.render_model_menu(appearance),
            ComposerMenu::Effort => self.render_effort_menu(appearance),
            ComposerMenu::Context => self.render_context_panel(appearance),
            ComposerMenu::Branch => self.render_branch_menu(appearance)?,
            ComposerMenu::Ci => self.render_ci_menu(appearance)?,
            ComposerMenu::Pr => self.render_pr_menu(appearance)?,
            ComposerMenu::Mcp => self.render_mcp_menu(appearance),
        };
        Some(
            // Cap the width so the dropdown stays a compact popover anchored to
            // its pill instead of stretching to the pane edge (issue #1). The
            // inner columns size to content up to this cap.
            ConstrainedBox::new(
                Container::new(content)
                    .with_padding(Padding::uniform(10.))
                    .with_background_color(theme.surface_2().into_solid())
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .finish(),
            )
            .with_max_width(COMPOSER_MENU_MAX_WIDTH)
            .finish(),
        )
    }

    /// The permission-mode dropdown (#2): a row per mode (title + the §25
    /// one-liner), the active one highlighted; clicking sets it and closes the
    /// menu.
    fn render_permission_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();
        const MODES: [PermissionMode; 4] = [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::BypassPermissions,
        ];
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.)
            .with_child(menu_header("Permission mode", muted, appearance));
        for (index, mode) in MODES.iter().enumerate() {
            let mode = *mode;
            let selected = self.permission_mode == mode;
            let mouse = {
                let mut pool = self.permission_menu_row_mouse.borrow_mut();
                while pool.len() <= index {
                    pool.push(MouseStateHandle::default());
                }
                pool[index].clone()
            };
            let title = prettify_permission_mode(mode.as_cli_arg());
            let label_col = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(1.)
                .with_child(context_segment(
                    appearance,
                    title,
                    if selected { accent } else { text_color },
                ))
                .with_child(context_segment(appearance, mode.label().to_owned(), muted))
                .finish();
            let mut row = Container::new(label_col)
                .with_padding_left(8.)
                .with_padding_right(8.)
                .with_padding_top(5.)
                .with_padding_bottom(5.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if selected {
                row = row.with_background_color(theme.surface_1().into_solid());
            }
            let row = row.finish();
            column.add_child(
                Hoverable::new(mouse, move |_| row)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::SetPermissionMode(mode));
                    })
                    .finish(),
            );
        }
        column.finish()
    }

    /// The model dropdown (#13): one row per `MODEL_CYCLE` entry, the active one
    /// highlighted; clicking sets the model and closes the menu.
    fn render_model_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();
        let current = self.model.as_deref();
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.)
            .with_child(menu_header("Model", muted, appearance));
        for (index, name) in MODEL_CYCLE.iter().enumerate() {
            // The first entry ("default") maps to no `--model` flag (None).
            let value: Option<String> = (*name != MODEL_CYCLE[0]).then(|| (*name).to_owned());
            let selected = current == value.as_deref();
            let mouse = {
                let mut pool = self.model_menu_row_mouse.borrow_mut();
                while pool.len() <= index {
                    pool.push(MouseStateHandle::default());
                }
                pool[index].clone()
            };
            let label = if *name == MODEL_CYCLE[0] {
                "Default".to_owned()
            } else {
                prettify_model(name)
            };
            let mut row = Container::new(context_segment(
                appearance,
                label,
                if selected { accent } else { text_color },
            ))
            .with_padding_left(8.)
            .with_padding_right(8.)
            .with_padding_top(5.)
            .with_padding_bottom(5.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if selected {
                row = row.with_background_color(theme.surface_1().into_solid());
            }
            let row = row.finish();
            let action_value = value.clone();
            column.add_child(
                Hoverable::new(mouse, move |_| row)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::SetModel(
                            action_value.clone(),
                        ));
                    })
                    .finish(),
            );
        }
        column.finish()
    }

    /// The effort slider (#13): a continuous slider snapped to the
    /// `EFFORT_CYCLE` levels (default … max). `on_change` is idempotent on the
    /// view side, so dragging across a level only restarts the session once.
    fn render_effort_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let max_index = (EFFORT_CYCLE.len() - 1) as f32;
        let current_index = EFFORT_CYCLE
            .iter()
            .position(|level| Some(*level) == self.effort.as_deref())
            .unwrap_or(0) as f32;
        let current_label = self.effort.as_deref().unwrap_or("default");
        let slider = appearance
            .ui_builder()
            .slider(self.effort_slider.clone())
            .with_range(0.0..max_index)
            .with_default_value(current_index)
            .with_style(UiComponentStyles {
                width: Some(240.),
                ..Default::default()
            })
            .on_change(|ctx, _, value| {
                let index = (value.round().max(0.0) as usize).min(EFFORT_CYCLE.len() - 1);
                // The first level ("default") maps to no `--effort` flag.
                let effort = (index > 0).then(|| EFFORT_CYCLE[index].to_owned());
                ctx.dispatch_typed_action(ClaudeCodeViewAction::SetEffort(effort));
            })
            .build()
            .finish();
        let labels = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(context_segment(appearance, "Default".to_owned(), muted))
            .with_child(context_segment(appearance, "Max".to_owned(), muted))
            .finish();
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.)
            .with_child(menu_header(
                &format!("Effort: {current_label}"),
                text_color,
                appearance,
            ))
            .with_child(ConstrainedBox::new(slider).with_width(240.).finish())
            .with_child(ConstrainedBox::new(labels).with_width(240.).finish())
            .finish()
    }

    /// The context-usage breakdown popover (#13). Shows what the headless
    /// stream-json actually exposes — total context used vs. the model's
    /// window, and the input / cache / output token split — not the per-source
    /// categories or plan limits the desktop app shows (those aren't in the
    /// CLI's output).
    fn render_context_panel(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();
        let track = theme.surface_3().into_solid();
        let Some(usage) = self.transcript.usage() else {
            return menu_header(
                "No context usage yet — send a message first.",
                muted,
                appearance,
            );
        };
        let used = usage.context_used();
        let summary = match usage.context_window {
            Some(window) if window > 0 => {
                let pct = (used as f64 / window as f64 * 100.0).round() as u64;
                format!("{} / {} ({pct}%)", fmt_tokens(used), fmt_tokens(window))
            }
            _ => format!("{} used", fmt_tokens(used)),
        };
        let fraction = usage
            .context_window
            .map(|window| {
                if window > 0 {
                    used as f32 / window as f32
                } else {
                    0.
                }
            })
            .unwrap_or(0.);
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.)
            .with_child(menu_header("Context window", muted, appearance))
            .with_child(context_segment(appearance, summary, text_color))
            .with_child(render_context_progress(fraction, accent, track))
            .with_child(usage_row(
                "Input",
                usage.input_tokens,
                muted,
                text_color,
                appearance,
            ))
            .with_child(usage_row(
                "Cache read",
                usage.cache_read_input_tokens,
                muted,
                text_color,
                appearance,
            ))
            .with_child(usage_row(
                "Cache write",
                usage.cache_creation_input_tokens,
                muted,
                text_color,
                appearance,
            ))
            .with_child(usage_row(
                "Output",
                usage.output_tokens,
                muted,
                text_color,
                appearance,
            ))
            .finish()
    }

    /// The branch pill's menu (#11): copy the branch name, open it on GitHub,
    /// and switch to another local branch. `None` if no branch resolved.
    fn render_branch_menu(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let context = self.repo_context.as_ref()?;
        let branch = context.branch.clone()?;
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();

        let mut rows = self.branch_menu_row_mouse.borrow_mut();
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(1.)
            .with_child(menu_header("Branch", muted, appearance));

        let mut index = 0;
        column.add_child(menu_action_row(
            "Copy branch name",
            text_color,
            pool_mouse(&mut rows, index),
            {
                let branch = branch.clone();
                move |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::CopyToClipboard(branch.clone()))
                }
            },
            appearance,
        ));
        index += 1;
        if context.branch_web_url().is_some() {
            column.add_child(menu_action_row(
                "Open branch in GitHub",
                text_color,
                pool_mouse(&mut rows, index),
                |ctx| ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenBranchInGitHub),
                appearance,
            ));
            index += 1;
        }

        // Switch to another local branch, most-recent first (capped).
        let others: Vec<String> = context
            .branches
            .iter()
            .filter(|b| **b != branch)
            .take(8)
            .cloned()
            .collect();
        if !others.is_empty() {
            column.add_child(menu_header("Switch to", muted, appearance));
            for name in others {
                column.add_child(menu_action_row(
                    &name,
                    text_color,
                    pool_mouse(&mut rows, index),
                    {
                        let name = name.clone();
                        move |ctx| {
                            ctx.dispatch_typed_action(ClaudeCodeViewAction::CheckoutBranch(
                                name.clone(),
                            ))
                        }
                    },
                    appearance,
                ));
                index += 1;
            }
        }
        Some(column.finish())
    }

    /// The CI pill's menu (#11): one row per status check, coloured by state,
    /// each opening its run on GitHub. `None` if there are no checks.
    fn render_ci_menu(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let context = self.repo_context.as_ref()?;
        if context.ci_checks.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let green = ColorU::new(0, 142, 65, 255);
        let red = ColorU::new(188, 54, 42, 255);
        let amber = ColorU::new(194, 128, 0, 255);

        let mut rows = self.ci_menu_row_mouse.borrow_mut();
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(1.)
            .with_child(menu_header("Checks", muted, appearance));
        for (index, check) in context.ci_checks.iter().take(12).enumerate() {
            while rows.len() <= index {
                rows.push(MouseStateHandle::default());
            }
            let color = match check.state {
                CiState::Passing => green,
                CiState::Failing => red,
                CiState::Pending => amber,
            };
            // A status glyph in the state colour, then the check name.
            let glyph = match check.state {
                CiState::Passing => "\u{2713}",
                CiState::Failing => "\u{2717}",
                CiState::Pending => "\u{25CB}",
            };
            let label = format!("{glyph}  {}", check.name);
            let url = check.url.clone();
            column.add_child(menu_action_row(
                &label,
                color,
                rows[index].clone(),
                move |ctx| {
                    if let Some(url) = &url {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenUrl(HyperlinkUrl {
                            url: url.clone(),
                        }));
                    }
                },
                appearance,
            ));
        }
        Some(column.finish())
    }

    /// The PR pill's menu (#11): open the PR on GitHub or copy its URL. `None`
    /// if there's no PR URL.
    fn render_pr_menu(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let context = self.repo_context.as_ref()?;
        let url = context.pr_url.clone()?;
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let header = match context.pr_number {
            Some(n) => format!("PR #{n}"),
            None => "Pull request".to_owned(),
        };
        let mut rows = self.ci_menu_row_mouse.borrow_mut();
        while rows.len() < 2 {
            rows.push(MouseStateHandle::default());
        }
        let open = menu_action_row(
            "Open PR in GitHub",
            text_color,
            rows[0].clone(),
            {
                let url = url.clone();
                move |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenUrl(HyperlinkUrl {
                        url: url.clone(),
                    }))
                }
            },
            appearance,
        );
        let copy = menu_action_row(
            "Copy PR URL",
            text_color,
            rows[1].clone(),
            move |ctx| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::CopyToClipboard(url.clone()))
            },
            appearance,
        );
        Some(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(1.)
                .with_child(menu_header(&header, muted, appearance))
                .with_child(open)
                .with_child(copy)
                .finish(),
        )
    }

    /// The composer context bar (#11): a row of pills above the input — folder,
    /// branch (→ branch menu), `+added −removed`, PR #n / Create PR, CI (→ check
    /// menu), and the worktree toggle. Each pill appears only when resolved
    /// ([`RepoContext`]); the whole bar is hidden until the first probe returns
    /// and stays hidden if nothing (not even a folder) resolved.
    fn render_repo_context_bar(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let context = self.repo_context.as_ref()?;
        if context.folder.is_none() && context.is_effectively_empty() {
            return None;
        }
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.surface_2()).into_solid();
        let accent = self.render_accent.get();
        // #3: semantic colours — green additions, red deletions (and the same
        // green/red/amber for CI as before).
        let green = ColorU::new(0, 142, 65, 255);
        let red = ColorU::new(188, 54, 42, 255);
        let wash = self.render_wash.get();
        // Semantic CI / diff colour for a state (mirroring success/error/warn).
        let ci_color = |ci: CiState| match ci {
            CiState::Passing => green,
            CiState::Failing => red,
            CiState::Pending => ColorU::new(194, 128, 0, 255),
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.);

        // Folder — static (#11).
        if let Some(folder) = &context.folder {
            row.add_child(render_context_pill(
                folder.clone(),
                text_color,
                wash,
                appearance,
            ));
        }
        // Branch — clickable, opens the branch menu (copy / open on GitHub /
        // switch). Always shown when resolved, including on the default branch.
        if let Some(branch) = &context.branch {
            row.add_child(render_context_menu_pill(
                branch.clone(),
                accent,
                wash,
                self.branch_pill_mouse.clone(),
                ComposerMenu::Branch.anchor_id(),
                |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                        ComposerMenu::Branch,
                    ))
                },
                appearance,
            ));
        }
        // Diff `+N −M` — static, as one unit.
        if context.added.is_some() || context.removed.is_some() {
            let added = context.added.unwrap_or(0);
            let removed = context.removed.unwrap_or(0);
            row.add_child(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(4.)
                        .with_child(context_segment(appearance, format!("+{added}"), green))
                        .with_child(context_segment(
                            appearance,
                            format!("\u{2212}{removed}"),
                            red,
                        ))
                        .finish(),
                )
                .with_padding_left(8.)
                .with_padding_right(8.)
                .with_padding_top(3.)
                .with_padding_bottom(3.)
                .with_background_color(wash)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
                .finish(),
            );
        }
        // PR — `PR #n` opens its menu when one exists; otherwise a "Create PR"
        // button when we have a branch on a GitHub remote.
        if let Some(pr) = context.pr_number {
            row.add_child(render_context_menu_pill(
                format!("PR #{pr}"),
                accent,
                wash,
                self.pr_pill_mouse.clone(),
                ComposerMenu::Pr.anchor_id(),
                |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                        ComposerMenu::Pr,
                    ))
                },
                appearance,
            ));
        } else if context.branch.is_some() && context.repo_web_url.is_some() {
            row.add_child(
                Container::new(
                    Hoverable::new(self.pr_pill_mouse.clone(), {
                        let pill =
                            render_context_pill("Create PR".to_owned(), accent, wash, appearance);
                        move |_| pill
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::CreatePr);
                    })
                    .finish(),
                )
                .finish(),
            );
        }
        // CI — clickable, opens the per-check menu.
        if let Some(ci) = context.ci {
            row.add_child(render_context_menu_pill(
                ci.label().to_owned(),
                ci_color(ci),
                wash,
                self.ci_pill_mouse.clone(),
                ComposerMenu::Ci.anchor_id(),
                |ctx| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleComposerMenu(
                        ComposerMenu::Ci,
                    ))
                },
                appearance,
            ));
        }
        // Worktree toggle (#11): only meaningful before a session starts — it
        // governs where the *first* turn spawns. Hidden once live.
        if self.message_tx.is_none() && context.branch.is_some() {
            let on = self.use_worktree;
            let label = format!("{} worktree", if on { "\u{2611}" } else { "\u{2610}" });
            row.add_child(
                Container::new(
                    Hoverable::new(self.worktree_toggle_mouse.clone(), {
                        let pill = render_context_pill(
                            label,
                            if on { accent } else { text_color },
                            wash,
                            appearance,
                        );
                        move |_| pill
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleWorktree);
                    })
                    .finish(),
                )
                .finish(),
            );
        }

        Some(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(row.finish())
                .finish(),
        )
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

        // #13: the Send / Stop action.
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
            // #10: the primary button is the pane's accent (the tab colour),
            // not the theme accent — so a custom-coloured button rather than
            // ButtonVariant::Accent. The label colour contrasts the fill.
            let accent = self.render_accent.get();
            let text_color = contrasting_text(accent);
            let button_label = appearance
                .ui_builder()
                .span(label.to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(text_color),
                    font_size: Some(13.),
                    ..Default::default()
                })
                .build()
                .finish();
            let button = Container::new(button_label)
                .with_padding_left(12.)
                .with_padding_right(12.)
                .with_padding_top(6.)
                .with_padding_bottom(6.)
                .with_background_color(accent)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish();
            Hoverable::new(self.submit_button.clone(), move |_| button)
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::Submit);
                })
                .finish()
        };

        // PRODUCT §51 (7l): the "＋ attach" control opens the OS file picker.
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

        // #13: Claude-style footer below the input — permission selector and
        // attach on the left; the context / model / effort controls (each opens
        // a dropdown / popover above the input) and the Send/Stop action on the
        // right. (#7: the streaming indicator moved out of here to below the
        // last message.)
        let left = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(self.render_permission_control(appearance))
            .with_child(attach)
            .finish();

        let right = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(self.render_mcp_control(appearance))
            .with_child(self.render_context_control(appearance))
            .with_child(self.render_model_control(appearance))
            .with_child(self.render_effort_control(appearance))
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
        // 7l: while a file drag hovers the pane, replace the editor with a
        // "Drop to attach" hint and light the card up — the composer becomes the
        // drop target. Otherwise it holds the message input (and its queue /
        // suggestions / attachment chips); the control pills moved out, below.
        if self.drag_active {
            let accent = self.render_accent.get();
            card_column.add_child(
                ConstrainedBox::new(
                    Align::new(
                        appearance
                            .ui_builder()
                            .span("Drop to attach files".to_owned())
                            .with_style(UiComponentStyles {
                                font_color: Some(accent),
                                font_size: Some(13.),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .finish(),
                )
                .with_height(COMPOSER_MAX_HEIGHT.min(56.))
                .finish(),
            );
        } else {
            card_column.add_child(editor);
        }
        // #4: the input card holds ONLY the message input (and its queue /
        // suggestions / attachment chips) — the control pills moved out, below.
        let (card_border, card_fill) = if self.drag_active {
            (
                Border::all(1.5).with_border_color(self.render_accent.get()),
                self.render_wash.get(),
            )
        } else {
            (
                Border::all(1.).with_border_fill(theme.outline()),
                theme.surface_1().into_solid(),
            )
        };
        let card = Container::new(card_column.finish())
            .with_padding(Padding::uniform(10.))
            .with_background_color(card_fill)
            .with_border(card_border)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                COMPOSER_CORNER_RADIUS,
            )))
            // The composer floats over the transcript; the shadow separates the
            // layers (same treatment as the input-suggestions detail panel).
            .with_drop_shadow(DropShadow::default())
            .finish();

        // Composer column, top → bottom: context bar (#11) above the input;
        // the input card; then the control pills (#4) below the input, outside
        // the card. The open dropdown / popover (#13/#2) is no longer a column
        // child — it floats as a positioned overlay anchored to its trigger
        // pill (below), so it overlaps neighbouring content instead of pushing
        // the layout open between the pills and the input.
        let mut composer_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.);
        if let Some(bar) = self.render_repo_context_bar(appearance) {
            composer_column.add_child(bar);
        }
        composer_column.add_child(card);
        composer_column.add_child(controls);

        let composer = Container::new(composer_column.finish())
            .with_padding_top(6.)
            .with_padding_bottom(12.)
            // Keep a gutter so the card doesn't touch the pane edges in a narrow
            // pane (the outer container no longer pads horizontally — that gutter
            // moved inside the transcript scroller so the scrollbar hugs the edge).
            .with_padding_left(TRANSCRIPT_GUTTER)
            .with_padding_right(TRANSCRIPT_GUTTER)
            .finish();

        // twarp: the floating "scroll to bottom" button, shown above the
        // composer's right edge only while the transcript has content and is
        // scrolled up off the bottom (`is_at_bottom` is true — so the button is
        // hidden — whenever the content fits or the view is following the
        // latest message). A scroll calls `notify()`, so `render` re-runs and
        // the button appears/disappears as the user scrolls.
        let scroll_button = (!self.transcript.is_empty()
            && !self.scroll_state.is_at_bottom(AUTOSCROLL_STICK_SLACK))
        .then(|| self.render_scroll_to_bottom_button(appearance));

        // The open dropdown floats above everything, anchored to the trigger
        // pill's saved position (#11/#13). Context-bar pills (branch / CI / PR)
        // sit above the input, so their menus drop downward; the bottom control
        // pills open upward. A click-outside `Dismiss` scrim closes it.
        let open_menu = self
            .composer_menu
            .zip(self.render_composer_menu(appearance));

        // Nothing floats over the composer — return it bare.
        if scroll_button.is_none() && open_menu.is_none() {
            return composer;
        }

        let mut stack = Stack::new();
        stack.add_child(composer);
        // Anchor the button's bottom-right just above the composer's top-right
        // (indented by the composer's own right gutter so it lines up with the
        // input card's edge). It rides with the centered, width-capped composer
        // card rather than the full pane, so it stays at the conversation's
        // right edge in a wide pane.
        if let Some(button) = scroll_button {
            // An *overlay* child, not a plain positioned one: the button floats
            // *outside* the composer rect (above its top edge, over the
            // transcript), so as a normal-layer child its click hit-test
            // (`Hoverable::is_mouse_over_element`) was rejected — the transcript
            // scrollable painted around it left the button's point reading as
            // `is_covered`, so a press silently did nothing even though the
            // button painted visibly. The overlay layer is unclipped and sits
            // above all normal content (the same reason the composer menu below
            // uses `add_positioned_overlay_child`), so the click lands.
            stack.add_positioned_overlay_child(
                button,
                OffsetPositioning::offset_from_parent(
                    vec2f(-TRANSCRIPT_GUTTER, -8.),
                    // The button floats *above* the composer's top edge, so it
                    // sits outside the parent (composer) rect. `ParentBySize`
                    // clamps the child's position back inside the parent's
                    // bounds (see `compute_child_position`), which pinned the
                    // button to the composer's top edge — hidden behind the
                    // context bar — so it never appeared. `Unbounded` lets it
                    // overflow above the composer as intended.
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopRight,
                    ChildAnchor::BottomRight,
                ),
            );
        }
        let Some((anchor, menu)) = open_menu else {
            return stack.finish();
        };
        let menu = Container::new(
            // NOT `prevent_interaction_with_other_elements`: that flag makes
            // `Dismiss` paint a window-spanning hit-recording rect under the
            // menu, so a click *outside* the menu is both dismissed AND
            // swallowed — it never reaches whatever was clicked. With the
            // composer dropdown open, that ate the first click into any other
            // input field in the app (terminal, search, another pane), so you
            // had to click twice. The layered (non-modal) `Dismiss` still fires
            // the dismiss on an outside click but propagates the event through,
            // so the underlying input focuses on the same click.
            Dismiss::new(menu)
                .on_dismiss(|ctx, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::CloseComposerMenu);
                })
                .finish(),
        )
        .with_drop_shadow(DropShadow::default())
        .finish();
        let (element_anchor, child_anchor, offset) = if anchor.opens_downward() {
            (
                PositionedElementAnchor::BottomLeft,
                ChildAnchor::TopLeft,
                vec2f(0., 6.),
            )
        } else {
            (
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
                vec2f(0., -6.),
            )
        };
        stack.add_positioned_overlay_child(
            menu,
            OffsetPositioning::offset_from_save_position_element(
                anchor.anchor_id(),
                offset,
                PositionedElementOffsetBounds::WindowByPosition,
                element_anchor,
                child_anchor,
            ),
        );
        stack.finish()
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
        log::info!(
            "FOCUSDBG claude on_focus session={} is_self_focused={} emit FocusSelf",
            self.session_id,
            focus_ctx.is_self_focused()
        );
        // PRODUCT §34: focus the input on entry so typing just works. Focusing a
        // child keeps the view itself in the responder chain, so in-pane
        // `ClaudeCodeViewAction` dispatches reach `handle_action` below.
        // In raw mode the embedded terminal owns the keyboard instead (§43).
        if focus_ctx.is_self_focused() {
            self.focus(ctx);
        }
        // #13: tell the pane group this pane is now the focused pane (it doesn't
        // watch editor focus itself), so Cmd+W / maximize / Open Changes target
        // this pane instead of whatever was focused before.
        //
        // Report it pane-specifically via `PaneEvent::FocusSelf` rather than the
        // generic `PaneGroupAction::HandleFocusChange`. `HandleFocusChange`
        // *rescans* every pane and keeps whichever currently passes
        // `has_application_focus` — fine with one pane, but with two Claude panes
        // in the same group that global rescan races: the click and the deferred
        // rescan straddle a focus-settling window, so it can latch onto the wrong
        // pane and the input feels stuck on one of them (the "side-by-side Claude
        // panes lock the composer" report). `FocusSelf` names *this* pane by id,
        // so the focused pane is deterministic regardless of timing.
        //
        // It also sidesteps the circular-view-reference panic that forced the old
        // deferral: `focus_pane_by_id` never calls `has_application_focus`, and
        // the event is delivered to the pane group's subscriber (outside this
        // view's own update), so nothing reads this view while we hold `&mut
        // self`.
        ctx.emit(ClaudeCodeViewEvent::Pane(PaneEvent::FocusSelf));
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // #10/#11: resolve the tab-derived accent + wash once, for the whole
        // render tree (read via `render_accent` / `render_wash`).
        self.render_accent.set(self.accent(app));
        self.render_wash.set(self.accent_wash(app));

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
                // Zero state pinned to the top-center so the "Welcome back"
                // launchpad (which grows with the recent-session list) clears
                // the floating composer at the pane bottom.
                Align::new(self.render_body(app)).top_center().finish()
            } else {
                // Full-height transcript: it scrolls behind the floating
                // composer so content dissolves under it (the bottom fade band
                // + the composer's COMPOSER_CLEARANCE bottom padding handle the
                // visual hand-off). The overlay scrollbar therefore runs the
                // full pane height at the right edge.
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
            // twarp: the floating background-scripts panel, pinned to the pane's
            // top-right above the transcript so a long-running dev server / watcher
            // Claude launched stays visible while the conversation scrolls under
            // it. `None` (and so nothing floated) until this chat launches one.
            if let Some(panel) = self.render_background_panel(app) {
                stack.add_positioned_child(
                    panel,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-12., 12.),
                        ParentOffsetBounds::ParentBySize,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                );
            }
            // No horizontal padding here: the transcript scrollable spans the
            // full pane width so its overlay scrollbar hugs the right edge
            // (the prose keeps its margins via TRANSCRIPT_GUTTER inside the
            // scroller). The floating composer is centred + width-capped, so it
            // is unaffected.
            Container::new(stack.finish())
                .with_background_color(theme.background().into_solid())
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
            // 7l: while a drag hovers the pane, light up the composer so the drop
            // target is obvious; clear it the moment the drag leaves the window.
            .on_file_drag(|ctx, _, _, in_bounds| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::SetDragActive(in_bounds));
                DispatchEventResult::StopPropagation
            })
            .on_file_drag_exit(|ctx, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::SetDragActive(false));
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
            ClaudeCodeViewAction::FocusInput => {
                log::info!(
                    "FOCUSDBG claude FocusInput handled session={} -> focus input_editor",
                    self.session_id
                );
                ctx.focus(&self.input_editor);
                // #13: a click on the pane focuses the editor — report it so the
                // pane group makes THIS pane (by id) the active one for Cmd+W /
                // maximize. `FocusSelf` rather than the global
                // `HandleFocusChange` rescan, which races between side-by-side
                // Claude panes (see `on_focus`).
                ctx.emit(ClaudeCodeViewEvent::Pane(PaneEvent::FocusSelf));
            }
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
            ClaudeCodeViewAction::ScrollToBottom => {
                self.scroll_to_bottom();
                ctx.notify();
            }
            ClaudeCodeViewAction::ToggleToolCard(id) => self.toggle_tool_card(id, ctx),
            ClaudeCodeViewAction::ToggleThinking(index) => self.toggle_thinking(*index, ctx),
            ClaudeCodeViewAction::ToggleTodos => {
                self.todos_expanded = !self.todos_expanded;
                ctx.notify();
            }
            ClaudeCodeViewAction::SetPermissionMode(mode) => self.set_permission_mode(*mode, ctx),
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
                self.drag_active = false;
                let paths = paths.iter().map(PathBuf::from).collect();
                self.attach_files(paths, ctx);
            }
            ClaudeCodeViewAction::SetDragActive(active) => {
                if self.drag_active != *active {
                    self.drag_active = *active;
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::AttachFromPicker => self.open_attach_picker(ctx),
            ClaudeCodeViewAction::RemoveDirectAttachment(index) => {
                if *index < self.direct_attachments.len() {
                    self.direct_attachments.remove(*index);
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::OpenSentImage(path) => {
                ctx.open_file_path(Path::new(path));
            }
            ClaudeCodeViewAction::ToggleComposerMenu(menu) => self.toggle_composer_menu(*menu, ctx),
            ClaudeCodeViewAction::CloseComposerMenu => {
                if self.composer_menu.take().is_some() {
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::CopyToClipboard(text) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()));
                self.composer_menu = None;
                ctx.notify();
            }
            ClaudeCodeViewAction::OpenBranchInGitHub => {
                if let Some(url) = self
                    .repo_context
                    .as_ref()
                    .and_then(|context| context.branch_web_url())
                {
                    ctx.open_url(&url);
                }
                self.composer_menu = None;
                ctx.notify();
            }
            ClaudeCodeViewAction::CheckoutBranch(name) => self.checkout_branch(name.clone(), ctx),
            ClaudeCodeViewAction::CreatePr => self.create_pr(ctx),
            ClaudeCodeViewAction::ToggleWorktree => {
                self.use_worktree = !self.use_worktree;
                ctx.notify();
            }
            ClaudeCodeViewAction::SetModel(model) => self.set_model(model.clone(), ctx),
            ClaudeCodeViewAction::SetEffort(effort) => self.set_effort(effort.clone(), ctx),
            ClaudeCodeViewAction::RemoveQueuedMessage(index) => {
                if *index < self.message_queue.len() {
                    self.message_queue.remove(*index);
                    // Row indices shift after a removal — drop the expansion
                    // state rather than leave it pointing at the wrong row.
                    self.queue_expanded.clear();
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::ToggleQueuedMessage(index) => {
                if *index < self.message_queue.len() {
                    if !self.queue_expanded.remove(index) {
                        self.queue_expanded.insert(*index);
                    }
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::SendQueuedMessageNow(index) => self.send_queued_now(*index, ctx),
            ClaudeCodeViewAction::ApprovePlan => self.approve_plan(ctx),
            ClaudeCodeViewAction::SelectQuestionOption {
                item,
                option,
                multi,
            } => self.select_question_option(*item, *option, *multi, ctx),
            ClaudeCodeViewAction::SubmitQuestionAnswers(item) => {
                self.submit_question_answers(*item, ctx)
            }
            ClaudeCodeViewAction::AnswerPermission { request_id, allow } => {
                self.answer_permission(request_id, *allow, ctx)
            }
            ClaudeCodeViewAction::SubmitQuestionDialog(item) => {
                self.submit_question_dialog(*item, ctx)
            }
            ClaudeCodeViewAction::ResumeRecentSession(index) => {
                self.resume_recent_session(*index, ctx)
            }
            ClaudeCodeViewAction::ForkConversation(index) => self.fork_conversation(*index, ctx),
            ClaudeCodeViewAction::ToggleBackgroundPanel => {
                self.background_scripts_expanded = !self.background_scripts_expanded;
                ctx.notify();
            }
            ClaudeCodeViewAction::ToggleBackgroundScript(id) => {
                if !self.background_expanded_rows.remove(id) {
                    self.background_expanded_rows.insert(id.clone());
                }
                ctx.notify();
            }
            ClaudeCodeViewAction::ToggleMcpServer(name) => {
                // One server expanded at a time (feature 13): re-clicking the
                // open server collapses it.
                self.mcp_expanded_server = if self.mcp_expanded_server.as_deref() == Some(name) {
                    None
                } else {
                    Some(name.clone())
                };
                ctx.notify();
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
            ClaudeCodeCustomAction::ToggleBackgroundPanel => {
                self.background_scripts_expanded = !self.background_scripts_expanded;
                ctx.notify();
            }
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
        // #7: a [ Chat UI | Raw CLI ] section toggle (a segmented control), the
        // active section highlighted. Entering raw mode is disabled mid-turn
        // (§42); leaving it is always allowed. Clicking the inactive section
        // flips via the pane framework's CustomAction (header chrome can't
        // dispatch an in-pane action).
        let appearance = Appearance::as_ref(app);
        // #10/#11: theme the toggle to the tab accent (the header renders
        // separately from the body, so compute it here rather than via the cell).
        let accent = self.accent(app);
        let wash = self.accent_wash(app);
        let raw_mode = self.raw_cli.is_some();
        let chat_segment = render_mode_segment(
            "Chat UI",
            !raw_mode, // active when in chat
            raw_mode,  // clickable only from raw mode (to exit)
            self.chat_ui_button.clone(),
            accent,
            wash,
            appearance,
        );
        let raw_segment = render_mode_segment(
            "Raw CLI",
            raw_mode,                     // active when in raw
            !raw_mode && !self.streaming, // enter raw only when idle
            self.raw_cli_button.clone(),
            accent,
            wash,
            appearance,
        );
        let theme = appearance.theme();
        let toggle = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(2.)
                .with_child(chat_segment)
                .with_child(raw_segment)
                .finish(),
        )
        .with_padding(Padding::uniform(2.))
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        // #5: breathing room between the toggle and the always-visible close ✕.
        .with_margin_right(10.)
        .finish();
        // twarp: the background-scripts icon button sits to the LEFT of the
        // toggle, opening the floating panel and badging the active run count.
        // Present only when this chat launched a background script — widen the
        // header control budget to fit it when it is.
        let background_button = self.render_background_button(app);
        let has_background_button = background_button.is_some();
        let left_of_overflow: Box<dyn Element> = match background_button {
            Some(button) => Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(button)
                .with_child(toggle)
                .finish(),
            None => toggle,
        };
        HeaderContent::Standard(StandardHeader {
            title: PANE_TITLE.to_owned(),
            title_secondary: cwd,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: Some(left_of_overflow),
            options: StandardHeaderOptions {
                // #5: keep the close ✕ (and overflow) visible without hovering.
                always_show_icons: true,
                // The [ Chat UI | Raw CLI ] segmented toggle is wider than the
                // default 80px right-edge budget — without this it overflowed
                // off the right of the window. Reserve room for the toggle plus
                // the close/overflow icons, and extra for the background-scripts
                // button when it is shown.
                control_container_width: Some(if has_background_button { 250. } else { 210. }),
                ..Default::default()
            },
            // twarp: double-clicking the "Claude Code" header title renames the
            // session's tab, reusing the workspace rename editor (the tab strip
            // sits directly above the pane header). Mirrors double-clicking the
            // tab itself, but reachable from the pane the user is looking at.
            title_on_double_click: Some(Box::new(|ctx| {
                ctx.dispatch_typed_action(WorkspaceAction::RenameActiveTab);
            })),
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
                let bubble = render_message_row(
                    true,
                    USER_ICON_SVG_PATH,
                    text,
                    self.render_accent.get(),
                    appearance,
                );
                // #8: any images sent with this turn preview above the bubble.
                match self.sent_images.get(&index) {
                    Some(paths) if !paths.is_empty() => Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_spacing(6.)
                        .with_child(self.render_sent_images(paths))
                        .with_child(bubble)
                        .finish(),
                    _ => bubble,
                }
            }
            TranscriptItem::Assistant { text, .. } => {
                self.render_assistant_response(index, text, self.is_reply_end(index), app)
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
                    name,
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
            // 7g (PRODUCT §24): an interactive permission prompt — the driver
            // wires `--permission-prompt-tool stdio`, so `claude` raises a
            // `can_use_tool` control_request the pane answers Allow/Deny inline.
            TranscriptItem::Permission {
                id,
                tool,
                input,
                decision,
            } => self.render_permission_card(id, tool, input, *decision, appearance),
            // 7g (PRODUCT §24/§1): an `AskUserQuestion` raised over the control
            // channel (`request_user_dialog`) — render the interactive question
            // card; the answer is sent back over the same channel.
            TranscriptItem::Question {
                id,
                payload,
                answered,
                ..
            } => self.render_question_dialog_card(index, id, payload, *answered, appearance),
        }
    }

    /// Render an interactive permission prompt (7g, PRODUCT §24): a `can_use_tool`
    /// control_request `claude` raised because a tool needs approval in a
    /// prompting mode. Shows the tool + a one-line summary of what it would do,
    /// with **Allow** / **Deny**. Once answered (`decision` set) the buttons are
    /// replaced by a static outcome line, so a historical card carries no dead
    /// control. While pending, the buttons answer over the control channel
    /// ([`Self::answer_permission`]).
    fn render_permission_card(
        &self,
        id: &str,
        tool: &str,
        input: &serde_json::Value,
        decision: Option<bool>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let surface = theme.surface_2();
        let text_color = theme.main_text_color(surface).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(
                ConstrainedBox::new(
                    Icon::new(crate::ui_components::icons::Icon::Lock.into(), accent).finish(),
                )
                .with_width(15.)
                .with_height(15.)
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .span("Permission".to_owned())
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
            .with_child(header)
            .with_child(
                appearance
                    .ui_builder()
                    .span(format!("Claude wants to use {tool}."))
                    .with_soft_wrap()
                    .with_style(UiComponentStyles {
                        font_color: Some(text_color),
                        font_size: Some(BODY_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );

        // A one-line summary of the concrete action (the command, the file…),
        // shown in a muted row so the user knows what they grant.
        if let Some(summary) = permission_summary(tool, input) {
            column.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(summary)
                        .with_soft_wrap()
                        .with_style(UiComponentStyles {
                            font_color: Some(muted),
                            font_size: Some(12.),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding(Padding::uniform(8.))
                .with_background_color(theme.surface_1().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
            );
        }

        match decision {
            // Pending: offer Allow / Deny while the session is still live to
            // receive the answer (PRODUCT §26: never offer a dead control).
            None if self.message_tx.is_some() => {
                let allow_mouse = self
                    .permission_button_mouse
                    .borrow_mut()
                    .entry((id.to_owned(), true))
                    .or_default()
                    .clone();
                let deny_mouse = self
                    .permission_button_mouse
                    .borrow_mut()
                    .entry((id.to_owned(), false))
                    .or_default()
                    .clone();
                let allow_id = id.to_owned();
                let deny_id = id.to_owned();
                let allow = appearance
                    .ui_builder()
                    .button(ButtonVariant::Accent, allow_mouse)
                    .with_text_label("Allow".to_owned())
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::AnswerPermission {
                            request_id: allow_id.clone(),
                            allow: true,
                        });
                    })
                    .finish();
                let deny = appearance
                    .ui_builder()
                    .button(ButtonVariant::Outlined, deny_mouse)
                    .with_text_label("Deny".to_owned())
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::AnswerPermission {
                            request_id: deny_id.clone(),
                            allow: false,
                        });
                    })
                    .finish();
                column.add_child(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(10.)
                        .with_child(allow)
                        .with_child(deny)
                        .finish(),
                );
            }
            // Answered (or the session is gone): a static outcome line.
            decision => {
                let label = match decision {
                    Some(true) => "Allowed",
                    Some(false) => "Denied",
                    None => "Awaiting a new session",
                };
                column.add_child(
                    appearance
                        .ui_builder()
                        .span(label.to_owned())
                        .with_style(UiComponentStyles {
                            font_color: Some(muted),
                            font_size: Some(12.),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }
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
        let accent = self.render_accent.get();

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
        // Tool-card path (PRODUCT §1): headless `claude` auto-dismisses the
        // `AskUserQuestion` tool and ends the turn, so controls are live only
        // while the session is idle and the pick is sent as the next turn.
        let questions = parse_questions(input);
        self.render_question_card_inner(index, &questions, !self.streaming, false, appearance)
    }

    /// 7g (PRODUCT §24/§1): an `AskUserQuestion` raised over the *control*
    /// channel (`request_user_dialog`). Same card as the tool path, but the
    /// session is paused on the dialog, so controls stay live until the user
    /// answers; submitting releases the dialog and resends the picks
    /// ([`Self::submit_question_dialog`]).
    fn render_question_dialog_card(
        &self,
        index: usize,
        _id: &str,
        payload: &serde_json::Value,
        answered: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let questions = parse_questions(payload);
        let interactive = !answered && self.message_tx.is_some();
        self.render_question_card_inner(index, &questions, interactive, true, appearance)
    }

    /// Shared body for both question cards (tool-card §1 and control-channel
    /// §24). `interactive` gates the clickable options + Send button; `dialog`
    /// selects which submit action fires (control-channel vs. next-turn).
    fn render_question_card_inner(
        &self,
        index: usize,
        questions: &[ParsedQuestion],
        interactive: bool,
        dialog: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let surface = theme.surface_2();
        let text_color = theme.main_text_color(surface).into_solid();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();
        let selected = self.question_selected.get(&index);

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

        for question in questions {
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
                    (true, true) => "\u{2611}",   // ballot box with check
                    (true, false) => "\u{2610}",  // ballot box
                    (false, true) => "\u{25C9}",  // fisheye (filled radio)
                    (false, false) => "\u{25CB}", // circle
                };
                let mut option_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
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
                // The label/description are soft-wrap text. A non-flexible child
                // of a row is laid out with an unbounded main-axis (width)
                // constraint, which a `Stretch` column turns into a *tight
                // infinite* width — and soft-wrap text under an infinite width
                // returns an infinite size, tripping the scene's finite-rect
                // assert (the "twarp keeps crashing" report). Make the label
                // column flexible so it receives the row's remaining (finite)
                // width and wraps within it, mirroring `render_message_row`.
                option_row.add_child(Shrinkable::new(1., label_col.finish()).finish());
                let mut row_container = Container::new(option_row.finish())
                    .with_padding_left(8.)
                    .with_padding_right(8.)
                    .with_padding_top(6.)
                    .with_padding_bottom(6.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
                if is_selected {
                    row_container =
                        row_container.with_background_color(theme.surface_1().into_solid());
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
                                ctx.dispatch_typed_action(if dialog {
                                    ClaudeCodeViewAction::SubmitQuestionDialog(index)
                                } else {
                                    ClaudeCodeViewAction::SubmitQuestionAnswers(index)
                                });
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

/// A one-line, human summary of the concrete action a permission prompt grants
/// (7g, PRODUCT §24): the command for `Bash`, the path for the file tools, the
/// URL for `WebFetch`. `None` for tools with no obvious single field — the card
/// then shows the tool name alone. Defensive: a missing field just degrades to
/// `None` (never panics).
fn permission_summary(tool: &str, input: &serde_json::Value) -> Option<String> {
    let field = |key: &str| input.get(key).and_then(|v| v.as_str());
    let value = match tool {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => field("file_path"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "Glob" | "Grep" => field("pattern"),
        _ => None,
    }?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    // Keep the card compact — a long command/path is truncated with an ellipsis.
    const MAX: usize = 240;
    if value.chars().count() > MAX {
        Some(format!("{}…", value.chars().take(MAX).collect::<String>()))
    } else {
        Some(value.to_owned())
    }
}

/// Whether a `can_use_tool` permission prompt for `tool` should be auto-allowed
/// rather than surfaced as an interactive Permission card (PRODUCT §1). Only
/// `AskUserQuestion`: gating it behind a prompt wedges the whole turn (claude
/// blocks on the permission) while its inline Question card is non-interactive
/// mid-stream, leaving the user no way to answer. Auto-allowing lets the tool
/// return at once so the turn ends and the live Question card takes the pick.
fn should_auto_allow_permission(tool: &str) -> bool {
    tool == "AskUserQuestion"
}

/// The `{questions:[..]}` source for a question card, whichever path produced
/// it: the `AskUserQuestion` tool input (§1) or the control-channel dialog
/// payload (§24). `None` for any other item, so a caller keyed by transcript
/// index degrades safely.
fn question_source(item: Option<&TranscriptItem>) -> Option<&serde_json::Value> {
    match item? {
        TranscriptItem::Tool { name, input, .. } if name == "AskUserQuestion" => Some(input),
        TranscriptItem::Question { payload, .. } => Some(payload),
        _ => None,
    }
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
/// Grow `pool` to cover `index` and hand back that slot's mouse handle — the
/// per-index pooling idiom the recent-session rows and fork affordances share.
fn pooled_mouse_state(
    pool: &std::cell::RefCell<Vec<MouseStateHandle>>,
    index: usize,
) -> MouseStateHandle {
    let mut states = pool.borrow_mut();
    while states.len() <= index {
        states.push(MouseStateHandle::default());
    }
    states[index].clone()
}

fn render_message_row(
    is_user: bool,
    icon_svg: &'static str,
    text: &str,
    accent: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    if is_user {
        // iMessage-style outgoing bubble: a tab-accent filled, rounded card that
        // hugs its content and is pushed to the right edge of the transcript.
        // No avatar glyph — like the sender's own bubble in Messages.
        let text_color = theme
            .main_text_color(warp_core::ui::theme::Fill::Solid(accent))
            .into_solid();
        let bubble = Container::new(render_markdown_body(text, text_color, appearance))
            .with_padding(Padding::uniform(10.).with_left(14.).with_right(14.))
            .with_background_color(accent)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                MESSAGE_CORNER_RADIUS,
            )));
        // Cap the bubble width so long messages wrap into a column instead of
        // spanning the whole pane, then right-align it within a full-width row.
        let constrained = ConstrainedBox::new(bubble.finish())
            .with_max_width(USER_BUBBLE_MAX_WIDTH)
            .finish();
        return Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(constrained)
                .finish(),
        )
        .with_margin_top(4.)
        .with_margin_bottom(4.)
        .finish();
    }

    // Assistant message: bare canvas, accent glyph, full-width prose.
    let text_color = theme.main_text_color(theme.background()).into_solid();
    let icon = ConstrainedBox::new(Icon::new(icon_svg, accent).finish())
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

    Container::new(row.finish())
        .with_padding(Padding::uniform(14.))
        .with_margin_top(4.)
        .with_margin_bottom(4.)
        .finish()
}

/// Max number of distinct markdown bodies kept parsed in [`MARKDOWN_CACHE`].
/// The cache is global (shared by every Claude pane), so this must comfortably
/// hold the *combined* settled bodies of several long transcripts open at once —
/// not just one. Sized so a few hundred messages per pane across a handful of
/// panes all stay resident; the live streaming message churns one slot per
/// delta and LRU keeps that churn from evicting the settled set.
const MARKDOWN_CACHE_CAP: usize = 2048;

thread_local! {
    /// Memoized `parse_markdown_with_gfm_tables` output, keyed by the raw body
    /// text. Every streaming delta calls `notify()` (see `apply_event`), which
    /// re-runs `render()` over the *whole* transcript — so without a cache an
    /// N-message conversation re-parses N markdown blobs on every token, and
    /// several panes streaming at once saturate the main thread (the multi-pane
    /// lag / force-quit hang). The parse depends only on the text, so settled
    /// messages hit the cache and only the still-streaming tail re-parses.
    ///
    /// Eviction is **LRU**, not FIFO: the streaming tail mints a *new* key on
    /// every delta, and under FIFO that flood evicted the oldest entries — the
    /// settled bodies that are re-touched (hit) on every single render. So with
    /// multiple panes the settled set was evicted as fast as it was inserted and
    /// the cache degenerated to "re-parse everything every frame," which is the
    /// saturation. LRU evicts the streaming garbage (touched once) and keeps the
    /// settled bodies (touched every render). Lives on the render (main) thread,
    /// so a plain `RefCell` is sufficient.
    static MARKDOWN_CACHE: std::cell::RefCell<MarkdownParseCache> =
        std::cell::RefCell::new(MarkdownParseCache::default());
}

struct MarkdownCacheEntry {
    value: FormattedText,
    /// `MarkdownParseCache::tick` at the entry's most recent access, for LRU
    /// eviction (the smallest is the least-recently used).
    last_used: u64,
}

#[derive(Default)]
struct MarkdownParseCache {
    map: HashMap<String, MarkdownCacheEntry>,
    /// Monotonic access counter; stamped onto an entry on every hit/insert.
    tick: u64,
}

/// Parse `text` as markdown, reusing a cached parse when the same body was seen
/// before. Returns `None` when the text fails to parse (callers fall back to
/// plain wrapped text); parse failures are not cached.
fn parse_markdown_cached(text: &str) -> Option<FormattedText> {
    MARKDOWN_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let tick = cache.tick.wrapping_add(1);
        cache.tick = tick;
        if let Some(entry) = cache.map.get_mut(text) {
            entry.last_used = tick;
            return Some(entry.value.clone());
        }
        let parsed = parse_markdown_with_gfm_tables(text).ok()?;
        // Evict the least-recently-used entry once at capacity. A scan is O(cap)
        // and only runs on a miss (first sight of a body, or a streaming delta),
        // which is negligible against the markdown parse it guards.
        if cache.map.len() >= MARKDOWN_CACHE_CAP {
            if let Some(lru_key) = cache
                .map
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            {
                cache.map.remove(&lru_key);
            }
        }
        cache.map.insert(
            text.to_owned(),
            MarkdownCacheEntry {
                value: parsed.clone(),
                last_used: tick,
            },
        );
        Some(parsed)
    })
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

    let Some(formatted) = parse_markdown_cached(text) else {
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
            // The transcript prose is read-only, but the user still needs to
            // highlight + copy it (same as the tool-output body in
            // inline_action.rs). FormattedTextElement carries the selection +
            // copy machinery; it's just off by default.
            .set_selectable(true)
            .finish(),
            MarkdownSegment::Code(code) => render_code_block(&code.code, appearance),
            MarkdownSegment::Table(table) => render_table(table, appearance),
        };
        column.add_child(child);
    }
    column.finish()
}

/// Render a GFM table (PRODUCT §13) as a real grid — a bordered, rounded box
/// (mirroring [`render_code_block`]) holding a header row over body rows. The
/// parser hands us each cell as a [`FormattedTextInline`], so cells keep their
/// inline styling (bold/code/links). Columns share width equally via
/// [`Expanded`]; per-column alignment comes from the GFM `:---:` separators.
fn render_table(table: FormattedTable, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let header_bg = theme.surface_3().into_solid();
    let column_count = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    let align_of = |col: usize| table.alignments.get(col).copied().unwrap_or_default();

    let mut grid = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    grid.add_child(
        Container::new(render_table_row(
            table.headers,
            column_count,
            &align_of,
            true,
            appearance,
        ))
        .with_background_color(header_bg)
        .finish(),
    );
    for (index, row) in table.rows.into_iter().enumerate() {
        if index > 0 {
            // A thin divider between body rows, so the grid reads as rows
            // without heavy full-grid lines.
            grid.add_child(
                ConstrainedBox::new(
                    Container::new(Flex::row().finish())
                        .with_background_color(theme.outline().into_solid())
                        .finish(),
                )
                .with_height(1.)
                .finish(),
            );
        }
        grid.add_child(render_table_row(
            row,
            column_count,
            &align_of,
            false,
            appearance,
        ));
    }

    Container::new(grid.finish())
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_margin_top(10.)
        .with_margin_bottom(10.)
        .finish()
}

/// One table row: `column_count` equal-width cells (missing trailing cells are
/// padded blank so short rows still line up under the header).
fn render_table_row(
    mut cells: Vec<FormattedTextInline>,
    column_count: usize,
    align_of: &impl Fn(usize) -> TableAlignment,
    header: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    cells.resize(column_count, Vec::new());
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (col, cell) in cells.into_iter().enumerate() {
        row.add_child(
            Expanded::new(
                1.,
                render_table_cell(cell, align_of(col), header, appearance),
            )
            .finish(),
        );
    }
    row.finish()
}

/// One table cell: the inline content, padded, horizontally aligned per the
/// column's GFM alignment. Header cells read in the strong text color.
fn render_table_cell(
    inline: FormattedTextInline,
    alignment: TableAlignment,
    header: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let inline_code_bg = theme.surface_3().into_solid();
    let text_color = if header {
        theme.main_text_color(theme.background()).into_solid()
    } else {
        theme.active_ui_text_color().into_solid()
    };
    let element = FormattedTextElement::new(
        FormattedText::new(vec![FormattedTextLine::Line(inline)]),
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
    .set_selectable(true)
    .finish();
    let main_axis_alignment = match alignment {
        TableAlignment::Left => MainAxisAlignment::Start,
        TableAlignment::Center => MainAxisAlignment::Center,
        TableAlignment::Right => MainAxisAlignment::End,
    };
    Container::new(
        Flex::row()
            .with_main_axis_alignment(main_axis_alignment)
            .with_child(element)
            .finish(),
    )
    .with_padding_left(10.)
    .with_padding_right(10.)
    .with_padding_top(6.)
    .with_padding_bottom(6.)
    .finish()
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

/// A short, one-line tab label derived from a user message (#7) — the first
/// non-blank line, ellipsized. Falls back to the empty string for blank input
/// (the caller then keeps the generic pane title).
fn pane_tab_title(text: &str) -> String {
    const MAX: usize = 40;
    let head = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if head.chars().count() > MAX {
        let truncated: String = head.chars().take(MAX).collect();
        format!("{truncated}…")
    } else {
        head.to_owned()
    }
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

/// twarp (#8): persist pasted clipboard image bytes to a temp file so the
/// attachment can be previewed as a thumbnail and re-opened in the OS default
/// app on click. The filename hashes the bytes so identical pastes dedupe and
/// the path is stable across renders. Returns `None` on any IO error — the
/// paste still ships via base64, it just won't preview.
fn persist_pasted_image(data: &[u8], media_type: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    let ext = match media_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    };
    let dir = std::env::temp_dir().join("twarp-claude-pasted");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("paste-{:016x}.{ext}", hasher.finish()));
    if !path.exists() {
        std::fs::write(&path, data).ok()?;
    }
    Some(path)
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
    /// A GFM pipe table (PRODUCT §13). Split out like code so it renders as a
    /// real grid instead of flattening back to literal `| a | b |` pipe-text
    /// (which is what a `FormattedTextElement` does with a `Table` line).
    Table(FormattedTable),
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
            FormattedTextLine::Table(table) => {
                if !running_prose.is_empty() {
                    segments.push(MarkdownSegment::Prose(FormattedText::new_trimmed(
                        std::mem::take(&mut running_prose),
                    )));
                }
                segments.push(MarkdownSegment::Table(table));
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
/// One plain coloured text segment of the composer context bar (#11). Unlike
/// [`render_pill`] there's no chip chrome — the bar reads as a quiet status
/// line above the input.
fn context_segment(appearance: &Appearance, text: String, color: ColorU) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .span(text)
        .with_style(UiComponentStyles {
            font_color: Some(color),
            font_size: Some(11.5),
            ..Default::default()
        })
        .build()
        .finish()
}

/// Image previews shown above a user message bubble (#8): a row of rounded
/// square thumbnails, right-aligned so they sit directly above the (also
/// right-aligned) message bubble.
impl ClaudeCodeView {
    /// twarp (#8): the row of sent-image thumbnails above a user turn. Each tile
    /// is clickable — opening the full image in the OS default app.
    fn render_sent_images(&self, paths: &[PathBuf]) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_spacing(6.);
        for path in paths {
            let path_str = path.display().to_string();
            let tile = ConstrainedBox::new(
                Image::new(
                    AssetSource::LocalFile {
                        path: path_str.clone(),
                    },
                    CacheOption::BySize,
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
            )
            .with_width(SENT_IMAGE_SIZE)
            .with_height(SENT_IMAGE_SIZE)
            .finish();
            let mouse = self
                .sent_image_mouse
                .borrow_mut()
                .entry(path_str.clone())
                .or_default()
                .clone();
            row.add_child(
                Hoverable::new(mouse, move |_| tile)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenSentImage(
                            path_str.clone(),
                        ));
                    })
                    .finish(),
            );
        }
        row.finish()
    }
}

/// One segment of the Chat UI / Raw CLI section toggle (#7). The active segment
/// is a filled, non-clickable highlight; an inactive-but-allowed segment is
/// clickable and flips the mode; an inactive-disabled segment (e.g. entering
/// raw mid-turn) is muted and inert.
fn render_mode_segment(
    label: &str,
    active: bool,
    clickable: bool,
    mouse: MouseStateHandle,
    accent: ColorU,
    wash: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // #10/#11: the active section reads in the tab accent over a faint wash.
    let color = if active {
        accent
    } else if clickable {
        theme.main_text_color(theme.background()).into_solid()
    } else {
        theme.nonactive_ui_text_color().into_solid()
    };
    let mut chip = Container::new(context_segment(appearance, label.to_owned(), color))
        .with_padding_left(8.)
        .with_padding_right(8.)
        .with_padding_top(3.)
        .with_padding_bottom(3.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
    if active {
        chip = chip.with_background_color(wash);
    }
    let chip = chip.finish();
    if clickable {
        Hoverable::new(mouse, move |_| chip)
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action::<PaneHeaderAction<(), ClaudeCodeCustomAction>>(
                    PaneHeaderAction::CustomAction(ClaudeCodeCustomAction::ToggleRawCli),
                );
            })
            .finish()
    } else {
        chip
    }
}

/// Compact token count for the context popover (#13): `365.8k`, `1.0M`.
fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// A small muted header line for a composer menu / popover (#13).
fn menu_header(text: &str, color: ColorU, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(context_segment(appearance, text.to_owned(), color))
        .with_padding_bottom(2.)
        .finish()
}

/// Fetch (growing on demand) the pooled mouse handle at `index` — the
/// Timeline/shortcut-row pattern for variable-length menu lists (#11).
fn pool_mouse(pool: &mut Vec<MouseStateHandle>, index: usize) -> MouseStateHandle {
    while pool.len() <= index {
        pool.push(MouseStateHandle::default());
    }
    pool[index].clone()
}

/// One clickable action row in a composer dropdown (#11): a hoverable label
/// that dispatches `on_click` and reads in `color`. Used by the branch / CI /
/// PR menus.
fn menu_action_row(
    label: &str,
    color: ColorU,
    mouse: MouseStateHandle,
    on_click: impl Fn(&mut warpui::EventContext) + 'static,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let row = Container::new(context_segment(appearance, label.to_owned(), color))
        .with_padding_left(8.)
        .with_padding_right(8.)
        .with_padding_top(5.)
        .with_padding_bottom(5.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .finish();
    Hoverable::new(mouse, move |_| row)
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| on_click(ctx))
        .finish()
}

/// One `label … count` row in the context breakdown popover (#13).
fn usage_row(
    label: &str,
    count: u64,
    label_color: ColorU,
    value_color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(context_segment(appearance, label.to_owned(), label_color))
        .with_child(context_segment(appearance, fmt_tokens(count), value_color))
        .finish()
}

/// A two-segment fill bar for the context popover (#13): `filled` proportion in
/// the accent colour, the remainder in the track colour.
fn render_context_progress(fraction: f32, filled: ColorU, track: ColorU) -> Box<dyn Element> {
    const WIDTH: f32 = 240.;
    const HEIGHT: f32 = 6.;
    let fraction = fraction.clamp(0., 1.);
    let filled_width = WIDTH * fraction;
    let bar = |width: f32, color: ColorU| -> Box<dyn Element> {
        ConstrainedBox::new(
            Container::new(Flex::row().finish())
                .with_background_color(color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                .finish(),
        )
        .with_width(width)
        .with_height(HEIGHT)
        .finish()
    };
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(bar(filled_width, filled))
        .with_child(bar(WIDTH - filled_width, track))
        .finish()
}

/// Black or white, whichever reads better on `bg` (#10) — for the primary
/// button label on an arbitrary tab colour.
fn contrasting_text(bg: ColorU) -> ColorU {
    let luminance = 0.299 * bg.r as f32 + 0.587 * bg.g as f32 + 0.114 * bg.b as f32;
    if luminance > 150. {
        ColorU::new(0, 0, 0, 255)
    } else {
        ColorU::new(255, 255, 255, 255)
    }
}

fn render_pill(label: &str, bg: ColorU, appearance: &Appearance) -> Box<dyn Element> {
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
    .with_background_color(bg)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
    .finish()
}

/// The §25 permission-mode selector pill: the muted pill chrome with hover +
/// pointer affordance; a click dispatches the cycle action. The label carries
/// a chevron-ish suffix so it reads as a control, not a static chip.
/// A context-bar pill (#11) with arbitrary `color` text on the tab wash — the
/// shared chrome for the folder / branch / diff / PR / CI chips. Static (no
/// interaction); [`render_context_menu_pill`] adds the click + anchor.
fn render_context_pill(
    label: String,
    color: ColorU,
    bg: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Container::new(
        appearance
            .ui_builder()
            .span(label)
            .with_style(UiComponentStyles {
                font_color: Some(color),
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
    .with_background_color(bg)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
    .finish()
}

/// A clickable context-bar pill (#11) that opens a floating menu — hover +
/// pointer affordance, `SavePosition`-wrapped so the dropdown can anchor to it.
fn render_context_menu_pill(
    label: String,
    color: ColorU,
    bg: ColorU,
    mouse_state: MouseStateHandle,
    position_id: &str,
    on_click: impl Fn(&mut warpui::EventContext) + 'static,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let pill = render_context_pill(format!("{label} ▾"), color, bg, appearance);
    SavePosition::new(
        Hoverable::new(mouse_state, move |_| pill)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| on_click(ctx))
            .finish(),
        position_id,
    )
    .finish()
}

fn render_clickable_pill(
    label: &str,
    mouse_state: MouseStateHandle,
    on_click: impl Fn(&mut warpui::EventContext) + 'static,
    bg: ColorU,
    position_id: &str,
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
    .with_background_color(bg)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
    .finish();
    // Wrap the pill in a `SavePosition` so the open dropdown can anchor its
    // floating overlay to this trigger (#11/#13).
    let trigger = SavePosition::new(
        Hoverable::new(mouse_state, move |_| pill)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| on_click(ctx))
            .finish(),
        position_id,
    )
    .finish();
    Container::new(trigger).with_margin_right(6.).finish()
}

/// Shorten a `claude` model id for the chip: drop the `claude-` prefix the CLI
/// prepends (`claude-fable-5[1m]` → `fable-5[1m]`).
fn prettify_model(model: &str) -> String {
    model.strip_prefix("claude-").unwrap_or(model).to_owned()
}

/// Shorten a slash-command description to a single readable line for the
/// suggestions panel (issue #3): clip at the first sentence boundary, then to a
/// hard character cap so a long "Use when…" blurb can't dominate the row.
fn truncate_description(description: &str) -> String {
    const MAX_CHARS: usize = 80;
    // Prefer the first sentence — skill descriptions lead with a summary then
    // a "Use when…" clause we don't need in the preview.
    let summary = description
        .split_once(". ")
        .map(|(first, _)| first)
        .unwrap_or(description)
        .trim_end_matches('.')
        .trim();
    if summary.chars().count() <= MAX_CHARS {
        return summary.to_owned();
    }
    let mut clipped: String = summary.chars().take(MAX_CHARS).collect();
    clipped.push('…');
    clipped
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

#[cfg(test)]
mod tests {
    use super::{format_metrics_line, queue_preview, should_auto_allow_permission};
    use claude_code::TurnMetrics;

    #[test]
    fn ask_user_question_permission_is_auto_allowed() {
        // §1: AskUserQuestion must never block the turn behind a permission
        // prompt — it is answered via the inline Question card once the turn
        // ends. Every other tool keeps its interactive Allow/Deny card.
        assert!(should_auto_allow_permission("AskUserQuestion"));
        assert!(!should_auto_allow_permission("Bash"));
        assert!(!should_auto_allow_permission("Write"));
    }

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
