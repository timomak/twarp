//! Collapsible thinking cards for the Claude Code pane (feature 07,
//! sub-phase 7f; PRODUCT §22).
//!
//! Extracted helpers from `ai/blocklist/block/view_impl/output.rs` at
//! `fea2f7ea` (the TECH matrix marks that file "extract helpers", not a
//! whole-file port — it is bound to the deleted streaming-props machinery):
//! the `Reasoning` arm's header text ("Thought for N seconds" /
//! "Thinking"), `common.rs::format_elapsed_seconds`, and the
//! `render_collapsible_header` visual contract — a muted header with a
//! trailing chevron (right collapsed / down expanded), pointer cursor,
//! click-to-toggle — over a muted markdown body.
//!
//! Collapsed by default, visually subordinate to assistant text (§22): the
//! body renders through the same ported markdown stack as assistant turns,
//! in the disabled text color.
//!
//! The headless stream-json reports no per-block thinking duration (the
//! interactive TUI measures its own); the label degrades to "Thinking" until
//! a duration source exists — same honesty rule as the effort chip (#74).

use std::time::Duration;

use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable, MainAxisSize,
        MouseStateHandle, ParentElement,
    },
    platform::Cursor,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, SingletonEntity,
};

use super::inline_action::icon_size;
use super::{render_markdown_body, ClaudeCodeViewAction, TRANSCRIPT_LEFT_MARGIN};
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as WarpIcon;

/// Per-card UI state owned by the view, keyed by the item's transcript index
/// (thinking items only ever append, so the index is stable).
#[derive(Default)]
pub(super) struct ThinkingUi {
    pub mouse_state: MouseStateHandle,
    /// Collapsed by default (PRODUCT §22).
    pub expanded: bool,
}

/// Port of `common.rs::format_elapsed_seconds`.
pub(super) fn format_elapsed_seconds(elapsed: Duration) -> String {
    let total_seconds = elapsed.as_secs();
    if total_seconds == 1 {
        "1 second".to_string()
    } else {
        format!("{total_seconds} seconds")
    }
}

/// Compact running-clock format for the live streaming status (#7): the two
/// most-significant non-zero units, e.g. `45s`, `1m 21s`, `2h 3m`, `1d 4h`.
/// Unlike [`format_elapsed_seconds`] (a prose "N seconds" for finished thinking
/// cards) this stays short as a turn runs into minutes or hours.
pub(super) fn format_compact_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    let seconds = total % 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// The header label — the `Reasoning` arm's text ("Thought for N seconds"
/// when a duration is known, "Thinking" otherwise).
pub(super) fn header_label(duration: Option<Duration>) -> String {
    match duration {
        Some(duration) => format!("Thought for {}", format_elapsed_seconds(duration)),
        None => "Thinking".to_string(),
    }
}

/// Render one collapsible thinking card (PRODUCT §22).
pub(super) fn render_thinking_card(
    index: usize,
    text: &str,
    duration: Option<Duration>,
    ui: Option<&ThinkingUi>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    // The ported header/body color: disabled-tier text — subordinate to
    // assistant prose (§22), themed (§33).
    let text_color = blended_colors::text_disabled(theme, theme.background());

    let expanded = ui.is_some_and(|ui| ui.expanded);
    let mouse_state = ui.map(|ui| ui.mouse_state.clone()).unwrap_or_default();

    // Port of `render_collapsible_header`: label + chevron, hover + pointer,
    // click toggles.
    let chevron = if expanded {
        WarpIcon::ChevronDown
    } else {
        WarpIcon::ChevronRight
    };
    let label = header_label(duration);
    let glyph_size = icon_size(app);
    let header_row = Hoverable::new(mouse_state, move |_is_hovered| {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                appearance
                    .ui_builder()
                    .span(label.clone())
                    .with_style(UiComponentStyles {
                        font_color: Some(text_color),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(chevron.to_warpui_icon(text_color.into()).finish())
                        .with_width(glyph_size - 2.)
                        .with_height(glyph_size - 2.)
                        .finish(),
                )
                .with_margin_left(4.)
                .finish(),
            )
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleThinking(index));
    })
    .finish();

    // `MainAxisSize::Min` keeps the click target on the label itself rather
    // than the whole row width (the ported header's shape).
    let header = Flex::row()
        .with_main_axis_size(MainAxisSize::Min)
        .with_child(header_row)
        .finish();

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_child(header);

    if expanded {
        column.add_child(
            Container::new(render_markdown_body(text, text_color, appearance))
                .with_padding_top(8.)
                .finish(),
        );
    }

    Container::new(column.finish())
        .with_padding_left(TRANSCRIPT_LEFT_MARGIN)
        .with_padding_right(20.)
        .with_padding_top(4.)
        .with_padding_bottom(8.)
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_seconds_pluralizes() {
        assert_eq!(format_elapsed_seconds(Duration::from_secs(1)), "1 second");
        assert_eq!(format_elapsed_seconds(Duration::from_secs(0)), "0 seconds");
        assert_eq!(
            format_elapsed_seconds(Duration::from_secs(42)),
            "42 seconds"
        );
    }

    #[test]
    fn header_label_uses_duration_when_available() {
        assert_eq!(header_label(None), "Thinking");
        assert_eq!(
            header_label(Some(Duration::from_secs(7))),
            "Thought for 7 seconds"
        );
    }

    #[test]
    fn compact_elapsed_scales_units() {
        let s = Duration::from_secs;
        assert_eq!(format_compact_elapsed(s(0)), "0s");
        assert_eq!(format_compact_elapsed(s(45)), "45s");
        assert_eq!(format_compact_elapsed(s(81)), "1m 21s");
        assert_eq!(format_compact_elapsed(s(60 * 60 + 3 * 60)), "1h 3m");
        assert_eq!(format_compact_elapsed(s(2 * 86_400 + 5 * 3_600)), "2d 5h");
    }
}
