//! twarp: hardcoded actionable rows for the palette zero state's "Suggested"
//! section — "Open terminal" and "Open agent panel". Rendered through the
//! same [`SearchItem`] machinery as every other palette row.

use ordered_float::OrderedFloat;
use twarp_core::ui::theme::Fill;
use twarpui::elements::{Align, ConstrainedBox, Flex, ParentElement, Shrinkable, Text};
use twarpui::fonts::{Properties, Weight};
use twarpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::action::search_item::styles;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon as UiIcon;

/// One hardcoded zero-state action row.
pub struct SuggestedActionItem {
    label: &'static str,
    icon: UiIcon,
    action: CommandPaletteItemAction,
}

impl SuggestedActionItem {
    /// "Open terminal" — a new terminal tab.
    pub fn open_terminal() -> Self {
        Self {
            label: "Open terminal",
            icon: UiIcon::Terminal,
            action: CommandPaletteItemAction::OpenTerminalTab,
        }
    }

    /// "Open agent panel" — a Claude Code pane in a new tab.
    pub fn open_agent_panel() -> Self {
        Self {
            label: "Open agent panel",
            icon: UiIcon::ClaudeLogo,
            action: CommandPaletteItemAction::OpenAgentPanel,
        }
    }
}

impl SearchItem for SuggestedActionItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color: Fill = appearance.theme().terminal_colors().normal.cyan.into();
        render_util::render_search_item_icon(
            appearance,
            self.icon,
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
        let label = Text::new_inline(
            self.label,
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        ConstrainedBox::new(
            Flex::row()
                .with_child(Shrinkable::new(1., Align::new(label).left().finish()).finish())
                .finish(),
        )
        .with_height(styles::SEARCH_ITEM_HEIGHT)
        .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(0.)
    }

    fn accept_result(&self) -> CommandPaletteItemAction {
        self.action.clone()
    }

    fn execute_result(&self) -> CommandPaletteItemAction {
        self.action.clone()
    }

    fn accessibility_label(&self) -> String {
        self.label.to_string()
    }
}
