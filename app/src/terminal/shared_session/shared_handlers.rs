// twarp: 2c-d — AI shared-session handlers stubbed; AI conversation/input/llm sync removed.
// `RemoteUpdateGuard` / `ActiveRemoteUpdate` are kept so consumers compile, even though
// there is no longer any AI-related state to suppress echoes for.

use std::cell::Cell;
use std::rc::Rc;

use session_sharing_protocol::common::{
    CLIAgentSessionState, InputMode, SelectedAgentModel, SelectedConversation,
    UniversalDeveloperInputContextUpdate,
};
use warpui::{AppContext, ModelHandle, WeakViewHandle};

use crate::terminal::TerminalView;

// twarp: 2c-d — AgentViewController re-exported from input for type unification.
pub use crate::terminal::input::AgentViewController;
// twarp: 2c-d — re-export from input for type unification.
pub use crate::terminal::input::BlocklistAIContextModel;
/// Stub for the deleted `BlocklistAIHistoryModel`.
pub struct BlocklistAIHistoryModel;
impl warpui::Entity for BlocklistAIHistoryModel { type Event = crate::terminal::input::BlocklistAIHistoryEvent; }
impl warpui::SingletonEntity for BlocklistAIHistoryModel {}
#[allow(dead_code)]
impl BlocklistAIHistoryModel {
    // twarp: 2c-d — bulk stubs
    pub fn all_live_conversations_for_terminal_view(&self, _: warpui::EntityId) -> Vec<crate::app_state::AIConversationId> { Vec::new() }
    pub fn conversation_id_for_action<A>(&self, _: A, _: warpui::EntityId) -> Option<crate::app_state::AIConversationId> { None }
    pub fn active_conversation_id(&self, _: warpui::EntityId) -> Option<crate::app_state::AIConversationId> { None }
    pub fn active_conversation(&self, _: warpui::EntityId) -> Option<crate::app_state::AIConversationId> { None }
    pub fn mark_terminal_view_as_ambient_agent_session_view<A>(&mut self, _: A) {}
    pub fn update_conversation_status<A, B, C, D>(&mut self, _: A, _: B, _: C, _: &mut D) {}
}

#[allow(dead_code)]
pub(crate) fn apply_selected_agent_model_update(
    _terminal_view_id: warpui::EntityId,
    _selected_model: &SelectedAgentModel,
    _guard: &ActiveRemoteUpdate,
    _ctx: &mut AppContext,
) {
}

#[allow(dead_code)]
pub(crate) fn apply_input_mode_update(
    _weak_view_handle: &WeakViewHandle<TerminalView>,
    _input_mode: &InputMode,
    _guard: &ActiveRemoteUpdate,
    _ctx: &mut AppContext,
) {
}

#[allow(dead_code)]
pub(crate) fn apply_auto_approve_agent_actions_update(
    _weak_view_handle: &WeakViewHandle<TerminalView>,
    _auto_approve: bool,
    _guard: &ActiveRemoteUpdate,
    _ctx: &mut AppContext,
) {
}

#[allow(dead_code)]
pub(crate) fn apply_selected_conversation_update(
    _weak_view_handle: &WeakViewHandle<TerminalView>,
    _selected_conversation: &SelectedConversation,
    _guard: &ActiveRemoteUpdate,
    _ctx: &mut AppContext,
) {
}

#[allow(dead_code)]
pub(crate) fn build_selected_conversation_update(
    _agent_view_controller: &ModelHandle<AgentViewController>,
    _context_model: &ModelHandle<BlocklistAIContextModel>,
    _ctx: &mut AppContext,
) -> Option<UniversalDeveloperInputContextUpdate> {
    None
}

#[allow(dead_code)]
pub(crate) fn apply_cli_agent_state_update(
    _weak_view_handle: &WeakViewHandle<TerminalView>,
    _cli_agent_session: &CLIAgentSessionState,
    _guard: &ActiveRemoteUpdate,
    _ctx: &mut AppContext,
) {
}

/// Shared guard that tracks whether we are currently applying a remote
/// session-sharing context update.
#[derive(Clone)]
pub(crate) struct RemoteUpdateGuard {
    inner: Rc<Cell<bool>>,
}

#[allow(dead_code)]
impl RemoteUpdateGuard {
    /// Creates a new guard, initially not suppressing broadcasts.
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(Cell::new(false)),
        }
    }

    /// Returns `true` when a context update originated locally and should be
    /// broadcast to the remote side. Returns `false` when we are in the middle
    /// of applying a remote update (i.e. the echo should be suppressed).
    pub(crate) fn should_broadcast(&self) -> bool {
        !self.inner.get()
    }

    /// Returns an RAII token that suppresses outgoing broadcasts until dropped.
    /// Wrap all `apply_*` calls for incoming remote updates in this so that
    /// the synchronous event dispatch sees the guard as active.
    pub(crate) fn start_remote_update(&self) -> ActiveRemoteUpdate {
        debug_assert!(
            !self.inner.get(),
            "RemoteUpdateGuard::start_remote_update called while already active"
        );
        self.inner.set(true);
        ActiveRemoteUpdate {
            inner: self.inner.clone(),
        }
    }
}

/// RAII token that suppresses outgoing broadcasts while held.
pub(crate) struct ActiveRemoteUpdate {
    inner: Rc<Cell<bool>>,
}

impl Drop for ActiveRemoteUpdate {
    fn drop(&mut self) {
        self.inner.set(false);
    }
}
