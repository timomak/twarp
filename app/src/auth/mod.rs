pub mod anonymous_id;
pub mod auth_manager;
pub mod auth_state;
pub mod credentials;
pub mod user;
pub mod user_uid;

// twarp: de-cloud (2b) — the login/signup UI (auth view, login slide, SSO-link
// view, auth-override warning, web handoff) and the log-out flow were deleted.
// Logged-out is the permanent, only state; there is no sign-up or account
// linking.

// twarp: 2c-d — AI conversations / blocklist / execution profiles deleted; stubs.
pub struct AgentConversationsModel;
impl twarpui::Entity for AgentConversationsModel {
    type Event = ();
}
impl twarpui::SingletonEntity for AgentConversationsModel {}
#[allow(dead_code)]
impl AgentConversationsModel {
    pub fn reset(&mut self) {}
}
pub struct BlocklistAIHistoryModel;
impl twarpui::Entity for BlocklistAIHistoryModel {
    type Event = crate::terminal::input::BlocklistAIHistoryEvent;
}
impl twarpui::SingletonEntity for BlocklistAIHistoryModel {}
#[allow(dead_code)]
impl BlocklistAIHistoryModel {
    pub fn reset(&mut self) {}
}
pub struct AIExecutionProfilesModel;
impl twarpui::Entity for AIExecutionProfilesModel {
    type Event = ();
}
impl twarpui::SingletonEntity for AIExecutionProfilesModel {}
#[allow(dead_code)]
impl AIExecutionProfilesModel {
    pub fn reset(&mut self) {}
}

// twarp: 2c-e — removed `use ai::index::full_source_code_embedding::manager::
// CodebaseIndexManager`; the AI codebase index manager and its logout-time
// `reset_codebase_indexing` call are gone with the `ai` crate.
pub use auth_manager::AuthManager;
pub use auth_state::AuthStateProvider;
pub use user_uid::UserUid;

/// Prefix for API keys used in authentication
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub const API_KEY_PREFIX: &str = "wk-";
