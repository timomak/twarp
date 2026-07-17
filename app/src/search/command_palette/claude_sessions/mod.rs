//! twarp: stored Claude Code sessions as command-palette results.
//!
//! Surfaces `claude`'s own on-disk session store for the active working
//! directory (see `claude_code::sessions`) in the Cmd/Ctrl-Shift-P palette —
//! one row per stored session (title + relative timestamp); accepting a row
//! resumes it in a Claude pane via `WorkspaceAction::ResumeClaudeCodeSession`.

pub mod data_source;
pub mod search_item;

pub use data_source::ClaudeSessionsDataSource;
pub use search_item::ClaudeSessionSearchItem;
