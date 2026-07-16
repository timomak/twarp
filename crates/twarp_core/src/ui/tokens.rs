//! Design tokens — the only source of geometric and typographic constants for
//! twarp UI code outside the terminal grid.
//!
//! These values implement `design/PHILOSOPHY.md`. Views must not use raw
//! numeric literals for spacing, corner radii, font sizes, or line heights;
//! they name a token instead. If no token fits, the fix is a philosophy
//! discussion (and possibly a new token), not a one-off literal.
//!
//! The terminal grid itself (cell metrics, PTY-driven content) is exempt — it
//! is governed by the user's terminal font settings, not by this module.

/// Spacing scale (pt). A 4pt grid with a 2pt micro-step.
///
/// `XXS` is reserved for icon↔text gaps and hairline breathing room; everything
/// structural starts at `XS`. There is deliberately no 6/10/14/20 — if a gap
/// looks wrong at one step, move a whole step, don't split the difference.
pub mod spacing {
    /// 2 — icon↔text micro-gap only.
    pub const XXS: f32 = 2.0;
    /// 4 — tight intra-row gaps (chip internals, glyph padding).
    pub const XS: f32 = 4.0;
    /// 8 — default gap between siblings in a dense row.
    pub const SM: f32 = 8.0;
    /// 12 — card/list-item internal padding.
    pub const MD: f32 = 12.0;
    /// 16 — panel edge insets, card-to-card gaps.
    pub const LG: f32 = 16.0;
    /// 24 — section breaks, conversation turn rhythm.
    pub const XL: f32 = 24.0;
    /// 32 — page-level margins, empty-state framing.
    pub const XXL: f32 = 32.0;
}

/// Corner radius scale (pt). Three stops; nothing nests a larger radius
/// inside a smaller one.
pub mod radius {
    /// 6 — chips, pills, badges, small buttons.
    pub const CHIP: f32 = 6.0;
    /// 10 — cards, inputs, standard buttons, list-selection fills.
    pub const CARD: f32 = 10.0;
    /// 14 — composer, floating panels, menus, popovers.
    pub const PANEL: f32 = 14.0;
}

/// A named text style: point size plus line-height ratio.
///
/// Apply with the existing plumbing (`font_size: Some(style.size)` +
/// `with_line_height_ratio(style.line_height)`); this module only names the
/// values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeStyle {
    pub size: f32,
    pub line_height: f32,
}

/// Type ramp. Six styles; UI code outside the terminal grid uses exactly
/// these.
///
/// `CODE` is for code, paths, and commands rendered inside UI surfaces
/// (tool cards, diff cards); monospace content inside the terminal grid keeps
/// `monospace_font_size()`. Mono is never used for UI labels.
pub mod type_ramp {
    use super::TypeStyle;

    /// 14/1.55 — conversation prose, empty-state copy. The reading style.
    pub const PROSE: TypeStyle = TypeStyle { size: 14.0, line_height: 1.55 };
    /// 13/1.4 — interactive UI: buttons, inputs, list items, tabs.
    pub const UI: TypeStyle = TypeStyle { size: 13.0, line_height: 1.4 };
    /// 12/1.35 — secondary labels, meta rows, pill text.
    pub const LABEL: TypeStyle = TypeStyle { size: 12.0, line_height: 1.35 };
    /// 11/1.3 — captions, timestamps, section headers (tracked-out caps).
    pub const CAPTION: TypeStyle = TypeStyle { size: 11.0, line_height: 1.3 };
    /// 12.5/1.45 — monospace code/paths/commands inside UI surfaces.
    pub const CODE: TypeStyle = TypeStyle { size: 12.5, line_height: 1.45 };
    /// 16/1.3 — the one heading size (semibold at the call site).
    pub const HEADING: TypeStyle = TypeStyle { size: 16.0, line_height: 1.3 };
}

/// Elevation: exactly two shadows. Anything floating uses one of these; fixed
/// chrome (sidebar, tab strip, cards in flow) uses none.
///
/// Values are (y-offset, blur, spread, black-alpha). Construct the concrete
/// `DropShadow` at the call site from these numbers.
pub mod elevation {
    /// Menus, popovers, tooltips: offset-y 2, blur 10, spread 0, alpha 0.09.
    pub const POPOVER: (f32, f32, f32, f32) = (2.0, 10.0, 0.0, 0.09);
    /// Detached panels / windows-within-window: offset-y 4, blur 24, spread 0,
    /// alpha 0.13.
    pub const PANEL: (f32, f32, f32, f32) = (4.0, 24.0, 0.0, 0.13);
}

/// Border rules carry no constants: every 1px border outside the terminal grid
/// is `theme().outline()` (the alpha hairline). Full-opacity strokes and
/// borders used for *grouping* (rather than marking an actionable surface) are
/// philosophy violations — group with whitespace or a 2–4% alpha fill instead.
pub mod border {
    /// The one border width. There is no 2px border in the design system.
    pub const HAIRLINE_WIDTH: f32 = 1.0;
}

/// Layout constants for conversation-class surfaces (the agent pane and
/// anything else that reads as a document).
pub mod measure {
    /// Maximum content width for prose columns, centered in the pane.
    pub const PROSE_MAX_WIDTH: f32 = 720.0;
}
