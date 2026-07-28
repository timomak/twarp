//! twarp 21: the Pull Requests surface — a full-page main-pane list of a
//! project's GitHub pull requests, fetched via the `gh` CLI.
//!
//! 21a ships the list page (grouping, CI/review badges, browser-pane open);
//! 21b adds the native in-page detail view (Conversation/Checks tabs + merge
//! box). The Files (diff) tab follows in 21c.

pub mod detail_page;
pub mod list_page;
pub mod store;

pub use store::PullRequestsStoreModel;
