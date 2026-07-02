//! The Claude Code pane's task list (feature 07, sub-phase 7f; PRODUCT §23).
//!
//! Port of `ai/blocklist/block/view_impl/todos.rs` (commit `fea2f7ea`),
//! bridged from the deleted `AIAgentTodo`/`AIConversation` model onto the
//! thin [`TodoItem`]/[`TodoStatus`] the driver emits. The visual contract is
//! unchanged: a collapsible "Tasks" header card (ported `HeaderConfig`
//! chrome), one row per item with a status glyph, completed items struck
//! through. The status icons are the ported `ai/agent/icons.rs` set
//! (`pending_icon` / `in_progress_icon` / `succeeded_icon`).
//!
//! Documented adaptations:
//! - The original struck through **cancelled** items (a state Warp's agent
//!   had and `TodoWrite` doesn't); PRODUCT §23 wants completed items to
//!   "remain visible (struck through / checked)", so the strikethrough
//!   treatment moves to **completed** — checked *and* struck.
//! - The "Outdated" badge slot carries a `done/total` progress label instead
//!   (there is exactly one live list — it updates in place, §23 — so
//!   outdatedness cannot arise).

use claude_code::{TodoItem, TodoStatus};

use twarp_core::ui::theme::AnsiColorIdentifier;
use twarpui::fonts::Properties;
use twarpui::text_layout::TextStyle;
use twarpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, MainAxisSize,
        MouseStateHandle, ParentElement, Shrinkable, Text,
    },
    AppContext, Element, SingletonEntity,
};

use super::inline_action::{icon_size, Disclosure, RightCluster, INLINE_ACTION_HORIZONTAL_PADDING};
use super::ClaudeCodeViewAction;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as WarpIcon;

/// Port of `agent::icons::todo_list_icon`.
fn todo_list_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::BulletedListBlock.into(),
        blended_colors::neutral_7(appearance.theme()),
    )
}

/// Port of `agent::icons::pending_icon`.
fn pending_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::Queued.into(),
        blended_colors::neutral_5(appearance.theme()),
    )
}

/// Port of `agent::icons::in_progress_icon`.
fn in_progress_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::Circle.into(),
        AnsiColorIdentifier::Magenta.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Port of `agent::icons::succeeded_icon`.
fn succeeded_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::Check.into(),
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// The `done/total` progress shown in the header's cluster slot.
pub(super) fn progress_label(todos: &[TodoItem]) -> String {
    let done = todos
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    format!("{done}/{} done", todos.len())
}

/// Render the in-place task list (PRODUCT §23) — the ported `render_todos`
/// shape: a collapsible "Tasks" header above one row per item. An empty list
/// renders nothing (there is nothing to show; the model keeps the slot for
/// in-place updates).
pub(super) fn render_todos(
    todos: &[TodoItem],
    expanded: bool,
    header_mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    if todos.is_empty() {
        return twarpui::elements::Empty::new().finish();
    }

    let title = Text::new_inline(
        "Tasks".to_owned(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(
        theme,
        blended_colors::neutral_2(theme),
    ))
    .with_selectable(false)
    .finish();

    let mut disclosure = Disclosure::new(title)
        .with_glyph(todo_list_icon(appearance))
        .expandable(true)
        .expanded(expanded)
        .with_cluster(RightCluster {
            label: Some(progress_label(todos)),
            icon: None,
        })
        .with_mouse_state(header_mouse_state)
        .on_toggle(|ctx| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleTodos);
        });

    if expanded {
        let mut rows = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);
        for todo in todos {
            rows.add_child(render_todo(todo, app));
        }
        disclosure = disclosure.with_body(rows.finish());
    }

    disclosure.render(app)
}

/// Port of the original `render_todo` row: status glyph + title; completed
/// items keep the row but read as done (PRODUCT §23).
fn render_todo(todo: &TodoItem, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let text_color = blended_colors::text_main(theme, theme.surface_1());
    let icon = match todo.status {
        TodoStatus::Pending => pending_icon(appearance),
        TodoStatus::InProgress => in_progress_icon(appearance),
        TodoStatus::Completed => succeeded_icon(appearance),
    };
    let item_icon = Container::new(
        ConstrainedBox::new(icon.finish())
            .with_width(icon_size(app) - 4.)
            .with_height(icon_size(app) - 4.)
            .finish(),
    )
    .with_margin_right(12.)
    .finish();

    let mut item_text = Text::new(
        todo.text.clone(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_style(Properties::default().weight(appearance.monospace_font_weight()));

    if todo.status == TodoStatus::Completed {
        // Adapted from the original's cancelled treatment (see module docs).
        let highlight_indices = (0..todo.text.chars().count()).collect();
        let strikethrough = Highlight::new().with_text_style(
            TextStyle::new()
                .with_show_strikethrough(true)
                .with_foreground_color(blended_colors::neutral_5(theme)),
        );
        item_text = item_text.with_single_highlight(strikethrough, highlight_indices);
    } else {
        item_text = item_text.with_color(text_color);
    }

    let item_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(item_icon)
        .with_child(Shrinkable::new(1., item_text.finish()).finish())
        .finish();

    Container::new(item_row)
        .with_margin_left(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_margin_bottom(12.)
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(text: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            text: text.to_owned(),
            status,
        }
    }

    #[test]
    fn progress_label_counts_completed() {
        let todos = vec![
            todo("a", TodoStatus::Completed),
            todo("b", TodoStatus::InProgress),
            todo("c", TodoStatus::Pending),
        ];
        assert_eq!(progress_label(&todos), "1/3 done");
    }
}
