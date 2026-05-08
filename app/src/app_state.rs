use pathfinder_geometry::rect::RectF;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use warpui::platform::FullscreenState;

use warpui::AppContext;

// twarp: 2c-d.4 — local stubs for deleted AI types referenced by persisted snapshots
// and various consumer files. These stubs keep code compiling; consumer call sites
// using these types are no longer wired to any real AI behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AIConversationId(pub uuid::Uuid);
impl From<String> for AIConversationId {
    fn from(s: String) -> Self {
        AIConversationId(uuid::Uuid::parse_str(&s).unwrap_or_default())
    }
}
#[allow(dead_code)]
impl AIConversationId {
    // twarp: 2c-d — bulk stubs treating bare AIConversationId as if it were the conversation
    pub fn new() -> Self {
        AIConversationId(uuid::Uuid::new_v4())
    }
    pub fn id(&self) -> AIConversationId {
        *self
    }
    pub fn is_child_agent_conversation(&self) -> bool {
        false
    }
    pub fn is_empty(&self) -> bool {
        false
    }
    pub fn status(&self) -> ConversationStatus {
        ConversationStatus::Failed
    }
    pub fn is_entirely_passive(&self) -> bool {
        false
    }
    pub fn title(&self) -> Option<String> {
        None
    }
    // twarp: 2c-d — additional stubs called on conversations
    pub fn exchange_count(&self) -> usize {
        0
    }
    pub fn last_modified_at(&self) -> Option<chrono::DateTime<chrono::Local>> {
        None
    }
    pub fn export_to_markdown<A>(&self, _: A) -> String {
        String::new()
    }
    pub fn get_task<A>(&self, _: A) -> Option<()> {
        None
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AmbientAgentTaskId(pub uuid::Uuid);
impl std::str::FromStr for AmbientAgentTaskId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AmbientAgentTaskId(s.parse()?))
    }
}
// twarp: 2c-d — stub kept; conversions removed in favor of direct use of terminal::input::InputConfig.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct InputConfig {}
impl From<InputConfig> for crate::terminal::input::InputConfig {
    fn from(_: InputConfig) -> Self {
        crate::terminal::input::InputConfig::empty()
    }
}
#[allow(dead_code)]
impl InputConfig {
    // twarp: 2c-d — stubbed; AI input config deleted.
    pub fn new<C>(_: &C) -> Self {
        Self {}
    }
    pub fn is_ai(&self) -> bool {
        false
    }
    pub fn is_shell(&self) -> bool {
        true
    }
    pub fn is_locked(&self) -> bool {
        false
    }
}
// twarp: 2c-d — From conversion to bridge terminal::input::InputConfig back to app_state
impl From<crate::terminal::input::InputConfig> for InputConfig {
    fn from(_: crate::terminal::input::InputConfig) -> Self {
        Self {}
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SerializedBlockListItem {
    Command {
        block: crate::terminal::model::block::SerializedBlock,
    },
}
// twarp: 2c-d — opaque stub for serialized command block (legacy alias).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SerializedBlockStub {
    pub start_ts: Option<chrono::DateTime<chrono::Local>>,
    pub completed_ts: Option<chrono::DateTime<chrono::Local>>,
}
#[allow(dead_code)]
impl SerializedBlockListItem {
    pub fn start_ts(&self) -> Option<chrono::DateTime<chrono::Local>> {
        match self {
            SerializedBlockListItem::Command { block } => block.start_ts,
        }
    }
}
// twarp: 2c-d — From<SerializedBlock> kept so legacy test call-sites compile.
impl From<crate::terminal::model::block::SerializedBlock> for SerializedBlockListItem {
    fn from(block: crate::terminal::model::block::SerializedBlock) -> Self {
        Self::Command { block }
    }
}
// twarp: 2c-d — From<persistence::model::Block> kept so persisted pane decoders compile.
impl From<crate::persistence::model::Block> for SerializedBlockListItem {
    fn from(_: crate::persistence::model::Block) -> Self {
        Self::Command {
            block: crate::terminal::model::block::SerializedBlock::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AIDocumentId(pub uuid::Uuid);
impl From<&str> for AIDocumentId {
    fn from(s: &str) -> Self {
        AIDocumentId(uuid::Uuid::parse_str(s).unwrap_or_default())
    }
}
impl TryFrom<String> for AIDocumentId {
    type Error = uuid::Error;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Ok(AIDocumentId(uuid::Uuid::parse_str(&s)?))
    }
}
// twarp: 2c-d — From<&str> for AIDocumentId already gives a TryFrom<&str> via blanket impl.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AIDocumentVersion(pub usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientProfileId(pub uuid::Uuid);
// twarp: 2c-e — re-export canonical LLMId from `onboarding` (the only remaining
// owner after `crates/ai` was deleted in 2c-e). Onboarding defines a local
// `LLMId` newtype that mirrors the original type from the deleted ai crate.
pub use onboarding::LLMId;

// twarp: 2c-e — local stubs for `ai::diff_validation::{DiffType, DiffDelta}`.
// The original types lived in the deleted `ai` workspace crate. These stubs
// preserve the field/variant shape so call sites still type-check; the diff
// behavior is no longer wired to AI plumbing, but the editor/code_review
// modules still reference these types in dead-but-compiled code paths.
#[derive(Clone, PartialEq, Eq)]
pub struct DiffDelta {
    pub replacement_line_range: std::ops::Range<usize>,
    pub insertion: String,
}

impl std::fmt::Debug for DiffDelta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffDelta")
            .field("replacement_line_range", &self.replacement_line_range)
            .field("insertion", &self.insertion)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffType {
    Create {
        delta: DiffDelta,
    },
    Update {
        deltas: Vec<DiffDelta>,
        rename: Option<std::path::PathBuf>,
    },
    Delete {
        delta: DiffDelta,
    },
}

// twarp: 2c-e — local stubs for `ai::agent::action::{InsertReviewComment,
// InsertedCommentLocation, InsertedCommentLine, CommentSide}`. The original
// types lived in the deleted `ai` workspace crate. The remaining call sites
// (PR-comment import in code_review, the dead `BlocklistAIActionEvent::Insert
// CodeReviewComments` variant) still mention these types but no AI agent
// emits them anymore.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertReviewComment {
    pub comment_id: String,
    pub author: String,
    pub last_modified_timestamp: String,
    pub comment_body: String,
    pub parent_comment_id: Option<String>,
    pub comment_location: Option<InsertedCommentLocation>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertedCommentLocation {
    pub relative_file_path: String,
    pub line: Option<InsertedCommentLine>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertedCommentLine {
    pub comment_line_range: std::ops::Range<usize>,
    pub diff_hunk_line_range: std::ops::Range<usize>,
    pub diff_hunk_text: String,
    pub side: Option<CommentSide>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CommentSide {
    Right,
    Left,
}

// twarp: 2c-e — local stubs for `ai::project_context::model::ProjectRulePath`
// and `ai::workspace::WorkspaceMetadata` (alias `CodeWorkspaceMetadata`). Both
// types lived in the deleted `ai` workspace crate but are referenced by the
// persistence layer (e.g. `ApplicationState::project_rules`,
// `PersistenceTransaction::UpsertProjectRules`,
// `PersistenceTransaction::UpsertCodebaseIndexMetadata`). The variants and
// fields are kept for shape; nothing in the persistence pipeline reads them
// after the AI removal.
#[derive(Debug, Default, Clone)]
pub struct CodeWorkspaceMetadata {
    pub path: std::path::PathBuf,
    pub navigated_ts: Option<chrono::DateTime<chrono::Utc>>,
    pub modified_ts: Option<chrono::DateTime<chrono::Utc>>,
    pub queried_ts: Option<chrono::DateTime<chrono::Utc>>,
}

#[allow(dead_code)]
impl CodeWorkspaceMetadata {
    /// Stub matching the original signature so the repo search code compiles.
    /// twarp: 2c-e — never invoked in practice; the data source returns an empty
    /// iterator now that the AI codebase index manager is gone.
    pub fn most_recently_navigated(_a: &Self, _b: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

// twarp: 2c-e — From impls bridging the stub to the persistence-crate row types.
// These mirror the impls that lived in the deleted `crates/ai/src/workspace.rs`
// so the existing sqlite save/load helpers in `app/src/persistence/sqlite.rs`
// continue to compile.
impl From<CodeWorkspaceMetadata> for persistence::model::NewWorkspaceMetadata {
    fn from(value: CodeWorkspaceMetadata) -> Self {
        Self {
            repo_path: value.path.to_string_lossy().into_owned(),
            navigated_ts: value.navigated_ts.map(|utc_dt| utc_dt.naive_utc()),
            modified_ts: value.modified_ts.map(|utc_dt| utc_dt.naive_utc()),
            queried_ts: value.queried_ts.map(|utc_dt| utc_dt.naive_utc()),
        }
    }
}

impl From<persistence::model::WorkspaceMetadata> for CodeWorkspaceMetadata {
    fn from(value: persistence::model::WorkspaceMetadata) -> Self {
        Self {
            path: std::path::PathBuf::from(value.repo_path),
            navigated_ts: value.navigated_ts.map(|naive_ts| naive_ts.and_utc()),
            modified_ts: value.modified_ts.map(|naive_ts| naive_ts.and_utc()),
            queried_ts: value.queried_ts.map(|naive_ts| naive_ts.and_utc()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRulePath {
    pub path: std::path::PathBuf,
    pub project_root: std::path::PathBuf,
}

// twarp: 2c-e — local stubs for `ai::skills::{SkillReference, SkillProvider}`.
// Both types lived in the deleted `ai` crate. The skill management UI was
// disconnected in earlier 2c phases but several call sites still match on
// these values, so we keep the variants intact.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SkillReference {
    Path(std::path::PathBuf),
    BundledSkillId(String),
}

impl std::fmt::Display for SkillReference {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SkillReference::Path(path) => path.display().fmt(f),
            SkillReference::BundledSkillId(id) => write!(f, "@warp-skill:{id}"),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    strum_macros::Display,
)]
pub enum SkillProvider {
    Warp,
    Agents,
    Claude,
    Codex,
    Cursor,
    Gemini,
    Copilot,
    Droid,
    Github,
    OpenCode,
}

// twarp: 2c-e — local stub for `ai::agent::action::AIAgentPtyWriteMode`. The
// canonical type lived in the deleted `ai` crate. The stub keeps the variants
// so the (now-dead) AgentInput PTY-write path still type-checks; no AI agent
// emits these writes any longer, so `decorate_bytes` simply returns the input.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub enum AIAgentPtyWriteMode {
    #[default]
    Raw,
    Line,
    Block,
}

#[allow(dead_code)]
impl AIAgentPtyWriteMode {
    pub fn decorate_bytes(
        self,
        bytes: impl Into<Vec<u8>>,
        _is_bracketed_paste_enabled: bool,
    ) -> Vec<u8> {
        bytes.into()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AIConversation {}
#[allow(dead_code)]
impl AIConversation {
    // twarp: 2c-d — bulk stubs for app_state::AIConversation
    pub fn is_empty(&self) -> bool {
        true
    }
    pub fn status(&self) -> ConversationStatus {
        ConversationStatus::Failed
    }
    pub fn title(&self) -> Option<String> {
        None
    }
    pub fn to_serialized_blocklist_items(&self) -> Vec<crate::app_state::SerializedBlockListItem> {
        Vec::new()
    }
    pub fn is_entirely_passive(&self) -> bool {
        false
    }
    pub fn id(&self) -> AIConversationId {
        AIConversationId::default()
    }
    pub fn is_child_agent_conversation(&self) -> bool {
        false
    }
    pub fn exchange_count(&self) -> usize {
        0
    }
    pub fn exchange_id_for_action(&self, _: &crate::terminal::view::AIAgentActionId) -> Option<()> {
        None
    }
    pub fn exchanges_reversed(&self) -> std::iter::Empty<()> {
        std::iter::empty()
    }
    pub fn forked_from_server_conversation_token(
        &self,
    ) -> Option<crate::app_state::ServerConversationToken> {
        None
    }
    pub fn has_active_subagent(&self) -> bool {
        false
    }
    pub fn has_opened_code_review(&self) -> bool {
        false
    }
    pub fn latest_exchange(&self) -> Option<()> {
        None
    }
    pub fn latest_user_query(&self) -> Option<String> {
        None
    }
    pub fn root_task_exchanges(&self) -> std::iter::Empty<()> {
        std::iter::empty()
    }
    pub fn server_conversation_token(&self) -> Option<crate::app_state::ServerConversationToken> {
        None
    }
    pub fn last_modified_at(&self) -> Option<chrono::DateTime<chrono::Local>> {
        None
    }
    pub fn get_task<I>(&self, _: I) -> Option<()> {
        None
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum CloudConversationData {
    Oz(Box<AIConversation>),
    CLIAgent(Box<()>),
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationStatus {
    InProgress,
    Done,
    Failed,
    Cancelled,
    Success,
    Blocked {},
    Error,
    Other,
}
impl std::fmt::Display for ConversationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConversationStatus::InProgress => "in progress",
            ConversationStatus::Done => "done",
            ConversationStatus::Failed => "failed",
            ConversationStatus::Cancelled => "cancelled",
            ConversationStatus::Success => "success",
            ConversationStatus::Blocked {} => "blocked",
            ConversationStatus::Error => "error",
            ConversationStatus::Other => "other",
        };
        f.write_str(s)
    }
}
#[allow(dead_code)]
impl ConversationStatus {
    pub fn render_icon<A>(&self, _: A) -> warpui::elements::Empty {
        warpui::elements::Empty::new()
    }
    pub fn status_icon_and_color<T>(&self, _: T) -> (warp_core::ui::Icon, warpui::color::ColorU) {
        (
            warp_core::ui::Icon::Terminal,
            warpui::color::ColorU::new(0, 0, 0, 0),
        )
    }
    // twarp: 2c-d — predicate stubs
    pub fn is_in_progress(&self) -> bool {
        matches!(self, ConversationStatus::InProgress)
    }
    pub fn is_blocked(&self) -> bool {
        false
    }
    pub fn is_error(&self) -> bool {
        matches!(self, ConversationStatus::Failed)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConversationsModelEvent {}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ServerConversationToken {}
#[allow(dead_code)]
impl ServerConversationToken {
    // twarp: 2c-d — bulk stubs
    pub fn new<S: Into<String>>(_: S) -> Self {
        Self {}
    }
    pub fn debug_link(&self) -> String {
        String::new()
    }
    pub fn from_uuid(_: uuid::Uuid) -> Self {
        Self {}
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentViewEntryOrigin {
    Input {
        was_prompt_autodetected: bool,
    },
    ChildAgent,
    Other,
    // twarp: 2c-d — additional variants kept so legacy call-sites compile.
    CodeReviewContext,
    LongRunningCommand,
    ImageAdded,
    OnboardingCallout,
    ProjectEntry,
    InlineConversationMenu,
    InlineHistoryMenu,
    CloudAgent,
    SlashCommand {
        name: String,
        // twarp: 2c-d — extra fields for AI-removed callers (Option<()> stub)
        trigger: Option<()>,
    },
    // twarp: 2c-d — additional variants for AI-removed callers
    AcceptedPromptSuggestion,
    AgentRequestedNewConversation,
    ClearBuffer,
    ContinueConversationButton,
    ConversationListView,
    ConversationSelector,
    CreateEnvironment,
    DefaultSessionMode,
    InlineCodeReview,
    Onboarding,
    ResumeConversationButton,
    SlashInit,
    ThirdPartyCloudAgent,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RestoredAIConversation {}
impl RestoredAIConversation {
    pub fn new(_c: AIConversation) -> Self {
        Self {}
    }
}

// twarp: 2c-d — CLIAgent stub (was crate::terminal::CLIAgent, deleted)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CLIAgent {
    Claude,
    Codex,
    Gemini,
    Unknown,
}
impl From<CLIAgent> for crate::server::telemetry::events::CLIAgentType {
    fn from(a: CLIAgent) -> Self {
        match a {
            CLIAgent::Claude => crate::server::telemetry::events::CLIAgentType::Claude,
            CLIAgent::Codex => crate::server::telemetry::events::CLIAgentType::Codex,
            CLIAgent::Gemini => crate::server::telemetry::events::CLIAgentType::Gemini,
            CLIAgent::Unknown => crate::server::telemetry::events::CLIAgentType::Unknown,
        }
    }
}
impl CLIAgent {
    pub fn from_serialized_name(name: &str) -> Self {
        match name {
            "claude" => CLIAgent::Claude,
            "codex" => CLIAgent::Codex,
            "gemini" => CLIAgent::Gemini,
            _ => CLIAgent::Unknown,
        }
    }
    // twarp: 2c-d — stub: AI CLI agent prefix detection deleted.
    pub fn command_prefix(&self) -> &'static str {
        ""
    }
    // twarp: 2c-d — stub: AI skill providers deleted.
    // twarp: 2c-e — `ai::skills::*` types now live as stubs in this module.
    pub fn supported_skill_providers(&self) -> &'static [SkillProvider] {
        &[]
    }
    // twarp: 2c-d — stub: AI agent icon deleted.
    pub fn icon(&self) -> Option<warp_core::ui::Icon> {
        None
    }
    // twarp: 2c-d — stub: AI brand colors deleted.
    pub fn brand_icon_color(&self) -> warpui::color::ColorU {
        warpui::color::ColorU::new(0, 0, 0, 0)
    }
    pub fn serialized_name(&self) -> &'static str {
        match self {
            CLIAgent::Claude => "claude",
            CLIAgent::Codex => "codex",
            CLIAgent::Gemini => "gemini",
            CLIAgent::Unknown => "",
        }
    }
    // twarp: 2c-d — additional bulk stubs for CLIAgent
    pub fn brand_color(&self) -> warpui::color::ColorU {
        warpui::color::ColorU::new(0, 0, 0, 0)
    }
    pub fn display_name(&self) -> &'static str {
        self.serialized_name()
    }
    pub fn skill_command_prefix(&self) -> &'static str {
        ""
    }
    pub fn supports_bash_mode(&self) -> bool {
        false
    }
    pub fn to_serialized_name(&self) -> &'static str {
        self.serialized_name()
    }
}
impl std::fmt::Display for CLIAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.serialized_name())
    }
}

impl std::fmt::Display for AIDocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::fmt::Display for AIConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::fmt::Display for AmbientAgentTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::fmt::Display for ClientProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
// twarp: 2c-d — LLMId Display impl provided by crates/ai.
use crate::code::editor_management::CodeSource;
use crate::drive::OpenWarpDriveObjectSettings;
use crate::root_view::quake_mode_window_id;
use crate::server::ids::SyncId;
use crate::settings_view::SettingsSection;
use crate::tab::SelectedTabColor;
use crate::terminal::ShellLaunchData;
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::view::left_panel::ToolPanelView;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub windows: Vec<WindowSnapshot>,
    pub active_window_index: Option<usize>,
    pub block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
    pub running_mcp_servers: Vec<uuid::Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaneUuid(pub Vec<u8>);

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub tabs: Vec<TabSnapshot>,
    pub active_tab_index: usize,
    pub bounds: Option<RectF>,
    pub fullscreen_state: FullscreenState,
    pub quake_mode: bool,
    pub universal_search_width: Option<f32>,
    pub warp_ai_width: Option<f32>,
    pub voltron_width: Option<f32>,
    pub warp_drive_index_width: Option<f32>,
    pub left_panel_open: bool,
    pub vertical_tabs_panel_open: bool,
    pub left_panel_width: Option<f32>,
    pub right_panel_width: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabSnapshot {
    pub custom_title: Option<String>,
    pub root: PaneNodeSnapshot,
    pub default_directory_color: Option<AnsiColorIdentifier>,
    pub selected_color: SelectedTabColor,
    pub left_panel: Option<LeftPanelSnapshot>,
    pub right_panel: Option<RightPanelSnapshot>,
}

impl TabSnapshot {
    pub(crate) fn color(&self) -> Option<AnsiColorIdentifier> {
        self.selected_color.resolve(self.default_directory_color)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "LeafSnapshot is significantly larger than BranchSnapshot due to nested snapshot types."
)]
pub enum PaneNodeSnapshot {
    Branch(BranchSnapshot),
    Leaf(LeafSnapshot),
}

impl PaneNodeSnapshot {
    pub fn has_horizontal_split(&self) -> bool {
        match self {
            PaneNodeSnapshot::Leaf(_) => false,
            PaneNodeSnapshot::Branch(BranchSnapshot {
                direction,
                children,
            }) => {
                let self_has_split = *direction == SplitDirection::Horizontal && children.len() > 1;
                self_has_split
                    || children
                        .iter()
                        .any(|(_, child)| child.has_horizontal_split())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BranchSnapshot {
    pub direction: SplitDirection,
    pub children: Vec<(PaneFlex, PaneNodeSnapshot)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeafSnapshot {
    pub is_focused: bool,
    pub custom_vertical_tabs_title: Option<String>,
    pub contents: LeafContents,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LeafContents {
    Terminal(TerminalPaneSnapshot),
    Notebook(NotebookPaneSnapshot),
    AIDocument(AIDocumentPaneSnapshot),
    Code(CodePaneSnapShot),
    EnvVarCollection(EnvVarCollectionPaneSnapshot),
    Workflow(WorkflowPaneSnapshot),
    Settings(SettingsPaneSnapshot),
    AIFact(AIFactPaneSnapshot),
    ExecutionProfileEditor,
    CodeReview(CodeReviewPaneSnapshot),
    AmbientAgent(AmbientAgentPaneSnapshot),
    /// The in-app network log pane. Not persisted across restarts because the
    /// backing log is an in-memory ring buffer that starts empty on launch.
    NetworkLog,
    /// An entrypoint pane type to launch other pane types from a search palette. The default view
    /// when creating a tab.
    Welcome {
        startup_directory: Option<PathBuf>,
    },
    /// A new first-time user experience which prioritizes choosing a coding repository.
    GetStarted,
}

#[cfg(feature = "local_fs")]
impl LeafContents {
    /// Whether this pane content should be written to (and later restored
    /// from) the SQLite app-state database.
    ///
    /// Non-persisted pane types are skipped entirely during the pane tree
    /// traversal in `save_app_state`, so no `pane_nodes` row is inserted for
    /// them. This is important: inserting a `pane_nodes` row with
    /// `is_leaf = true` but no matching `pane_leaves` row leaves an orphan
    /// that `read_node` cannot resolve, which causes the surrounding tab's
    /// restoration to fail and the whole tab to disappear on restart.
    pub(crate) fn is_persisted(&self) -> bool {
        match self {
            // Network log: the backing log is an in-memory ring buffer that
            // starts empty on launch; persisting would also regress back to
            // an on-disk log via the app-state database.
            LeafContents::NetworkLog => false,
            LeafContents::Terminal(_)
            | LeafContents::Notebook(_)
            | LeafContents::AIDocument(_)
            | LeafContents::Code(_)
            | LeafContents::EnvVarCollection(_)
            | LeafContents::Workflow(_)
            | LeafContents::Settings(_)
            | LeafContents::AIFact(_)
            | LeafContents::ExecutionProfileEditor
            | LeafContents::CodeReview(_)
            | LeafContents::AmbientAgent(_)
            | LeafContents::Welcome { .. }
            | LeafContents::GetStarted => true,
        }
    }
}

/// Snapshot of an ambient agent pane.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientAgentPaneSnapshot {
    pub uuid: Vec<u8>,
    pub task_id: Option<AmbientAgentTaskId>,
}

/// Snapshot of the contents of a terminal pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPaneSnapshot {
    pub uuid: Vec<u8>,
    pub cwd: Option<String>,
    pub shell_launch_data: Option<ShellLaunchData>,
    pub is_active: bool,
    pub is_read_only: bool,
    pub input_config: Option<InputConfig>,
    pub llm_model_override: Option<String>,
    pub active_profile_id: Option<SyncId>,
    pub conversation_ids_to_restore: Vec<AIConversationId>,
    /// The active conversation ID if the agent view was open in fullscreen mode.
    /// When `Some`, the agent view should be restored to fullscreen for this conversation.
    pub active_conversation_id: Option<AIConversationId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotebookPaneSnapshot {
    CloudNotebook {
        /// The ID of the notebook that was open in this pane. There are 3 possibilities:
        /// 1. The pane contains a newly-created notebook that has not been edited yet. It might not
        ///    have an ID yet (client or server), so this will be `None`.
        /// 2. The pane contains a notebook that hasn't been synced to the server yet, so this will
        ///    contain a client ID that should exist in SQLite.
        /// 3. The pane contains a notebook that's known to the server, so this will contain the
        ///    server ID.
        notebook_id: Option<SyncId>,
        // Settings for the notebook pane when it's opened (such as a folder to focus upon opening)
        settings: OpenWarpDriveObjectSettings,
    },
    LocalFileNotebook {
        /// The path to the local file that was open in this pane. This may be `None` if
        /// the pane contained an unreadable file.
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIDocumentPaneSnapshot {
    Local {
        document_id: String,
        version: i32,
        content: Option<String>,
        title: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodePaneTabSnapshot {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodePaneSnapShot {
    Local {
        tabs: Vec<CodePaneTabSnapshot>,
        active_tab_index: usize,
        /// The full `CodeSource` for this pane, serialized as JSON in the DB.
        source: Option<CodeSource>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowPaneSnapshot {
    CloudWorkflow {
        workflow_id: Option<SyncId>,
        // Settings for the workflow pane when it's opened (such as a folder to focus upon opening)
        settings: OpenWarpDriveObjectSettings,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EnvVarCollectionPaneSnapshot {
    // CloudEnvVarCollection snapshots operate under the same heuristics
    // as NotebookPaneSnapshot::CloudNotebook
    CloudEnvVarCollection {
        env_var_collection_id: Option<SyncId>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsPaneSnapshot {
    Local {
        current_page: SettingsSection,
        search_query: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIFactPaneSnapshot {
    Personal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeReviewPaneSnapshot {
    Local {
        terminal_uuid: Vec<u8>,
        repo_path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LeftPanelDisplayedTab {
    FileTree,
    GlobalSearch,
    WarpDrive,
    ConversationListView,
}

impl From<ToolPanelView> for LeftPanelDisplayedTab {
    fn from(view: ToolPanelView) -> Self {
        match view {
            ToolPanelView::ProjectExplorer => LeftPanelDisplayedTab::FileTree,
            ToolPanelView::GlobalSearch { .. } => LeftPanelDisplayedTab::GlobalSearch,
            ToolPanelView::WarpDrive => LeftPanelDisplayedTab::WarpDrive,
            ToolPanelView::ConversationListView => LeftPanelDisplayedTab::ConversationListView,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeftPanelSnapshot {
    pub left_panel_displayed_tab: LeftPanelDisplayedTab,
    pub pane_group_id: String,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RightPanelSnapshot {
    pub pane_group_id: String,
    pub width: usize,
    pub is_maximized: bool,
}

/// Copied from pane group model, which should be private to pane group.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneFlex(pub f32);

pub fn get_app_state(app: &AppContext) -> AppState {
    let active_window_id = app.windows().active_window();
    let quake_mode_id = quake_mode_window_id();

    let mut active_window_index = None;

    let mut windows = vec![];

    for (index, window_id) in app.window_ids().enumerate() {
        // Determine index of active window
        if let Some(active_window_id) = active_window_id {
            if active_window_id == window_id {
                active_window_index = Some(index);
            }
        }

        if let Some(first_workspace) = app
            .views_of_type::<Workspace>(window_id)
            .as_ref()
            .and_then(|workspaces| workspaces.first())
        {
            let ws = first_workspace.as_ref(app);
            if ws.is_drag_preview_workspace() {
                continue;
            }
            let snapshot = ws.snapshot(
                window_id,
                quake_mode_id.map(|id| id == window_id).unwrap_or(false),
                app,
            );
            if !snapshot.tabs.is_empty() {
                windows.push(snapshot);
            }
        }
    }

    AppState {
        windows,
        active_window_index,
        block_lists: Default::default(),
        running_mcp_servers: Vec::new(),
    }
}

#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;
