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

/// Stub for the deleted `AgentViewController` so callers can pass a model handle.
pub struct AgentViewController;
impl warpui::Entity for AgentViewController { type Event = (); }
/// Stub for the deleted `BlocklistAIContextModel` so callers can pass a model handle.
pub struct BlocklistAIContextModel;
impl warpui::Entity for BlocklistAIContextModel { type Event = (); }
/// Stub for the deleted `BlocklistAIHistoryModel`.
pub struct BlocklistAIHistoryModel;
impl warpui::Entity for BlocklistAIHistoryModel { type Event = (); }
impl warpui::SingletonEntity for BlocklistAIHistoryModel {}

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
