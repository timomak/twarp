//! twarp 21: the Pull Requests surface — a full-page main-pane list of a
//! project's GitHub pull requests, fetched via the `gh` CLI.
//!
//! 21a ships the list page (grouping, CI/review badges, browser-pane open);
//! a native detail view follows in 21b.

pub mod list_page;
pub mod store;

pub use store::PullRequestsStoreModel;
