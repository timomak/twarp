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

mod agents;
mod background_scripts;
mod composer;
mod diff_cards;
mod inline_action;
mod repo_context;
mod thinking;
mod timeline;
mod todos;
mod tool_cards;
mod turn_presentation;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::settings::Setting;
use async_channel::Sender;
use base64::Engine as _;
use claude_code::codex::CodexDriver;
use claude_code::diff::diff_for_tool;
use claude_code::driver::{
    interrupt, send_control_response, send_interrupt, send_user_message, spawn_session,
    AgentProvider, Child, Decision, OutgoingImage, OutgoingMessage, PermissionMode, SpawnOptions,
    SpawnedSession,
};
use claude_code::launch::LaunchOptions;
use claude_code::{
    sessions, ToolStatus, Transcript, TranscriptEvent, TranscriptItem, TurnMetrics, Usage,
};
use futures::StreamExt;
use markdown_parser::{
    parse_markdown_with_gfm_tables, weight::CustomWeight, FormattedTable, FormattedText,
    FormattedTextInline, FormattedTextLine, TableAlignment,
};
use parking_lot::RwLock;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use twarp_core::features::FeatureFlag;
use twarp_core::ui::theme::AnsiColorIdentifier;
use twarp_core::ui::tokens::{border, elevation, measure, radius, spacing, type_ramp};
use twarp_editor::editor::NavigationKey;
use twarpui::assets::asset_cache::AssetSource;
use twarpui::clipboard::ClipboardContent;
use twarpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use twarpui::platform::{FilePickerConfiguration, MicrophoneAccessState};
use twarpui::r#async::Timer;
use twarpui::ui_components::button::ButtonVariant;
use twarpui::ui_components::slider::SliderStateHandle;
use twarpui::units::IntoPixels;
use twarpui::{
    elements::{
        Align, AnchorPair, Border, CacheOption, ChildAnchor, Clipped, ClippedScrollStateHandle,
        ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
        DispatchEventResult, DropShadow, Element, Empty, EventDispatchMode, EventHandler, Expanded,
        Fill, Flex, FormattedTextElement, Highlight, HighlightedHyperlink, HighlightedRange,
        Hoverable, HyperlinkUrl, Icon, Image, MainAxisAlignment, MainAxisSize, MouseStateHandle,
        OffsetPositioning, OffsetType, Padding, ParentAnchor, ParentElement, ParentOffsetBounds,
        PositionedElementAnchor, PositionedElementOffsetBounds, PositioningAxis, PulsingIcon,
        PulsingIconStateHandle, Radius, SavePosition, ScrollTarget, ScrollToPositionMode,
        ScrollbarWidth, SelectableArea, SelectionHandle, Shrinkable, SizeConstraintCondition,
        SizeConstraintSwitch, Stack, Text, XAxisAnchor, YAxisAnchor,
    },
    platform::Cursor,
    presenter::ChildView,
    text_layout::{ClipConfig, TextStyle},
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView,
    View, ViewContext, ViewHandle, WindowId,
};
use twarpui_extras::secure_storage;

use self::agents::{AgentRun, AgentRunState};
use self::background_scripts::{BackgroundScript, BackgroundScriptState};
use self::composer::{SuggestionKind, SuggestionQuery};
use self::diff_cards::DiffCard;
use self::repo_context::{CiState, RepoContext};
use self::thinking::ThinkingUi;
use self::timeline::{project_timeline, TimelineEntry};
use self::tool_cards::{render_tool_card, ToolCardUi};
use self::turn_presentation::{
    file_edit_summaries, project_turns, FileEditSummary, TurnPresentation,
};
use crate::agent_suggestions::{
    ComposerPlaceholderSuggestionContext, DefaultSuggestionProvider, ReplySuggestionContext,
    SuggestionContext, SuggestionProvider,
};
use crate::app_state::{CLIAgent, ConversationStatus};
use crate::appearance::Appearance;
use crate::claude_code_session_defaults::ClaudeSessionDefaultsModel;
use crate::computer_control::{
    ComputerControlChrome, ComputerControlCoordinator, ComputerControlState,
};
use crate::editor::{
    AutosuggestionLocation, AutosuggestionType, EditorOptions, EditorView, Event as EditorEvent,
    PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions},
    BackingView, PaneConfiguration, PaneEvent, PaneHeaderAction,
};
use crate::settings::{self as app_settings, AgentSettings};
#[cfg(all(feature = "local_fs", feature = "local_tty"))]
use crate::terminal::local_shell::LocalShellState;
use crate::terminal::{
    session_settings::{NotificationsMode, SessionSettings},
    view::{Event as TerminalViewEvent, NotificationsTrigger},
    TerminalManager, TerminalView,
};
use crate::util::path::{resolve_executable, resolve_executable_in_path};
use crate::workspace::{WorkspaceAction, WorkspaceRegistry};

/// The executable the pane drives. Resolved on `PATH`; its absence is the
/// unavailable state (PRODUCT §4).
const CLAUDE_BINARY: &str = "claude";
const CODEX_BINARY: &str = "codex";

/// Zero-state "Welcome back" session list paging: how many rows show before
/// the first "Load more" click, and how many each click reveals. The initial
/// cap keeps a long history from running under the floating composer; paging
/// (rather than a hard cap) makes the zero state a full home for past sessions
/// now that the sidebar "Code" tab is gone.
const ZERO_STATE_INITIAL_SESSIONS: usize = 6;
const ZERO_STATE_SESSIONS_PER_PAGE: usize = 10;

/// Avatar glyphs for the message rows (the Agent-Mode shape: icon + body).
const USER_ICON_SVG_PATH: &str = "bundled/svg/user.svg";
const ASSISTANT_ICON_SVG_PATH: &str = "bundled/svg/claude.svg";
const CODEX_ICON_SVG_PATH: &str = "bundled/svg/openai.svg";
/// Branch glyph for the hover "Fork" affordance below an assistant response.
const FORK_ICON_SVG_PATH: &str = "bundled/svg/git-branch-02.svg";
/// Copy glyph for the hover "copy response" affordance beside the Fork button.
const COPY_ICON_SVG_PATH: &str = "bundled/svg/copy.svg";
/// Pencil glyph for the hover "Edit" affordance below a sent user message.
const EDIT_ICON_SVG_PATH: &str = "bundled/svg/pencil-line.svg";
/// Down-chevron for the floating "scroll to bottom" button (shown above the
/// composer's right edge while the transcript is scrolled up off the bottom).
const SCROLL_TO_BOTTOM_ICON_SVG_PATH: &str = "bundled/svg/chevron-down.svg";

#[derive(Clone, Copy)]
struct ProviderCopy {
    pane_title: &'static str,
    assistant_icon: &'static str,
    composer_placeholder: &'static str,
    empty_state: &'static str,
    unavailable_title: &'static str,
    unavailable_body: &'static str,
    install_body: &'static str,
    needs_attention_prefix: &'static str,
}

fn provider_copy(provider: AgentProvider) -> ProviderCopy {
    match provider {
        AgentProvider::Claude => ProviderCopy {
            pane_title: "Claude Code",
            assistant_icon: ASSISTANT_ICON_SVG_PATH,
            composer_placeholder: "Message Claude Code…",
            empty_state: "Type a message below — twarp drives the local `claude` CLI and renders its replies, tool calls, and diffs here. Your existing Claude Code login is used; twarp adds no account or billing.",
            unavailable_title: "Claude Code isn't available.",
            unavailable_body: "The `claude` command wasn't found on your PATH.",
            install_body: "Install Claude Code (https://docs.claude.com/en/docs/claude-code), make sure `claude` is available in a terminal, then check again.",
            needs_attention_prefix: "Claude",
        },
        AgentProvider::Codex => ProviderCopy {
            pane_title: "Codex",
            assistant_icon: CODEX_ICON_SVG_PATH,
            composer_placeholder: "Message Codex…",
            empty_state: "Type a message below — twarp drives the local `codex app-server` CLI and renders its replies, tool calls, and diffs here. Your existing Codex login is used; twarp adds no account or billing.",
            unavailable_title: "Codex isn't available.",
            unavailable_body: "The `codex` command wasn't found on your PATH.",
            install_body: "Install the Codex CLI, make sure `codex` is available in a terminal, then check again.",
            needs_attention_prefix: "Codex",
        },
    }
}

fn supports_raw_cli(provider: AgentProvider) -> bool {
    matches!(provider, AgentProvider::Claude | AgentProvider::Codex)
}

fn raw_cli_menu_label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Open Claude CLI",
        AgentProvider::Codex => "Open Codex CLI",
    }
}

fn build_raw_cli_flags(
    provider: AgentProvider,
    model: Option<&str>,
    effort: Option<&str>,
    permission_mode: PermissionMode,
) -> String {
    let mut flags = Vec::new();
    if let Some(model) = model {
        flags.push(format!(
            "--model {}",
            shell_words::quote(model).into_owned()
        ));
    }
    match provider {
        AgentProvider::Claude => {
            if let Some(effort) = effort {
                flags.push(format!(
                    "--effort {}",
                    shell_words::quote(effort).into_owned()
                ));
            }
            flags.push(format!(
                "--permission-mode {}",
                permission_mode.as_cli_arg()
            ));
        }
        AgentProvider::Codex => {
            if let Some(effort) = effort {
                let config = format!("model_reasoning_effort=\"{effort}\"");
                flags.push(format!(
                    "--config {}",
                    shell_words::quote(&config).into_owned()
                ));
            }
            let (sandbox, approval) = match permission_mode {
                PermissionMode::Plan => ("read-only", "never"),
                PermissionMode::Default | PermissionMode::AcceptEdits => {
                    ("workspace-write", "never")
                }
                PermissionMode::BypassPermissions => ("danger-full-access", "never"),
            };
            flags.push(format!("--sandbox {sandbox} --ask-for-approval {approval}"));
        }
    }
    flags.join(" ")
}

/// Body / code font sizes. A point past the deleted `ai_assistant::transcript`
/// renderer for the airier, Claude-app reading rhythm (shell-polish pass —
/// PRODUCT §32 visual gate).
const BODY_FONT_SIZE: f32 = type_ramp::PROSE.size;
const CODE_FONT_SIZE: f32 = type_ramp::CODE.size;
const TRANSCRIPT_LEFT_MARGIN: f32 = spacing::LG;

/// Shell-polish layout constants (the Claude-app frame): a floating rounded
/// composer, muted context pills, and a zero-state heading. (Per owner
/// feedback on the 7d review: the chat fills the pane, and the composer
/// floats above it at the bottom-center instead of stacking below.)
const COMPOSER_MAX_HEIGHT: f32 = 184.;
/// The floating composer's width cap — it stays a centered card even in a
/// wide pane (the chat behind it is full-width).
const COMPOSER_MAX_WIDTH: f32 = 760.;
/// Width cap for the ⋯ header menu card (twarp): a compact status card pinned
/// to the pane's top-right, narrow enough to clear the centered reading column
/// behind it, but wide enough for the inline agent / script rows it expands.
const HEADER_MENU_MAX_WIDTH: f32 = 360.;
/// Saved-position id for anchoring the floating menu to its ⋯ trigger.
const HEADER_MENU_BUTTON_POSITION_ID: &str = "claude_header_menu_button";
/// Duration of the header menu's inline section reveal (Agent runs / Scripts
/// expanding in place).
const HEADER_MENU_REVEAL_DURATION: Duration = Duration::from_millis(150);
/// Interval between reveal animation frames (self-rearming `notify()` ticks —
/// warpui has no transition framework; see `left_panel_slide.rs`).
const HEADER_MENU_REVEAL_TICK: Duration = Duration::from_millis(8);
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
const TRANSCRIPT_GUTTER: f32 = spacing::LG;
const COMPOSER_CORNER_RADIUS: f32 = radius::PANEL;
/// A short conversation does not need a navigator; showing one would add
/// chrome without improving orientation.
const TIMELINE_MIN_TURNS: usize = 3;
/// Slack (px) for the streaming follow-to-bottom check. While following, the
/// view sits exactly at the bottom; this only absorbs sub-pixel/line-height
/// rounding so a genuine upward scroll (tens of px) reliably pauses the follow.
const AUTOSCROLL_STICK_SLACK: f32 = 16.;
const MESSAGE_CORNER_RADIUS: f32 = radius::CARD;
/// Cap the outgoing (user) iMessage bubble so long messages wrap into a column
/// hugging the right edge instead of stretching across the whole transcript.
const USER_BUBBLE_MAX_WIDTH: f32 = 520.;
/// Side length of a sent-image preview thumbnail (#8): a fixed square so
/// attachments sit as uniform tiles above the message bubble.
const SENT_IMAGE_SIZE: f32 = 120.;
const PILL_CORNER_RADIUS: f32 = radius::CHIP;
const HEADING_FONT_SIZE: f32 = type_ramp::HEADING.size;

/// Below this composer width the controls row steps down to
/// their compact tier (folder pill dropped, branch truncated, MCP chip
/// dropped) — roughly a half-window pane.
const COMPOSER_COMPACT_MAX_WIDTH: f32 = 560.;
/// Below this width they step down again to the tiny tier (diff counts and
/// the read-only info chips dropped, branch truncated harder) — roughly a
/// three-up pane.
const COMPOSER_TINY_MAX_WIDTH: f32 = 430.;

/// Density tier for the composer's controls row. The composer
/// is width-capped and shrinks with the pane; each row is wrapped in a
/// [`SizeConstraintSwitch`] that steps down through these tiers at layout
/// time so three side-by-side Claude panes degrade gracefully instead of
/// overflowing pills off the card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ComposerDensity {
    Full,
    Compact,
    Tiny,
}

/// Middle-truncate `text` to at most `max_chars` characters (branch names in
/// the Environment menu): `feature/very-long-branch-name` →
/// `feature/ve…ch-name`.
fn truncate_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    let head = keep * 3 / 5;
    let tail = keep - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}\u{2026}{end}")
}

