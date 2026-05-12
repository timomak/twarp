//! Repo header row for the Open Changes panel (PRODUCT §4).
//!
//! Renders `<repo-name> · <branch>`. 5a omits the upstream-tracking
//! tooltip; 5d adds the ahead/behind indicator.

use warpui::elements::{Element, Flex, MainAxisSize, ParentElement};
use warpui::ui_components::components::UiComponent;

use crate::appearance::Appearance;
use crate::open_changes::repo::RepoState;

pub fn render(state: &RepoState, appearance: &Appearance) -> Box<dyn Element> {
    let repo_name = if state.repo_name.is_empty() {
        "(unknown repo)".to_string()
    } else {
        state.repo_name.clone()
    };
    let branch = state.branch.display_label();
    let separator = if branch.is_empty() { "" } else { " · " };
    let label = format!("{repo_name}{separator}{branch}");

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(
            appearance
                .ui_builder()
                .span(label)
                .with_soft_wrap()
                .build()
                .finish(),
        )
        .finish()
}
