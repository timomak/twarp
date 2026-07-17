//! twarp: one stored Claude Code session rendered as a palette row.

use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use twarp_core::ui::theme::Fill;
use twarpui::elements::{Align, ConstrainedBox, Flex, Highlight, ParentElement, Shrinkable, Text};
use twarpui::fonts::{Properties, Weight};
use twarpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::action::search_item::styles;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon as UiIcon;
use crate::util::time_format::format_approx_duration_from_now;

/// A stored Claude Code session for the active cwd, ready to be resumed.
#[derive(Debug)]
pub struct ClaudeSessionSearchItem {
    pub session_id: String,
    pub title: String,
    pub timestamp: SystemTime,
    pub jsonl_path: PathBuf,
    /// The cwd the session belongs to (resume must launch `claude` there).
    pub cwd: PathBuf,
    pub match_result: FuzzyMatchResult,
}

impl ClaudeSessionSearchItem {
    /// Relative "time ago" label for the session's last activity.
    fn relative_timestamp(&self) -> String {
        format_approx_duration_from_now(DateTime::<Local>::from(self.timestamp))
    }

    fn render(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let title = Text::new_inline(
            self.title.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(item_highlight_state.main_text_fill(appearance).into_solid())
        .with_single_highlight(
            Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_foreground_color(
                    item_highlight_state.main_text_fill(appearance).into_solid(),
                ),
            self.match_result.matched_indices.clone(),
        )
        .finish();

        let timestamp = Text::new_inline(
            self.relative_timestamp(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(item_highlight_state.sub_text_fill(appearance).into_solid())
        .finish();

        let row = Flex::row()
            .with_child(Shrinkable::new(1., Align::new(title).left().finish()).finish())
            .with_child(timestamp)
            .finish();

        ConstrainedBox::new(row)
            .with_height(styles::SEARCH_ITEM_HEIGHT)
            .finish()
    }
}

impl SearchItem for ClaudeSessionSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color: Fill = appearance.theme().terminal_colors().normal.magenta.into();
        render_util::render_search_item_icon(
            appearance,
            UiIcon::ClaudeLogo,
            icon_color.into_solid(),
            highlight_state,
        )
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render(highlight_state, appearance)
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> CommandPaletteItemAction {
        CommandPaletteItemAction::ResumeClaudeSession {
            session_id: self.session_id.clone(),
            jsonl_path: self.jsonl_path.clone(),
            cwd: self.cwd.clone(),
        }
    }

    fn execute_result(&self) -> CommandPaletteItemAction {
        self.accept_result()
    }

    fn dedup_key(&self) -> Option<String> {
        Some(format!("claude-session:{}", self.session_id))
    }

    fn accessibility_label(&self) -> String {
        format!(
            "Agent session: {}, {}",
            self.title,
            self.relative_timestamp()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> ClaudeSessionSearchItem {
        ClaudeSessionSearchItem {
            session_id: "abc-123".into(),
            title: "fix the flaky test".into(),
            timestamp: SystemTime::UNIX_EPOCH,
            jsonl_path: PathBuf::from("/home/u/.claude/projects/x/abc-123.jsonl"),
            cwd: PathBuf::from("/repo"),
            match_result: FuzzyMatchResult::no_match(),
        }
    }

    #[test]
    fn accept_result_carries_resume_identity() {
        match item().accept_result() {
            CommandPaletteItemAction::ResumeClaudeSession {
                session_id,
                jsonl_path,
                cwd,
            } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(
                    jsonl_path,
                    PathBuf::from("/home/u/.claude/projects/x/abc-123.jsonl")
                );
                assert_eq!(cwd, PathBuf::from("/repo"));
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn dedup_key_is_session_scoped() {
        assert_eq!(
            item().dedup_key().as_deref(),
            Some("claude-session:abc-123")
        );
    }
}