// The model selector's entries (PRODUCT §52, 7m) are no longer a hardcoded
// cycle: see [`crate::claude_code_models`] (Models-API discovery with an alias
// fallback) and [`ClaudeCodeView::model_menu_entries`]. Selecting one reuses
// the §25 detach→`--resume` mechanism so the conversation continues under the
// new model.

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
    /// twarp 07 (7p): the session state the tab indicator reads (working /
    /// blocked / finished) changed. The pane host forwards this to the
    /// workspace so the tab bar repaints — the view's own `notify()` only
    /// redraws the pane body.
    TabStatusChanged,
    /// Swap this pane for the provider's raw interactive CLI resuming
    /// `session_id`. The pane group creates and embeds the terminal.
    SwapToRawCli {
        provider: AgentProvider,
        session_id: String,
        cwd: Option<PathBuf>,
        /// Resolved against the captured login-shell PATH so GUI launches can
        /// find the same provider executable as an interactive terminal.
        binary: String,
        /// Provider-native model, effort, and access flags, pre-joined and
        /// shell-quoted by the view that owns those settings.
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
    /// Jump to the user message that starts a turn from the compact timeline
    /// shown at the left edge of wide agent panes.
    JumpToTurn(usize),
    /// Expand / collapse the tool card with this tool-use id (PRODUCT §19).
    ToggleToolCard(String),
    /// Expand / collapse the chronological work hidden behind a completed
    /// turn's compact final-result presentation.
    ToggleCompletedTurn(usize),
    /// Open one file listed by a completed turn's edit artifact card.
    OpenArtifactFile(String),
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
    /// Close the header's Environment / Activity menu.
    CloseHeaderMenu,
    /// Open the code-review panel for this pane's repository without closing
    /// an already-visible panel.
    OpenChanges,
    /// twarp: copy a string (branch name, PR URL, …) to the clipboard and close
    /// the open menu.
    CopyToClipboard(String),
    /// twarp: open the current branch on GitHub (`…/tree/<branch>`).
    OpenBranchInGitHub,
    /// twarp: check out a different local branch from the branch menu, then
    /// refresh Environment. Runs `git checkout <name>` in the cwd.
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
    /// transcript index. Primary path (PRODUCT §1): claude is holding the turn
    /// on the question's `can_use_tool`, so the picks are sent back as the
    /// tool's `answers` and the model continues in the same turn. Fallback (the
    /// turn already ended): the answer is resent as the next user turn.
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
    /// twarp: clear the zero-state session search field (the inline X button).
    ClearSessionSearch,
    /// twarp: reveal the next page of rows in the zero-state session list
    /// (the "Load more" affordance below the visible rows).
    ShowMoreRecentSessions,
    /// twarp: fork the conversation at the assistant response with this
    /// transcript index ("Fork conversation" — Claude's `--fork-session`).
    /// Branches the session up to that turn into a new pane to the right,
    /// leaving this one untouched. Shown on hover below a response.
    ForkConversation(usize),
    /// twarp: copy the whole assistant reply ending at this transcript index to
    /// the clipboard — the one-click alternative to drag-selecting a long
    /// response. Shown on hover beside the Fork button.
    CopyResponse(usize),
    /// twarp: load the sent user message at this transcript index back into the
    /// composer for revision. Shown on hover below a user bubble; prefills and
    /// focuses the composer — sending goes through the normal Submit path.
    EditUserMessage(usize),
    /// twarp: copy one fenced code block's contents (the per-block copy button
    /// in the transcript's code cards).
    CopyCodeBlock(String),
    /// twarp: expand / collapse the floating background-scripts panel — the
    /// pill listing this chat's `run_in_background` Bash launches.
    ToggleBackgroundPanel,
    /// twarp: expand / collapse one background-script row's captured output,
    /// keyed by the launching tool-use id.
    ToggleBackgroundScript(String),
    /// twarp: clear every *non-running* background script from the panel
    /// (finished / stopped / failed-to-start). Running scripts are left alone —
    /// twarp only hides rows it can be sure are done, since it can't kill a
    /// live shell. Hidden ids are remembered so they stay cleared across renders.
    ClearBackgroundScripts,
    /// twarp: expand / collapse the floating agents panel — the background
    /// panel's twin listing this chat's sub-agent (`Task`/`Agent`) launches.
    /// Opening it closes the background-scripts panel (they share the anchor).
    ToggleAgentsPanel,
    /// twarp: expand / collapse one agent row's returned result, keyed by the
    /// launching tool-use id.
    ToggleAgentRow(String),
    /// twarp: clear every *non-running* agent from the panel (finished /
    /// failed / stopped). Running agents are left alone — twarp only hides
    /// rows it can be sure are done, since it can't stop an agent it didn't
    /// launch. Hidden ids are remembered so they stay cleared across renders.
    ClearAgents,
    /// twarp: expand / collapse one server's tool list in the MCP viewer popover
    /// (feature 13). Only one server is expanded at a time; re-dispatching the
    /// open server collapses it.
    ToggleMcpServer(String),
    /// twarp 17: the composer mic button (PRODUCT 17 §1–§11). Idle → start
    /// recording; recording → stop and transcribe; transcribing → no-op.
    ToggleVoiceRecording,
    /// twarp 17: the composer speaker button (PRODUCT 17 §12–§14) — toggles
    /// spoken replies for this pane; while speaking it also stops playback.
    ToggleSpeakReplies,
    /// twarp: swap between the rendered chat and the raw interactive CLI from
    /// the header menu's segmented toggle. The menu overlay lives in this
    /// view's own tree (unlike the old header toggle), so it dispatches the
    /// in-pane action directly.
    ToggleRawCli,
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
    /// twarp 15b: start or stop the feature-flagged computer-control overlay
    /// lifecycle for this Claude session. Header chrome must route through a
    /// pane custom action because it is rendered by the parent `PaneView`.
    ToggleComputerControl,
    /// Expand / collapse the consolidated Environment / Activity menu. The
    /// button lives in the parent `PaneView` header tree, so it routes through
    /// the pane framework.
    ToggleHeaderMenu,
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

    /// Whether this menu renders inline in Environment instead of as a
    /// composer-anchored popover.
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
        decision: Decision,
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

/// An `AskUserQuestion` `can_use_tool` held open for the user (PRODUCT §1):
/// the control-protocol `request_id` to answer plus the tool's proposed
/// `input` (its `questions`), kept so a typed free-text reply can build the
/// `answers` without re-finding the tool card in the transcript.
struct HeldQuestionPermission {
    request_id: String,
    input: serde_json::Value,
}

pub struct ClaudeCodeView {
    /// The conversation the pane renders, fed by the live driver's event stream
    /// via [`Self::on_transcript_event`] on the main thread (PRODUCT §9–§13).
    transcript: Transcript,
    /// Provider backing this pane. Claude remains the default for old
    /// snapshots; Codex is enabled only behind `CodexAgentBackend`.
    provider: AgentProvider,
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
    /// Environment from the user's interactive login shell. This is required
    /// for Codex model providers configured with shell-defined credentials,
    /// especially after a GUI relaunch loses the terminal's environment.
    interactive_env_vars: Option<HashMap<String, String>>,
    /// A restored Codex pane waits for interactive environment capture before
    /// spawning `thread/resume`, so the first resumed turn uses the same provider
    /// credentials as a terminal-launched session.
    codex_restore_pending: bool,
    scroll_state: ClippedScrollStateHandle,
    /// Per-view namespace for transcript position ids. Position caches are
    /// window-global, so session ids alone are not enough when the same
    /// conversation is open in two side-by-side panes.
    timeline_position_key: String,
    /// Stable hover/click state for each user-turn marker.
    timeline_turn_mouse: std::cell::RefCell<HashMap<usize, MouseStateHandle>>,
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
    /// Codex app-server creates the real thread id asynchronously. The first
    /// user turn waits here until `SessionInit` carries that id; Claude still
    /// sends immediately because it owns the session id at spawn.
    pending_initial_turn: Option<OutgoingMessage>,
    /// A provider process is being created on the background executor. Kept
    /// separately from `child`, which is populated only after spawn resolves.
    session_spawn_pending: bool,
    /// True while `claude` is producing output for the current turn (PRODUCT §9):
    /// the composer shows Stop and sending is disabled until the turn ends.
    streaming: bool,
    /// True between a user Stop and the turn's terminal event (PRODUCT §11). The
    /// interrupt makes `claude` end the turn with an error `result`, but the
    /// session stays alive — this flag re-labels that terminal event as a clean
    /// `Interrupted` so it shows the "Interrupted." notice instead of a spurious
    /// error and keeps the session resumable.
    interrupt_pending: bool,
    /// The last turn's outcome, for the tab status dot (7p): `Some(true)`
    /// finished cleanly, `Some(false)` failed. On revisit (focus) a ✗ is
    /// cleared, while a ✓ merely flips from unreviewed blue to reviewed green
    /// (see [`Self::tab_attention_seen`]); both are masked by the
    /// in-progress/blocked states while the next turn runs. Stop
    /// (`Interrupted`) sets nothing: the user did it.
    tab_attention: Option<bool>,
    /// Whether the user has visited the pane since [`Self::tab_attention`] was
    /// set. A completed turn shows a blue ✓ until the first revisit, then a
    /// green ✓ — so reviewed and unreviewed chats read apart at a glance.
    tab_attention_seen: bool,
    /// A turn completed while background scripts / sub-agents were still
    /// running: the ✓ and the desktop notification are held here (the
    /// notification body) until the last one retires, so the chat isn't
    /// declared complete while work is still in flight.
    deferred_completion: Option<String>,
    /// Stable mouse-state handles kept across renders so a click's
    /// mousedown/mouseup hit the same handle.
    submit_button: MouseStateHandle,
    refresh_button: MouseStateHandle,
    stop_button: MouseStateHandle,
    /// Per-tool-card UI state (stable mouse handle + the user's expand/collapse
    /// choice), keyed by tool-use id. An entry is created when the card's
    /// `ToolCall` event arrives (PRODUCT §16, §19).
    tool_card_ui: HashMap<String, ToolCardUi>,
    /// User-message transcript indices for completed turns whose chronological
    /// work disclosure is expanded.
    expanded_completed_turns: HashSet<usize>,
    /// Stable mouse state for each completed-turn disclosure row.
    completed_turn_mouse: std::cell::RefCell<HashMap<usize, MouseStateHandle>>,
    /// Stable mouse state for each clickable file in a completed edit artifact.
    artifact_file_mouse: std::cell::RefCell<HashMap<(usize, String), MouseStateHandle>>,
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
    /// A Codex CLI switch requested before app-server has returned the real
    /// thread id. The switch completes as soon as `SessionInit` arrives.
    raw_cli_pending: bool,
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
    /// Monotonic cancellation token for chat reply ghost-text generation. Any
    /// user edit, focus loss, or new turn increments it so stale provider
    /// results cannot repopulate the composer.
    reply_suggestion_generation: u64,
    /// Monotonic cancellation token for empty-composer placeholder generation.
    /// Stale async results are ignored when the composer gains text, a turn
    /// starts, or the setting is disabled.
    composer_placeholder_generation: u64,
    /// The currently rendered AI placeholder text, if any. Tab accepts this
    /// into the empty composer only when no editor autosuggestion is active.
    composer_placeholder_suggestion: Option<String>,
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
    /// Question cards whose answers have been submitted, keyed by transcript
    /// index → the chosen (flattened) option indices. Once submitted, the card
    /// locks: it keeps the picks visibly checked (so the answer doesn't appear
    /// to vanish) and drops its live controls while `claude` produces the reply.
    question_submitted: HashMap<usize, HashSet<usize>>,
    /// Transcript indices of `User` items that are typed `AskUserQuestion`
    /// answers (PRODUCT §1). They render as user bubbles but travel back as the
    /// question's `can_use_tool` answer, so the session file never stores them
    /// as user turns — fork turn-counting must skip them to stay aligned with
    /// the file ([`Self::fork_conversation`]).
    question_answer_items: HashSet<usize>,
    /// `AskUserQuestion` permissions held open for the user to answer inline
    /// (PRODUCT §1), keyed by the gated `tool_use_id`. `claude` blocks the turn
    /// on this `can_use_tool` until we respond; the question card stays
    /// interactive while an entry is present, and submitting sends the picks
    /// back as the tool's `answers` so the model continues in the *same* turn —
    /// never auto-dismissed (§54/7m). The held `input` (the tool's `questions`)
    /// rides along so a typed free-text answer can respond even when the tool
    /// card isn't findable at the transcript top level (e.g. nested under a
    /// Task card).
    pending_question_permission: HashMap<String, HeldQuestionPermission>,
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
    /// Folder / branch / local diff / PR / CI shown in Environment.
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
    /// twarp: the zero-state session search field — with the sidebar "Code"
    /// tab gone, the "Welcome back" panel is the home for past sessions, so it
    /// gets the same title filter the sidebar had. Single-line; the Edited
    /// subscription re-renders (and resets paging) as the query changes.
    session_search: ViewHandle<EditorView>,
    /// How many filtered session rows the zero state currently shows.
    /// Starts at [`ZERO_STATE_INITIAL_SESSIONS`]; "Load more" adds
    /// [`ZERO_STATE_SESSIONS_PER_PAGE`]; reset whenever the query changes.
    sessions_shown: usize,
    /// Mouse state for the search field's inline clear (X) button.
    session_search_clear_mouse: MouseStateHandle,
    /// Mouse state for the session list's "Load more" row.
    sessions_load_more_mouse: MouseStateHandle,
    /// Pooled mouse handles, per transcript index, for the hover "Fork"
    /// affordance below assistant responses: `fork_row_mouse` senses the hover
    /// over the response block, `fork_button_mouse` drives the button itself.
    fork_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    fork_button_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Like [`Self::fork_button_mouse`], for the copy-response button beside it.
    copy_button_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Pooled mouse handles, per transcript index, for the hover "Edit"
    /// affordance below sent user messages: `user_row_mouse` senses the hover
    /// over the bubble, `edit_button_mouse` drives the pill itself.
    user_row_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    edit_button_mouse: std::cell::RefCell<Vec<MouseStateHandle>>,
    /// Mouse handles for the Environment menu's branch / CI / PR actions.
    branch_pill_mouse: MouseStateHandle,
    ci_pill_mouse: MouseStateHandle,
    pr_pill_mouse: MouseStateHandle,
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
    /// twarp: launch ids the user cleared from the background-scripts panel,
    /// keyed by the launching tool-use id. The list itself is derived read-only
    /// from the transcript (we can't delete a tool call), so "Clear" hides rows
    /// by remembering their ids here and filtering them out of every render.
    /// Clear only ever adds *non-running* scripts, so a still-live script is
    /// never accidentally hidden.
    background_dismissed: HashSet<String>,
    /// Stable per-row mouse handles for the background-script rows, keyed by
    /// launch id and created on demand (the `sent_image_mouse` pattern) so hover
    /// state survives across renders even though the rows are derived.
    background_row_mouse: std::cell::RefCell<HashMap<String, MouseStateHandle>>,
    /// Mouse state for the Scripts section's "Clear" button (clears non-running
    /// scripts).
    background_clear_mouse: MouseStateHandle,
    /// twarp: whether the floating agents panel is expanded. The panel lists
    /// this chat's sub-agent (`Task`/`Agent`) launches, derived from the
    /// transcript each render via [`agents::collect`] — the background-scripts
    /// panel's twin, sharing its top-right anchor (only one is open at a time).
    agents_panel_expanded: bool,
    /// Per-agent result disclosure, keyed by the launching tool-use id. An id
    /// is present iff the user expanded that row's returned result.
    agent_expanded_rows: HashSet<String>,
    /// twarp: agent launch ids the user cleared from the agents panel. The
    /// list is derived read-only from the transcript, so "Clear" hides rows by
    /// remembering their ids here — only ever *non-running* agents.
    agents_dismissed: HashSet<String>,
    /// Stable per-row mouse handles for the agent rows, keyed by launch id and
    /// created on demand (the `background_row_mouse` pattern).
    agent_row_mouse: std::cell::RefCell<HashMap<String, MouseStateHandle>>,
    /// Whether the consolidated Environment / Activity header menu is open.
    header_menu_expanded: bool,
    /// In-flight reveal animation for one of the menu's inline sections
    /// (Agent runs / Scripts expanding or collapsing in place). `None` when no
    /// section is animating; the expanded flags alone then decide visibility.
    header_menu_reveal: Option<(HeaderMenuSection, SectionReveal)>,
    /// Mouse state for the header's ⋯ menu button.
    header_menu_button_mouse: MouseStateHandle,
    /// Scroll position for Environment / Activity when expanded activity rows
    /// exceed the popover height cap.
    header_menu_scroll_state: ClippedScrollStateHandle,
    /// Mouse states for the Environment section's top-level actions.
    header_menu_changes_row_mouse: MouseStateHandle,
    header_menu_local_row_mouse: MouseStateHandle,
    header_menu_commit_row_mouse: MouseStateHandle,
    header_menu_compare_row_mouse: MouseStateHandle,
    header_menu_raw_cli_row_mouse: MouseStateHandle,
    /// Mouse states for the header menu's "Agent runs" and "Scripts" rows.
    header_menu_agents_row_mouse: MouseStateHandle,
    header_menu_scripts_row_mouse: MouseStateHandle,
    /// Mouse state for the Agent-runs section's "Clear" button.
    agents_clear_mouse: MouseStateHandle,
    /// Memoized [`agents::collect`] output, keyed by the transcript revision
    /// it was derived from — same rationale as [`Self::background_scripts_memo`].
    agents_memo: std::cell::RefCell<Option<(u64, std::rc::Rc<Vec<agents::AgentRun>>)>>,
    /// twarp 15b: per-Claude-session computer-control overlay lifecycle. The
    /// native AppKit windows live outside the Warp scene, so the coordinator is
    /// interior-mutable for render-time color refreshes when the active tab
    /// color changes.
    computer_control: std::cell::RefCell<ComputerControlCoordinator>,
    /// Mouse state for the feature-flagged computer-control header entry
    /// point.
    computer_control_button_mouse: MouseStateHandle,
    /// Memoized [`background_scripts::collect`] output, keyed by the transcript
    /// revision it was derived from. The list is recomputed (walking the whole
    /// transcript, descending into Task children) by both the header button and
    /// the floating panel; without this it ran twice per render. `Rc` so the
    /// two readers share one allocation; invalidated whenever the transcript's
    /// revision moves on.
    background_scripts_memo:
        std::cell::RefCell<Option<(u64, std::rc::Rc<Vec<background_scripts::BackgroundScript>>)>>,
    /// twarp 17: the live mic recording, when this pane is recording
    /// (PRODUCT 17 §4). `Some` iff this pane holds the app-wide recording slot.
    voice_recorder: Option<crate::voice::capture::Recorder>,
    /// twarp 17: a transcription request is in flight (PRODUCT 17 §5) — the mic
    /// button shows a pending state and further clicks are no-ops.
    voice_transcribing: bool,
    /// Guards stale transcription results after cancel / pane churn.
    voice_generation: u64,
    /// twarp 17: whether the composer was empty when recording started — the
    /// PRODUCT 17 §7 auto-send eligibility bit.
    voice_composer_was_empty: bool,
    /// twarp 17: one-line voice status / error under the composer controls
    /// (PRODUCT 17 §2–§3, §8, §17). Cleared when the next voice action starts.
    voice_status: Option<String>,
    /// twarp 17: speak replies aloud (PRODUCT 17 §12). Per-pane, off by
    /// default, not persisted.
    speak_replies: bool,
    /// twarp 17: the playback thread, spawned lazily on first speech and kept
    /// for the pane's lifetime (dropping it silences and tears down the
    /// output stream — PRODUCT 17 §18's close-pane stop for free).
    voice_player: Option<crate::voice::playback::Player>,
    /// Guards stale TTS chunks after a newer turn started speaking (§15) or
    /// the toggle flipped off (§14).
    voice_tts_generation: u64,
    /// twarp 17 §30–§31: the composer suffix owned by live transcription
    /// (including its leading separator). Live updates replace exactly this
    /// suffix and stand down the moment the user edits it.
    voice_live_text: String,
    /// Whether live transcription still owns the composer suffix (§31).
    voice_live_owned: bool,
    /// One live transcription request in flight at a time (§30).
    voice_live_inflight: bool,
    /// When the previous live transcription request was sent (§30 cadence).
    voice_live_last: Option<Instant>,
    /// A snapshot encode pending on the capture thread (§30) — polled with
    /// `try_recv` on the voice tick so the UI thread never blocks on audio.
    voice_live_snapshot:
        Option<std::sync::mpsc::Receiver<Result<Vec<u8>, crate::voice::VoiceError>>>,
    /// twarp 17 §32: transcript index of the assistant item being spoken.
    voice_tts_item: Option<usize>,
    /// The prose already queued for synthesis, verbatim — comparing against a
    /// fresh markdown→prose pass keeps offsets honest while text streams.
    voice_tts_spoken_prose: String,
    /// Sentences awaiting synthesis (§32) — synthesized one at a time so the
    /// audio queue stays ordered.
    voice_tts_pending: std::collections::VecDeque<String>,
    /// A synthesis request is in flight (§32).
    voice_tts_inflight: bool,
    /// The next queued chunk starts a new utterance (player `play`, not
    /// `append`) — set when a new reply begins speaking (§15).
    voice_tts_first_chunk: bool,
    /// twarp 17 §33: spoken chunks of the current utterance —
    /// `(text, start_secs, len_secs)` at the TTS sample rate — mapped against
    /// the player position for the karaoke highlight.
    voice_karaoke_chunks: Vec<(String, f32, f32)>,
    /// Mouse state for the composer mic button.
    mic_button_mouse: MouseStateHandle,
    /// Pulse state for the mic glyph while recording (PRODUCT 17 §4).
    mic_pulse: PulsingIconStateHandle,
    /// Mouse state for the composer speaker button.
    speaker_button_mouse: MouseStateHandle,
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
            provider,
            prompt,
            permission_mode,
            model,
            effort,
            resume_session_id,
        } = launch;

        // Feature 16a: the Agent settings Chat row is authoritative for fresh
        // panes. Explicit launch flags still win, but the old last-used
        // `claude_session_defaults` row no longer seeds new panes.
        let settings = AgentSettings::as_ref(ctx);
        let chat_config = settings.chat_launch_config();
        let chat_config_matches_provider = chat_config.provider.agent_provider() == Some(provider);
        let model = model.or_else(|| {
            chat_config_matches_provider
                .then_some(chat_config.model)
                .flatten()
        });
        let effort = effort.or_else(|| {
            chat_config_matches_provider
                .then_some(chat_config.effort)
                .flatten()
        });
        let permission_mode = permission_mode.unwrap_or(chat_config.permission_mode);

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
                // 16e / PRODUCT 16 decision #10: a Tab-acceptable reply
                // suggestion in the empty composer replaces the static
                // "Message Claude Code…" placeholder instead of hiding
                // under it.
                autosuggestion_overrides_placeholder: true,
                ..Default::default()
            };
            EditorView::new(options, ctx)
        });
        ctx.subscribe_to_view(&input_editor, Self::handle_editor_event);
        input_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(provider_copy(provider).composer_placeholder, ctx);
        });

        // twarp: the zero-state session search field (mirrors the old sidebar
        // sessions search — left_panel 08e). Single line; Edited re-renders the
        // filtered list and resets paging to the first page.
        let session_search = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_font_size(appearance),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });
        ctx.subscribe_to_view(&session_search, |me, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(_)) {
                me.sessions_shown = ZERO_STATE_INITIAL_SESSIONS;
                ctx.notify();
            }
        });
        session_search.update(ctx, |editor, ctx| {
            editor.set_placeholder_text("Search sessions", ctx);
        });

        let pane_configuration =
            ctx.add_model(|_ctx| PaneConfiguration::new(provider_copy(provider).pane_title));

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

        let restored_session = resume.is_some();

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
            provider,
            window_id: ctx.window_id(),
            input_editor,
            pane_configuration,
            focus_handle: None,
            cwd,
            interactive_path: None,
            interactive_env_vars: None,
            codex_restore_pending: restored_session && provider == AgentProvider::Codex,
            scroll_state: ClippedScrollStateHandle::default(),
            timeline_position_key: uuid::Uuid::new_v4().to_string(),
            timeline_turn_mouse: Default::default(),
            selection_handle: Default::default(),
            transcript_selection: Default::default(),
            child: None,
            message_tx: None,
            pending_initial_turn: None,
            session_spawn_pending: false,
            streaming: false,
            tab_attention: None,
            tab_attention_seen: false,
            deferred_completion: None,
            interrupt_pending: false,
            submit_button: MouseStateHandle::default(),
            refresh_button: MouseStateHandle::default(),
            stop_button: MouseStateHandle::default(),
            tool_card_ui: HashMap::new(),
            expanded_completed_turns: HashSet::new(),
            completed_turn_mouse: Default::default(),
            artifact_file_mouse: Default::default(),
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
            raw_cli_pending: false,
            suggestion_query: None,
            suggestions: Vec::new(),
            suggestion_selected: 0,
            suggestion_row_mouse: std::cell::RefCell::new(Vec::new()),
            reply_suggestion_generation: 0,
            composer_placeholder_generation: 0,
            composer_placeholder_suggestion: None,
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
            question_submitted: HashMap::new(),
            question_answer_items: HashSet::new(),
            pending_question_permission: HashMap::new(),
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
            session_search,
            sessions_shown: ZERO_STATE_INITIAL_SESSIONS,
            session_search_clear_mouse: MouseStateHandle::default(),
            sessions_load_more_mouse: MouseStateHandle::default(),
            fork_row_mouse: std::cell::RefCell::new(Vec::new()),
            fork_button_mouse: std::cell::RefCell::new(Vec::new()),
            copy_button_mouse: std::cell::RefCell::new(Vec::new()),
            user_row_mouse: std::cell::RefCell::new(Vec::new()),
            edit_button_mouse: std::cell::RefCell::new(Vec::new()),
            branch_pill_mouse: MouseStateHandle::default(),
            ci_pill_mouse: MouseStateHandle::default(),
            pr_pill_mouse: MouseStateHandle::default(),
            branch_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            ci_menu_row_mouse: std::cell::RefCell::new(Vec::new()),
            use_worktree: false,
            background_scripts_expanded: false,
            background_expanded_rows: HashSet::new(),
            background_dismissed: HashSet::new(),
            background_row_mouse: std::cell::RefCell::new(HashMap::new()),
            background_clear_mouse: MouseStateHandle::default(),
            agents_panel_expanded: false,
            agent_expanded_rows: HashSet::new(),
            agents_dismissed: HashSet::new(),
            agent_row_mouse: std::cell::RefCell::new(HashMap::new()),
            header_menu_expanded: false,
            header_menu_reveal: None,
            header_menu_button_mouse: MouseStateHandle::default(),
            header_menu_scroll_state: ClippedScrollStateHandle::default(),
            header_menu_changes_row_mouse: MouseStateHandle::default(),
            header_menu_local_row_mouse: MouseStateHandle::default(),
            header_menu_commit_row_mouse: MouseStateHandle::default(),
            header_menu_compare_row_mouse: MouseStateHandle::default(),
            header_menu_raw_cli_row_mouse: MouseStateHandle::default(),
            header_menu_agents_row_mouse: MouseStateHandle::default(),
            header_menu_scripts_row_mouse: MouseStateHandle::default(),
            agents_clear_mouse: MouseStateHandle::default(),
            agents_memo: std::cell::RefCell::new(None),
            computer_control: std::cell::RefCell::new(ComputerControlCoordinator::default()),
            computer_control_button_mouse: MouseStateHandle::default(),
            background_scripts_memo: std::cell::RefCell::new(None),
            voice_recorder: None,
            voice_transcribing: false,
            voice_generation: 0,
            voice_composer_was_empty: false,
            voice_status: None,
            speak_replies: false,
            voice_player: None,
            voice_tts_generation: 0,
            voice_live_text: String::new(),
            voice_live_owned: false,
            voice_live_inflight: false,
            voice_live_last: None,
            voice_live_snapshot: None,
            voice_tts_item: None,
            voice_tts_spoken_prose: String::new(),
            voice_tts_pending: std::collections::VecDeque::new(),
            voice_tts_inflight: false,
            voice_tts_first_chunk: true,
            voice_karaoke_chunks: Vec::new(),
            mic_button_mouse: MouseStateHandle::default(),
            mic_pulse: PulsingIconStateHandle::default(),
            speaker_button_mouse: MouseStateHandle::default(),
        };

        // PRODUCT §4/§8: capture the login-shell environment up front so CLI
        // discovery and restored provider credentials survive a GUI
        // (launchd-minimal) launch. Resolves asynchronously and re-renders.
        Self::capture_interactive_path(ctx);

        // Kick off the once-per-app-run model discovery so the model dropdown
        // lists what the account can actually use. Best-effort: no key or no
        // network just leaves the alias fallback in place.
        Self::discover_models(ctx);

        // PRODUCT §36: a resumed pane renders the stored history up front —
        // through the same ingest path as live events so tool/diff/thinking
        // card state exists — and continues live on the next message.
        if let Some(resume) = resume {
            if provider == AgentProvider::Claude {
                for event in sessions::load_history(&resume.jsonl_path) {
                    view.ingest_event(event, ctx);
                }
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
            ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
            view.turn_started = Some(started);
            view.schedule_elapsed_tick(started, ctx);
            view.begin_session(Some(OutgoingMessage::text(prompt)), ctx);
        }

        // twarp: a bare `claude`/`codex` opens to the zero state — load the
        // cwd's recent sessions for the pane's own provider so the empty pane
        // doubles as a launchpad (pick one up, or type to start fresh).
        // Skipped when the pane already has content (a resumed pane or
        // `claude <prompt>`), where the transcript replaces the panel.
        if view.transcript.is_empty() {
            view.recent_sessions = view
                .cwd
                .as_deref()
                .map(|cwd| sessions::list_sessions_for(provider, cwd))
                .unwrap_or_default();
        }

        // #7: name the tab from the first user message (resumed history or the
        // `claude <prompt>` first turn); stays "Claude Code" for a bare `claude`
        // until the user sends something.
        view.update_pane_title(ctx);

        // Populate Environment for the pane's directory.
        view.refresh_repo_context(ctx);
        view.maybe_request_composer_placeholder_suggestion(ctx);

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

    pub fn provider(&self) -> AgentProvider {
        self.provider
    }

    pub fn has_provider_session(&self) -> bool {
        match self.provider {
            AgentProvider::Claude => self.transcript.session_id().is_some(),
            AgentProvider::Codex => self.transcript.session_id().is_some(),
        }
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
        crate::workspace::view::active_tab_accent(self.window_id, app)
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
    fn provider_binary(&self) -> &'static str {
        match self.provider {
            AgentProvider::Claude => CLAUDE_BINARY,
            AgentProvider::Codex => CODEX_BINARY,
        }
    }

    fn provider_available(&self) -> bool {
        let binary = self.provider_binary();
        if let Some(path) = &self.interactive_path {
            if resolve_executable_in_path(binary, std::ffi::OsStr::new(path)).is_some() {
                return true;
            }
        }
        resolve_executable(binary).is_some()
    }

    /// Kick off (or refresh) the async capture of the interactive login-shell
    /// environment, storing its PATH and exported variables on the view. The
    /// underlying capture is cached by `LocalShellState`, so repeated calls
    /// (e.g. the "Check again" button) are cheap. No-op when the local shell
    /// integration isn't compiled in; availability then uses the process PATH.
    #[cfg(all(feature = "local_fs", feature = "local_tty"))]
    fn capture_interactive_path(ctx: &mut ViewContext<Self>) {
        let fut = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_env_vars(ctx)
        });
        ctx.spawn(fut, |me, env_vars, ctx| {
            let path = env_vars
                .as_ref()
                .and_then(|env_vars| env_vars.get("PATH").cloned());
            me.interactive_env_vars = env_vars;
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
            }
            if me.codex_restore_pending {
                me.codex_restore_pending = false;
                // Codex history is exposed by app-server thread/resume, not a
                // stable local JSONL. Resume only after the login-shell
                // environment is available so provider credentials survive a
                // Finder/Dock relaunch.
                let first_prompt = me.pending_initial_turn.take();
                me.begin_session(first_prompt, ctx);
            }
            ctx.notify();
        });
    }

    #[cfg(not(all(feature = "local_fs", feature = "local_tty")))]
    fn capture_interactive_path(ctx: &mut ViewContext<Self>) {
        ctx.defer(|me, ctx| {
            if me.codex_restore_pending {
                me.codex_restore_pending = false;
                let first_prompt = me.pending_initial_turn.take();
                me.begin_session(first_prompt, ctx);
            }
        });
    }

    /// Fetch the account's model list from the Anthropic Models API once per
    /// app run (first pane wins the claim) and re-render when it lands, so the
    /// model dropdown auto-includes newly launched models without a rebuild.
    fn discover_models(ctx: &mut ViewContext<Self>) {
        if !crate::claude_code_models::claim_fetch() {
            return;
        }
        ctx.spawn(crate::claude_code_models::fetch(), |_me, models, ctx| {
            if let Some(models) = models {
                crate::claude_code_models::store(models);
                ctx.notify();
            }
        });
    }

    /// Refresh Environment: run `git`/`gh` in the user's
    /// login shell (so they resolve and the right repo/PR are visible) and store
    /// the parsed folder / branch / diff / PR / CI. Best-effort and off the main
    /// thread — a missing repo, absent `gh`, or a slow network call just leaves
    /// the menu partial or unchanged. Called on open and after each turn (the
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
    /// then refresh Environment. Best-effort: any failure leaves the context
    /// unchanged. Shared by the branch-switch and Create-PR menu actions.
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
        if self.streaming {
            ctx.notify();
            return;
        }
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
                    self.clear_reply_suggestion(ctx);
                    self.clear_composer_placeholder_suggestion(ctx);
                    self.submit(ctx);
                }
            }
            // Live-filter the suggestions + attachment chips as the draft
            // changes (PRODUCT §15a–§15b).
            EditorEvent::Edited(_) => {
                self.clear_reply_suggestion(ctx);
                self.refresh_composer_intelligence(ctx);
                let composer_empty = self
                    .input_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().is_empty());
                if composer_empty {
                    self.maybe_request_composer_placeholder_suggestion(ctx);
                } else {
                    self.clear_composer_placeholder_suggestion(ctx);
                }
            }
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
                // twarp 17 (PRODUCT 17 §6): Esc while recording cancels the
                // recording — audio discarded, no request — and consumes the
                // keypress; the composer behaviors below keep Esc otherwise.
                if self.voice_recorder.is_some() {
                    self.cancel_voice_recording(ctx);
                    return;
                }
                if self.header_menu_expanded {
                    self.header_menu_expanded = false;
                    self.header_menu_reveal = None;
                    self.composer_menu = None;
                    ctx.notify();
                    return;
                }
                if self.composer_menu.take().is_some() {
                    ctx.notify();
                    return;
                }
                self.clear_reply_suggestion(ctx);
                self.clear_composer_placeholder_suggestion(ctx);
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
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.accept_composer_placeholder_suggestion(ctx);
            }
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

    fn clear_reply_suggestion(&mut self, ctx: &mut ViewContext<Self>) {
        self.reply_suggestion_generation = self.reply_suggestion_generation.wrapping_add(1);
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_autosuggestion(ctx));
    }

    fn clear_composer_placeholder_suggestion(&mut self, ctx: &mut ViewContext<Self>) {
        self.composer_placeholder_generation = self.composer_placeholder_generation.wrapping_add(1);
        self.composer_placeholder_suggestion = None;
        self.input_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(provider_copy(self.provider).composer_placeholder, ctx);
        });
    }

    fn composer_placeholder_repo_context(&self) -> Option<String> {
        let context = self.repo_context.as_ref()?;
        let mut parts = Vec::new();
        if let Some(folder) = &context.folder {
            parts.push(format!("folder {folder}"));
        }
        if let Some(branch) = &context.branch {
            parts.push(format!("branch {branch}"));
        }
        if context.local_added.is_some() || context.local_removed.is_some() {
            parts.push(format!(
                "diff +{} -{}",
                context.local_added.unwrap_or_default(),
                context.local_removed.unwrap_or_default()
            ));
        }
        if let Some(pr_number) = context.pr_number {
            parts.push(format!("PR #{pr_number}"));
        }
        if let Some(ci) = context.ci {
            parts.push(ci.label().to_owned());
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    fn maybe_request_composer_placeholder_suggestion(&mut self, ctx: &mut ViewContext<Self>) {
        if self.provider != AgentProvider::Claude {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        }
        if !*AgentSettings::as_ref(ctx)
            .enable_composer_placeholder_suggestions
            .value()
        {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        }
        if self.streaming || self.raw_cli.is_some() || !self.message_queue.is_empty() {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        }
        let composer_ready = self.input_editor.read(ctx, |editor, ctx| {
            editor.buffer_text(ctx).trim().is_empty() && !editor.active_autosuggestion()
        });
        if !composer_ready {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        }

        let cwd = self
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let context = ComposerPlaceholderSuggestionContext::new(
            &self.transcript,
            Some(cwd.display().to_string()),
            self.composer_placeholder_repo_context(),
        );
        let Some(context) = context else {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        };

        let settings = AgentSettings::as_ref(ctx);
        let mut config = settings.placeholder_suggest_config();
        let chat_config = settings.chat_launch_config();
        if config.provider.is_inherit() {
            config.model = config.model.or(chat_config.model);
            config.effort = config.effort.or(chat_config.effort);
        }
        let chat_provider = settings.chat_provider_agent();
        let resolved_provider = config.provider.resolve(chat_provider);
        let request = crate::agent_suggestions::SuggestionRequest {
            config,
            chat_provider,
            api_key: self.api_key_for_agent(resolved_provider, ctx),
            cwd,
            path_env: self.interactive_path.clone(),
            context: SuggestionContext::ComposerPlaceholder(context),
        };

        self.composer_placeholder_generation = self.composer_placeholder_generation.wrapping_add(1);
        let generation = self.composer_placeholder_generation;
        ctx.spawn(
            DefaultSuggestionProvider.suggest(request),
            move |view, suggestion, ctx| {
                if view.composer_placeholder_generation != generation {
                    return;
                }
                view.apply_composer_placeholder_suggestion(suggestion, ctx);
            },
        );
    }

    fn apply_composer_placeholder_suggestion(
        &mut self,
        suggestion: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !*AgentSettings::as_ref(ctx)
            .enable_composer_placeholder_suggestions
            .value()
        {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        }
        if self.streaming || self.raw_cli.is_some() {
            return;
        }
        let composer_ready = self.input_editor.read(ctx, |editor, ctx| {
            editor.buffer_text(ctx).trim().is_empty() && !editor.active_autosuggestion()
        });
        if !composer_ready {
            return;
        }

        let Some(suggestion) = suggestion else {
            self.clear_composer_placeholder_suggestion(ctx);
            return;
        };
        self.composer_placeholder_suggestion = Some(suggestion.clone());
        self.input_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(suggestion, ctx);
        });
    }

    fn accept_composer_placeholder_suggestion(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !*AgentSettings::as_ref(ctx)
            .enable_composer_placeholder_suggestions
            .value()
        {
            self.clear_composer_placeholder_suggestion(ctx);
            return false;
        }
        let Some(suggestion) = self.composer_placeholder_suggestion.clone() else {
            return false;
        };
        let composer_ready = self.input_editor.read(ctx, |editor, ctx| {
            editor.buffer_text(ctx).trim().is_empty() && !editor.active_autosuggestion()
        });
        if !composer_ready {
            return false;
        }

        self.clear_composer_placeholder_suggestion(ctx);
        self.input_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text(&suggestion, ctx));
        true
    }

    fn maybe_request_reply_suggestion(&mut self, ctx: &mut ViewContext<Self>) {
        if self.provider != AgentProvider::Claude {
            self.clear_reply_suggestion(ctx);
            return;
        }
        if !*AgentSettings::as_ref(ctx).enable_reply_suggestions.value() {
            self.clear_reply_suggestion(ctx);
            return;
        }
        if self.streaming || self.raw_cli.is_some() || !self.message_queue.is_empty() {
            self.clear_reply_suggestion(ctx);
            return;
        }
        let composer_ready = self.input_editor.read(ctx, |editor, ctx| {
            editor.is_focused() && editor.buffer_text(ctx).trim().is_empty()
        });
        if !composer_ready {
            self.clear_reply_suggestion(ctx);
            return;
        }

        let Some(context) = ReplySuggestionContext::from_transcript(&self.transcript) else {
            self.clear_reply_suggestion(ctx);
            return;
        };

        let settings = AgentSettings::as_ref(ctx);
        let mut config = settings.reply_suggest_config();
        let chat_config = settings.chat_launch_config();
        if config.provider.is_inherit() {
            config.model = config.model.or(chat_config.model);
            config.effort = config.effort.or(chat_config.effort);
        }
        let resolved_provider = config.provider.resolve(settings.chat_provider_agent());
        let api_key = self.api_key_for_agent(resolved_provider, ctx);
        let request = crate::agent_suggestions::SuggestionRequest {
            config,
            chat_provider: settings.chat_provider_agent(),
            api_key,
            cwd: self
                .cwd
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
            path_env: self.interactive_path.clone(),
            context: SuggestionContext::Reply(context),
        };

        self.reply_suggestion_generation = self.reply_suggestion_generation.wrapping_add(1);
        let generation = self.reply_suggestion_generation;
        ctx.spawn(
            DefaultSuggestionProvider.suggest(request),
            move |view, suggestion, ctx| {
                if view.reply_suggestion_generation != generation {
                    return;
                }
                view.apply_reply_suggestion(suggestion, ctx);
            },
        );
    }

    fn api_key_for_agent(&self, agent: CLIAgent, ctx: &AppContext) -> Option<String> {
        if !app_settings::api_key_presence(AgentSettings::as_ref(ctx), agent) {
            return None;
        }
        let storage_key = app_settings::api_key_storage_key(agent)?;
        secure_storage::Model::handle(ctx)
            .as_ref(ctx)
            .read_value(&storage_key)
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
    }

    fn apply_reply_suggestion(&mut self, suggestion: Option<String>, ctx: &mut ViewContext<Self>) {
        if !*AgentSettings::as_ref(ctx).enable_reply_suggestions.value() {
            self.clear_reply_suggestion(ctx);
            return;
        }
        if self.streaming || self.raw_cli.is_some() || suggestion.is_none() {
            return;
        }
        let composer_ready = self.input_editor.read(ctx, |editor, ctx| {
            editor.is_focused() && editor.buffer_text(ctx).trim().is_empty()
        });
        if !composer_ready {
            return;
        }
        let suggestion = suggestion.unwrap();
        self.clear_composer_placeholder_suggestion(ctx);
        self.input_editor.update(ctx, |editor, ctx| {
            editor.set_autosuggestion(
                suggestion,
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::AgentModeQuery {
                    context_block_ids: Vec::new(),
                    was_intelligent_autosuggestion: true,
                },
                ctx,
            );
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
        self.clear_reply_suggestion(ctx);
        self.clear_composer_placeholder_suggestion(ctx);
        // PRODUCT §1: claude is parked on an AskUserQuestion — a typed message
        // is the user's answer (the free-form "Other" path), not type-ahead
        // for after the question. Consume it as the held permission's answer.
        if self.try_answer_pending_question(&message, ctx) {
            return;
        }
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
        ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
        self.turn_started = Some(started);
        self.schedule_elapsed_tick(started, ctx);
        if self.codex_restore_pending {
            self.pending_initial_turn = Some(message);
        } else {
            match &self.message_tx {
                // Session already running — write the turn to its stdin.
                Some(tx) => {
                    let _ = tx.try_send(StdinCommand::Turn(message));
                }
                // First message: spawn the session, forwarding this as turn one.
                None => self.begin_session(Some(message), ctx),
            }
        }
        // #7: the first user turn names the tab.
        self.update_pane_title(ctx);
        // PRODUCT §14: a user turn always jumps back to the live bottom.
        self.scroll_to_bottom();
        ctx.notify();
    }

    /// A typed message while claude is holding the turn on an `AskUserQuestion`
    /// `can_use_tool` (PRODUCT §1) answers the question directly — the free-form
    /// counterpart of the card's "Other" option — instead of queueing behind a
    /// turn that can't advance until the question is answered. Fills every
    /// question on the most recent pending card with the typed text, releases
    /// the held permission, and echoes the text as a user bubble so the answer
    /// stays visible in the transcript. Returns `true` if the message was
    /// consumed this way. (Attached images can't ride along in a tool answer
    /// and are dropped.)
    fn try_answer_pending_question(
        &mut self,
        message: &OutgoingMessage,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self.pending_question_permission.is_empty() {
            return self.try_answer_pending_question_dialog(message, ctx);
        }
        // The most recent pending question card is the one on screen. The
        // transcript lookup is best-effort card locking — a question raised by
        // a Task sub-agent nests under the Task card and isn't at the top
        // level, but the held permission must still be answered (otherwise the
        // typed reply queues behind a turn that can't advance, PRODUCT §1).
        let item =
            self.transcript.items().iter().enumerate().rev().find_map(
                |(index, entry)| match entry {
                    TranscriptItem::Tool { id, name, .. }
                        if name == "AskUserQuestion"
                            && self.pending_question_permission.contains_key(id) =>
                    {
                        Some((index, id.clone()))
                    }
                    _ => None,
                },
            );
        let tool_use_id = match &item {
            Some((_, id)) => id.clone(),
            // No top-level card (nested / not yet streamed in): answer the
            // only held question, or bail if several are ambiguous.
            None if self.pending_question_permission.len() == 1 => self
                .pending_question_permission
                .keys()
                .next()
                .expect("len == 1")
                .clone(),
            None => return false,
        };
        let Some(held) = self.pending_question_permission.remove(&tool_use_id) else {
            return false;
        };
        log::warn!("QDIAG free-text answer to pending question item={item:?}");
        let mut answers = serde_json::Map::new();
        for question in parse_questions(&held.input) {
            answers.insert(
                question.question.clone(),
                serde_json::Value::String(message.text.clone()),
            );
        }
        let request_id = held.request_id;
        let mut updated_input = held.input;
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("answers".to_owned(), serde_json::Value::Object(answers));
        }
        // Lock the card (no live controls) and show the typed reply as the
        // user's answer beneath it.
        if let Some((item, _)) = item {
            let picks = self.question_selected.remove(&item).unwrap_or_default();
            self.question_submitted.insert(item, picks);
        }
        self.transcript
            .apply(TranscriptEvent::UserMessage(message.text.clone()));
        // The bubble is display-only — the file records the answer inside the
        // tool call, not as a user turn (see `question_answer_items`).
        self.question_answer_items
            .insert(self.transcript.items().len().saturating_sub(1));
        if let Some(tx) = &self.message_tx {
            let _ = tx.try_send(StdinCommand::Control {
                request_id,
                decision: Decision::allow_once(updated_input),
            });
        }
        // claude continues the same turn with the answer; make sure the
        // "Working…" status is live (mirrors submit_question_answers).
        if !self.streaming {
            let started = Instant::now();
            self.streaming = true;
            ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
            self.turn_started = Some(started);
            self.schedule_elapsed_tick(started, ctx);
        }
        self.scroll_to_bottom();
        ctx.notify();
        true
    }

    /// The control-channel counterpart: a typed message while an unanswered
    /// `request_user_dialog` question (PRODUCT §24/§1) is parked releases the
    /// dialog (cancelled, the same §26 never-hang route `submit_question_dialog`
    /// takes) so the text can go out as the next turn instead of queueing
    /// behind a dialog that never advances. Returns `false` either way — the
    /// message itself still flows through the normal send path.
    fn try_answer_pending_question_dialog(
        &mut self,
        _message: &OutgoingMessage,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some((item, request_id)) = self.transcript.items().iter().enumerate().rev().find_map(
            |(index, entry)| match entry {
                TranscriptItem::Question { id, answered, .. } if !answered => {
                    Some((index, id.clone()))
                }
                _ => None,
            },
        ) else {
            return false;
        };
        // Lock the card and release the dialog; the typed text follows as the
        // next user turn once claude ends the (now unblocked) turn.
        self.transcript.answer_question(&request_id);
        if let Some(picks) = self.question_selected.remove(&item) {
            self.question_submitted.insert(item, picks);
        }
        if let Some(tx) = &self.message_tx {
            let _ = tx.try_send(StdinCommand::Control {
                request_id,
                decision: Decision::cancelled(),
            });
        }
        false
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

    /// Submit the chosen answers for the `AskUserQuestion` card at `item`
    /// (PRODUCT §1). If claude is still holding the turn on this question's
    /// `can_use_tool` (the common case), answer it inline with the picks as the
    /// tool's `answers` so the model continues the same turn. Otherwise (the
    /// turn already ended) fall back to resending the picks as the next user
    /// turn; the model reads the question from the transcript above.
    fn submit_question_answers(&mut self, item: usize, ctx: &mut ViewContext<Self>) {
        let (tool_use_id, input, parsed) = {
            let Some(TranscriptItem::Tool {
                id, name, input, ..
            }) = self.transcript.items().get(item)
            else {
                return;
            };
            if name != "AskUserQuestion" {
                return;
            }
            (id.clone(), input.clone(), parse_questions(input))
        };
        let Some(selected) = self.question_selected.get(&item) else {
            return;
        };
        // `answers` maps each question's text → the chosen label(s) (multi-select
        // joined by ", "), matching `AskUserQuestionInput.answers` — the field
        // claude's "permission component" fills in. `lines` is the human form for
        // the next-turn fallback below.
        let mut answers = serde_json::Map::new();
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
            answers.insert(
                question.question.clone(),
                serde_json::Value::String(picks.join(", ")),
            );
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
        log::warn!(
            "QDIAG submit_question_answers item={item} streaming={} has_pending={} has_tx={}",
            self.streaming,
            self.pending_question_permission.contains_key(&tool_use_id),
            self.message_tx.is_some(),
        );
        // Lock the card into an answered state: keep the chosen options
        // visible (so the answer doesn't appear to vanish) and stop offering
        // live controls now that the answer is on its way.
        if let Some(picks) = self.question_selected.remove(&item) {
            self.question_submitted.insert(item, picks);
        }

        // Preferred path: a held `can_use_tool` is still parked on this question
        // (PRODUCT §1). Answer it inline — claude reads `answers` back as the
        // tool result and continues the *same* turn, so nothing is skipped.
        if let Some(held) = self.pending_question_permission.remove(&tool_use_id) {
            let request_id = held.request_id;
            let mut updated_input = input;
            if let Some(obj) = updated_input.as_object_mut() {
                obj.insert("answers".to_owned(), serde_json::Value::Object(answers));
            }
            if let Some(tx) = &self.message_tx {
                let _ = tx.try_send(StdinCommand::Control {
                    request_id,
                    decision: Decision::allow_once(updated_input),
                });
            }
            // claude now continues the turn to produce its reply. Make sure the
            // "Working…" status is live and scroll it into view so the user has a
            // clear loading indicator instead of a card that just went quiet.
            if !self.streaming {
                let started = Instant::now();
                self.streaming = true;
                ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
                self.turn_started = Some(started);
                self.schedule_elapsed_tick(started, ctx);
            }
            self.scroll_to_bottom();
            ctx.notify();
            return;
        }

        // Fallback (the turn already ended without the held permission): resend
        // the picks as an ordinary next turn; the model reads the question from
        // the transcript above.
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
        let decision = if allow {
            Decision::allow_once(input)
        } else {
            Decision::deny()
        };
        if let Some(tx) = &self.message_tx {
            let _ = tx.try_send(StdinCommand::Control {
                request_id: request_id.to_owned(),
                decision,
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
        if let Some(picks) = self.question_selected.remove(&item) {
            self.question_submitted.insert(item, picks);
        }
        if let Some(tx) = &self.message_tx {
            let _ = tx.try_send(StdinCommand::Control {
                request_id,
                decision: Decision::cancelled(),
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
        self.clear_reply_suggestion(ctx);
        self.queue_expanded.clear();
        self.transcript
            .apply(TranscriptEvent::UserMessage(message.text.clone()));
        let started = Instant::now();
        self.streaming = true;
        ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
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
        if self.streaming && self.pending_question_permission.is_empty() {
            // Mid-stream there's nothing to send onto yet — jump the queue.
            self.message_queue.insert(0, message);
            ctx.notify();
        } else {
            // Idle, or the turn is parked on an AskUserQuestion (in which case
            // submit_message consumes this as the question's free-text answer).
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
            provider: self.provider,
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
            mcp_config: claude_mcp_config_json(&self.session_id, ctx),
            path_env: self.interactive_path.clone(),
            env_vars: (self.provider == AgentProvider::Codex)
                .then(|| self.interactive_env_vars.clone())
                .flatten(),
        };
        self.session_spawn_pending = true;
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
        self.session_spawn_pending = false;
        let SpawnedSession {
            child,
            stdin,
            mut events,
            codex_state,
        } = match result {
            Ok(session) => session,
            Err(err) => {
                // PRODUCT §28/§30: surface the spawn failure verbatim.
                self.streaming = false;
                self.raw_cli_pending = false;
                // A failed resume must not wedge the pane on the dead id —
                // the next message starts fresh (PRODUCT §37).
                self.resume_session_id = None;
                self.pending_initial_turn = None;
                self.transcript.apply(TranscriptEvent::Ended {
                    reason: claude_code::EndReason::Error(format!(
                        "Couldn't start `{}`: {err}",
                        self.provider_binary()
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
        let provider = self.provider;
        ctx.background_executor()
            .spawn(async move {
                let mut stdin = stdin;
                let codex_driver = codex_state.map(CodexDriver::new);
                while let Ok(command) = message_rx.recv().await {
                    let wrote = match command {
                        StdinCommand::Turn(message) => match (provider, codex_driver.as_ref()) {
                            (AgentProvider::Codex, Some(driver)) => {
                                driver.send_user_message(&mut stdin, &message).await
                            }
                            _ => send_user_message(&mut stdin, &message).await,
                        },
                        StdinCommand::Control {
                            request_id,
                            decision,
                        } => match (provider, codex_driver.as_ref()) {
                            (AgentProvider::Codex, Some(driver)) => {
                                driver.answer(&mut stdin, &request_id, decision).await
                            }
                            _ => send_control_response(&mut stdin, &request_id, decision).await,
                        },
                        StdinCommand::Interrupt { request_id } => {
                            match (provider, codex_driver.as_ref()) {
                                (AgentProvider::Codex, Some(driver)) => {
                                    driver.interrupt(&mut stdin).await
                                }
                                _ => send_interrupt(&mut stdin, &request_id).await,
                            }
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
            if self.provider == AgentProvider::Codex {
                self.pending_initial_turn = Some(prompt);
            } else {
                let _ = tx.try_send(StdinCommand::Turn(prompt));
            }
        }
        ctx.notify();
    }

    fn send_pending_initial_turn(&mut self) {
        let Some(prompt) = self.pending_initial_turn.take() else {
            return;
        };
        let Some(tx) = &self.message_tx else {
            self.pending_initial_turn = Some(prompt);
            return;
        };
        if tx.try_send(StdinCommand::Turn(prompt)).is_err() {
            self.pending_initial_turn = None;
        }
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
        // PRODUCT §1: an `AskUserQuestion` arrives as a `can_use_tool` that
        // blocks the turn until we answer. We *hold* it open and answer it with
        // the user's picks (`submit_question_answers`) — claude reads those back
        // as the tool's `answers` and continues in the same turn. The inline
        // question card (the §1 tool-card path) stays interactive while the
        // permission is pending, so the user always has a live control. This
        // replaces the old auto-allow, which resolved as "the user did not
        // answer the questions" and effectively skipped the question.
        //
        // We key the held request by its `tool_use_id` so the card can find it.
        // If claude ever omits that id (shouldn't happen), auto-allow as a
        // safety valve so the turn never wedges (§26: never hang).
        if let TranscriptEvent::PermissionRequest {
            id,
            tool,
            input,
            tool_use_id,
        } = &event
        {
            if should_hold_question_permission(tool) {
                if let Some(tool_use_id) = tool_use_id {
                    log::warn!(
                        "QDIAG hold question permission tool_use_id={tool_use_id} streaming={}",
                        self.streaming,
                    );
                    self.pending_question_permission.insert(
                        tool_use_id.clone(),
                        HeldQuestionPermission {
                            request_id: id.clone(),
                            input: input.clone(),
                        },
                    );
                    // 7p: the turn is now parked on the user. Notify if
                    // they're away, and flip the tab dot to blocked.
                    self.maybe_send_attention_notification(
                        NotificationsTrigger::NeedsAttention,
                        format!(
                            "{} is asking you a question",
                            provider_copy(self.provider).needs_attention_prefix
                        ),
                        ctx,
                    );
                    ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
                    ctx.notify();
                    return;
                }
                if let Some(tx) = &self.message_tx {
                    let _ = tx.try_send(StdinCommand::Control {
                        request_id: id.clone(),
                        decision: Decision::allow_once(input.clone()),
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
        // 7p: a regular permission request (the Allow/Deny card) also parks
        // the turn on the user — notify if they're away. The tab flips to
        // blocked once the card lands in the transcript below. The held
        // AskUserQuestion path notified above and returned.
        if let TranscriptEvent::PermissionRequest { tool, .. } = &event {
            self.maybe_send_attention_notification(
                NotificationsTrigger::NeedsAttention,
                format!(
                    "{} needs permission to use {tool}",
                    provider_copy(self.provider).needs_attention_prefix
                ),
                ctx,
            );
            ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
        }
        // A background agent retiring can be what finally flips the tab from
        // the working spinner to the turn's ✓/✗ — repaint the tab bar.
        if matches!(&event, TranscriptEvent::TaskNotification(_)) {
            ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
        }
        if let TranscriptEvent::Ended { reason } = &event {
            log::warn!(
                "QDIAG turn Ended (streaming->false), had_pending_questions={}",
                !self.pending_question_permission.is_empty(),
            );
            self.streaming = false;
            // A turn that ended (e.g. Stop) with a question still parked can no
            // longer have that permission answered — `claude` has released it.
            // Drop the held requests; the idle card stays answerable as a
            // next-turn fallback (`submit_question_answers`).
            self.pending_question_permission.clear();
            // 7p: mark the tab with the turn's outcome and notify an
            // away user. Interrupted/Exited stay quiet — the user caused
            // the former, and the latter is either a mode-change restart
            // or follows an Error that already reported.
            // Whatever completion was still held for a previous turn is
            // superseded by this turn's outcome.
            self.deferred_completion = None;
            match reason {
                claude_code::EndReason::Completed => {
                    let body = self.last_assistant_text().unwrap_or_default();
                    if self.has_active_background_work() {
                        // Background scripts / sub-agents are still running:
                        // hold the ✓ and the notification until the last one
                        // retires (`maybe_fire_deferred_completion`).
                        self.deferred_completion = Some(body);
                    } else {
                        self.tab_attention = Some(true);
                        self.tab_attention_seen = false;
                        self.maybe_send_attention_notification(
                            NotificationsTrigger::AgentTaskCompleted(true),
                            body,
                            ctx,
                        );
                    }
                    // twarp 17 (PRODUCT 17 §13/§18, §32): speak whatever the
                    // live pump hasn't yet — for a turn that streamed with the
                    // toggle on this is just the trailing partial sentence; with
                    // the toggle flipped on late it's the whole reply. Only this
                    // branch — interrupted turns stay silent.
                    self.pump_live_tts(true, ctx);
                }
                claude_code::EndReason::Error(message) => {
                    self.tab_attention = Some(false);
                    self.tab_attention_seen = false;
                    let message = message.clone();
                    self.maybe_send_attention_notification(
                        NotificationsTrigger::AgentTaskCompleted(false),
                        message,
                        ctx,
                    );
                }
                claude_code::EndReason::Interrupted | claude_code::EndReason::Exited => {}
            }
            ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
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
        let completed_end = matches!(
            &event,
            TranscriptEvent::Ended {
                reason: claude_code::EndReason::Completed
            }
        );
        let session_initialized = matches!(&event, TranscriptEvent::SessionInit { .. });
        let ended = matches!(event, TranscriptEvent::Ended { .. });
        let assistant_text_grew = matches!(
            &event,
            TranscriptEvent::AssistantTextDelta { .. } | TranscriptEvent::AssistantTextDone
        );
        self.ingest_event(event, ctx);
        if session_initialized {
            if self.provider == AgentProvider::Codex {
                ctx.emit(ClaudeCodeViewEvent::Pane(PaneEvent::AppStateChanged));
            }
            self.send_pending_initial_turn();
            if self.raw_cli_pending && !self.streaming {
                self.raw_cli_pending = false;
                self.open_raw_cli(ctx);
            }
        }
        // twarp 17 §32: sentence-by-sentence readout keeps pace with the
        // stream. Cheap when the toggle is off (single bool check).
        if assistant_text_grew {
            self.pump_live_tts(false, ctx);
        }
        if turn_completed {
            self.drain_message_queue(ctx);
        }
        if ended {
            // #11: the turn may have edited files, committed, or pushed —
            // refresh the diff / branch / PR / CI bar.
            self.refresh_repo_context(ctx);
        }
        if completed_end {
            let reply_suggestion_takes_precedence =
                *AgentSettings::as_ref(ctx).enable_reply_suggestions.value()
                    && ReplySuggestionContext::from_transcript(&self.transcript).is_some();
            self.maybe_request_reply_suggestion(ctx);
            if !reply_suggestion_takes_precedence {
                self.maybe_request_composer_placeholder_suggestion(ctx);
            }
        }
        // PRODUCT §14: follow streaming output to the bottom as it arrives —
        // but only while the user is still pinned to the bottom. If they've
        // scrolled up to read earlier output mid-turn, leave their position
        // untouched so scrolling stays smooth instead of yanking them back down.
        if self.scroll_state.is_at_bottom(AUTOSCROLL_STICK_SLACK) {
            self.scroll_to_bottom();
        }
        // Runs after `ingest_event` so the agent/script memos see the event
        // that may have just retired the last piece of background work.
        self.maybe_fire_deferred_completion(ctx);
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
        // 7p: the working spinner in the tab must stop with the stream.
        ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
        self.interrupt_pending = false;
        self.child = None;
        self.message_tx = None;
        self.pending_initial_turn = None;
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

    /// Kill the live provider process (if any) and keep the conversation as
    /// the resume target for the next spawn. The epoch bump makes the killed
    /// session's EOF/events stale so they can't spam the transcript or wipe
    /// the next session's handles.
    fn detach_live_session(&mut self) {
        self.resume_session_id = Some(self.session_id.clone());
        self.session_epoch += 1;
        self.child = None; // kill_on_drop
        self.message_tx = None;
        self.pending_initial_turn = None;
        self.streaming = false;
        self.interrupt_pending = false;
    }

    /// Flip between the rendered chat and the provider's raw interactive CLI.
    /// Codex needs its app-server thread id before `codex resume` can launch,
    /// so a fresh pane initializes the thread first and completes the switch
    /// when `SessionInit` arrives.
    fn toggle_raw_cli(&mut self, ctx: &mut ViewContext<Self>) {
        if !supports_raw_cli(self.provider) {
            return;
        }
        if self.raw_cli.is_some() {
            self.exit_raw_mode(ctx);
            return;
        }
        if self.streaming {
            return;
        }
        if self.provider == AgentProvider::Codex && !self.has_provider_session() {
            self.raw_cli_pending = true;
            if !self.codex_restore_pending && !self.session_spawn_pending && self.child.is_none() {
                self.begin_session(None, ctx);
            }
            ctx.notify();
            return;
        }
        self.open_raw_cli(ctx);
    }

    fn open_raw_cli(&mut self, ctx: &mut ViewContext<Self>) {
        // twarp 17 (PRODUCT 17 §9): switching to Raw CLI cancels a live
        // recording silently and stops any spoken reply.
        self.cancel_voice_recording(ctx);
        if let Some(player) = &self.voice_player {
            player.stop();
        }
        self.detach_live_session();
        ctx.emit(ClaudeCodeViewEvent::SwapToRawCli {
            provider: self.provider,
            session_id: self.session_id.clone(),
            cwd: self.cwd.clone(),
            binary: self.resolve_provider_binary(),
            flags: build_raw_cli_flags(
                self.provider,
                self.model.as_deref(),
                self.effort.as_deref(),
                self.permission_mode,
            ),
        });
        ctx.notify();
    }

    /// Resolve the provider executable against the captured login-shell PATH.
    /// An absolute path also prevents the terminal's bare-agent trigger from
    /// intercepting the command and opening a second rich pane.
    fn resolve_provider_binary(&self) -> String {
        let binary = self.provider_binary();
        if let Some(path) = &self.interactive_path {
            if let Some(resolved) = resolve_executable_in_path(binary, std::ffi::OsStr::new(path)) {
                return resolved.display().to_string();
            }
        }
        resolve_executable(binary)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| binary.to_owned())
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

    /// Leave raw mode, reload the provider-owned conversation, and hand focus
    /// back to the composer. Claude reloads its JSONL directly; Codex resumes
    /// app-server so its stable API replays the thread history.
    fn exit_raw_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(raw_cli) = self.raw_cli.take() else {
            return;
        };
        ctx.unsubscribe_to_view(&raw_cli.view);
        drop(raw_cli);
        self.resume_session_id = Some(self.session_id.clone());
        match self.provider {
            AgentProvider::Claude => self.refresh_from_disk(ctx),
            AgentProvider::Codex => {
                self.clear_transcript_for_reload();
                self.begin_session(None, ctx);
            }
        }
        ctx.focus(&self.input_editor);
        ctx.notify();
    }

    fn clear_transcript_for_reload(&mut self) {
        self.transcript.clear();
        self.timeline_turn_mouse.borrow_mut().clear();
        self.tool_card_ui.clear();
        self.expanded_completed_turns.clear();
        self.completed_turn_mouse.borrow_mut().clear();
        self.artifact_file_mouse.borrow_mut().clear();
        self.diff_cards.clear();
        self.thinking_ui.clear();
        self.pending_question_permission.clear();
        self.question_answer_items.clear();
        self.streaming = false;
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
        self.clear_transcript_for_reload();
        for event in history {
            self.ingest_event(event, ctx);
        }
        self.resume_session_id = Some(self.session_id.clone());
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
            position_id: self.transcript_bottom_position_id(),
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    fn transcript_top_position_id(&self) -> String {
        format!("agent_transcript_top:{}", self.timeline_position_key)
    }

    fn transcript_bottom_position_id(&self) -> String {
        format!("agent_transcript_bottom:{}", self.timeline_position_key)
    }

    fn transcript_viewport_position_id(&self) -> String {
        format!("agent_transcript_viewport:{}", self.timeline_position_key)
    }

    fn turn_position_id(&self, turn_start: usize) -> String {
        format!(
            "agent_transcript_turn:{}:{turn_start}",
            self.timeline_position_key
        )
    }

    /// Align a selected turn with the top of the viewport. Saved turn bounds
    /// and the top sentinel share the same scroll translation, so subtracting
    /// them yields an exact document offset even when the pane is scrolled.
    /// Fall back to the scrollable's built-in position targeting during the
    /// first frame, before saved geometry exists.
    fn jump_to_turn(&self, turn_start: usize, ctx: &ViewContext<Self>) {
        let turn_position_id = self.turn_position_id(turn_start);
        let top_position_id = self.transcript_top_position_id();
        match (
            ctx.element_position_by_id(&turn_position_id),
            ctx.element_position_by_id(&top_position_id),
        ) {
            (Some(turn), Some(top)) => {
                let target = (turn.min_y() - top.min_y())
                    .max(0.0)
                    .min(self.scroll_state.max_scroll().as_f32());
                self.scroll_state.scroll_to(target.into_pixels());
            }
            _ => self.scroll_state.scroll_to_position(ScrollTarget {
                position_id: turn_position_id,
                mode: ScrollToPositionMode::TopIntoView,
            }),
        }
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
        let title = self.derived_tab_title();
        self.pane_configuration
            .update(ctx, |config, ctx| config.set_title(title, ctx));
    }

    /// The tab title derived from the conversation — the first user message,
    /// or the generic pane title before one exists. Also the desktop
    /// notification's title (7p), so the notification names the tab to look at.
    fn derived_tab_title(&self) -> String {
        self.transcript
            .items()
            .iter()
            .find_map(|item| match item {
                TranscriptItem::User(text) => Some(pane_tab_title(text)),
                _ => None,
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| provider_copy(self.provider).pane_title.to_owned())
    }

    /// The session state the tab indicator shows (7p), reusing the agent
    /// indicator's status set: blocked-on-the-user outranks working, and an
    /// idle pane shows the last turn's outcome until the user comes back to
    /// it. `None` renders no indicator (a fresh or revisited idle pane).
    pub fn tab_status(&self) -> Option<ConversationStatus> {
        if self.streaming
            && (!self.pending_question_permission.is_empty()
                || self.transcript.has_pending_prompt())
        {
            return Some(ConversationStatus::Blocked {});
        }
        if self.streaming {
            return Some(ConversationStatus::InProgress);
        }
        // A turn can end while background sub-agents are still working — the
        // session stays live between turns and delivers their terminal state
        // as task notifications. Until the last one retires, the tab keeps
        // the working spinner rather than declaring the turn complete. Gated
        // on a live process: once `claude` is gone no notification can ever
        // arrive, so a Running agent in a dead/restored transcript must not
        // pin the spinner forever.
        if self.has_active_background_work() {
            return Some(ConversationStatus::InProgress);
        }
        self.tab_attention.map(|succeeded| {
            if succeeded {
                if self.tab_attention_seen {
                    ConversationStatus::Success
                } else {
                    ConversationStatus::SuccessUnseen
                }
            } else {
                ConversationStatus::Error
            }
        })
    }

    /// Whether background scripts or sub-agents launched by this chat are still
    /// running. Gated on a live process: once `claude` is gone no task
    /// notification can ever arrive, so a Running entry in a dead/restored
    /// transcript must not count as in-flight work forever.
    fn has_active_background_work(&self) -> bool {
        self.child.is_some()
            && (self
                .agent_runs()
                .iter()
                .any(|agent| agent.state.is_active())
                || self
                    .background_scripts()
                    .iter()
                    .any(|script| script.state.is_active()))
    }

    /// A turn completed while background work was still running; once the last
    /// script/agent retires (and no new turn has started), deliver the held ✓
    /// and desktop notification. Called after every ingested transcript event.
    fn maybe_fire_deferred_completion(&mut self, ctx: &mut ViewContext<Self>) {
        if self.deferred_completion.is_none() || self.streaming || self.has_active_background_work()
        {
            return;
        }
        let body = self.deferred_completion.take().unwrap_or_default();
        self.tab_attention = Some(true);
        self.tab_attention_seen = false;
        self.maybe_send_attention_notification(
            NotificationsTrigger::AgentTaskCompleted(true),
            body,
            ctx,
        );
        ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
    }

    /// The user has seen the pane (7p): a failure's ✗ is dropped, and a
    /// completed turn's blue ✓ turns into the reviewed green ✓ (it stays until
    /// the next turn overwrites it).
    fn clear_tab_attention(&mut self, ctx: &mut ViewContext<Self>) {
        match self.tab_attention {
            Some(true) if !self.tab_attention_seen => {
                self.tab_attention_seen = true;
                ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
                ctx.notify();
            }
            Some(false) => {
                self.tab_attention = None;
                ctx.emit(ClaudeCodeViewEvent::TabStatusChanged);
                ctx.notify();
            }
            _ => {}
        }
    }

    /// The tail of the assistant's latest prose, as the completion
    /// notification's body (7p) — the end of the reply is what the user
    /// missed (`create_notification_content` truncates from the front).
    fn last_assistant_text(&self) -> Option<String> {
        self.transcript
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Assistant { text, .. } if !text.trim().is_empty() => {
                    Some(text.clone())
                }
                _ => None,
            })
    }

    /// Same gate as the terminal's `is_navigated_away_from_window` (7p):
    /// desktop notifications only fire while this pane's window isn't the
    /// active one — if the user is in the app, the pane itself is the signal.
    fn is_navigated_away_from_window(&self, ctx: &mut ViewContext<Self>) -> bool {
        Some(ctx.window_id()) != ctx.windows().active_window()
    }

    /// Fire a desktop notification titled with the tab's display title (7p),
    /// through the same workspace handler as the terminal's command-completion
    /// notifications (sound setting, permission-failure banner). Mirrors
    /// `send_agent_desktop_notification_or_show_banner`'s setting gates, minus
    /// the discovery banner — that's a terminal-view inline banner the chat
    /// pane has no host for, so `NotificationsMode::Unset` is simply a no-op
    /// here (the terminal flow is where the setting gets discovered).
    fn maybe_send_attention_notification(
        &mut self,
        trigger: NotificationsTrigger,
        body: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let session_settings = SessionSettings::as_ref(ctx);
        if !session_settings
            .notifications
            .is_supported_on_current_platform()
        {
            return;
        }
        let settings = session_settings.notifications.value().clone();
        if !matches!(settings.mode, NotificationsMode::Enabled) {
            return;
        }
        let enabled = match trigger {
            NotificationsTrigger::AgentTaskCompleted(_) => settings.is_agent_task_completed_enabled,
            _ => settings.is_needs_attention_enabled,
        };
        if !enabled || !self.is_navigated_away_from_window(ctx) {
            return;
        }
        // The pane group bakes the title from the tab's display title (custom
        // rename included), falling back to our derived title.
        ctx.emit(ClaudeCodeViewEvent::Pane(PaneEvent::SendChatNotification {
            trigger,
            body,
            fallback_title: self.derived_tab_title(),
        }));
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

    // ---- twarp 17: voice chat (mic → transcript, spoken replies) ----------

    /// Read a voice API key from the OS keychain (the `api_key_for_agent`
    /// pattern; keys never live in settings — PRODUCT 17 §24).
    fn voice_api_key(&self, storage_key: &str, ctx: &AppContext) -> Option<String> {
        secure_storage::Model::handle(ctx)
            .as_ref(ctx)
            .read_value(storage_key)
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
    }

    /// The mic button (PRODUCT 17 §2–§5, §10): idle → start recording,
    /// recording → stop + transcribe, transcribing → no-op.
    fn toggle_voice_recording(&mut self, ctx: &mut ViewContext<Self>) {
        if self.voice_transcribing {
            return; // §5: a second click during transcription is a no-op.
        }
        if self.voice_recorder.is_some() {
            self.finish_voice_recording(ctx);
            return;
        }

        self.voice_status = None;
        // §2: unconfigured → point at the settings page, don't record.
        if AgentSettings::as_ref(ctx).voice_stt_config().is_none() {
            self.voice_status = Some("Configure voice in Settings → Agent".to_owned());
            ctx.notify();
            return;
        }
        // §3: denied → System Settings pointer. NotDetermined proceeds — the
        // first stream build triggers the TCC prompt.
        match ctx.microphone_access_state() {
            MicrophoneAccessState::Denied | MicrophoneAccessState::Restricted => {
                self.voice_status = Some(
                    "Microphone access is off — enable twarp in System Settings → Privacy & Security → Microphone".to_owned(),
                );
                ctx.notify();
                return;
            }
            MicrophoneAccessState::NotDetermined | MicrophoneAccessState::Authorized => {}
        }
        // §10: one recording across the app.
        if !crate::voice::try_begin_recording() {
            self.voice_status = Some("Already recording in another pane".to_owned());
            ctx.notify();
            return;
        }
        match crate::voice::capture::Recorder::start() {
            Ok(recorder) => {
                self.voice_composer_was_empty = self
                    .input_editor
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().is_empty());
                self.voice_recorder = Some(recorder);
                // §30: live transcription owns an (initially empty) composer
                // suffix for this recording. Bump the generation so results
                // from a previous recording can't land in this one.
                self.voice_generation = self.voice_generation.wrapping_add(1);
                self.voice_live_text.clear();
                self.voice_live_owned = true;
                self.voice_live_inflight = false;
                self.voice_live_last = None;
                self.voice_live_snapshot = None;
                self.schedule_voice_tick(ctx);
            }
            Err(error) => {
                crate::voice::end_recording();
                self.voice_status = Some(error.to_string());
            }
        }
        ctx.notify();
    }

    /// §6: discard the recording without a request (Esc, pane teardown, Raw
    /// CLI switch).
    fn cancel_voice_recording(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(recorder) = self.voice_recorder.take() {
            recorder.cancel();
            crate::voice::end_recording();
            // §6/§31: a cancel discards the live transcript too — remove the
            // live suffix, but only if the user hasn't edited over it.
            if self.voice_live_owned && !self.voice_live_text.is_empty() {
                let live = std::mem::take(&mut self.voice_live_text);
                self.input_editor.update(ctx, |editor, ctx| {
                    if editor.buffer_text(ctx).ends_with(&live) {
                        editor.replace_last_n_characters(
                            string_offset::CharOffset::from(live.chars().count()),
                            "",
                            ctx,
                        );
                    }
                });
            }
            self.voice_live_owned = false;
            ctx.notify();
        }
    }

    /// §5/§7–§8: stop capturing, upload, and land the transcript in the
    /// composer (or surface the error, leaving the draft untouched).
    fn finish_voice_recording(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(recorder) = self.voice_recorder.take() else {
            return;
        };
        crate::voice::end_recording();
        let wav = match recorder.stop() {
            Ok(wav) => wav,
            Err(error) => {
                self.voice_status = Some(error.to_string());
                ctx.notify();
                return;
            }
        };
        let Some(mut config) = AgentSettings::as_ref(ctx).voice_stt_config() else {
            self.voice_status = Some("Configure voice in Settings → Agent".to_owned());
            ctx.notify();
            return;
        };
        match self.voice_api_key(app_settings::VOICE_STT_API_KEY_STORAGE_KEY, ctx) {
            Some(key) => config.api_key = key,
            None => {
                self.voice_status = Some("Configure voice in Settings → Agent".to_owned());
                ctx.notify();
                return;
            }
        }

        self.voice_transcribing = true;
        self.voice_generation = self.voice_generation.wrapping_add(1);
        let generation = self.voice_generation;
        ctx.spawn(
            crate::voice::stt::transcribe(config, wav),
            move |view, result, ctx| {
                if view.voice_generation != generation {
                    return;
                }
                view.voice_transcribing = false;
                match result {
                    Ok(text) => view.insert_voice_transcript(text, ctx),
                    Err(error) => view.voice_status = Some(error.to_string()),
                }
                ctx.notify();
            },
        );
        ctx.notify();
    }

    /// §30: while recording, periodically transcribe everything captured so
    /// far and mirror it into the composer. The whole buffer is re-transcribed
    /// each pass (no streaming API needed) so earlier words self-correct as
    /// context accumulates; one request in flight at a time.
    fn pump_live_transcription(&mut self, ctx: &mut ViewContext<Self>) {
        if self.voice_live_inflight || !self.voice_live_owned {
            return;
        }
        // A snapshot encode is pending on the capture thread: poll it, and
        // when the WAV lands, start the upload.
        if let Some(pending) = &self.voice_live_snapshot {
            match pending.try_recv() {
                Ok(Ok(wav)) => {
                    self.voice_live_snapshot = None;
                    self.spawn_live_transcription(wav, ctx);
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Capture trouble surfaces via the stop path; stay quiet.
                    self.voice_live_snapshot = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            return;
        }
        let Some(recorder) = &self.voice_recorder else {
            return;
        };
        // Wait for a beat of audio before the first pass, then hold the
        // cadence; stretch it as the recording (and thus the upload) grows.
        let elapsed = recorder.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let interval = LIVE_STT_INTERVAL.max(elapsed / 20);
        if self
            .voice_live_last
            .is_some_and(|last| last.elapsed() < interval)
        {
            return;
        }
        // Config is checked again (with the key) when the snapshot lands.
        if AgentSettings::as_ref(ctx).voice_stt_config().is_none() {
            return;
        }
        self.voice_live_last = Some(Instant::now());
        self.voice_live_snapshot = recorder.request_snapshot();
    }

    /// Second half of §30: upload a finished snapshot for transcription.
    fn spawn_live_transcription(&mut self, wav: Vec<u8>, ctx: &mut ViewContext<Self>) {
        let Some(mut config) = AgentSettings::as_ref(ctx).voice_stt_config() else {
            return;
        };
        let Some(key) = self.voice_api_key(app_settings::VOICE_STT_API_KEY_STORAGE_KEY, ctx) else {
            return;
        };
        config.api_key = key;
        self.voice_live_inflight = true;
        let generation = self.voice_generation;
        ctx.spawn(
            crate::voice::stt::transcribe(config, wav),
            move |view, result, ctx| {
                view.voice_live_inflight = false;
                // Only apply while THIS recording is still running — the final
                // stop-transcription supersedes any in-flight live pass.
                if view.voice_generation != generation || view.voice_recorder.is_none() {
                    return;
                }
                if let Ok(text) = result {
                    view.apply_live_transcript(&text, ctx);
                }
                // Errors stay quiet mid-recording (§30): transient API trouble
                // shouldn't interrupt the take; the stop path reports for real.
            },
        );
    }

    /// §31: land a live transcript in the composer by replacing the previous
    /// live suffix. If the user edited that suffix, live updates stand down
    /// for the rest of the recording (their edit wins).
    fn apply_live_transcript(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let mut new_suffix = None;
        let live = self.voice_live_text.clone();
        self.input_editor.update(ctx, |editor, ctx| {
            let existing = editor.buffer_text(ctx);
            if !existing.ends_with(&live) {
                return;
            }
            let base = &existing[..existing.len() - live.len()];
            let suffix = if base.trim().is_empty() || base.ends_with(char::is_whitespace) {
                text.to_owned()
            } else {
                format!(" {text}")
            };
            editor.replace_last_n_characters(
                string_offset::CharOffset::from(live.chars().count()),
                &suffix,
                ctx,
            );
            new_suffix = Some(suffix);
        });
        match new_suffix {
            Some(suffix) => {
                self.voice_live_text = suffix;
                ctx.notify();
            }
            None => self.voice_live_owned = false,
        }
    }

    /// §7: append the transcript to the draft (separating space when needed)
    /// and keep focus in the composer; auto-send only when enabled and the
    /// composer was — and still is — empty.
    fn insert_voice_transcript(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        // §31: the final transcript replaces the live suffix (it's the same
        // audio, transcribed once more with the complete recording). If the
        // user edited the live text, their edit wins and the final transcript
        // is dropped.
        let live = std::mem::take(&mut self.voice_live_text);
        self.voice_live_owned = false;
        let mut was_empty = false;
        let mut inserted = true;
        self.input_editor.update(ctx, |editor, ctx| {
            let existing = editor.buffer_text(ctx);
            if !existing.ends_with(&live) {
                inserted = false;
                return;
            }
            let base = &existing[..existing.len() - live.len()];
            was_empty = base.trim().is_empty();
            let suffix = if was_empty || base.ends_with(char::is_whitespace) {
                text.clone()
            } else {
                format!(" {text}")
            };
            editor.replace_last_n_characters(
                string_offset::CharOffset::from(live.chars().count()),
                &suffix,
                ctx,
            );
        });
        if !inserted {
            return;
        }
        self.refresh_composer_intelligence(ctx);
        let auto_send = *AgentSettings::as_ref(ctx).voice_auto_send.value();
        if auto_send && self.voice_composer_was_empty && was_empty {
            self.submit(ctx);
        }
    }

    /// The speaker button (PRODUCT 17 §12, §14): off → on (when configured);
    /// on → stop playback and off.
    fn toggle_speak_replies(&mut self, ctx: &mut ViewContext<Self>) {
        self.voice_status = None;
        if self.speak_replies {
            self.speak_replies = false;
            self.voice_tts_generation = self.voice_tts_generation.wrapping_add(1);
            self.voice_tts_item = None;
            self.voice_tts_spoken_prose.clear();
            self.voice_tts_pending.clear();
            self.voice_tts_inflight = false;
            self.voice_karaoke_chunks.clear();
            if let Some(player) = &self.voice_player {
                player.stop();
            }
        } else if AgentSettings::as_ref(ctx).voice_tts_config().is_none() {
            // §12: unconfigured → same settings pointer as the mic.
            self.voice_status = Some("Configure voice in Settings → Agent".to_owned());
        } else {
            self.speak_replies = true;
        }
        ctx.notify();
    }

    /// §32: speak the trailing assistant reply as it streams — each newly
    /// completed sentence is queued for synthesis the moment it lands; `flush`
    /// (turn end) speaks whatever remains, including an unterminated tail.
    /// Also the §13 whole-reply path: with nothing spoken yet, a flush covers
    /// the full text.
    fn pump_live_tts(&mut self, flush: bool, ctx: &mut ViewContext<Self>) {
        if !self.speak_replies {
            return;
        }
        let Some((item_index, text)) =
            self.transcript
                .items()
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, item)| match item {
                    TranscriptItem::Assistant { text, .. } if !text.trim().is_empty() => {
                        Some((index, text.clone()))
                    }
                    _ => None,
                })
        else {
            return;
        };
        if self.voice_tts_item != Some(item_index) {
            // §15: a new reply replaces the previous utterance.
            self.voice_tts_item = Some(item_index);
            self.voice_tts_spoken_prose.clear();
            self.voice_tts_pending.clear();
            self.voice_tts_first_chunk = true;
            self.voice_tts_inflight = false;
            self.voice_karaoke_chunks.clear();
            self.voice_tts_generation = self.voice_tts_generation.wrapping_add(1);
        }
        // While streaming, only complete lines feed the prose conversion — a
        // full line's markdown→prose mapping never changes as more text
        // arrives (fence state included), so what we've spoken stays a stable
        // prefix of every later pass.
        let markdown = if flush {
            text.as_str()
        } else {
            &text[..text.rfind('\n').map(|i| i + 1).unwrap_or(0)]
        };
        let prose = crate::voice::prose::markdown_to_prose(markdown);
        let spoken = &self.voice_tts_spoken_prose;
        let fresh = match prose.strip_prefix(spoken.as_str()) {
            Some(fresh) => fresh,
            // Defensive: if the prefix drifted (it shouldn't), fall back to a
            // length cut (floored to a char boundary) rather than re-speaking
            // from the top.
            None => {
                let mut at = spoken.len().min(prose.len());
                while !prose.is_char_boundary(at) {
                    at -= 1;
                }
                &prose[at..]
            }
        };
        let cut = if flush {
            fresh.len()
        } else {
            crate::voice::tts::complete_sentence_prefix_len(fresh)
        };
        if cut == 0 {
            return;
        }
        let speakable = &fresh[..cut];
        self.voice_tts_spoken_prose.push_str(speakable);
        for chunk in crate::voice::tts::chunk_text(speakable, crate::voice::tts::TTS_INPUT_CAP) {
            self.voice_tts_pending.push_back(chunk);
        }
        self.spawn_next_live_tts(ctx);
    }

    /// Synthesize the next pending sentence and queue its audio, then recurse
    /// — sequential so chunks arrive in order (§16); a failed chunk surfaces
    /// once and drops the rest of the queue (§17).
    fn spawn_next_live_tts(&mut self, ctx: &mut ViewContext<Self>) {
        if self.voice_tts_inflight {
            return;
        }
        let Some(text) = self.voice_tts_pending.pop_front() else {
            return;
        };
        let Some(mut config) = AgentSettings::as_ref(ctx).voice_tts_config() else {
            // Keys were removed while the toggle was on (§22).
            self.voice_status = Some("Configure voice in Settings → Agent".to_owned());
            ctx.notify();
            return;
        };
        let key_account = AgentSettings::as_ref(ctx).voice_tts_key_storage_key();
        match self.voice_api_key(key_account, ctx) {
            Some(key) => config.api_key = key,
            None => {
                self.voice_status = Some("Configure voice in Settings → Agent".to_owned());
                ctx.notify();
                return;
            }
        }
        if self.voice_player.is_none() {
            match crate::voice::playback::Player::start() {
                Ok(player) => self.voice_player = Some(player),
                Err(error) => {
                    self.voice_status = Some(error.to_string());
                    ctx.notify();
                    return;
                }
            }
        }

        self.voice_tts_inflight = true;
        let generation = self.voice_tts_generation;
        let request_text = text.clone();
        ctx.spawn(
            async move { crate::voice::tts::synthesize(&config, &request_text).await },
            move |view, result, ctx| {
                view.voice_tts_inflight = false;
                // A newer utterance (§15) or the toggle flipping off (§14)
                // obsoletes this chunk.
                if view.voice_tts_generation != generation || !view.speak_replies {
                    return;
                }
                match result {
                    Ok(pcm) => {
                        let secs =
                            (pcm.len() / 2) as f32 / crate::voice::tts::TTS_SAMPLE_RATE as f32;
                        if let Some(player) = &view.voice_player {
                            if view.voice_tts_first_chunk {
                                view.voice_tts_first_chunk = false;
                                view.voice_karaoke_chunks.clear();
                                player.play(pcm, crate::voice::tts::TTS_SAMPLE_RATE);
                            } else {
                                player.append(pcm, crate::voice::tts::TTS_SAMPLE_RATE);
                            }
                        }
                        // §33: log the chunk's audio span for the karaoke clock.
                        let start = view
                            .voice_karaoke_chunks
                            .last()
                            .map(|(_, start, len)| start + len)
                            .unwrap_or(0.0);
                        view.voice_karaoke_chunks.push((text, start, secs));
                        view.spawn_next_live_tts(ctx);
                        view.schedule_voice_tick(ctx);
                        ctx.notify();
                    }
                    Err(error) => {
                        view.voice_status = Some(error.to_string());
                        view.voice_tts_pending.clear();
                        ctx.notify();
                    }
                }
            },
        );
    }

    /// §33: the sentence being spoken right now and how far through it the
    /// audio is (linear chars-over-time estimate — the TTS API returns no word
    /// timings). `None` when idle, so the highlight vanishes when speech ends.
    fn active_karaoke(&self, app: &AppContext) -> Option<KaraokeHighlight> {
        if !self.speak_replies {
            return None;
        }
        let player = self.voice_player.as_ref()?;
        if !player.is_active() {
            return None;
        }
        let item_index = self.voice_tts_item?;
        let pos = player.position_secs();
        let (chunk, start, len) = self
            .voice_karaoke_chunks
            .iter()
            .find(|(_, start, len)| pos >= *start && pos < start + len)?;
        let frac = ((pos - start) / len.max(0.001)).clamp(0.0, 1.0);
        let chunk_chars: Vec<char> = chunk.chars().collect();
        let char_pos = ((chunk_chars.len() as f32 * frac) as usize).min(chunk_chars.len());
        // A chunk can pack several sentences (§16); karaoke lights the one the
        // audio is inside, with a proportional fill.
        let mut sentence_start = 0;
        let mut sentence_end = chunk_chars.len();
        let mut consumed = 0;
        for piece in crate::voice::tts::split_sentences(chunk) {
            let piece_chars = piece.chars().count();
            if char_pos < consumed + piece_chars || consumed + piece_chars >= chunk_chars.len() {
                sentence_start = consumed;
                sentence_end = consumed + piece_chars;
                break;
            }
            consumed += piece_chars;
        }
        let sentence: String = chunk_chars[sentence_start..sentence_end].iter().collect();
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            return None;
        }
        let leading_ws = sentence.chars().take_while(|c| c.is_whitespace()).count();
        let spoken_chars = char_pos
            .saturating_sub(sentence_start + leading_ws)
            .min(trimmed.chars().count());
        let accent = self.accent(app);
        Some(KaraokeHighlight {
            item_index,
            sentence: trimmed.to_owned(),
            spoken_chars,
            sentence_color: ColorU::new(accent.r, accent.g, accent.b, 36),
            spoken_color: ColorU::new(accent.r, accent.g, accent.b, 96),
        })
    }

    /// The voice beat (the `schedule_elapsed_tick` pattern): while recording
    /// it repaints the elapsed label and enforces the §9 cap / §11 device-loss
    /// stop; while speaking it repaints so the speaker button's active state
    /// clears when playback drains. Dies on its own once both are idle.
    fn schedule_voice_tick(&self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(Duration::from_millis(120)).await;
            },
            move |me, _, ctx| {
                let mut rearm = false;
                if let Some(recorder) = &me.voice_recorder {
                    if recorder.hit_cap() || recorder.stream_ended() {
                        // §9/§11: stop-and-transcribe best effort.
                        me.finish_voice_recording(ctx);
                    } else {
                        rearm = true;
                    }
                }
                if me.voice_recorder.is_some() {
                    // §30: the live transcription cadence rides the same beat.
                    me.pump_live_transcription(ctx);
                }
                if let Some(player) = &me.voice_player {
                    if player.is_active() {
                        rearm = true;
                    }
                }
                ctx.notify();
                if rearm {
                    me.schedule_voice_tick(ctx);
                }
            },
        );
    }

    fn computer_control_entrypoint_available() -> bool {
        crate::computer_control::platform_supported() && FeatureFlag::LocalComputerUse.is_enabled()
    }

    fn computer_control_session_label(&self) -> String {
        let short_session = self.session_id.get(..8).unwrap_or(self.session_id.as_str());
        format!("Claude Code session {short_session}")
    }

    fn computer_control_chrome(&self, app: &AppContext) -> ComputerControlChrome {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let panel_color = crate::workspace::view::floating_panel_surface_fill(app).into_solid();
        let panel_fill = twarp_core::ui::theme::Fill::Solid(panel_color);

        ComputerControlChrome {
            panel_color,
            text_color: theme.main_text_color(panel_fill).into_solid(),
            muted_text_color: theme.sub_text_color(panel_fill).into_solid(),
            glow_color: self.accent(app),
        }
    }

    fn sync_computer_control_chrome(&self, app: &AppContext) {
        let mut computer_control = self.computer_control.borrow_mut();
        if !Self::computer_control_entrypoint_available() {
            computer_control.stop();
            return;
        }

        if computer_control.state().needs_poll() {
            computer_control.update_chrome(
                self.computer_control_session_label(),
                self.computer_control_chrome(app),
            );
        }
    }

    fn toggle_computer_control(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::computer_control_entrypoint_available() {
            self.computer_control.borrow_mut().stop();
            ctx.notify();
            return;
        }

        let should_stop = self.computer_control.borrow().state().is_live();
        if should_stop {
            self.computer_control.borrow_mut().stop();
            ctx.notify();
            return;
        }

        self.computer_control.borrow_mut().start(
            self.computer_control_session_label(),
            self.computer_control_chrome(ctx),
        );
        let generation = self.computer_control.borrow().generation();
        if self.computer_control.borrow().state().needs_poll() {
            self.schedule_computer_control_poll(generation, ctx);
        }
        ctx.notify();
    }

    fn schedule_computer_control_poll(&self, generation: u64, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(Duration::from_millis(250)).await;
            },
            move |me, _, ctx| {
                let generation_matches = {
                    let computer_control = me.computer_control.borrow();
                    computer_control.state().needs_poll()
                        && computer_control.generation() == generation
                };
                if !generation_matches {
                    return;
                }

                if me.computer_control.borrow_mut().poll_native_events() {
                    ctx.notify();
                }

                let next_generation = {
                    let computer_control = me.computer_control.borrow();
                    if computer_control.state().needs_poll() {
                        Some(computer_control.generation())
                    } else {
                        None
                    }
                };
                if let Some(next_generation) = next_generation {
                    me.schedule_computer_control_poll(next_generation, ctx);
                }
            },
        );
    }

    /// `compact` drops the text label (icon-only) for narrow panes — the
    /// state colours still read through the glyph.
    fn render_computer_control_button(
        &self,
        app: &AppContext,
        compact: bool,
    ) -> Option<Box<dyn Element>> {
        if !Self::computer_control_entrypoint_available() {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let state = self.computer_control.borrow().state().clone();
        let live = state.is_live();
        let blocked = matches!(state, ComputerControlState::Blocked(_));
        let failed = matches!(state, ComputerControlState::Failed(_));
        let accent = self.accent(app);
        let wash = self.accent_wash(app);
        let text_color = if live {
            accent
        } else if blocked || failed {
            twarp_core::ui::theme::Fill::warn().into_solid()
        } else {
            // Idle chrome stays neutral gray; the accent is reserved for
            // "something is running right now".
            theme.sub_text_color(theme.background()).into_solid()
        };
        let label = if live {
            "Stop control"
        } else if blocked {
            "Grant permissions"
        } else if failed {
            "Retry control"
        } else {
            "Computer control"
        };
        let icon = if live {
            crate::ui_components::icons::Icon::StopFilled
        } else {
            crate::ui_components::icons::Icon::Eye
        };

        let glyph = ConstrainedBox::new(Icon::new(icon.into(), text_color).finish())
            .with_width(14.)
            .with_height(14.)
            .finish();
        let content = if compact {
            glyph
        } else {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(5.)
                .with_child(glyph)
                .with_child(context_segment(appearance, label.to_owned(), text_color))
                .finish()
        };

        let mouse = self.computer_control_button_mouse.clone();
        Some(
            Hoverable::new(mouse, move |state| {
                let mut body = Container::new(content)
                    .with_padding_left(8.)
                    .with_padding_right(8.)
                    .with_padding_top(4.)
                    .with_padding_bottom(4.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
                if live || state.is_hovered() {
                    body = body.with_background_color(wash);
                }
                body.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action::<PaneHeaderAction<(), ClaudeCodeCustomAction>>(
                    PaneHeaderAction::CustomAction(ClaudeCodeCustomAction::ToggleComputerControl),
                );
            })
            .finish(),
        )
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
                provider_copy(self.provider).assistant_icon,
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
        // history refresh all target the right session. Codex history is not
        // readable from disk — it is replayed by the app-server's
        // `thread/resume` response, so a Codex pane spawns eagerly below.
        if session.provider == AgentProvider::Claude {
            for event in sessions::load_history(&session.jsonl_path) {
                self.ingest_event(event, ctx);
            }
        }
        self.resume_session_id = Some(session.id.clone());
        self.session_id = session.id;
        // The panel is gone the moment the transcript has content.
        self.recent_sessions = Vec::new();
        self.recent_session_mouse.borrow_mut().clear();
        if session.provider == AgentProvider::Codex {
            self.begin_session(None, ctx);
        }

        // Open at the latest message, name the tab from the history, and refresh
        // Environment for the resumed conversation.
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
        // counts — except typed `AskUserQuestion` answers, which render as user
        // bubbles but are stored inside the tool call, not as user turns — so
        // the boundary the user sees and the file cut agree.
        let keep_user_turns = self.transcript.items()[..=index]
            .iter()
            .enumerate()
            .filter(|(i, item)| {
                matches!(item, TranscriptItem::User(_)) && !self.question_answer_items.contains(i)
            })
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
            provider: self.provider,
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
        // §33: only the item being spoken carries a karaoke highlight.
        let karaoke = self
            .active_karaoke(app)
            .filter(|karaoke| karaoke.item_index == index);
        let row = render_message_row(
            false,
            provider_copy(self.provider).assistant_icon,
            text,
            self.render_accent.get(),
            appearance,
            karaoke.as_ref(),
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
        // Hoverable suppresses LeftMouseDragged by default while clicked, which
        // starved the transcript's SelectableArea of drag events — the final
        // response of a turn (the only message wrapped in this Hoverable) could
        // never be drag-selected. Selection drags must pass through; hover and
        // the pill clicks are unaffected.
        .with_propagate_drag()
        .finish()
    }

    /// A settled turn leads with the final answer as plain document prose. The
    /// provider glyph belongs to the live chronological timeline; omitting it
    /// here gives the result the same quiet hierarchy as the Codex reference
    /// while preserving Fork and Copy on hover.
    fn render_final_assistant_response(&self, index: usize, app: &AppContext) -> Box<dyn Element> {
        let Some(TranscriptItem::Assistant { text, .. }) = self.transcript.items().get(index)
        else {
            return Container::new(Flex::column().finish()).finish();
        };
        let appearance = Appearance::as_ref(app);
        let text_color = appearance
            .theme()
            .main_text_color(appearance.theme().background())
            .into_solid();
        let response = Container::new(render_markdown_body(text, text_color, appearance, None))
            .with_horizontal_padding(spacing::LG)
            .with_vertical_padding(spacing::SM)
            .finish();

        let fork_visible = self.render_fork_affordance(index, true, app);
        let fork_hidden = self.render_fork_affordance(index, false, app);
        let row_mouse = pooled_mouse_state(&self.fork_row_mouse, index);
        Hoverable::new(row_mouse, move |state| {
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(response)
                .with_child(if state.is_hovered() {
                    fork_visible
                } else {
                    fork_hidden
                })
                .finish()
        })
        .with_propagate_drag()
        .finish()
    }

    /// The full prose of the reply ending at transcript `index`: every
    /// assistant text segment since the previous user turn, joined with blank
    /// lines. Tool cards, thinking blocks, and metrics between the segments are
    /// skipped — copying a response means copying what Claude *said*.
    fn reply_text(&self, index: usize) -> String {
        let items = self.transcript.items();
        if index >= items.len() {
            return String::new();
        }
        if project_turns(items, self.streaming)
            .iter()
            .any(|turn| turn.compact && turn.final_response == Some(index))
        {
            return match &items[index] {
                TranscriptItem::Assistant { text, .. } => text.trim().to_owned(),
                _ => String::new(),
            };
        }
        let start = items[..index]
            .iter()
            .rposition(|item| matches!(item, TranscriptItem::User(_)))
            .map_or(0, |pos| pos + 1);
        items[start..=index]
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Assistant { text, .. } => Some(text.trim()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
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

    /// twarp: the "⑂ Fork" + copy-response buttons shown under a response on
    /// hover. Aligned under
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
        let fork_mouse = pooled_mouse_state(&self.fork_button_mouse, index);
        let copy_mouse = pooled_mouse_state(&self.copy_button_mouse, index);

        let pill = |content: Box<dyn Element>| {
            Container::new(content)
                .with_padding_left(spacing::SM)
                .with_padding_right(spacing::SM)
                .with_padding_top(spacing::XS)
                .with_padding_bottom(spacing::XS)
                .with_background_color(pill_bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
                .finish()
        };
        let small_icon = |path: &'static str| {
            ConstrainedBox::new(Icon::new(path, label_color).finish())
                .with_width(12.)
                .with_height(12.)
                .finish()
        };

        let label = appearance
            .ui_builder()
            .span("Fork")
            .with_style(UiComponentStyles {
                font_color: Some(label_color),
                font_size: Some(type_ramp::LABEL.size),
                ..Default::default()
            })
            .build()
            .finish();
        let fork_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::XS)
            .with_child(small_icon(FORK_ICON_SVG_PATH))
            .with_child(label)
            .finish();
        let fork_pill = pill(fork_content);
        let copy_pill = pill(small_icon(COPY_ICON_SVG_PATH));

        let fork_button = Hoverable::new(fork_mouse, move |_| fork_pill);
        let copy_button = Hoverable::new(copy_mouse, move |_| copy_pill);
        // Only the painted state is interactive — the transparent placeholder
        // must not catch clicks or show the pointer cursor in the empty gap.
        let (fork_button, copy_button) = if visible {
            (
                fork_button
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::ForkConversation(index));
                    }),
                copy_button
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::CopyResponse(index));
                    }),
            )
        } else {
            (fork_button, copy_button)
        };
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.)
            .with_child(fork_button.finish())
            .with_child(copy_button.finish())
            .finish();
        // Indent under the prose (avatar gutter ≈ 14 padding + 16 icon + 12
        // margin) so it reads as belonging to the message above it.
        Container::new(row)
            .with_margin_left(spacing::LG + spacing::LG + spacing::MD)
            .with_margin_bottom(spacing::SM)
            .finish()
    }

    /// twarp: the "✎ Edit" button shown under a sent user message on hover.
    /// Right-aligned under the bubble to match its alignment. Same
    /// visible/transparent-placeholder mechanics as
    /// [`Self::render_fork_affordance`] — identical layout in both states so
    /// the hover bounds never change.
    fn render_edit_affordance(
        &self,
        index: usize,
        visible: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
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
        let edit_mouse = pooled_mouse_state(&self.edit_button_mouse, index);

        let icon = ConstrainedBox::new(Icon::new(EDIT_ICON_SVG_PATH, label_color).finish())
            .with_width(12.)
            .with_height(12.)
            .finish();
        let label = appearance
            .ui_builder()
            .span("Edit")
            .with_style(UiComponentStyles {
                font_color: Some(label_color),
                font_size: Some(type_ramp::LABEL.size),
                ..Default::default()
            })
            .build()
            .finish();
        let pill = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(spacing::XS)
                .with_child(icon)
                .with_child(label)
                .finish(),
        )
        .with_padding_left(spacing::SM)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::XS)
        .with_padding_bottom(spacing::XS)
        .with_background_color(pill_bg)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
        .finish();

        let button = Hoverable::new(edit_mouse, move |_| pill);
        // Only the painted state is interactive — the transparent placeholder
        // must not catch clicks or show the pointer cursor in the empty gap.
        let button = if visible {
            button
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ClaudeCodeViewAction::EditUserMessage(index));
                })
        } else {
            button
        };
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(button.finish())
                .finish(),
        )
        .with_margin_bottom(spacing::SM)
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
        let copy = provider_copy(self.provider);

        // Header: the provider glyph beside a "Welcome back" heading.
        let glyph = ConstrainedBox::new(Icon::new(copy.assistant_icon, accent).finish())
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
            .with_spacing(spacing::MD)
            .with_child(glyph)
            .with_child(heading)
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::LG)
            .with_child(header);

        if self.recent_sessions.is_empty() {
            // First-run: no stored sessions for this directory yet.
            let explanation = appearance
                .ui_builder()
                .span(copy.empty_state.to_owned())
                .with_soft_wrap()
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(type_ramp::PROSE.size),
                    ..Default::default()
                })
                .build()
                .finish();
            column.add_child(explanation);
        } else {
            // A "Sessions" section — the home for this directory's past
            // sessions (the sidebar list is gone): a title search bar above
            // the rows, then the filtered list, paged so a long history
            // doesn't run under the floating composer.
            let query = self.session_search.as_ref(app).buffer_text(app);
            let has_query = !query.trim().is_empty();
            let matched = sessions::filter_session_indices(&self.recent_sessions, &query);

            // The search bar: a bordered, fill-less single-line field with an
            // inline clear (X) button while a query is active. Mirrors the old
            // sidebar sessions search so the affordance carries over.
            let editor_cell =
                Shrinkable::new(1.0, ChildView::new(&self.session_search).finish()).finish();
            let mut search_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(spacing::SM)
                .with_child(editor_cell);
            if has_query {
                let main = theme.main_text_color(theme.background()).into_solid();
                let clear_button =
                    Hoverable::new(self.session_search_clear_mouse.clone(), move |state| {
                        let fill = if state.is_hovered() { main } else { muted };
                        ConstrainedBox::new(
                            Icon::new(crate::ui_components::icons::Icon::X.into(), fill).finish(),
                        )
                        .with_width(14.)
                        .with_height(14.)
                        .finish()
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::ClearSessionSearch);
                    })
                    .finish();
                search_row = search_row.with_child(clear_button);
            }
            let search_field = Container::new(search_row.finish())
                .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
                .with_padding_left(spacing::SM)
                .with_padding_right(spacing::SM)
                .with_padding_top(spacing::SM - spacing::XXS)
                .with_padding_bottom(spacing::SM - spacing::XXS)
                .finish();

            let section = appearance
                .ui_builder()
                .span("Sessions".to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(muted),
                    font_size: Some(type_ramp::CAPTION.size),
                    ..Default::default()
                })
                .build()
                .finish();

            let body: Box<dyn Element> = if has_query && matched.is_empty() {
                // Distinct no-match empty state — different copy from the
                // first-run branch so "your filter hid everything" is obvious.
                appearance
                    .ui_builder()
                    .span("No matching sessions".to_owned())
                    .with_style(UiComponentStyles {
                        font_color: Some(muted),
                        font_size: Some(type_ramp::PROSE.size),
                        ..Default::default()
                    })
                    .build()
                    .finish()
            } else {
                let shown = matched.len().min(self.sessions_shown);
                let mut rows = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(spacing::XXS);
                // Rows carry the ORIGINAL index into `recent_sessions` (the
                // filter returns original indices) so resume targets the right
                // session even when the list is a filtered subset.
                for &idx in matched.iter().take(shown) {
                    rows.add_child(self.render_recent_session_row(
                        idx,
                        &self.recent_sessions[idx],
                        app,
                    ));
                }
                if matched.len() > shown {
                    rows.add_child(self.render_load_more_row(matched.len() - shown, app));
                }
                rows.finish()
            };

            column.add_child(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(spacing::SM)
                    .with_child(section)
                    .with_child(search_field)
                    .with_child(body)
                    .finish(),
            );
        }

        Container::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(measure::PROSE_MAX_WIDTH)
                .finish(),
        )
        .with_padding_left(spacing::XL)
        .with_padding_right(spacing::XL)
        .with_padding_top(spacing::XXL)
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

    /// twarp: the "Load more" row at the bottom of the zero-state session list
    /// when more filtered matches exist than are shown — a quiet text-style
    /// affordance (muted label, accent-wash hover, like the session rows) that
    /// reveals the next page.
    fn render_load_more_row(&self, remaining: usize, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();

        let label = appearance
            .ui_builder()
            .span(format!("Load more ({remaining})"))
            .with_style(UiComponentStyles {
                font_color: Some(muted),
                font_size: Some(type_ramp::UI.size),
                ..Default::default()
            })
            .build()
            .finish();

        let highlight = self.accent_wash(app);
        Hoverable::new(self.sessions_load_more_mouse.clone(), move |state| {
            let mut body = Container::new(label)
                .with_padding_left(spacing::MD)
                .with_padding_right(spacing::MD)
                .with_padding_top(spacing::SM)
                .with_padding_bottom(spacing::SM)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)));
            if state.is_hovered() {
                body = body.with_background_color(highlight);
            }
            body.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ShowMoreRecentSessions);
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
        let scripts = std::rc::Rc::new(background_scripts::collect(
            self.transcript.items(),
            self.transcript.task_notifications(),
        ));
        *self.background_scripts_memo.borrow_mut() = Some((revision, scripts.clone()));
        scripts
    }

    /// twarp: the per-chat agents list, derived from the transcript via
    /// [`agents::collect`] and memoized on the transcript's revision — the
    /// [`Self::background_scripts`] twin. The ⋯ header menu (button badge and
    /// inline section) reads it each render.
    fn agent_runs(&self) -> std::rc::Rc<Vec<agents::AgentRun>> {
        let revision = self.transcript.revision();
        if let Some((rev, agents)) = self.agents_memo.borrow().as_ref() {
            if *rev == revision {
                return agents.clone();
            }
        }
        let runs = std::rc::Rc::new(agents::collect(
            self.transcript.items(),
            self.transcript.task_notifications(),
        ));
        *self.agents_memo.borrow_mut() = Some((revision, runs.clone()));
        runs
    }

    /// twarp: the header's consolidated **⋯ menu button** — replaces the old
    /// row of globe / agents / scripts icons and the always-visible segmented
    /// toggle. Opens the floating [`render_header_menu`](Self::render_header_menu)
    /// card. While any agent run or background script is active, an accent
    /// notification bubble overlays the glyph with the combined count.
    fn render_header_menu_button(&self, app: &AppContext) -> Box<dyn Element> {
        let active_agents = self
            .agent_runs()
            .iter()
            .filter(|a| !self.agents_dismissed.contains(&a.id) && a.state.is_active())
            .count();
        let active_scripts = self
            .background_scripts()
            .iter()
            .filter(|s| !self.background_dismissed.contains(&s.id) && s.state.is_active())
            .count();
        let active = active_agents + active_scripts;

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let accent = self.accent(app);
        let wash = self.accent_wash(app);
        let expanded = self.header_menu_expanded;

        // Idle affordances read as neutral gray chrome; the full accent is
        // reserved for "something is running right now".
        let glyph_color = if active > 0 {
            accent
        } else {
            theme.sub_text_color(theme.background()).into_solid()
        };
        let glyph = ConstrainedBox::new(
            Icon::new(
                crate::ui_components::icons::Icon::DotsHorizontal.into(),
                glyph_color,
            )
            .finish(),
        )
        .with_width(15.)
        .with_height(15.)
        .finish();

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
        let button = Hoverable::new(self.header_menu_button_mouse.clone(), move |state| {
            let mut body = Container::new(inner)
                .with_padding(Padding::uniform(4.))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
            if expanded || state.is_hovered() {
                body = body.with_background_color(wash);
            }
            body.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action::<PaneHeaderAction<(), ClaudeCodeCustomAction>>(
                PaneHeaderAction::CustomAction(ClaudeCodeCustomAction::ToggleHeaderMenu),
            );
        })
        .finish();
        SavePosition::new(button, HEADER_MENU_BUTTON_POSITION_ID).finish()
    }

    /// The ⋯ button's sectioned session inspector. Repository state and git
    /// entry points live in Environment instead of consuming a row above the
    /// composer; agent/script activity remains expandable inline. The current
    /// provider's raw CLI is a secondary action at the bottom.
    fn render_header_menu(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self.header_menu_expanded {
            return None;
        }
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let accent = self.render_accent.get();
        let wash = self.render_wash.get();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let main = theme.main_text_color(theme.background()).into_solid();
        let green = theme.ui_green_color();
        let red = theme.ui_error_color();

        // Rows the user cleared are hidden but still live in the transcript,
        // so filter them out here (not in the memos, which stay pure
        // derivations).
        let agent_runs = self.agent_runs();
        let visible_agents: Vec<&AgentRun> = agent_runs
            .iter()
            .filter(|a| !self.agents_dismissed.contains(&a.id))
            .collect();
        let scripts = self.background_scripts();
        let visible_scripts: Vec<&BackgroundScript> = scripts
            .iter()
            .filter(|s| !self.background_dismissed.contains(&s.id))
            .collect();
        let active_agents = visible_agents
            .iter()
            .filter(|a| a.state.is_active())
            .count();
        let active_scripts = visible_scripts
            .iter()
            .filter(|s| s.state.is_active())
            .count();

        // One section row: glyph + label left, a muted count / Clear / chevron
        // right; clicking expands the section's rows inline underneath (the
        // reveal-eased accordion). Clear lives inside the row, so the row
        // defers its click to the child to keep a Clear tap from also
        // collapsing the section.
        let section_row = |icon: crate::ui_components::icons::Icon,
                           label: &str,
                           active: usize,
                           expanded: bool,
                           clearable: bool,
                           mouse: MouseStateHandle,
                           clear_mouse: MouseStateHandle,
                           action: ClaudeCodeViewAction,
                           clear_action: ClaudeCodeViewAction|
         -> Box<dyn Element> {
            let glyph_color = if active > 0 { accent } else { muted };
            let glyph = ConstrainedBox::new(Icon::new(icon.into(), glyph_color).finish())
                .with_width(15.)
                .with_height(15.)
                .finish();
            let title = appearance
                .ui_builder()
                .span(label.to_owned())
                .with_style(UiComponentStyles {
                    font_color: Some(main),
                    font_size: Some(12.5),
                    ..Default::default()
                })
                .build()
                .finish();
            let count = appearance
                .ui_builder()
                .span(if active > 0 {
                    format!("{active} running")
                } else {
                    String::new()
                })
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
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_child(glyph)
                .with_child(title)
                .with_child(Shrinkable::new(1., twarpui::elements::Empty::new().finish()).finish())
                .with_child(count);
            if expanded && clearable {
                let clear_label = appearance
                    .ui_builder()
                    .span("Clear".to_owned())
                    .with_style(UiComponentStyles {
                        font_color: Some(muted),
                        font_size: Some(11.),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                let clear = Hoverable::new(clear_mouse, move |state| {
                    let mut body = Container::new(clear_label)
                        .with_padding_left(8.)
                        .with_padding_right(8.)
                        .with_padding_top(3.)
                        .with_padding_bottom(3.)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
                    if state.is_hovered() {
                        body = body.with_background_color(wash);
                    }
                    body.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(clear_action.clone());
                })
                .finish();
                row = row.with_child(clear);
            }
            let row = row.with_child(chevron).finish();
            Hoverable::new(mouse, move |state| {
                let mut body = Container::new(row)
                    .with_padding_left(12.)
                    .with_padding_right(12.)
                    .with_padding_top(8.)
                    .with_padding_bottom(8.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
                if state.is_hovered() {
                    body = body.with_background_color(wash);
                }
                body.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .with_defer_events_to_children()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            })
            .finish()
        };

        // A section's inline empty-state line (shown instead of rows so an
        // expanded section with nothing to list doesn't look broken).
        let empty_line = |text: &str| -> Box<dyn Element> {
            Container::new(
                appearance
                    .ui_builder()
                    .span(text.to_owned())
                    .with_style(UiComponentStyles {
                        font_color: Some(muted),
                        font_size: Some(11.5),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_left(35.)
            .with_padding_right(12.)
            .with_padding_bottom(8.)
            .finish()
        };

        let section_heading = |label: &str| -> Box<dyn Element> {
            Container::new(
                appearance
                    .ui_builder()
                    .span(label.to_owned())
                    .with_style(UiComponentStyles {
                        font_color: Some(muted),
                        font_size: Some(type_ramp::CAPTION.size),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_left(spacing::MD)
            .with_padding_right(spacing::MD)
            .with_padding_top(spacing::MD)
            .with_padding_bottom(spacing::XS)
            .finish()
        };
        let meta_text = |text: String, color: ColorU| -> Box<dyn Element> {
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_color: Some(color),
                    font_size: Some(type_ramp::LABEL.size),
                    ..Default::default()
                })
                .build()
                .finish()
        };
        let menu_row = |icon: crate::ui_components::icons::Icon,
                        label: String,
                        trailing: Option<Box<dyn Element>>,
                        mouse: MouseStateHandle,
                        action: Option<ClaudeCodeViewAction>,
                        position_id: Option<&'static str>|
         -> Box<dyn Element> {
            let enabled = action.is_some();
            let row_color = if enabled { main } else { muted };
            let glyph = ConstrainedBox::new(Icon::new(icon.into(), row_color).finish())
                .with_width(spacing::LG)
                .with_height(spacing::LG)
                .finish();
            let title = appearance
                .ui_builder()
                .span(label)
                .with_style(UiComponentStyles {
                    font_color: Some(row_color),
                    font_size: Some(type_ramp::UI.size),
                    ..Default::default()
                })
                .build()
                .finish();
            let mut contents = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(spacing::SM)
                .with_child(glyph)
                .with_child(title)
                .with_child(Shrinkable::new(1., twarpui::elements::Empty::new().finish()).finish());
            if let Some(trailing) = trailing {
                contents.add_child(trailing);
            }
            let contents = contents.finish();
            let row: Box<dyn Element> = if let Some(action) = action {
                Hoverable::new(mouse, move |state| {
                    let mut body = Container::new(contents)
                        .with_padding_left(spacing::MD)
                        .with_padding_right(spacing::MD)
                        .with_padding_top(spacing::SM)
                        .with_padding_bottom(spacing::SM)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
                    if state.is_hovered() {
                        body = body.with_background_color(wash);
                    }
                    body.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish()
            } else {
                Container::new(contents)
                    .with_padding_left(spacing::MD)
                    .with_padding_right(spacing::MD)
                    .with_padding_top(spacing::SM)
                    .with_padding_bottom(spacing::SM)
                    .finish()
            };
            if let Some(position_id) = position_id {
                SavePosition::new(row, position_id).finish()
            } else {
                row
            }
        };
        let separator = || -> Box<dyn Element> {
            Container::new(
                ConstrainedBox::new(twarpui::elements::Empty::new().finish())
                    .with_height(border::HAIRLINE_WIDTH)
                    .finish(),
            )
            .with_margin_left(spacing::MD)
            .with_margin_right(spacing::MD)
            .with_margin_top(spacing::SM)
            .with_background_color(theme.outline().into())
            .finish()
        };
        let inline_environment_menu = |menu: Box<dyn Element>| -> Box<dyn Element> {
            Container::new(menu)
                .with_margin_left(spacing::LG)
                .with_margin_right(spacing::SM)
                .with_padding(Padding::uniform(spacing::SM))
                .with_background_color(theme.surface_2().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
                .finish()
        };

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        column.add_child(section_heading("Environment"));
        let context = self.repo_context.as_ref();
        let changes = context.and_then(|context| {
            (context.local_files_changed.is_some()
                || context.local_added.is_some()
                || context.local_removed.is_some())
            .then(|| {
                let mut stats = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::XS);
                if let Some(files) = context.local_files_changed {
                    stats.add_child(meta_text(files.to_string(), muted));
                }
                if let Some(added) = context.local_added {
                    stats.add_child(meta_text(format!("+{added}"), green));
                }
                if let Some(removed) = context.local_removed {
                    stats.add_child(meta_text(format!("−{removed}"), red));
                }
                stats.finish()
            })
        });
        column.add_child(menu_row(
            crate::ui_components::icons::Icon::Diff,
            "Changes".to_owned(),
            changes,
            self.header_menu_changes_row_mouse.clone(),
            context
                .and_then(|context| context.branch.as_ref())
                .map(|_| ClaudeCodeViewAction::OpenChanges),
            None,
        ));
        let local_meta = context
            .and_then(|context| context.folder.as_ref())
            .map(|folder| {
                let label = if self.use_worktree {
                    "Worktree".to_owned()
                } else {
                    truncate_middle(folder, 24)
                };
                meta_text(label, if self.use_worktree { accent } else { muted })
            });
        let can_toggle_worktree = self.message_tx.is_none()
            && context
                .and_then(|context| context.branch.as_ref())
                .is_some();
        column.add_child(menu_row(
            crate::ui_components::icons::Icon::Laptop,
            "Local".to_owned(),
            local_meta,
            self.header_menu_local_row_mouse.clone(),
            can_toggle_worktree.then_some(ClaudeCodeViewAction::ToggleWorktree),
            None,
        ));
        if let Some(branch) = context.and_then(|context| context.branch.as_ref()) {
            let branch_trailing = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::XS)
                .with_child(meta_text(truncate_middle(branch, 24), muted))
                .with_child(
                    ConstrainedBox::new(
                        Icon::new(crate::ui_components::icons::Icon::ChevronDown.into(), muted)
                            .finish(),
                    )
                    .with_width(spacing::MD)
                    .with_height(spacing::MD)
                    .finish(),
                )
                .finish();
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::GitBranch,
                "Branch".to_owned(),
                Some(branch_trailing),
                self.branch_pill_mouse.clone(),
                Some(ClaudeCodeViewAction::ToggleComposerMenu(
                    ComposerMenu::Branch,
                )),
                Some(ComposerMenu::Branch.anchor_id()),
            ));
            if self.composer_menu == Some(ComposerMenu::Branch) {
                if let Some(menu) = self.render_branch_menu(appearance) {
                    column.add_child(inline_environment_menu(menu));
                }
            }
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::GitCommit,
                "Commit or push".to_owned(),
                None,
                self.header_menu_commit_row_mouse.clone(),
                Some(ClaudeCodeViewAction::OpenChanges),
                None,
            ));
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::SwitchHorizontal01,
                "Compare branch".to_owned(),
                Some(
                    ConstrainedBox::new(
                        Icon::new(
                            crate::ui_components::icons::Icon::LinkExternal.into(),
                            muted,
                        )
                        .finish(),
                    )
                    .with_width(spacing::LG)
                    .with_height(spacing::LG)
                    .finish(),
                ),
                self.header_menu_compare_row_mouse.clone(),
                Some(ClaudeCodeViewAction::OpenChanges),
                None,
            ));
        }
        if let Some(pr_number) = context.and_then(|context| context.pr_number) {
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::Github,
                format!("Pull request #{pr_number}"),
                None,
                self.pr_pill_mouse.clone(),
                Some(ClaudeCodeViewAction::ToggleComposerMenu(ComposerMenu::Pr)),
                Some(ComposerMenu::Pr.anchor_id()),
            ));
            if self.composer_menu == Some(ComposerMenu::Pr) {
                if let Some(menu) = self.render_pr_menu(appearance) {
                    column.add_child(inline_environment_menu(menu));
                }
            }
        } else if context
            .is_some_and(|context| context.branch.is_some() && context.repo_web_url.is_some())
        {
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::Github,
                "Create pull request".to_owned(),
                None,
                self.pr_pill_mouse.clone(),
                Some(ClaudeCodeViewAction::CreatePr),
                None,
            ));
        }
        if let Some(ci) = context.and_then(|context| context.ci) {
            let ci_color = match ci {
                CiState::Passing => green,
                CiState::Failing => red,
                CiState::Pending => theme.ui_warning_color(),
            };
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::Github,
                "Checks".to_owned(),
                Some(meta_text(ci.label().to_owned(), ci_color)),
                self.ci_pill_mouse.clone(),
                Some(ClaudeCodeViewAction::ToggleComposerMenu(ComposerMenu::Ci)),
                Some(ComposerMenu::Ci.anchor_id()),
            ));
            if self.composer_menu == Some(ComposerMenu::Ci) {
                if let Some(menu) = self.render_ci_menu(appearance) {
                    column.add_child(inline_environment_menu(menu));
                }
            }
        }

        column.add_child(separator());
        column.add_child(section_heading("Activity"));
        column.add_child(section_row(
            crate::ui_components::icons::Icon::ArrowSplit,
            "Agent runs",
            active_agents,
            self.agents_panel_expanded,
            visible_agents.iter().any(|a| !a.state.is_active()),
            self.header_menu_agents_row_mouse.clone(),
            self.agents_clear_mouse.clone(),
            ClaudeCodeViewAction::ToggleAgentsPanel,
            ClaudeCodeViewAction::ClearAgents,
        ));
        // The expanded flag stays set while a collapse animation runs (the
        // body must remain in the tree to shrink); the reveal fraction is
        // what actually eases the body open and closed.
        if self.agents_panel_expanded {
            let mut body = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min);
            if visible_agents.is_empty() {
                body.add_child(empty_line("No agents"));
            } else {
                for agent in visible_agents.iter() {
                    body.add_child(self.render_agent_row(agent, app));
                }
            }
            column.add_child(Box::new(RevealClip::new(
                body.finish(),
                self.header_menu_section_fraction(HeaderMenuSection::Agents),
            )));
        }
        column.add_child(section_row(
            crate::ui_components::icons::Icon::Terminal,
            "Scripts",
            active_scripts,
            self.background_scripts_expanded,
            visible_scripts.iter().any(|s| !s.state.is_active()),
            self.header_menu_scripts_row_mouse.clone(),
            self.background_clear_mouse.clone(),
            ClaudeCodeViewAction::ToggleBackgroundPanel,
            ClaudeCodeViewAction::ClearBackgroundScripts,
        ));
        if self.background_scripts_expanded {
            let mut body = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min);
            if visible_scripts.is_empty() {
                body.add_child(empty_line("No background scripts"));
            } else {
                for script in visible_scripts.iter() {
                    body.add_child(self.render_background_row(script, app));
                }
            }
            column.add_child(Box::new(RevealClip::new(
                body.finish(),
                self.header_menu_section_fraction(HeaderMenuSection::Scripts),
            )));
        }
        if supports_raw_cli(self.provider) {
            column.add_child(separator());
            column.add_child(menu_row(
                crate::ui_components::icons::Icon::Terminal,
                raw_cli_menu_label(self.provider).to_owned(),
                Some(
                    ConstrainedBox::new(
                        Icon::new(
                            crate::ui_components::icons::Icon::LinkExternal.into(),
                            muted,
                        )
                        .finish(),
                    )
                    .with_width(spacing::LG)
                    .with_height(spacing::LG)
                    .finish(),
                ),
                self.header_menu_raw_cli_row_mouse.clone(),
                (!self.streaming).then_some(ClaudeCodeViewAction::ToggleRawCli),
                None,
            ));
        }
        column.add_child(
            ConstrainedBox::new(twarpui::elements::Empty::new().finish())
                .with_height(spacing::XS)
                .finish(),
        );
        let column = column.finish();
        let column = ClippedScrollable::vertical(
            self.header_menu_scroll_state.clone(),
            column,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let card = Container::new(column)
            .with_background_color(theme.surface_1().into_solid())
            .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::PANEL)))
            .finish();
        let card = Dismiss::new(card)
            .on_dismiss(|ctx, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::CloseHeaderMenu);
            })
            .finish();
        let (offset_y, blur_radius, spread_radius, alpha) = elevation::POPOVER;
        let card = Container::new(card)
            .with_drop_shadow(DropShadow {
                color: ColorU::new(0, 0, 0, (alpha * 255.).round() as u8),
                offset: vec2f(0., offset_y),
                blur_radius,
                spread_radius,
            })
            .finish();
        // Both min AND max width: a positioned overlay child is measured
        // against the whole pane's constraints, so without a max the card
        // stretches to the full pane width.
        Some(
            ConstrainedBox::new(card)
                .with_min_width(240.)
                .with_max_width(HEADER_MENU_MAX_WIDTH)
                .with_max_height(measure::POPOVER_MAX_HEIGHT)
                .finish(),
        )
    }

    /// Current reveal fraction (0 collapsed → 1 open) for a header-menu
    /// section: the in-flight animation's eased value while that section is
    /// animating, else fully open/closed per its expanded flag.
    fn header_menu_section_fraction(&self, section: HeaderMenuSection) -> f32 {
        match &self.header_menu_reveal {
            Some((s, reveal)) if *s == section => reveal.fraction(),
            _ => {
                let expanded = match section {
                    HeaderMenuSection::Agents => self.agents_panel_expanded,
                    HeaderMenuSection::Scripts => self.background_scripts_expanded,
                };
                if expanded {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// Toggle one of the header menu's inline sections with the reveal ease.
    /// Opening a section snaps the other closed (an accordion — at most one
    /// section open). Closing leaves the expanded flag set until the collapse
    /// animation lands; [`Self::tick_header_menu_reveal`] clears it once the
    /// body has fully shrunk. Starting from the current fraction means a rapid
    /// re-toggle mid-animation reverses smoothly instead of jumping.
    fn toggle_header_menu_section(
        &mut self,
        section: HeaderMenuSection,
        ctx: &mut ViewContext<Self>,
    ) {
        let expanded = match section {
            HeaderMenuSection::Agents => self.agents_panel_expanded,
            HeaderMenuSection::Scripts => self.background_scripts_expanded,
        };
        let from = self.header_menu_section_fraction(section);
        let to = match &self.header_menu_reveal {
            // Mid-animation re-toggle: reverse towards the opposite end.
            Some((s, reveal)) if *s == section => 1.0 - reveal.to,
            _ if expanded => 0.0,
            _ => 1.0,
        };
        if to > 0.5 {
            match section {
                HeaderMenuSection::Agents => {
                    self.agents_panel_expanded = true;
                    self.background_scripts_expanded = false;
                }
                HeaderMenuSection::Scripts => {
                    self.background_scripts_expanded = true;
                    self.agents_panel_expanded = false;
                }
            }
        }
        self.header_menu_reveal = Some((section, SectionReveal::new(from, to)));
        self.tick_header_menu_reveal(ctx);
    }

    /// One reveal animation frame: repaint, then re-arm until the ease
    /// completes (the self-rearming `notify()` timer pattern — warpui never
    /// re-runs `render()` on its own; see `left_panel_slide.rs`). A collapse
    /// that finishes clears the section's expanded flag so the body leaves
    /// the element tree.
    fn tick_header_menu_reveal(&mut self, ctx: &mut ViewContext<Self>) {
        let Some((section, reveal)) = &self.header_menu_reveal else {
            return;
        };
        let section = *section;
        let closing = reveal.to <= 0.0;
        if reveal.is_done() {
            if closing {
                match section {
                    HeaderMenuSection::Agents => self.agents_panel_expanded = false,
                    HeaderMenuSection::Scripts => self.background_scripts_expanded = false,
                }
            }
            self.header_menu_reveal = None;
            ctx.notify();
            return;
        }
        ctx.notify();
        ctx.spawn(
            async move {
                Timer::after(HEADER_MENU_REVEAL_TICK).await;
            },
            |me, _, ctx| me.tick_header_menu_reveal(ctx),
        );
    }
    /// One agent row in the expanded panel: a status glyph + "<type>:
    /// <description>" + the state label, expanding on click into the agent's
    /// returned result — the background-script row's chrome.
    fn render_agent_row(&self, agent: &AgentRun, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let main = theme.main_text_color(theme.background()).into_solid();
        let wash = self.render_wash.get();

        let status_icon: Box<dyn Element> = match agent.state {
            AgentRunState::Running => inline_action::running_icon(appearance).finish(),
            AgentRunState::Finished => inline_action::green_check_icon(appearance).finish(),
            AgentRunState::Failed => inline_action::red_x_icon(appearance).finish(),
            AgentRunState::Stopped => {
                Icon::new(crate::ui_components::icons::Icon::Stop.into(), muted).finish()
            }
        };
        let status_icon = ConstrainedBox::new(status_icon)
            .with_width(13.)
            .with_height(13.)
            .finish();

        let title_text = appearance
            .ui_builder()
            .span(agent.title())
            .with_style(UiComponentStyles {
                font_color: Some(main),
                font_size: Some(11.5),
                ..Default::default()
            })
            .build()
            .finish();
        let state_label = appearance
            .ui_builder()
            .span(agent.state.label().to_owned())
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
            .with_child(Shrinkable::new(1., Clipped::new(title_text).finish()).finish())
            .with_child(state_label)
            .finish();

        let row_mouse = {
            let mut states = self.agent_row_mouse.borrow_mut();
            states.entry(agent.id.clone()).or_default().clone()
        };
        let id = agent.id.clone();
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
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleAgentRow(id.clone()));
        })
        .finish();

        // The returned result, revealed on click. Capped the same way
        // tool-card results are so a verbose agent can't stall layout.
        let result_open =
            self.agent_expanded_rows.contains(&agent.id) && !agent.result.trim().is_empty();
        if !result_open {
            return header;
        }
        let (shown, hidden) = tool_cards::truncate_output(agent.result.trim());
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
            BackgroundScriptState::LaunchFailed | BackgroundScriptState::Failed => {
                inline_action::red_x_icon(appearance).finish()
            }
            BackgroundScriptState::Killed => {
                Icon::new(crate::ui_components::icons::Icon::Stop.into(), muted).finish()
            }
        };
        let status_icon = ConstrainedBox::new(status_icon)
            .with_width(13.)
            .with_height(13.)
            .finish();

        let command = tool_cards::format_command_text(&script.command);
        let command_text = twarpui::elements::Text::new_inline(
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
            states.entry(script.id.clone()).or_default().clone()
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

    /// Resolve a marker's document-relative position and whether its turn
    /// intersects the current viewport. Saved positions are scroll-translated,
    /// but subtracting the saved document-top position cancels that translation.
    fn timeline_marker_layout(
        &self,
        entry: &TimelineEntry,
        entry_index: usize,
        entry_count: usize,
        app: &AppContext,
    ) -> (f32, bool) {
        let fallback_ratio = (entry_index + 1) as f32 / (entry_count + 1) as f32;
        let geometry = (
            app.element_position_by_id_at_last_frame(
                self.window_id,
                self.transcript_top_position_id(),
            ),
            app.element_position_by_id_at_last_frame(
                self.window_id,
                self.transcript_bottom_position_id(),
            ),
            app.element_position_by_id_at_last_frame(
                self.window_id,
                self.transcript_viewport_position_id(),
            ),
            app.element_position_by_id_at_last_frame(
                self.window_id,
                self.turn_position_id(entry.turn_start),
            ),
        );
        let (Some(document_top), Some(document_bottom), Some(viewport), Some(turn)) = geometry
        else {
            let visible = self.scroll_state.is_at_bottom(AUTOSCROLL_STICK_SLACK)
                && entry_index + 1 == entry_count;
            return (fallback_ratio, visible);
        };

        let document_height = document_bottom.max_y() - document_top.min_y();
        if document_height <= border::HAIRLINE_WIDTH {
            return (fallback_ratio, false);
        }

        let turn_top = (turn.min_y() - document_top.min_y()).max(0.0);
        let turn_bottom = (turn.max_y() - document_top.min_y()).max(turn_top);
        let viewport_height = viewport.height();
        let scroll_top = self.scroll_state.scroll_start().as_f32();
        let visible = turn_bottom >= scroll_top && turn_top <= scroll_top + viewport_height;

        let mut ratio = (turn_top / document_height).clamp(0.0, 1.0);
        // Keep the marker's hit target inside the rail at both extremes.
        if viewport_height > spacing::LG {
            let inset_ratio = spacing::SM / viewport_height;
            ratio = ratio.clamp(inset_ratio, 1.0 - inset_ratio);
        }
        (ratio, visible)
    }

    fn render_timeline_preview(entry: &TimelineEntry, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let surface = theme.surface_1();
        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::XS)
            .with_child(
                Text::new_inline(
                    entry.prompt_preview.clone(),
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_color(theme.main_text_color(surface.clone()).into_solid())
                .with_selectable(false)
                .finish(),
            );
        if let Some(response) = &entry.response_preview {
            content.add_child(
                Text::new_inline(
                    response.clone(),
                    appearance.ui_font_family(),
                    type_ramp::LABEL.size,
                )
                .with_color(theme.sub_text_color(surface.clone()).into_solid())
                .with_selectable(false)
                .finish(),
            );
        }

        let card = Container::new(content.finish())
            .with_padding(Padding::uniform(spacing::MD))
            .with_background_color(surface.into_solid())
            .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::PANEL)))
            .finish();
        let (offset_y, blur_radius, spread_radius, alpha) = elevation::POPOVER;
        Container::new(card)
            .with_drop_shadow(DropShadow {
                color: ColorU::new(0, 0, 0, (alpha * 255.0).round() as u8),
                offset: vec2f(0.0, offset_y),
                blur_radius,
                spread_radius,
            })
            .finish()
    }

    fn render_timeline_marker(
        &self,
        entry: &TimelineEntry,
        visible: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mouse = self
            .timeline_turn_mouse
            .borrow_mut()
            .entry(entry.turn_start)
            .or_default()
            .clone();
        let turn_start = entry.turn_start;
        let entry = TimelineEntry {
            turn_start,
            prompt_preview: entry.prompt_preview.clone(),
            response_preview: entry.response_preview.clone(),
        };

        Hoverable::new(mouse, move |state| {
            let hovered = state.is_hovered();
            let marker_width = if hovered {
                spacing::XL
            } else if visible {
                spacing::LG
            } else {
                spacing::SM
            };
            let marker_color = if hovered {
                theme.main_text_color(theme.background()).into_solid()
            } else if visible {
                theme.sub_text_color(theme.background()).into_solid()
            } else {
                theme.outline().into_solid()
            };
            let marker = ConstrainedBox::new(
                Container::new(Empty::new().finish())
                    .with_background_color(marker_color)
                    .finish(),
            )
            .with_width(marker_width)
            .with_height(border::HAIRLINE_WIDTH)
            .finish();
            let target = ConstrainedBox::new(Align::new(marker).left().finish())
                .with_width(spacing::XXL)
                .with_height(spacing::MD)
                .finish();

            let mut marker_stack = Stack::new().with_child(target);
            if hovered {
                marker_stack.add_positioned_overlay_child(
                    Self::render_timeline_preview(&entry, appearance),
                    OffsetPositioning::offset_from_parent(
                        vec2f(spacing::XS, 0.0),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::MiddleRight,
                        ChildAnchor::MiddleLeft,
                    ),
                );
            }
            marker_stack.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::JumpToTurn(turn_start));
        })
        .finish()
    }

    fn render_timeline(&self, entries: &[TimelineEntry], app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut rail = Stack::new().with_event_dispatch_mode(EventDispatchMode::Waterfall);
        // A full-constraint inert child gives percentage-positioned markers the
        // same height as the transcript viewport without creating a hit target.
        rail.add_child(Align::new(Empty::new().finish()).finish());
        for (index, entry) in entries.iter().enumerate() {
            let (ratio, visible) = self.timeline_marker_layout(entry, index, entries.len(), app);
            rail.add_positioned_overlay_child(
                self.render_timeline_marker(entry, visible, appearance),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_parent(
                        ParentOffsetBounds::Unbounded,
                        OffsetType::Pixel(spacing::SM),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_parent(
                        ParentOffsetBounds::Unbounded,
                        OffsetType::Percentage(ratio),
                        AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Middle),
                    ),
                ),
            );
        }

        // The rail must never compete with prose or message avatars. It appears
        // only when the pane is wider than the complete document measure plus
        // its two horizontal gutters.
        Box::new(SizeConstraintSwitch::new(
            rail.finish(),
            [(
                SizeConstraintCondition::WidthLessThan(measure::PROSE_MAX_WIDTH + spacing::XXL),
                Align::new(Empty::new().finish()).finish(),
            )],
        ))
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
        let mut turns = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(spacing::XL)
            .with_main_axis_size(MainAxisSize::Min);

        let items = self.transcript.items();
        let timeline_entries = project_timeline(items, self.streaming);
        let mut next_index = 0;
        for turn in project_turns(items, self.streaming) {
            // Defensive prefix handling: restored/provider transcripts can
            // contain a notice before their first user message.
            for (index, item) in items.iter().enumerate().take(turn.start).skip(next_index) {
                turns.add_child(self.render_item(index, item, app));
            }

            let mut content = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min);
            if turn.compact {
                content.add_child(self.render_item(turn.start, &items[turn.start], app));
                if !turn.hidden_work.is_empty() {
                    content.add_child(self.render_completed_turn_work(&turn, app));
                }
                if let Some(final_response) = turn.final_response {
                    content.add_child(self.render_final_assistant_response(final_response, app));
                }
                if let Some(artifacts) = self.render_file_edit_artifacts(&turn, app) {
                    content.add_child(artifacts);
                }
            } else {
                for (index, item) in items.iter().enumerate().take(turn.end).skip(turn.start) {
                    content.add_child(self.render_item(index, item, app));
                }
            }
            let position_id = self.turn_position_id(turn.start);
            turns.add_child(
                SavePosition::new(content.finish(), &position_id)
                    .for_single_frame()
                    .finish(),
            );
            next_index = turn.end;
        }
        for (index, item) in items.iter().enumerate().skip(next_index) {
            turns.add_child(self.render_item(index, item, app));
        }

        // #7: a live status line below the last message while a turn streams —
        // an animated label + elapsed, replacing the composer's "Working…".
        if self.streaming {
            turns.add_child(self.render_streaming_status(app));
        }

        // Clearance spacer between the last message and the end marker. It must
        // sit *above* the sentinel: `scroll_to_bottom` aligns the sentinel to the
        // viewport's bottom edge, so this spacer is what lifts the last message
        // clear of the floating composer (trailing padding *below* the sentinel
        // would just be scrolled out of view, behind the composer).
        turns.add_child(
            ConstrainedBox::new(Container::new(Flex::row().finish()).finish())
                .with_height(COMPOSER_CLEARANCE)
                .finish(),
        );

        // PRODUCT §14: a minimal marker pinned to the end of the transcript.
        // [`Self::scroll_to_bottom`] scrolls this into view to follow streaming
        // output and to open a resumed session at its latest message.
        let bottom_position_id = self.transcript_bottom_position_id();
        turns.add_child(
            SavePosition::new(
                ConstrainedBox::new(Container::new(Flex::row().finish()).finish())
                    .with_height(border::HAIRLINE_WIDTH)
                    .finish(),
                &bottom_position_id,
            )
            .for_single_frame()
            .finish(),
        );

        let top_position_id = self.transcript_top_position_id();
        let document = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(
                SavePosition::new(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_height(border::HAIRLINE_WIDTH)
                        .finish(),
                    &top_position_id,
                )
                .for_single_frame()
                .finish(),
            )
            .with_child(turns.finish())
            .finish();

        // The composer floats over the bottom of the pane; this clearance is
        // inside the scroll content so the newest message can scroll out from
        // underneath it (PRODUCT §15).
        let content = Container::new(
            Align::new(
                ConstrainedBox::new(document)
                    .with_max_width(measure::PROSE_MAX_WIDTH)
                    .finish(),
            )
            .top_center()
            .finish(),
        )
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
        // (see `twarpui` `table-sample` and `NewScrollable`). The selected text is
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
        let selectable = SelectableArea::new(
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
        .finish();
        let viewport_position_id = self.transcript_viewport_position_id();
        let selectable = SavePosition::new(selectable, &viewport_position_id)
            .for_single_frame()
            .finish();

        let mut transcript_stack =
            Stack::new().with_event_dispatch_mode(EventDispatchMode::Waterfall);
        transcript_stack.add_child(selectable);
        if timeline_entries.len() >= TIMELINE_MIN_TURNS {
            transcript_stack.add_child(self.render_timeline(&timeline_entries, app));
        }
        transcript_stack.finish()
    }

    /// The quiet, turn-wide disclosure shown after work settles. Expanding it
    /// restores every hidden item in its original order; the transcript model
    /// itself is never summarized or discarded.
    fn render_completed_turn_work(
        &self,
        turn: &TurnPresentation,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let label = match self.turn_duration(turn) {
            Some(duration) => format!("Worked for {}", thinking::format_compact_elapsed(duration)),
            None => "Worked".to_owned(),
        };
        let muted = theme.sub_text_color(theme.background()).into_solid();
        let title = Text::new_inline(label, appearance.ui_font_family(), type_ramp::UI.size)
            .with_color(muted)
            .with_selectable(false)
            .finish();
        let expanded = self.expanded_completed_turns.contains(&turn.start);
        let chevron = Icon::new(
            if expanded {
                crate::ui_components::icons::Icon::ChevronDown
            } else {
                crate::ui_components::icons::Icon::ChevronRight
            }
            .into(),
            muted,
        );
        let chevron = ConstrainedBox::new(chevron.finish())
            .with_width(spacing::LG)
            .with_height(spacing::LG)
            .finish();
        let row = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::XS)
                .with_child(title)
                .with_child(chevron)
                .finish(),
        )
        .with_horizontal_padding(spacing::LG)
        .with_vertical_padding(spacing::SM)
        .with_border(Border::bottom(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
        .finish();
        let mouse_state = self
            .completed_turn_mouse
            .borrow_mut()
            .entry(turn.start)
            .or_default()
            .clone();
        let start = turn.start;
        let header = Hoverable::new(mouse_state, |_| row)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleCompletedTurn(start));
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        let mut disclosure = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(header);
        if expanded {
            let mut work = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(spacing::SM);
            for &index in &turn.hidden_work {
                work.add_child(self.render_item(index, &self.transcript.items()[index], app));
            }
            disclosure.add_child(
                Container::new(work.finish())
                    .with_margin_top(spacing::SM)
                    .with_margin_bottom(spacing::SM)
                    .finish(),
            );
        }
        disclosure.finish()
    }

    fn turn_duration(&self, turn: &TurnPresentation) -> Option<Duration> {
        self.transcript.items()[turn.start + 1..turn.end]
            .iter()
            .find_map(|item| match item {
                TranscriptItem::Metrics(metrics) => metrics.duration_ms.map(Duration::from_millis),
                _ => None,
            })
    }

    /// Consolidate successful Edit/Write calls into one durable result card.
    /// The full inline diffs remain available inside the expanded work row;
    /// this card is the calm, post-completion inventory with direct file-open
    /// actions.
    fn render_file_edit_artifacts(
        &self,
        turn: &TurnPresentation,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let mut files: Vec<FileEditSummary> = Vec::new();
        for &index in &turn.file_edits {
            let TranscriptItem::Tool { id, input, .. } = &self.transcript.items()[index] else {
                continue;
            };
            let summaries = match self.diff_cards.get(id) {
                Some(card) => vec![FileEditSummary {
                    path: card.file_path.clone(),
                    added: card.added,
                    removed: card.removed,
                }],
                None => file_edit_summaries(input),
            };
            for summary in summaries {
                if let Some(existing) = files
                    .iter_mut()
                    .find(|existing| existing.path == summary.path)
                {
                    existing.added += summary.added;
                    existing.removed += summary.removed;
                } else {
                    files.push(summary);
                }
            }
        }
        if files.is_empty() {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let main = theme.main_text_color(theme.background()).into_solid();
        let green: ColorU = AnsiColorIdentifier::Green
            .to_ansi_color(&theme.terminal_colors().normal)
            .into();
        let red: ColorU = AnsiColorIdentifier::Red
            .to_ansi_color(&theme.terminal_colors().normal)
            .into();
        let total_added = files.iter().map(|file| file.added).sum::<usize>();
        let total_removed = files.iter().map(|file| file.removed).sum::<usize>();
        let title = if files.len() == 1 {
            "Edited 1 file".to_owned()
        } else {
            format!("Edited {} files", files.len())
        };

        let glyph = ConstrainedBox::new(
            Icon::new(
                crate::ui_components::icons::Icon::Pencil.into(),
                crate::ui_components::blended_colors::neutral_7(theme),
            )
            .finish(),
        )
        .with_width(spacing::LG)
        .with_height(spacing::LG)
        .finish();
        let glyph = Container::new(glyph)
            .with_padding(Padding::uniform(spacing::SM))
            .with_background(theme.surface_overlay_1())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)))
            .finish();
        let stats = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::XS)
            .with_child(
                Text::new_inline(
                    format!("+{total_added}"),
                    appearance.ui_font_family(),
                    type_ramp::LABEL.size,
                )
                .with_color(green)
                .with_selectable(false)
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    format!("−{total_removed}"),
                    appearance.ui_font_family(),
                    type_ramp::LABEL.size,
                )
                .with_color(red)
                .with_selectable(false)
                .finish(),
            )
            .finish();
        let heading = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::XXS)
            .with_child(
                Text::new_inline(title, appearance.ui_font_family(), type_ramp::UI.size)
                    .with_color(main)
                    .with_selectable(false)
                    .finish(),
            )
            .with_child(stats)
            .finish();
        let header = Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::MD)
                .with_child(glyph)
                .with_child(heading)
                .finish(),
        )
        .with_horizontal_padding(spacing::LG)
        .with_vertical_padding(spacing::MD)
        .finish();

        let mut card = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(header);
        for file in files {
            let file_stats = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::XS)
                .with_child(
                    Text::new_inline(
                        format!("+{}", file.added),
                        appearance.ui_font_family(),
                        type_ramp::LABEL.size,
                    )
                    .with_color(green)
                    .with_selectable(false)
                    .finish(),
                )
                .with_child(
                    Text::new_inline(
                        format!("−{}", file.removed),
                        appearance.ui_font_family(),
                        type_ramp::LABEL.size,
                    )
                    .with_color(red)
                    .with_selectable(false)
                    .finish(),
                )
                .finish();
            let path = file.path;
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(spacing::SM)
                .with_child(
                    Expanded::new(
                        1.,
                        Text::new_inline(
                            path.clone(),
                            appearance.monospace_font_family(),
                            type_ramp::CODE.size,
                        )
                        .with_color(main)
                        .with_selectable(true)
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(file_stats)
                .with_child(
                    ConstrainedBox::new(
                        Icon::new(
                            crate::ui_components::icons::Icon::LinkExternal.into(),
                            theme.sub_text_color(theme.background()).into_solid(),
                        )
                        .finish(),
                    )
                    .with_width(spacing::LG)
                    .with_height(spacing::LG)
                    .finish(),
                )
                .finish();
            let mouse_state = self
                .artifact_file_mouse
                .borrow_mut()
                .entry((turn.start, path.clone()))
                .or_default()
                .clone();
            let open_path = path;
            let outline = theme.outline();
            let hover_fill = theme.surface_overlay_1();
            let row = Hoverable::new(mouse_state, move |state| {
                let mut container = Container::new(row)
                    .with_horizontal_padding(spacing::LG)
                    .with_vertical_padding(spacing::MD)
                    .with_border(Border::top(border::HAIRLINE_WIDTH).with_border_fill(outline));
                if state.is_hovered() {
                    container = container.with_background(hover_fill);
                }
                container.finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenArtifactFile(
                    open_path.clone(),
                ));
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
            card.add_child(row);
        }

        Some(
            Container::new(card.finish())
                .with_background(theme.background())
                .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
                .with_margin_left(spacing::LG)
                .with_margin_right(spacing::LG)
                .with_margin_top(spacing::SM)
                .finish(),
        )
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

    /// Rows for the model dropdown: "Default" (no `--model`, let the CLI
    /// choose) first, then either the models discovered from the Anthropic
    /// Models API (full IDs, newest first) or — until/unless discovery
    /// succeeds — the built-in alias fallback. A current selection that isn't
    /// in the list (an alias picked before discovery landed, or a launch-flag
    /// model) gets its own row so it stays visible and re-selectable.
    fn model_menu_entries(&self) -> Vec<(Option<String>, String)> {
        let mut entries = vec![(None, "Default".to_owned())];
        match crate::claude_code_models::discovered() {
            Some(models) => {
                for model in models {
                    entries.push((Some(model.id.clone()), model.display_name.clone()));
                }
            }
            None => {
                for alias in crate::claude_code_models::FALLBACK_MODEL_ALIASES {
                    entries.push((Some((*alias).to_owned()), prettify_model(alias)));
                }
            }
        }
        if let Some(current) = self.model.as_deref() {
            if !entries
                .iter()
                .any(|(value, _)| value.as_deref() == Some(current))
            {
                entries.push((Some(current.to_owned()), prettify_model(current)));
            }
        }
        entries
    }

    /// The model dropdown (#13): one row per [`Self::model_menu_entries`]
    /// entry, the active one highlighted; clicking sets the model and closes
    /// the menu.
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
        for (index, (value, label)) in self.model_menu_entries().into_iter().enumerate() {
            let selected = current == value.as_deref();
            let mouse = {
                let mut pool = self.model_menu_row_mouse.borrow_mut();
                while pool.len() <= index {
                    pool.push(MouseStateHandle::default());
                }
                pool[index].clone()
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

        // Switching branches under a live agent can invalidate its cwd and
        // edits. Keep read-only branch actions available, but only expose the
        // switch list while idle.
        if !self.streaming {
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
        let green = theme.ui_green_color();
        let red = theme.ui_error_color();
        let amber = theme.ui_warning_color();

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

    /// The docked composer (PRODUCT §15): a rounded, bordered card holding the
    /// message input above a controls row — muted context pills on the left, the
    /// Send / Stop action on the right — pinned to the bottom of the reading
    /// column, Claude-app style. Mirrors `GlobalSearchView`'s bordered query box.
    /// twarp 17 (PRODUCT 17 §1, §4–§5): the composer mic button, right of the
    /// paperclip. Idle → muted glyph; recording → pulsing accent glyph plus an
    /// m:ss elapsed label; transcribing → muted glyph plus an ellipsis label.
    fn render_mic_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let accent = self.render_accent.get();
        let recording = self.voice_recorder.is_some();

        let glyph: Box<dyn Element> = if recording {
            PulsingIcon::new(
                crate::ui_components::icons::Icon::Microphone.into(),
                accent,
                self.mic_pulse.clone(),
            )
            .finish()
        } else {
            Icon::new(crate::ui_components::icons::Icon::Microphone.into(), muted).finish()
        };
        let button = Hoverable::new(self.mic_button_mouse.clone(), {
            move |_| {
                ConstrainedBox::new(glyph)
                    .with_width(16.)
                    .with_height(16.)
                    .finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleVoiceRecording);
        })
        .finish();

        let label = if let Some(recorder) = &self.voice_recorder {
            let elapsed = recorder.elapsed().as_secs();
            Some(format!("{}:{:02}", elapsed / 60, elapsed % 60))
        } else if self.voice_transcribing {
            Some("…".to_owned())
        } else {
            None
        };
        let Some(label) = label else {
            return button;
        };
        let label_color = if recording { accent } else { muted };
        let label = appearance
            .ui_builder()
            .span(label)
            .with_style(UiComponentStyles {
                font_color: Some(label_color),
                font_size: Some(12.),
                ..Default::default()
            })
            .build()
            .finish();
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(button)
            .with_child(label)
            .finish()
    }

    /// twarp 17 (PRODUCT 17 §12, §14): the speaker toggle right of the mic —
    /// accent while on, muted while off.
    fn render_speaker_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let muted = theme.nonactive_ui_text_color().into_solid();
        let speaking = self
            .voice_player
            .as_ref()
            .is_some_and(|player| player.is_active());
        let color = if self.speak_replies {
            self.render_accent.get()
        } else {
            muted
        };
        let glyph: Box<dyn Element> = if speaking && self.speak_replies {
            PulsingIcon::new(
                crate::ui_components::icons::Icon::Speaker.into(),
                color,
                self.mic_pulse.clone(),
            )
            .finish()
        } else {
            Icon::new(crate::ui_components::icons::Icon::Speaker.into(), color).finish()
        };
        Hoverable::new(self.speaker_button_mouse.clone(), move |_| {
            ConstrainedBox::new(glyph)
                .with_width(16.)
                .with_height(16.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleSpeakReplies);
        })
        .finish()
    }

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

        // #13: the Send / Stop action. Built per density tier below, so a
        // closure rather than a one-shot element.
        let make_action = || -> Box<dyn Element> {
            if self.streaming {
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
                        font_size: Some(type_ramp::UI.size),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                let button = Container::new(button_label)
                    .with_padding_left(spacing::MD)
                    .with_padding_right(spacing::MD)
                    .with_padding_top(spacing::SM)
                    .with_padding_bottom(spacing::SM)
                    .with_background_color(accent)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
                    .finish();
                Hoverable::new(self.submit_button.clone(), move |_| button)
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ClaudeCodeViewAction::Submit);
                    })
                    .finish()
            }
        };

        // PRODUCT §51 (7l): the "＋ attach" control opens the OS file picker.
        let make_attach = || -> Box<dyn Element> {
            Hoverable::new(self.attach_button.clone(), {
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
            .finish()
        };

        // #13: Claude-style footer below the input — permission selector and
        // attach on the left; the context / model / effort controls (each opens
        // a dropdown / popover above the input) and the Send/Stop action on the
        // right. (#7: the streaming indicator moved out of here to below the
        // last message.)
        //
        // Responsive: in a narrow pane the read-only info chips step out first
        // (compact drops `MCP · N`, tiny also drops the context-usage and
        // effort chips) so the interactive controls and the Send/Stop action
        // never overflow the card. Every tier is built up front and a
        // SizeConstraintSwitch picks one at layout time.
        let controls_for = |density: ComposerDensity| -> Box<dyn Element> {
            let left = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM)
                .with_child(self.render_permission_control(appearance))
                .with_child(make_attach())
                // twarp 17 (PRODUCT 17 §1, §12): voice input + spoken replies.
                .with_child(self.render_mic_button(appearance))
                .with_child(self.render_speaker_button(appearance))
                .finish();

            let mut right = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::SM);
            if density == ComposerDensity::Full {
                right.add_child(self.render_mcp_control(appearance));
            }
            if density != ComposerDensity::Tiny {
                right.add_child(self.render_context_control(appearance));
            }
            right.add_child(self.render_model_control(appearance));
            if density != ComposerDensity::Tiny {
                right.add_child(self.render_effort_control(appearance));
            }
            right.add_child(make_action());

            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(left)
                .with_child(right.finish())
                .finish()
        };
        let controls: Box<dyn Element> = Box::new(SizeConstraintSwitch::new(
            controls_for(ComposerDensity::Full),
            [
                (
                    SizeConstraintCondition::WidthLessThan(COMPOSER_TINY_MAX_WIDTH),
                    controls_for(ComposerDensity::Tiny),
                ),
                (
                    SizeConstraintCondition::WidthLessThan(COMPOSER_COMPACT_MAX_WIDTH),
                    controls_for(ComposerDensity::Compact),
                ),
            ],
        ));

        let mut card_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::SM);
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
                                font_size: Some(type_ramp::UI.size),
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
        let (composer_border, composer_fill) = if self.drag_active {
            (
                Border::all(border::HAIRLINE_WIDTH).with_border_color(self.render_accent.get()),
                self.render_wash.get(),
            )
        } else {
            (
                Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()),
                theme.surface_1().into_solid(),
            )
        };
        let input_area = card_column.finish();

        // Composer column, top → bottom: input card, then controls that affect
        // sending. Repository/environment chrome moved to the header menu.
        let mut composer_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::SM);
        composer_column.add_child(input_area);
        composer_column.add_child(controls);
        // twarp 17 (PRODUCT 17 §2–§3, §8, §17): one-line voice status / error
        // under the controls; non-blocking, cleared by the next voice action.
        if let Some(status) = &self.voice_status {
            composer_column.add_child(
                appearance
                    .ui_builder()
                    .span(status.clone())
                    .with_style(UiComponentStyles {
                        font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                        font_size: Some(type_ramp::LABEL.size),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );
        }

        let composer_panel = Container::new(composer_column.finish())
            .with_padding(Padding::uniform(spacing::MD))
            .with_background_color(composer_fill)
            .with_border(composer_border)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                COMPOSER_CORNER_RADIUS,
            )))
            .finish();

        let composer = Container::new(composer_panel)
            .with_padding_top(spacing::SM)
            .with_padding_bottom(spacing::MD)
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
        // pill's saved position (#13). Environment menus render inline in the
        // header card; bottom control pills open upward.
        let open_menu = self
            .composer_menu
            .filter(|menu| !(self.header_menu_expanded && menu.opens_downward()))
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
                    vec2f(-TRANSCRIPT_GUTTER, -spacing::SM),
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
                vec2f(0., spacing::SM),
            )
        } else {
            (
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
                vec2f(0., -spacing::SM),
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
        let copy = provider_copy(self.provider);
        let title = appearance
            .ui_builder()
            .span(format!(
                "{} {}",
                copy.unavailable_title, copy.unavailable_body
            ))
            .with_soft_wrap()
            .build()
            .finish();
        let hint = appearance
            .ui_builder()
            .span(copy.install_body.to_owned())
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
                .with_spacing(spacing::MD)
                .with_child(title)
                .with_child(hint)
                .with_child(check)
                .finish(),
        )
        .with_uniform_padding(spacing::LG)
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
            // 7p: the user is looking at the pane — the ✓/✗ tab dot has
            // served its purpose.
            self.clear_tab_attention(ctx);
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

    fn on_blur(&mut self, _blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        self.clear_reply_suggestion(ctx);
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // #10/#11: resolve the tab-derived accent + wash once, for the whole
        // render tree (read via `render_accent` / `render_wash`).
        self.render_accent.set(self.accent(app));
        self.render_wash.set(self.accent_wash(app));
        self.sync_computer_control_chrome(app);

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
        let contents = if self.provider_available() {
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
            // Anchor the menu to the ⋯ trigger in the pane header. `Dismiss`
            // closes it on an outside click without swallowing that click.
            if let Some(menu) = self.render_header_menu(app) {
                stack.add_positioned_overlay_child(
                    menu,
                    OffsetPositioning::offset_from_save_position_element(
                        HEADER_MENU_BUTTON_POSITION_ID,
                        vec2f(0., spacing::SM),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::BottomRight,
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

impl Drop for ClaudeCodeView {
    fn drop(&mut self) {
        self.computer_control.borrow_mut().stop();
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
            ClaudeCodeViewAction::JumpToTurn(turn_start) => {
                self.jump_to_turn(*turn_start, ctx);
                ctx.notify();
            }
            ClaudeCodeViewAction::ToggleToolCard(id) => self.toggle_tool_card(id, ctx),
            ClaudeCodeViewAction::ToggleCompletedTurn(start) => {
                if !self.expanded_completed_turns.remove(start) {
                    self.expanded_completed_turns.insert(*start);
                }
                ctx.notify();
            }
            ClaudeCodeViewAction::OpenArtifactFile(path) => {
                let path = PathBuf::from(path);
                let full_path = if path.is_absolute() {
                    path
                } else {
                    self.cwd.as_ref().map(|cwd| cwd.join(&path)).unwrap_or(path)
                };
                ctx.dispatch_typed_action(&WorkspaceAction::OpenFileInNewTab {
                    full_path,
                    line_and_column: None,
                });
            }
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
            ClaudeCodeViewAction::ToggleVoiceRecording => self.toggle_voice_recording(ctx),
            ClaudeCodeViewAction::ToggleSpeakReplies => self.toggle_speak_replies(ctx),
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
            ClaudeCodeViewAction::CloseHeaderMenu => {
                if self.header_menu_expanded {
                    self.header_menu_expanded = false;
                    self.header_menu_reveal = None;
                    if self.composer_menu.is_some_and(ComposerMenu::opens_downward) {
                        self.composer_menu = None;
                    }
                    ctx.notify();
                }
            }
            ClaudeCodeViewAction::OpenChanges => {
                self.header_menu_expanded = false;
                self.header_menu_reveal = None;
                self.composer_menu = None;
                ctx.dispatch_typed_action(&WorkspaceAction::OpenRightPanel);
                ctx.notify();
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
            ClaudeCodeViewAction::ClearSessionSearch => {
                self.session_search
                    .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
                // The Edited subscription also resets paging; do it here too so
                // the reset doesn't depend on the editor emitting an event for
                // a programmatic clear.
                self.sessions_shown = ZERO_STATE_INITIAL_SESSIONS;
                ctx.notify();
            }
            ClaudeCodeViewAction::ShowMoreRecentSessions => {
                self.sessions_shown = self
                    .sessions_shown
                    .saturating_add(ZERO_STATE_SESSIONS_PER_PAGE);
                ctx.notify();
            }
            ClaudeCodeViewAction::ForkConversation(index) => self.fork_conversation(*index, ctx),
            ClaudeCodeViewAction::CopyResponse(index) => {
                let text = self.reply_text(*index);
                if !text.is_empty() {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                }
            }
            ClaudeCodeViewAction::EditUserMessage(index) => {
                let text = match self.transcript.items().get(*index) {
                    Some(TranscriptItem::User(text)) => text.clone(),
                    _ => return,
                };
                self.input_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(&text, ctx);
                });
                ctx.focus(&self.input_editor);
                ctx.notify();
            }
            ClaudeCodeViewAction::CopyCodeBlock(code) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(code.clone()));
            }
            ClaudeCodeViewAction::ToggleBackgroundPanel => {
                self.toggle_header_menu_section(HeaderMenuSection::Scripts, ctx);
            }
            ClaudeCodeViewAction::ToggleBackgroundScript(id) => {
                if !self.background_expanded_rows.remove(id) {
                    self.background_expanded_rows.insert(id.clone());
                }
                ctx.notify();
            }
            ClaudeCodeViewAction::ClearBackgroundScripts => {
                // Hide every script that isn't still running. We never hide a
                // live one — the user chose "clear non-running", and twarp can't
                // stop a shell it didn't launch.
                let scripts = self.background_scripts();
                for script in scripts.iter() {
                    if !script.state.is_active() {
                        self.background_dismissed.insert(script.id.clone());
                    }
                }
                ctx.notify();
            }
            ClaudeCodeViewAction::ToggleAgentsPanel => {
                self.toggle_header_menu_section(HeaderMenuSection::Agents, ctx);
            }
            ClaudeCodeViewAction::ToggleRawCli => {
                self.header_menu_expanded = false;
                self.toggle_raw_cli(ctx);
            }
            ClaudeCodeViewAction::ToggleAgentRow(id) => {
                if !self.agent_expanded_rows.remove(id) {
                    self.agent_expanded_rows.insert(id.clone());
                }
                ctx.notify();
            }
            ClaudeCodeViewAction::ClearAgents => {
                // Hide every agent that isn't still running. We never hide a
                // live one — the user chose "clear non-running", and twarp
                // can't stop an agent it didn't launch.
                let agent_runs = self.agent_runs();
                for agent in agent_runs.iter() {
                    if !agent.state.is_active() {
                        self.agents_dismissed.insert(agent.id.clone());
                    }
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
            ClaudeCodeCustomAction::ToggleComputerControl => self.toggle_computer_control(ctx),
            ClaudeCodeCustomAction::ToggleHeaderMenu => {
                let opening = !self.header_menu_expanded;
                self.header_menu_expanded = opening;
                if opening {
                    self.refresh_repo_context(ctx);
                }
                // Section expanded state persists across close/reopen; only
                // an in-flight reveal animation is dropped (it would tick a
                // card that is no longer on screen).
                self.header_menu_reveal = None;
                if !self.header_menu_expanded
                    && self.composer_menu.is_some_and(ComposerMenu::opens_downward)
                {
                    self.composer_menu = None;
                }
                ctx.notify();
            }
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // 7c also tears down the live `claude` process here (PRODUCT §8); 7b has
        // no driver, so closing the pane just drops the synthetic transcript.
        self.computer_control.borrow_mut().stop();
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
        self.sync_computer_control_chrome(app);
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
        // PRODUCT §39 (7i): the Raw CLI toggle — embeds the provider's real
        // interactive CLI (resuming this conversation) in place of the chat,
        // and back. Entering is hidden while a turn streams (§42); in raw mode
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
        let theme = appearance.theme();
        // The toggle's inactive label stays neutral gray; only the active
        // segment reads in the tab accent.
        let inactive_color = theme.sub_text_color(theme.background()).into_solid();
        let toggle_border: twarp_core::ui::theme::Fill = self
            .tab_accent(app)
            .map(|accent| ColorU::new(accent.r, accent.g, accent.b, 90).into())
            .unwrap_or_else(|| theme.outline());
        // twarp: the header cluster is now a single ⋯ menu button (plus the
        // flag-gated computer-control entry point). The globe moved to the
        // window titlebar; repository context and activity now live inside
        // the ⋯ button's floating menu.
        //
        // Exception: in raw mode the pane body IS the embedded terminal, so
        // the in-pane menu overlay can't render — the header keeps the
        // segmented toggle there as the always-visible way back (§39/§40).
        let (left_of_overflow, control_container_width) = if raw_mode {
            let segmented_toggle = |chat_label: &str, raw_label: &str| -> Box<dyn Element> {
                let chat_segment = render_mode_segment(
                    chat_label,
                    false, // raw mode: chat is the inactive, clickable way back
                    true,
                    self.chat_ui_button.clone(),
                    accent,
                    wash,
                    inactive_color,
                    appearance,
                );
                let raw_segment = render_mode_segment(
                    raw_label,
                    true,
                    false,
                    self.raw_cli_button.clone(),
                    accent,
                    wash,
                    inactive_color,
                    appearance,
                );
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(2.)
                        .with_child(chat_segment)
                        .with_child(raw_segment)
                        .finish(),
                )
                .with_padding(Padding::uniform(2.))
                .with_border(Border::all(1.).with_border_fill(toggle_border.clone()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                // #5: breathing room between the toggle and the close ✕.
                .with_margin_right(10.)
                .finish()
            };
            let left_of_overflow = Shrinkable::new(
                1.,
                Box::new(SizeConstraintSwitch::new(
                    segmented_toggle("Chat UI", "Raw CLI"),
                    [(
                        SizeConstraintCondition::WidthLessThan(130.),
                        segmented_toggle("Chat", "CLI"),
                    )],
                )),
            )
            .finish();
            (left_of_overflow, 210.)
        } else {
            let computer_control_button = self.render_computer_control_button(app, false);
            let has_computer_control_button = computer_control_button.is_some();
            let assemble_cluster =
                |computer_control_button: Option<Box<dyn Element>>| -> Box<dyn Element> {
                    let mut controls = Flex::row()
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(6.);
                    if let Some(button) = computer_control_button {
                        controls = controls.with_child(button);
                    }
                    controls
                        .with_child(
                            Container::new(self.render_header_menu_button(app))
                                // Breathing room between the button and the close ✕.
                                .with_margin_right(10.)
                                .finish(),
                        )
                        .finish()
                };
            let full_cluster = assemble_cluster(computer_control_button);
            let compact_cluster = assemble_cluster(self.render_computer_control_button(app, true));
            let full_cluster_width = 50.
                + if has_computer_control_button {
                    132.
                } else {
                    0.
                };
            let left_of_overflow = Shrinkable::new(
                1.,
                Box::new(SizeConstraintSwitch::new(
                    full_cluster,
                    [(
                        SizeConstraintCondition::WidthLessThan(full_cluster_width),
                        compact_cluster,
                    )],
                )),
            )
            .finish();
            let control_container_width = 130.
                + if has_computer_control_button {
                    132.
                } else {
                    0.
                };
            (left_of_overflow, control_container_width)
        };
        HeaderContent::Standard(StandardHeader {
            title: provider_copy(self.provider).pane_title.to_owned(),
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
                control_container_width: Some(control_container_width),
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
                    None,
                );
                // #8: any images sent with this turn preview above the bubble.
                let row = match self.sent_images.get(&index) {
                    Some(paths) if !paths.is_empty() => Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_spacing(6.)
                        .with_child(self.render_sent_images(paths))
                        .with_child(bubble)
                        .finish(),
                    _ => bubble,
                };
                // Hover "Edit" affordance below the bubble — same
                // stable-layout visible/hidden toggle as the Fork affordance
                // under assistant responses (see render_assistant_response).
                let edit_visible = self.render_edit_affordance(index, true, app);
                let edit_hidden = self.render_edit_affordance(index, false, app);
                let row_mouse = pooled_mouse_state(&self.user_row_mouse, index);
                Hoverable::new(row_mouse, move |state| {
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_child(row)
                        .with_child(if state.is_hovered() {
                            edit_visible
                        } else {
                            edit_hidden
                        })
                        .finish()
                })
                .with_propagate_drag()
                .finish()
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
            // rather than a generic tool card. claude holds the turn on the
            // gating `can_use_tool` while the user picks; submitting answers it
            // inline (`tool_use_id` ties the card to the held request).
            TranscriptItem::Tool {
                id, name, input, ..
            } if name == "AskUserQuestion"
                && input.get("questions").and_then(|v| v.as_array()).is_some() =>
            {
                self.render_question_card(index, id, input, appearance)
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
            .with_spacing(spacing::SM)
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
                        font_size: Some(type_ramp::UI.size),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::MD)
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
                            font_size: Some(type_ramp::LABEL.size),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding(Padding::uniform(spacing::SM))
                .with_background_color(theme.surface_1().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
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
                            font_size: Some(type_ramp::LABEL.size),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }
        }

        Container::new(column.finish())
            .with_padding(Padding::uniform(spacing::LG))
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::XS)
            .with_margin_left(TRANSCRIPT_LEFT_MARGIN)
            .with_margin_right(spacing::XL)
            .with_background_color(surface.into_solid())
            .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
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
            .with_spacing(spacing::SM)
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
                        font_size: Some(type_ramp::UI.size),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::MD)
            .with_child(header)
            .with_child(render_markdown_body(plan, text_color, appearance, None));

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
            .with_padding(Padding::uniform(spacing::LG))
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::XS)
            .with_margin_left(TRANSCRIPT_LEFT_MARGIN)
            .with_margin_right(spacing::XL)
            .with_background_color(surface.into_solid())
            .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
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
        tool_use_id: &str,
        input: &serde_json::Value,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // Tool-card path (PRODUCT §1): controls are live while claude is holding
        // the turn on this question's `can_use_tool` (the pending permission) —
        // submitting answers it inline. They're also live while the session is
        // idle, so a question left unanswered when the turn somehow ended can
        // still be replied to (sent as the next turn).
        let questions = parse_questions(input);
        let pending = self.pending_question_permission.contains_key(tool_use_id);
        let answered = self.question_submitted.contains_key(&index);
        let interactive = !answered && (pending || !self.streaming);
        self.render_question_card_inner(index, &questions, interactive, false, appearance)
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
        // Once a card is answered the live selection is moved into
        // `question_submitted`; fall back to it so the chosen options stay
        // visibly checked after the answer is on its way.
        let selected = self
            .question_selected
            .get(&index)
            .or_else(|| self.question_submitted.get(&index));

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
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
                    .span(question_card_title(questions).to_owned())
                    .with_line_height_ratio(type_ramp::UI.line_height)
                    .with_style(UiComponentStyles {
                        font_color: Some(text_color),
                        font_size: Some(type_ramp::UI.size),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::MD)
            .with_child(header);

        for question in questions {
            let mut block = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(spacing::SM);
            // A single question promotes its short semantic header into the
            // card header above. Multi-question cards need each header beside
            // its own prompt so none of the provider payload is discarded.
            if questions.len() > 1 && !question.header.trim().is_empty() {
                block.add_child(
                    appearance
                        .ui_builder()
                        .span(question.header.clone())
                        .with_soft_wrap()
                        .with_line_height_ratio(type_ramp::LABEL.line_height)
                        .with_style(UiComponentStyles {
                            font_color: Some(muted),
                            font_size: Some(type_ramp::LABEL.size),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }
            if !question.question.trim().is_empty() {
                block.add_child(
                    appearance
                        .ui_builder()
                        .span(question.question.clone())
                        .with_soft_wrap()
                        .with_line_height_ratio(type_ramp::PROSE.line_height)
                        .with_style(UiComponentStyles {
                            font_color: Some(text_color),
                            font_size: Some(type_ramp::PROSE.size),
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
                    .with_spacing(spacing::SM)
                    .with_child(
                        appearance
                            .ui_builder()
                            .span(marker.to_owned())
                            .with_line_height_ratio(type_ramp::PROSE.line_height)
                            .with_style(UiComponentStyles {
                                font_color: Some(if is_selected { accent } else { muted }),
                                font_size: Some(type_ramp::PROSE.size),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    );
                let mut label_col = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(spacing::XXS)
                    .with_child(
                        appearance
                            .ui_builder()
                            .span(option.label.clone())
                            .with_soft_wrap()
                            .with_line_height_ratio(type_ramp::UI.line_height)
                            .with_style(UiComponentStyles {
                                font_color: Some(text_color),
                                font_size: Some(type_ramp::UI.size),
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
                                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                                .with_style(UiComponentStyles {
                                    font_color: Some(muted),
                                    font_size: Some(type_ramp::CAPTION.size),
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
                    .with_padding_left(spacing::SM)
                    .with_padding_right(spacing::SM)
                    .with_padding_top(spacing::SM)
                    .with_padding_bottom(spacing::SM)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
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
            .with_padding(Padding::uniform(spacing::LG))
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::XS)
            .with_margin_left(TRANSCRIPT_LEFT_MARGIN)
            .with_margin_right(spacing::XL)
            .with_background_color(surface.into_solid())
            .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
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

/// The card title should carry the provider's semantic label when there is one
/// question. A multi-question card keeps the generic plural title because each
/// question renders its own header beside its prompt.
fn question_card_title(questions: &[ParsedQuestion]) -> &str {
    match questions {
        [question] if !question.header.trim().is_empty() => question.header.trim(),
        [_, _, ..] => "Questions",
        _ => "Question",
    }
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

/// Whether a `can_use_tool` permission prompt for `tool` should be *held open*
/// for the inline question card to answer, rather than surfaced as a generic
/// Allow/Deny Permission card (PRODUCT §1). Only `AskUserQuestion`: claude
/// blocks the turn on this permission, and answering it with the user's picks
/// (as the tool's `answers`) lets the model continue in the same turn — so the
/// question genuinely waits for the user instead of being auto-dismissed.
fn should_hold_question_permission(tool: &str) -> bool {
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

/// twarp 17 §30: how often a live transcription pass may start (stretched as
/// the recording grows — see `pump_live_transcription`).
const LIVE_STT_INTERVAL: Duration = Duration::from_millis(2500);

/// twarp 17 §33: the karaoke state for the sentence currently being spoken —
/// computed per render from the player position, applied as background
/// highlights on the rendered prose.
struct KaraokeHighlight {
    /// Transcript index of the assistant item being spoken.
    item_index: usize,
    /// The (prose-form, trimmed) sentence currently audible.
    sentence: String,
    /// Chars of the sentence estimated already spoken (the "fill").
    spoken_chars: usize,
    /// Wash behind the whole active sentence.
    sentence_color: ColorU,
    /// Stronger wash behind the spoken prefix.
    spoken_color: ColorU,
}

fn render_message_row(
    is_user: bool,
    icon_svg: &'static str,
    text: &str,
    accent: ColorU,
    appearance: &Appearance,
    karaoke: Option<&KaraokeHighlight>,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    if is_user {
        // Quiet outgoing bubble: a neutral wash keeps the tab accent reserved
        // for identity/status and matches the completed-result hierarchy.
        let bubble_fill = theme.surface_overlay_2();
        // `surface_overlay_2` can be translucent. Asking the theme for text
        // contrast against that uncomposited fill can choose light text even
        // though the visible bubble is light, producing white-on-gray turns.
        // Use the canvas foreground role so user messages stay readable in
        // both light and dark themes.
        let text_color = theme.main_text_color(theme.background()).into_solid();
        let bubble = Container::new(render_markdown_body(text, text_color, appearance, None))
            .with_padding(
                Padding::uniform(spacing::SM)
                    .with_left(spacing::LG)
                    .with_right(spacing::LG),
            )
            .with_background(bubble_fill)
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
        .with_margin_top(spacing::XS)
        .with_margin_bottom(spacing::XS)
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
                .with_margin_right(spacing::MD)
                .with_margin_top(spacing::XXS)
                .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Container::new(render_markdown_body(text, text_color, appearance, karaoke))
                    .finish(),
            )
            .finish(),
        );

    Container::new(row.finish())
        .with_padding(Padding::uniform(spacing::LG))
        .with_margin_top(spacing::XS)
        .with_margin_bottom(spacing::XS)
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
    karaoke: Option<&KaraokeHighlight>,
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
    // §33: the karaoke highlight lands on the first rendered line containing
    // the spoken sentence; applied at most once across segments.
    let mut karaoke = karaoke;
    for segment in split_markdown_segments(formatted) {
        let child = match segment {
            MarkdownSegment::Prose(formatted_text) => {
                let line_texts: Vec<String> = karaoke
                    .map(|_| {
                        formatted_text
                            .lines
                            .iter()
                            .map(|line| line.raw_text())
                            .collect()
                    })
                    .unwrap_or_default();
                let mut element = FormattedTextElement::new(
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
                .with_line_height_ratio(type_ramp::PROSE.line_height)
                // The transcript prose is read-only, but the user still needs to
                // highlight + copy it (same as the tool-output body in
                // inline_action.rs). FormattedTextElement carries the selection +
                // copy machinery; it's just off by default.
                .set_selectable(true);
                if let Some(active) = karaoke {
                    if apply_karaoke_highlight(&mut element, &line_texts, active) {
                        karaoke = None;
                    }
                }
                element.finish()
            }
            MarkdownSegment::Code(code) => render_code_block(&code, appearance),
            MarkdownSegment::Table(table) => render_table(table, appearance),
            MarkdownSegment::Rule => render_thematic_break(appearance),
            MarkdownSegment::Quote(quote) => render_blockquote(quote, appearance),
        };
        column.add_child(child);
    }
    column.finish()
}

/// Paint the §33 karaoke washes over the rendered line containing the spoken
/// sentence: a light wash over the whole sentence, a stronger one over the
/// estimated spoken prefix. Char-index based, matching how hyperlink styles
/// address the same lines. Returns whether the sentence was found.
fn apply_karaoke_highlight(
    element: &mut FormattedTextElement,
    line_texts: &[String],
    karaoke: &KaraokeHighlight,
) -> bool {
    let sentence_chars: Vec<char> = karaoke.sentence.chars().collect();
    if sentence_chars.is_empty() {
        return false;
    }
    for (line_index, line_text) in line_texts.iter().enumerate() {
        let line_chars: Vec<char> = line_text.trim_end_matches('\n').chars().collect();
        if line_chars.len() < sentence_chars.len() {
            continue;
        }
        let Some(start) = (0..=line_chars.len() - sentence_chars.len())
            .find(|&at| line_chars[at..at + sentence_chars.len()] == sentence_chars[..])
        else {
            continue;
        };
        let mut styles = Vec::new();
        let spoken_end = start + karaoke.spoken_chars.min(sentence_chars.len());
        if start < spoken_end {
            styles.push(HighlightedRange {
                highlight_indices: (start..spoken_end).collect(),
                highlight: Highlight::new()
                    .with_text_style(TextStyle::new().with_background_color(karaoke.spoken_color)),
            });
        }
        if spoken_end < start + sentence_chars.len() {
            styles.push(HighlightedRange {
                highlight_indices: (spoken_end..start + sentence_chars.len()).collect(),
                highlight: Highlight::new().with_text_style(
                    TextStyle::new().with_background_color(karaoke.sentence_color),
                ),
            });
        }
        element.add_styles(line_index, styles);
        return true;
    }
    false
}

/// Render a GFM table (PRODUCT §13) as a real grid — a bordered, rounded box
/// (mirroring [`render_code_block`]) holding a header row over body rows. The
/// parser hands us each cell as a [`FormattedTextInline`], so cells keep their
/// inline styling (bold/code/links). Columns size to content — short
/// identifier columns are pinned, prose columns flex (see
/// [`table_column_sizings`]); per-column alignment comes from the GFM `:---:`
/// separators.
fn render_table(table: FormattedTable, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let column_count = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    let align_of = |col: usize| table.alignments.get(col).copied().unwrap_or_default();

    // Size columns to their content rather than forcing every column to an equal
    // share of the pane. A tiny "Built?" column no longer steals half the width
    // from a paragraph-heavy "Reality" column, which keeps wide columns readable
    // instead of cramming/clipping them.
    let col_sizings = table_column_sizings(&table, column_count);

    // Semibold header text over a hairline reads as a header without the heavy
    // full-width fill band the first cut used (the fill competed with the
    // surrounding prose; the philosophy groups with lines last, fills rarely).
    let mut grid = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    grid.add_child(render_table_row(
        table.headers,
        column_count,
        &col_sizings,
        &align_of,
        true,
        appearance,
    ));
    for row in table.rows {
        // A thin divider under the header and between body rows, so the grid
        // reads as rows without heavy full-grid lines.
        grid.add_child(
            ConstrainedBox::new(
                Container::new(Flex::row().finish())
                    .with_background_color(theme.outline().into_solid())
                    .finish(),
            )
            .with_height(border::HAIRLINE_WIDTH)
            .finish(),
        );
        grid.add_child(render_table_row(
            row,
            column_count,
            &col_sizings,
            &align_of,
            false,
            appearance,
        ));
    }

    Container::new(grid.finish())
        .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .with_margin_top(spacing::MD)
        .with_margin_bottom(spacing::MD)
        .finish()
}

/// How one table column gets its width.
enum TableColumnSizing {
    /// Short identifier-style columns ("THI-759", "Valid — keep") get the exact
    /// width their longest cell needs, so they never soft-wrap a ticket id
    /// across three lines just because a prose column next door is long.
    Fixed(f32),
    /// Prose columns share the remaining width proportionally to content.
    Weight(f32),
}

/// Per-column sizing derived from cell content length. The plain-text length of
/// each cell is a cheap proxy for how much width a column wants: columns whose
/// longest cell is short are pinned to that content width; the rest share the
/// leftover space as flex weights, clamped so a single very long cell never
/// starves its neighbours.
fn table_column_sizings(table: &FormattedTable, column_count: usize) -> Vec<TableColumnSizing> {
    /// Longest-cell length (chars) at or under which a column is pinned to its
    /// content width instead of flexing.
    const SHORT_COLUMN_MAX_CHARS: usize = 16;
    const MIN_WEIGHT: usize = 4;
    const MAX_WEIGHT: usize = 60;
    /// Average glyph width as a fraction of the font size — generous enough for
    /// the UI font's wider glyphs so pinned columns still fit on one line.
    const CHAR_WIDTH_RATIO: f32 = 0.62;

    let inline_len = |cell: &FormattedTextInline| -> usize {
        cell.iter().map(|frag| frag.text.chars().count()).sum()
    };
    let mut widths = vec![0usize; column_count];
    let mut consider = |row: &[FormattedTextInline]| {
        for (col, cell) in row.iter().take(column_count).enumerate() {
            widths[col] = widths[col].max(inline_len(cell));
        }
    };
    consider(&table.headers);
    for row in &table.rows {
        consider(row);
    }
    // If every column is short the pinning would leave the grid narrower than
    // its border with nothing to flex, so keep at least one weighted column.
    let widest = widths.iter().copied().max().unwrap_or(0);
    widths
        .into_iter()
        .map(|len| {
            if len <= SHORT_COLUMN_MAX_CHARS && len < widest {
                TableColumnSizing::Fixed(
                    len as f32 * BODY_FONT_SIZE * CHAR_WIDTH_RATIO + 2. * spacing::MD,
                )
            } else {
                TableColumnSizing::Weight(len.clamp(MIN_WEIGHT, MAX_WEIGHT) as f32)
            }
        })
        .collect()
}

/// One table row: `column_count` cells whose widths follow `col_sizings` (missing
/// trailing cells are padded blank so short rows still line up under the header).
fn render_table_row(
    mut cells: Vec<FormattedTextInline>,
    column_count: usize,
    col_sizings: &[TableColumnSizing],
    align_of: &impl Fn(usize) -> TableAlignment,
    header: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    cells.resize(column_count, Vec::new());
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (col, cell) in cells.into_iter().enumerate() {
        let cell = render_table_cell(cell, align_of(col), header, appearance);
        row.add_child(match col_sizings.get(col) {
            Some(TableColumnSizing::Fixed(width)) => {
                ConstrainedBox::new(cell).with_width(*width).finish()
            }
            Some(TableColumnSizing::Weight(weight)) => Expanded::new(*weight, cell).finish(),
            None => Expanded::new(1., cell).finish(),
        });
    }
    row.finish()
}

/// One table cell: the inline content, padded, horizontally aligned per the
/// column's GFM alignment. Header cells read in the strong text color.
fn render_table_cell(
    mut inline: FormattedTextInline,
    alignment: TableAlignment,
    header: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let inline_code_bg = theme.surface_3().into_solid();
    let text_color = if header {
        // Semibold + strong color carries the header now that it has no fill
        // band behind it.
        for fragment in &mut inline {
            fragment.styles.weight = Some(CustomWeight::Semibold);
        }
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
    .with_line_height_ratio(type_ramp::PROSE.line_height)
    .with_inline_code_properties(
        Some(theme.nonactive_ui_text_color().into()),
        Some(inline_code_bg),
    )
    .register_default_click_handlers(|url, ctx, _| {
        ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenUrl(url));
    })
    .set_selectable(true)
    .finish();
    // The content must be placed so it still receives the cell's bounded width —
    // otherwise the text is measured at unbounded width, never wraps, and the
    // wide columns overflow and clip. A bare Container (Left) and `Align`
    // (Center/Right) both forward the width constraint to the child; the old
    // inner `Flex::row` did not, which is what cut off the rightmost columns.
    let aligned: Box<dyn Element> = match alignment {
        TableAlignment::Left => element,
        TableAlignment::Center => Align::new(element).top_center().finish(),
        TableAlignment::Right => Align::new(element).top_right().finish(),
    };
    Container::new(aligned)
        .with_padding_left(spacing::MD)
        .with_padding_right(spacing::MD)
        .with_padding_top(spacing::SM)
        .with_padding_bottom(spacing::SM)
        .finish()
}

thread_local! {
    /// Hover state for the per-code-block copy buttons, keyed by a hash of the
    /// block's contents. `render_code_block` is a free function shared by the
    /// message and thinking renderers, so the pool lives beside
    /// [`MARKDOWN_CACHE`] rather than on any one view; identical blocks
    /// sharing a slot is harmless (the state only drives hover paint).
    static CODE_COPY_MOUSE: std::cell::RefCell<HashMap<u64, MouseStateHandle>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Bound the [`CODE_COPY_MOUSE`] pool: past the cap the whole map resets (a
/// momentary hover-state loss, invisible in practice) instead of growing for
/// the life of the process.
const CODE_COPY_MOUSE_CAP: usize = 512;

fn code_copy_mouse_state(code: &str) -> MouseStateHandle {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    let key = hasher.finish();
    CODE_COPY_MOUSE.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() >= CODE_COPY_MOUSE_CAP && !pool.contains_key(&key) {
            pool.clear();
        }
        pool.entry(key).or_default().clone()
    })
}

/// Port of `ai_assistant::transcript`'s code-block branch, minus the Warp-AI
/// affordances (paste-in-terminal / save-as-workflow / per-block selection)
/// that don't apply to a read-only Claude Code transcript: a bordered,
/// rounded, monospace box with a header strip carrying the fence's language
/// label and a copy button.
fn render_code_block(
    code: &markdown_parser::CodeBlockText,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let muted = theme.nonactive_ui_text_color().into_solid();

    // Fence info strings can carry extra metadata ("python path=… start=1");
    // only the first word is the language label.
    let lang = code.lang.split_whitespace().next().unwrap_or("");
    let mut header = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);
    header.add_child(
        appearance
            .ui_builder()
            .span(lang.to_owned())
            .with_style(UiComponentStyles {
                font_color: Some(muted),
                font_size: Some(type_ramp::LABEL.size),
                ..Default::default()
            })
            .build()
            .finish(),
    );
    let copy_icon = ConstrainedBox::new(Icon::new(COPY_ICON_SVG_PATH, muted).finish())
        .with_width(12.)
        .with_height(12.)
        .finish();
    let code_text = code.code.clone();
    header.add_child(
        Hoverable::new(code_copy_mouse_state(&code.code), move |mouse| {
            Container::new(copy_icon)
                .with_uniform_padding(spacing::XS)
                .with_background_color(if mouse.is_hovered() {
                    theme.surface_3().into_solid()
                } else {
                    ColorU::new(0, 0, 0, 0)
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::CopyCodeBlock(code_text.clone()));
        })
        .finish(),
    );

    let divider = ConstrainedBox::new(
        Container::new(Flex::row().finish())
            .with_background_color(theme.outline().into_solid())
            .finish(),
    )
    .with_height(border::HAIRLINE_WIDTH)
    .finish();
    let body = Container::new(
        appearance
            .ui_builder()
            .wrappable_text(code.code.clone(), true)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.monospace_font_family()),
                font_size: Some(CODE_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_uniform_padding(spacing::MD)
    .finish();

    let column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(
            // Tighter vertical padding than the body so the strip reads as
            // chrome, not content; identical horizontal padding per the Card
            // anatomy.
            Container::new(header.finish())
                .with_padding_left(spacing::MD)
                .with_padding_right(spacing::MD)
                .with_padding_top(spacing::XS)
                .with_padding_bottom(spacing::XS)
                .finish(),
        )
        .with_child(divider)
        .with_child(body)
        .finish();

    Container::new(column)
        .with_border(Border::all(border::HAIRLINE_WIDTH).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .with_margin_top(spacing::MD)
        .with_margin_bottom(spacing::MD)
        .finish()
}

/// A markdown `---` thematic break: the hairline the parser promises but
/// `FormattedTextElement` would flatten to a blank line.
fn render_thematic_break(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        ConstrainedBox::new(
            Container::new(Flex::row().finish())
                .with_background_color(theme.outline().into_solid())
                .finish(),
        )
        .with_height(border::HAIRLINE_WIDTH)
        .finish(),
    )
    .with_margin_top(spacing::LG)
    .with_margin_bottom(spacing::LG)
    .finish()
}

/// A `> ` blockquote run: a 2px neutral bar down the left, muted prose to the
/// right — the standard chat-app callout shape, kept neutral so it never
/// competes with semantic color.
fn render_blockquote(quote: FormattedText, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let element = FormattedTextElement::new(
        quote,
        BODY_FONT_SIZE,
        appearance.ui_font_family(),
        appearance.monospace_font_family(),
        theme.nonactive_ui_text_color().into_solid(),
        HighlightedHyperlink::default(),
    )
    .with_inline_code_properties(
        Some(theme.nonactive_ui_text_color().into()),
        Some(theme.surface_3().into_solid()),
    )
    .register_default_click_handlers(|url, ctx, _| {
        ctx.dispatch_typed_action(ClaudeCodeViewAction::OpenUrl(url));
    })
    .with_line_height_ratio(type_ramp::PROSE.line_height)
    .set_selectable(true)
    .finish();
    let bar = Container::new(Flex::column().finish())
        .with_background_color(theme.outline().into_solid())
        .finish();
    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(ConstrainedBox::new(bar).with_width(2.).finish())
        .with_child(
            Expanded::new(
                1.,
                Container::new(element)
                    .with_padding_left(spacing::MD)
                    .finish(),
            )
            .finish(),
        )
        .finish();
    Container::new(row)
        .with_margin_top(spacing::SM)
        .with_margin_bottom(spacing::SM)
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
                font_size: Some(type_ramp::LABEL.size),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_padding_top(spacing::XXS)
    .with_padding_bottom(spacing::SM)
    .with_padding_left(TRANSCRIPT_LEFT_MARGIN)
    .with_padding_right(spacing::XL)
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
    .with_padding_top(spacing::SM)
    .with_padding_bottom(spacing::SM)
    .with_padding_left(TRANSCRIPT_LEFT_MARGIN)
    .with_padding_right(spacing::XL)
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
    /// A `---` thematic break. The parser emits it, but `FormattedTextElement`
    /// flattens it to a blank line — split it out so it paints as a hairline.
    Rule,
    /// A `> `-prefixed blockquote run. The parser has no quote support, so the
    /// lines arrive as plain paragraphs carrying the literal `>` marker; the
    /// splitter strips the markers and groups consecutive quote lines here.
    Quote(FormattedText),
}

/// Port of `ai_assistant::utils::translate_formatted_text_into_markdown_segments`
/// (AI-agnostic): split a parsed [`FormattedText`] into code-block vs contiguous
/// non-code runs so code blocks can render in their own box.
fn split_markdown_segments(formatted: FormattedText) -> Vec<MarkdownSegment> {
    let mut segments = Vec::new();
    let mut running_prose: Vec<FormattedTextLine> = Vec::new();
    let mut running_quote: Vec<FormattedTextLine> = Vec::new();

    let mut flush_prose = |running: &mut Vec<FormattedTextLine>, segments: &mut Vec<_>| {
        if !running.is_empty() {
            segments.push(MarkdownSegment::Prose(FormattedText::new_trimmed(
                std::mem::take(running),
            )));
        }
    };
    let flush_quote = |running: &mut Vec<FormattedTextLine>,
                       segments: &mut Vec<MarkdownSegment>| {
        if !running.is_empty() {
            segments.push(MarkdownSegment::Quote(FormattedText::new_trimmed(
                std::mem::take(running),
            )));
        }
    };

    for line in formatted.lines {
        // A consecutive run of `> ` paragraphs groups into one quote segment;
        // any other line type ends the run.
        if let Some(stripped) = strip_blockquote_marker(&line) {
            flush_prose(&mut running_prose, &mut segments);
            running_quote.push(stripped);
            continue;
        }
        // A blank line between two quote lines keeps the quote together (the
        // parser hands paragraphs over with interleaved breaks).
        if matches!(line, FormattedTextLine::LineBreak) && !running_quote.is_empty() {
            running_quote.push(line);
            continue;
        }
        flush_quote(&mut running_quote, &mut segments);
        match line {
            FormattedTextLine::CodeBlock(mut code) => {
                flush_prose(&mut running_prose, &mut segments);
                code.code = code.code.trim().to_string();
                segments.push(MarkdownSegment::Code(code));
            }
            FormattedTextLine::Table(table) => {
                flush_prose(&mut running_prose, &mut segments);
                segments.push(MarkdownSegment::Table(table));
            }
            FormattedTextLine::HorizontalRule => {
                flush_prose(&mut running_prose, &mut segments);
                segments.push(MarkdownSegment::Rule);
            }
            other => running_prose.push(other),
        }
    }
    flush_quote(&mut running_quote, &mut segments);
    flush_prose(&mut running_prose, &mut segments);
    segments
}

/// If `line` is a paragraph carrying a literal `> ` blockquote marker (the
/// parser has no quote grammar, so the marker survives into the first inline
/// fragment), return a copy with the marker stripped. `None` for everything
/// else.
fn strip_blockquote_marker(line: &FormattedTextLine) -> Option<FormattedTextLine> {
    let FormattedTextLine::Line(fragments) = line else {
        return None;
    };
    let first = fragments.first()?;
    let rest = first.text.strip_prefix('>')?;
    // `>` glued to non-space text (e.g. a pasted `>>>` shell prompt) is not a
    // quote; require the marker to be `>` alone or `> …`.
    let rest = if rest.is_empty() {
        rest
    } else {
        rest.strip_prefix(' ')?
    };
    let mut fragments = fragments.clone();
    fragments[0].text = rest.to_owned();
    Some(FormattedTextLine::Line(fragments))
}

/// One plain coloured text segment used by compact controls and status rows.
fn context_segment(appearance: &Appearance, text: String, color: ColorU) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .span(text)
        .with_style(UiComponentStyles {
            font_color: Some(color),
            font_size: Some(type_ramp::LABEL.size),
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
    inactive_color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // #10/#11: the active section reads in the tab accent over a faint wash;
    // the inactive-but-clickable section follows the tab colour too (muted)
    // when one is set, so the whole toggle tints with the tab.
    let color = if active {
        accent
    } else if clickable {
        inactive_color
    } else {
        theme.nonactive_ui_text_color().into_solid()
    };
    let mut chip = Container::new(context_segment(appearance, label.to_owned(), color))
        .with_padding_left(spacing::SM)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::XS)
        .with_padding_bottom(spacing::XS)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)));
    if active {
        chip = chip.with_background_color(wash);
    }
    let chip = chip.finish();
    if clickable {
        Hoverable::new(mouse, move |_| chip)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
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
    on_click: impl Fn(&mut twarpui::EventContext) + 'static,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let row = Container::new(context_segment(appearance, label.to_owned(), color))
        .with_padding_left(spacing::SM)
        .with_padding_right(spacing::SM)
        .with_padding_top(spacing::XS)
        .with_padding_bottom(spacing::XS)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CHIP)))
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
                font_size: Some(type_ramp::LABEL.size),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_padding_left(spacing::SM)
    .with_padding_right(spacing::SM)
    .with_padding_top(spacing::XS)
    .with_padding_bottom(spacing::XS)
    .with_margin_right(spacing::SM)
    .with_background_color(bg)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_CORNER_RADIUS)))
    .finish()
}

/// The §25 permission-mode selector pill: the muted pill chrome with hover +
/// pointer affordance; a click dispatches the cycle action. The label carries
/// a chevron-ish suffix so it reads as a control, not a static chip.
fn render_clickable_pill(
    label: &str,
    mouse_state: MouseStateHandle,
    on_click: impl Fn(&mut twarpui::EventContext) + 'static,
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
                font_size: Some(type_ramp::LABEL.size),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_padding_left(spacing::SM)
    .with_padding_right(spacing::SM)
    .with_padding_top(spacing::XS)
    .with_padding_bottom(spacing::XS)
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
    Container::new(trigger)
        .with_margin_right(spacing::SM)
        .finish()
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

fn claude_mcp_config_json(session_id: &str, app: &AppContext) -> Option<String> {
    #[cfg(target_family = "wasm")]
    {
        let _ = (session_id, app);
        None
    }

    #[cfg(not(target_family = "wasm"))]
    {
        let mut servers = serde_json::Map::new();
        // twarp 14j: session-scoped endpoint — browser tools target/open
        // panes in THIS session's tab and stay bound to them across moves.
        merge_mcp_servers(
            &mut servers,
            crate::browser_mcp::BrowserMcpBridge::as_ref(app)
                .mcp_config_json_for_session(session_id),
        );
        if FeatureFlag::LocalComputerUse.is_enabled() {
            merge_mcp_servers(
                &mut servers,
                crate::computer_control::ComputerControlMcpBridge::as_ref(app).mcp_config_json(),
            );
        }

        (!servers.is_empty()).then(|| serde_json::json!({ "mcpServers": servers }).to_string())
    }
}

#[cfg(not(target_family = "wasm"))]
fn merge_mcp_servers(
    servers: &mut serde_json::Map<String, serde_json::Value>,
    config: Option<String>,
) {
    let Some(config) = config else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&config) else {
        return;
    };
    let Some(config_servers) = value
        .get_mut("mcpServers")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    servers.extend(std::mem::take(config_servers));
}

/// Which of the ⋯ header menu's inline sections a reveal animation targets.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderMenuSection {
    Agents,
    Scripts,
}

/// In-flight reveal animation for a header-menu section body. warpui has no
/// transition framework (`render()` never re-runs on its own), so
/// [`ClaudeCodeView::tick_header_menu_reveal`] drives this with the
/// self-rearming `notify()` timer pattern: each tick computes an eased
/// fraction from the `Instant` start time until the ~150ms ease is done —
/// `left_panel_slide::LeftPanelSlide`'s shape, vertical instead of
/// horizontal.
struct SectionReveal {
    started_at: Instant,
    from: f32,
    to: f32,
}

impl SectionReveal {
    fn new(from: f32, to: f32) -> Self {
        Self {
            started_at: Instant::now(),
            from,
            to,
        }
    }

    /// Current visible fraction of the section body's height, eased
    /// (ease-out cubic, the sidebar slide's curve).
    fn fraction(&self) -> f32 {
        let t = self.started_at.elapsed().as_secs_f32()
            / HEADER_MENU_REVEAL_DURATION.as_secs_f32().max(f32::EPSILON);
        let inv = 1.0 - t.clamp(0.0, 1.0);
        let eased = 1.0 - inv * inv * inv;
        self.from + (self.to - self.from) * eased
    }

    fn is_done(&self) -> bool {
        self.started_at.elapsed() >= HEADER_MENU_REVEAL_DURATION
    }
}

/// Clips its child to `fraction` of the child's natural height so a header-menu
/// section's rows ease open (and closed) in place — the vertical sibling of
/// `left_panel_slide::SlideClip`, revealing top-down without shifting the
/// child. The child is laid out with its unmodified incoming constraint; only
/// the reported height (and the paint clip) shrink to the fraction.
struct RevealClip {
    child: Box<dyn Element>,
    /// Visible fraction of the child's height, in `[0, 1]`.
    fraction: f32,
    origin: Option<twarpui::elements::Point>,
    size: Option<pathfinder_geometry::vector::Vector2F>,
}

impl RevealClip {
    fn new(child: Box<dyn Element>, fraction: f32) -> Self {
        Self {
            child,
            fraction: fraction.clamp(0.0, 1.0),
            origin: None,
            size: None,
        }
    }
}

impl Element for RevealClip {
    fn layout(
        &mut self,
        mut constraint: twarpui::SizeConstraint,
        ctx: &mut twarpui::LayoutContext,
        app: &AppContext,
    ) -> pathfinder_geometry::vector::Vector2F {
        // The child sizes itself to its full height; only the animated
        // visible height is reported upward. Relax the min so the column
        // can't force us taller than the animated height.
        constraint.min.set_y(0.0);
        let child_size = self.child.layout(constraint, ctx, app);
        let size = vec2f(child_size.x(), (child_size.y() * self.fraction).round());
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut twarpui::AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(
        &mut self,
        origin: pathfinder_geometry::vector::Vector2F,
        ctx: &mut twarpui::PaintContext,
        app: &AppContext,
    ) {
        let Some(size) = self.size else {
            return;
        };
        let origin_point = twarpui::elements::Point::from_vec2f(origin, ctx.scene.z_index());
        self.origin = Some(origin_point);
        if size.y() <= 0.0 {
            return;
        }
        // Clip to the visible strip and paint the child top-anchored, so rows
        // reveal downward from under the section header.
        let Some(bounds) = ctx.scene.visible_rect(origin_point, size) else {
            return;
        };
        ctx.scene
            .start_layer(twarpui::ClipBounds::BoundedBy(bounds));
        self.child.paint(origin, ctx, app);
        ctx.scene.stop_layer();
    }

    fn dispatch_event(
        &mut self,
        event: &twarpui::event::DispatchedEvent,
        ctx: &mut twarpui::EventContext,
        app: &AppContext,
    ) -> bool {
        // The reveal lasts ~150ms; suppress child hit-testing while partially
        // hidden so clicks can't land on half-revealed rows.
        if self.fraction >= 1.0 {
            self.child.dispatch_event(event, ctx, app)
        } else {
            false
        }
    }

    fn size(&self) -> Option<pathfinder_geometry::vector::Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<twarpui::elements::Point> {
        self.origin
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_raw_cli_flags, format_metrics_line, parse_markdown_cached, parse_questions,
        question_card_title, queue_preview, raw_cli_menu_label, should_hold_question_permission,
        split_markdown_segments, supports_raw_cli, truncate_middle, MarkdownSegment,
    };
    use claude_code::driver::{AgentProvider, PermissionMode};
    use claude_code::TurnMetrics;
    use serde_json::json;

    #[test]
    fn raw_cli_entry_is_available_for_both_providers() {
        assert!(supports_raw_cli(AgentProvider::Claude));
        assert!(supports_raw_cli(AgentProvider::Codex));
        assert_eq!(raw_cli_menu_label(AgentProvider::Codex), "Open Codex CLI");
    }

    #[test]
    fn codex_raw_cli_flags_use_native_reasoning_and_access_options() {
        let flags = build_raw_cli_flags(
            AgentProvider::Codex,
            Some("gpt-5.4"),
            Some("high"),
            PermissionMode::AcceptEdits,
        );
        assert!(flags.contains("--model gpt-5.4"));
        assert!(flags.contains("model_reasoning_effort=\"high\""));
        assert!(flags.contains("--sandbox workspace-write"));
        assert!(flags.contains("--ask-for-approval never"));
    }

    #[test]
    fn horizontal_rules_split_into_rule_segments() {
        let formatted = parse_markdown_cached("before\n\n---\n\nafter").unwrap();
        let segments = split_markdown_segments(formatted);
        assert!(
            segments.iter().any(|s| matches!(s, MarkdownSegment::Rule)),
            "a --- between paragraphs must produce a Rule segment"
        );
    }

    #[test]
    fn blockquote_lines_group_and_lose_their_markers() {
        let formatted =
            parse_markdown_cached("intro\n\n> quoted one\n> quoted two\n\noutro").unwrap();
        let segments = split_markdown_segments(formatted);
        let quotes: Vec<_> = segments
            .iter()
            .filter_map(|s| match s {
                MarkdownSegment::Quote(text) => Some(text.raw_text()),
                _ => None,
            })
            .collect();
        assert_eq!(quotes.len(), 1, "consecutive quote lines share one segment");
        assert!(quotes[0].contains("quoted one") && quotes[0].contains("quoted two"));
        assert!(!quotes[0].contains('>'), "the > markers are stripped");
    }

    #[test]
    fn glued_gt_is_not_a_blockquote() {
        // A pasted `>>>` prompt (no space after `>`) stays prose.
        let formatted = parse_markdown_cached(">>> import this").unwrap();
        let segments = split_markdown_segments(formatted);
        assert!(
            !segments
                .iter()
                .any(|s| matches!(s, MarkdownSegment::Quote(_))),
            ">>> without a following space must not become a quote"
        );
    }

    #[test]
    fn truncate_middle_keeps_short_names_and_middle_truncates_long_ones() {
        // Short branch names pass through untouched.
        assert_eq!(truncate_middle("main", 24), "main");
        assert_eq!(truncate_middle("exactly-sixteen!", 16), "exactly-sixteen!");
        // Long ones keep the start and end around a single ellipsis, capped
        // at max_chars total.
        let truncated = truncate_middle("feature/very-long-branch-name-here", 14);
        assert_eq!(truncated.chars().count(), 14);
        assert!(truncated.starts_with("feature"));
        assert!(truncated.contains('\u{2026}'));
        assert!(truncated.ends_with("here"));
        // Multi-byte safe (chars, not bytes).
        let unicode = truncate_middle("brænçh-ñäme-with-àccents-everywhere", 14);
        assert_eq!(unicode.chars().count(), 14);
    }

    #[test]
    fn ask_user_question_permission_is_held_for_inline_answer() {
        // §1: AskUserQuestion's `can_use_tool` is held open so the inline
        // Question card can answer it with the user's picks (the model then
        // continues the same turn). Every other tool keeps its interactive
        // Allow/Deny Permission card.
        assert!(should_hold_question_permission("AskUserQuestion"));
        assert!(!should_hold_question_permission("Bash"));
        assert!(!should_hold_question_permission("Write"));
    }

    #[test]
    fn question_parser_preserves_all_visible_payload_fields() {
        let questions = parse_questions(&json!({
            "questions": [{
                "header": "Safety net",
                "question": "How should this be guarded?",
                "multiSelect": true,
                "options": [{
                    "label": "Daily sync job",
                    "description": "Automatically repairs a mismatch within a day."
                }]
            }]
        }));

        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].header, "Safety net");
        assert_eq!(questions[0].question, "How should this be guarded?");
        assert!(questions[0].multi);
        assert_eq!(questions[0].options.len(), 1);
        assert_eq!(questions[0].options[0].label, "Daily sync job");
        assert_eq!(
            questions[0].options[0].description.as_deref(),
            Some("Automatically repairs a mismatch within a day.")
        );
    }

    #[test]
    fn question_card_title_uses_semantic_header_without_losing_multi_headers() {
        let single = parse_questions(&json!({
            "questions": [{"header": "  Safety net  ", "question": "Guard it?"}]
        }));
        assert_eq!(question_card_title(&single), "Safety net");

        let multiple = parse_questions(&json!({
            "questions": [
                {"header": "Backend", "question": "Which service?"},
                {"header": "Rollout", "question": "Which cohort?"}
            ]
        }));
        assert_eq!(question_card_title(&multiple), "Questions");

        let unlabeled = parse_questions(&json!({
            "questions": [{"question": "Proceed?"}]
        }));
        assert_eq!(question_card_title(&unlabeled), "Question");
        assert_eq!(question_card_title(&[]), "Question");
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
