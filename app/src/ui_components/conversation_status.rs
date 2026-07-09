//! Renders conversation-status chips (working / blocked / done / failed).
//! Restored from upstream's deleted `ai/conversation_status_ui.rs` (twarp
//! 2c-d) when the 7p Claude-pane tab indicator started using the status set
//! again — the previous `Empty` stub laid out at its max constraint, so tabs
//! reserved an invisible 22px box and the title rendered off-center.

use crate::app_state::ConversationStatus;
use crate::appearance::Appearance;
use twarp_core::ui::color::coloru_with_opacity;
use twarp_core::ui::theme::Fill;
use twarpui::elements::{ConstrainedBox, Container, CornerRadius, Radius};
use twarpui::Element;

/// Padding around the status icon
pub const STATUS_ELEMENT_PADDING: f32 = 2.;

/// Render the status element used by the tab indicator and inline-history rows.
pub fn render_status_element(
    status: &ConversationStatus,
    icon_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let (icon, color) = status.status_icon_and_color(theme);

    Container::new(
        ConstrainedBox::new(icon.to_warpui_icon(Fill::from(color)).finish())
            .with_width(icon_size)
            .with_height(icon_size)
            .finish(),
    )
    .with_uniform_padding(STATUS_ELEMENT_PADDING)
    .with_background(coloru_with_opacity(color, 10))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .finish()
}
