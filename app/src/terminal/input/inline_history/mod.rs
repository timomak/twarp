//! Inline history menu for up-arrow history when `FeatureFlag::AgentView` is enabled.
//!
//! Shows both live conversations for the terminal view and command history in the terminal
//! view, and prompts and command history in the agent view.
// twarp: 2c-d — visibility raised to pub(crate) so lib.rs can register the
// file-local AI singleton stub in this module.
pub(crate) mod data_source;
mod search_item;
mod view;

pub use data_source::AcceptHistoryItem;
pub use view::{HistoryTab, InlineHistoryMenuEvent, InlineHistoryMenuView};
