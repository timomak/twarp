//! twarp 21: the Pull Requests surface — a full-page main-pane list of a
//! project's GitHub pull requests, fetched via the `gh` CLI.
//!
//! 21a ships the list page (grouping, CI/review badges, browser-pane open);
//! 21b adds the native in-page detail view (Conversation/Checks tabs + merge
//! box); 21c adds the Files tab (per-file diff cards with inline review
//! threads); 21d adds the review write path (PR comments, thread replies and
//! resolution, drafted inline comments batched into one submitted review).

pub mod detail_page;
pub mod diff;
pub mod files_tab;
pub mod list_page;
pub mod review;
pub mod store;

pub use store::PullRequestsStoreModel;
