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
    pub fn id(&self) -> AIConversationId { *self }
    pub fn is_child_agent_conversation(&self) -> bool { false }
    pub fn is_empty(&self) -> bool { false }
    pub fn status(&self) -> Option<ConversationStatus> { None }
    pub fn is_entirely_passive(&self) -> bool { false }
    pub fn title(&self) -> Option<String> { None }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AmbientAgentTaskId(pub uuid::Uuid);
impl std::str::FromStr for AmbientAgentTaskId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AmbientAgentTaskId(s.parse()?))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct InputConfig {}
#[allow(dead_code)]
impl InputConfig {
    // twarp: 2c-d — stubbed; AI input config deleted.
    pub fn new<C>(_: &C) -> Self { Self {} }
    pub fn is_ai(&self) -> bool { false }
    pub fn is_shell(&self) -> bool { true }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SerializedBlockListItem {
    Command { block: SerializedBlockStub },
}
// twarp: 2c-d — opaque stub for serialized command block.
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
// twarp: 2c-d — From<persistence::model::Block> kept so persisted pane decoders compile.
impl From<crate::persistence::model::Block> for SerializedBlockListItem {
    fn from(_: crate::persistence::model::Block) -> Self {
        Self::Command {
            block: SerializedBlockStub::default(),
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
impl TryFrom<&str> for AIDocumentId {
    type Error = uuid::Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Ok(AIDocumentId(uuid::Uuid::parse_str(s)?))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AIDocumentVersion(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientProfileId(pub uuid::Uuid);
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LLMId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct AIConversation {}
#[allow(dead_code)]
impl AIConversation {
    // twarp: 2c-d — bulk stubs for app_state::AIConversation
    pub fn is_empty(&self) -> bool { true }
    pub fn status(&self) -> Option<ConversationStatus> { None }
    pub fn title(&self) -> Option<String> { None }
    pub fn to_serialized_blocklist_items(&self) -> Vec<()> { Vec::new() }
}
#[derive(Clone, Debug, PartialEq)]
pub enum CloudConversationData {
    Oz(Box<AIConversation>),
    CLIAgent(Box<()>),
}
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationStatus { InProgress, Done, Failed, Cancelled }
#[allow(dead_code)]
impl ConversationStatus {
    pub fn render_icon<A>(&self, _: A) -> warpui::elements::Empty {
        warpui::elements::Empty::new()
    }
    pub fn status_icon_and_color<T>(&self, _: T) -> (warp_core::ui::Icon, warpui::color::ColorU) {
        (warp_core::ui::Icon::Terminal, warpui::color::ColorU::new(0, 0, 0, 0))
    }
    // twarp: 2c-d — predicate stubs
    pub fn is_in_progress(&self) -> bool { matches!(self, ConversationStatus::InProgress) }
    pub fn is_blocked(&self) -> bool { false }
    pub fn is_error(&self) -> bool { matches!(self, ConversationStatus::Failed) }
}
#[derive(Clone, Debug, PartialEq)]
pub struct AgentConversationsModelEvent {}
#[derive(Clone, Debug, PartialEq)]
pub struct ServerConversationToken {}
#[allow(dead_code)]
impl ServerConversationToken {
    // twarp: 2c-d — bulk stubs
    pub fn new<S: Into<String>>(_: S) -> Self { Self {} }
    pub fn debug_link(&self) -> String { String::new() }
}
#[derive(Clone, Debug, PartialEq)]
pub enum AgentViewEntryOrigin {
    Input {
        is_new_conversation: bool,
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
impl RestoredAIConversation { pub fn new(_c: AIConversation) -> Self { Self {} } }

// twarp: 2c-d — CLIAgent stub (was crate::terminal::CLIAgent, deleted)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CLIAgent {
    Claude,
    Codex,
    Gemini,
    Unknown,
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
    pub fn supported_skill_providers(&self) -> &'static [ai::skills::SkillProvider] {
        &[]
    }
    // twarp: 2c-d — stub: AI agent icon deleted.
    pub fn icon(&self) -> warp_core::ui::Icon {
        warp_core::ui::Icon::Terminal
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
    pub fn display_name(&self) -> &'static str { self.serialized_name() }
    pub fn skill_command_prefix(&self) -> &'static str { "" }
    pub fn supports_bash_mode(&self) -> bool { false }
    pub fn to_serialized_name(&self) -> &'static str { self.serialized_name() }
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
impl std::fmt::Display for LLMId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
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
