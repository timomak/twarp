//! The Agent-Mode `inline_action` card chrome, **ported** for the Claude Code
//! pane's tool cards (feature 07, sub-phase 7d).
//!
//! Port sources (commit `fea2f7ea`, the pre-deletion snapshot feature 02
//! removed):
//!
//! - `ai/blocklist/inline_action/inline_action_icons.rs` — `icon_size` + the
//!   status icons (the running icon comes from `ai/agent/icons.rs::
//!   yellow_running_icon`, the same glyph family).
//! - `ai/blocklist/inline_action/inline_action_header.rs` — [`HeaderConfig`] +
//!   [`InteractionMode`] / [`ExpandedConfig`] and the padding constants.
//! - `ai/blocklist/inline_action/requested_action.rs` — [`RenderableAction`]
//!   (the card builder) + the body-row / body-text renderers.
//! - `ai/blocklist/block/view_impl.rs::WithContentItemSpacing` — the co-ported
//!   spacing trait every card is wrapped with.
//!
//! Documented adaptations (port-and-adapt per TECH.md §Per-file decision
//! matrix; the visual contract — surfaces, paddings, radii, hover/chevron
//! behavior — is unchanged):
//!
//! - `HeaderConfig.badge` (a bordered pill) is replaced by the muted
//!   [`RightCluster`] — result label + status icon — that the sibling leaf
//!   `search_results_common.rs` rendered in the same slot; tool cards need the
//!   §19 collapsed result summary there, not a pill.
//! - The Warp-AI service couplings are trimmed: the Run/Cancel keystroke
//!   buttons (`render_header_buttons` et al.) belong to 7g's permission cards,
//!   and the `ActionButtons`/`RightClickable` interaction modes, markdown
//!   titles, and right-click callbacks had no consumer here. They return with
//!   the sub-phase that needs them rather than landing as dead code.
//! - The spacing constants are re-derived from this pane's message-row
//!   geometry (see [`WithContentItemSpacing`]) so cards align with the
//!   transcript's text column, exactly as Agent Mode aligned cards with the
//!   user query.

use std::borrow::Cow;
use std::rc::Rc;

use pathfinder_color::ColorU;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::{
    elements::{
        Align, Border, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Expanded, Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, Shrinkable, Text, Wrap, WrapFill,
    },
    fonts::FamilyId,
    platform::Cursor,
    AppContext, Element, EventContext, SingletonEntity,
};

use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as WarpIcon;
use crate::view_components::compactible_action_button::render_expansion_icon;

/// Ported padding constants — same values as the original for consistency.
pub(super) const INLINE_ACTION_HORIZONTAL_PADDING: f32 = 16.;
/// The vertical padding applied to the requested action row's content body.
pub(super) const INLINE_ACTION_VERTICAL_PADDING: f32 = 12.;
pub(super) const INLINE_ACTION_HEADER_VERTICAL_PADDING: f32 = 10.;
pub(super) const ICON_MARGIN: f32 = 8.;

/// The space below each card, ported from the AI block's
/// `CONTENT_ITEM_VERTICAL_MARGIN`.
const CONTENT_ITEM_VERTICAL_MARGIN: f32 = 16.;
/// Adapted: the message rows in this pane (`render_message_row`) use a uniform
/// 14px container padding, a 16px avatar, and a 12px avatar margin — so the
/// transcript's text column starts 42px in. Cards indent to that column the
/// way Agent Mode indented output items to the user query.
const CONTENT_HORIZONTAL_PADDING: f32 = 14.;
const MESSAGE_AVATAR_WIDTH: f32 = 16.;
const MESSAGE_AVATAR_MARGIN: f32 = 12.;

/// Returns the size for icons in the card chrome, scaled to the user's current
/// font size. (Port of `inline_action_icons::icon_size`.)
pub(super) fn icon_size(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);
    app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    )
}

/// Status icon for a running tool (port of `agent::icons::yellow_running_icon`).
pub(super) fn running_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        WarpIcon::Circle.into(),
        AnsiColorIdentifier::Yellow.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Status icon for a completed tool (port of `green_check_icon`).
pub(super) fn green_check_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        WarpIcon::Check.into(),
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Status icon for a failed tool (port of `red_x_icon`).
pub(super) fn red_x_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        WarpIcon::X.into(),
        AnsiColorIdentifier::Red.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub(super) type OnToggleExpandedCallback = Rc<dyn Fn(&mut EventContext) + 'static>;

/// Configuration for manual expansion behavior (ported; the right-click and
/// expands-upwards options had no consumer here and are trimmed).
#[derive(Clone)]
pub(super) struct ExpandedConfig {
    pub is_expanded: bool,
    /// Callback for when the expansion toggle is clicked.
    pub on_toggle_expanded: Option<OnToggleExpandedCallback>,
    /// Mouse state handle for the expansion toggle.
    pub toggle_mouse_state: MouseStateHandle,
}

impl ExpandedConfig {
    pub fn new(is_expanded: bool, toggle_mouse_state: MouseStateHandle) -> Self {
        Self {
            is_expanded,
            on_toggle_expanded: None,
            toggle_mouse_state,
        }
    }

    pub fn with_toggle_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut EventContext) + 'static,
    {
        self.on_toggle_expanded = Some(Rc::new(callback));
        self
    }
}

/// Ported interaction modes. Only `ManuallyExpandable` survives the trim — the
/// `ActionButtons` mode returns with 7g's permission cards.
#[derive(Clone)]
pub(super) enum InteractionMode {
    /// Renders an expansion chevron, with caller-specified click handler.
    ManuallyExpandable(ExpandedConfig),
}

/// The muted right-hand cluster of a card row: an optional result label and an
/// optional status icon. (Adaptation: this is the right-label + icon slot the
/// sibling leaf `search_results_common.rs` rendered; it replaces the ported
/// header's bordered `badge`.)
#[derive(Default)]
pub(super) struct RightCluster {
    pub label: Option<String>,
    pub icon: Option<warpui::elements::Icon>,
}

impl RightCluster {
    /// Render the cluster's label + icon, each separated by the ported
    /// 8px margin (`search_results_common` spacing).
    pub(super) fn render(self, appearance: &Appearance, app: &AppContext) -> Vec<Box<dyn Element>> {
        let theme = appearance.theme();
        let mut children: Vec<Box<dyn Element>> = Vec::new();
        if let Some(label) = self.label {
            children.push(
                Container::new(
                    Text::new_inline(
                        label,
                        appearance.ui_font_family(),
                        appearance.monospace_font_size() - 1.,
                    )
                    .with_color(theme.sub_text_color(theme.surface_2()).into())
                    .with_selectable(false)
                    .finish(),
                )
                .with_margin_right(ICON_MARGIN)
                .finish(),
            );
        }
        if let Some(icon) = self.icon {
            children.push(
                Container::new(
                    ConstrainedBox::new(icon.finish())
                        .with_width(icon_size(app) - 4.)
                        .with_height(icon_size(app) - 4.)
                        .finish(),
                )
                .with_margin_right(ICON_MARGIN)
                .finish(),
            );
        }
        children
    }
}

/// Ported card header (port of `inline_action_header::HeaderConfig`).
pub(super) struct HeaderConfig {
    pub title: Cow<'static, str>,
    pub font_family: FamilyId,
    pub icon: Option<warpui::elements::Icon>,
    pub right_cluster: RightCluster,
    pub interaction_mode: Option<InteractionMode>,
    pub is_text_selectable: bool,
    pub font_color_override: Option<ColorU>,
    pub soft_wrap_title: bool,
}

impl HeaderConfig {
    pub fn new(title: impl Into<Cow<'static, str>>, app: &AppContext) -> Self {
        Self {
            title: title.into(),
            font_family: Appearance::as_ref(app).ui_font_family(),
            icon: None,
            right_cluster: RightCluster::default(),
            interaction_mode: None,
            is_text_selectable: false,
            font_color_override: None,
            soft_wrap_title: false,
        }
    }

    pub fn with_icon(mut self, icon: warpui::elements::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_right_cluster(mut self, right_cluster: RightCluster) -> Self {
        self.right_cluster = right_cluster;
        self
    }

    pub fn with_interaction_mode(mut self, interaction_mode: InteractionMode) -> Self {
        self.interaction_mode = Some(interaction_mode);
        self
    }

    /// Render the header row (ported layout: left icon + title, right cluster +
    /// interaction content; `surface_2` background; top-rounded when expanded
    /// downwards, fully rounded otherwise; hover + click when expandable).
    pub fn render(self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let interaction_mode_content = self.interaction_mode.as_ref().map(
            |InteractionMode::ManuallyExpandable(expansion_config)| {
                ConstrainedBox::new(render_expansion_icon(
                    expansion_config.is_expanded,
                    false,
                    appearance,
                    app,
                ))
                .with_height(icon_size(app))
                .with_width(icon_size(app))
                .finish()
            },
        );
        self.render_header(app, interaction_mode_content)
    }

    fn render_header(
        self,
        app: &AppContext,
        interaction_mode_content: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let header_background = theme.surface_2();

        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let mut left_content_container = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(icon) = self.icon {
            left_content_container.add_child(
                Container::new(
                    ConstrainedBox::new(icon.finish())
                        .with_width(icon_size(app))
                        .with_height(icon_size(app))
                        .finish(),
                )
                .with_margin_right(ICON_MARGIN)
                .finish(),
            )
        }

        let text_color = self
            .font_color_override
            .unwrap_or_else(|| blended_colors::text_main(theme, header_background));

        let title_element = Text::new_inline(
            self.title.clone(),
            self.font_family,
            appearance.monospace_font_size(),
        )
        .soft_wrap(self.soft_wrap_title)
        .with_selectable(self.is_text_selectable)
        .with_color(text_color)
        .finish();

        left_content_container.add_child(
            Expanded::new(
                1.,
                Container::new(title_element).with_margin_right(8.).finish(),
            )
            .finish(),
        );

        header_row.add_child(Shrinkable::new(1., left_content_container.finish()).finish());

        let mut right_side = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        for child in self.right_cluster.render(appearance, app) {
            right_side.add_child(child);
        }
        if let Some(interaction_mode_content) = interaction_mode_content {
            right_side.add_child(interaction_mode_content);
        }
        header_row.add_child(right_side.finish());

        let looks_expanded_downwards =
            self.interaction_mode
                .as_ref()
                .is_some_and(|mode| match mode {
                    InteractionMode::ManuallyExpandable(expansion_config) => {
                        expansion_config.is_expanded
                    }
                });
        let container = Container::new(header_row.finish())
            .with_padding_left(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_padding_right(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
            .with_background(header_background)
            .with_corner_radius(if looks_expanded_downwards {
                CornerRadius::with_top(Radius::Pixels(8.))
            } else {
                CornerRadius::with_all(Radius::Pixels(8.))
            })
            .finish();

        if let Some(InteractionMode::ManuallyExpandable(expansion_config)) = &self.interaction_mode
        {
            if let Some(callback) = &expansion_config.on_toggle_expanded {
                let callback = Rc::clone(callback);
                let mouse_state = expansion_config.toggle_mouse_state.clone();
                return Hoverable::new(mouse_state, |_| container)
                    .on_click(move |ctx, _, _| {
                        callback(ctx);
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish();
            }
        }

        container
    }
}

pub(super) enum FormattedTextOrElement {
    FormattedText(Box<FormattedTextElement>),
    Element(Box<dyn Element>),
}

/// Configuration for rendering a tool/action card using the builder pattern
/// (port of `requested_action::RenderableAction`).
pub(super) struct RenderableAction {
    body: FormattedTextOrElement,
    pub icon: Option<Box<dyn Element>>,
    pub header: Option<HeaderConfig>,
    pub footer: Option<Box<dyn Element>>,
    /// The right-hand element of the body row. In the original this slot held
    /// the Run/Cancel buttons; tool cards put the [`RightCluster`] (+ chevron)
    /// of header-less compact cards here.
    pub trailing: Option<Box<dyn Element>>,
    pub background_color: ColorU,
    /// Adaptation for PRODUCT §19 nesting: a `Task` child card rendered inside
    /// its parent's footer skips the transcript-column spacing wrap and gets a
    /// tight bottom margin instead.
    nested: bool,
    /// Makes the body row itself a hover + click toggle target — the
    /// header-less compact card's equivalent of the ported header's
    /// `ManuallyExpandable` click handling. Scoped to the row (never the
    /// footer) so expanded output stays selectable.
    row_click: Option<(MouseStateHandle, OnToggleExpandedCallback)>,
}

impl RenderableAction {
    pub fn new_with_element(element: Box<dyn Element>, app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Self {
            body: FormattedTextOrElement::Element(element),
            icon: None,
            header: None,
            footer: None,
            trailing: None,
            background_color: blended_colors::neutral_2(theme),
            nested: false,
            row_click: None,
        }
    }

    pub fn new_with_formatted_text(formatted_text: FormattedTextElement, app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Self {
            body: FormattedTextOrElement::FormattedText(Box::new(formatted_text)),
            icon: None,
            header: None,
            footer: None,
            trailing: None,
            background_color: blended_colors::neutral_2(theme),
            nested: false,
            row_click: None,
        }
    }

    pub fn with_nested(mut self, nested: bool) -> Self {
        self.nested = nested;
        self
    }

    pub fn with_row_click<F>(mut self, mouse_state: MouseStateHandle, callback: F) -> Self
    where
        F: Fn(&mut EventContext) + 'static,
    {
        self.row_click = Some((mouse_state, Rc::new(callback)));
        self
    }

    pub fn with_icon(mut self, icon: Box<dyn Element>) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_header(mut self, header: HeaderConfig) -> Self {
        self.header = Some(header);
        self
    }

    pub fn with_footer(mut self, footer: Box<dyn Element>) -> Self {
        self.footer = Some(footer);
        self
    }

    pub fn with_trailing(mut self, trailing: Box<dyn Element>) -> Self {
        self.trailing = Some(trailing);
        self
    }

    /// Renders the card with the current configuration (ported `render`).
    pub fn render(self, app: &AppContext) -> Container {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut has_header = false;
        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(header) = self.header {
            content.add_child(Clipped::new(header.render(app)).finish());
            has_header = true;
        }

        let row =
            render_requested_action_row(self.body, self.icon, self.trailing, true, has_header, app);
        content.add_child(match self.row_click {
            Some((mouse_state, callback)) => Hoverable::new(mouse_state, |_| row)
                .on_click(move |ctx, _, _| callback(ctx))
                .with_cursor(Cursor::PointingHand)
                .finish(),
            None => row,
        });

        if let Some(footer) = self.footer {
            content.add_child(
                Container::new(footer)
                    .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                    .with_vertical_padding(4.)
                    .with_background(theme.surface_1())
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                    .finish(),
            );
        }

        let container = Container::new(content.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(self.background_color)
            .with_border(Border::all(1.).with_border_fill(theme.surface_2()));

        if self.nested {
            Container::new(container.finish()).with_margin_bottom(8.)
        } else if has_header {
            container.finish().with_content_item_spacing()
        } else {
            container.finish().with_agent_output_item_spacing()
        }
    }
}

/// Port of `render_requested_action_body_text`: plain text rendered line-by-line
/// as a selectable [`FormattedTextElement`].
pub(super) fn render_requested_action_body_text(
    text: Cow<str>,
    font_family: FamilyId,
    app: &AppContext,
) -> FormattedTextElement {
    use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let lines = text
        .lines()
        .map(|line| {
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(line.to_owned())])
        })
        .collect::<Vec<_>>();

    let formatted_text = FormattedText::new(lines);
    FormattedTextElement::new(
        formatted_text,
        appearance.monospace_font_size(),
        font_family,
        font_family,
        blended_colors::text_main(theme, theme.background()),
        Default::default(),
    )
    .with_color(blended_colors::text_main(theme, theme.background()))
    .set_selectable(true)
}

/// Port of `render_requested_action_row`: a full-width row with an optional
/// icon, the body content, and an optional trailing element that wraps to a
/// second line on narrow panes.
pub(super) fn render_requested_action_row(
    text: FormattedTextOrElement,
    icon: Option<Box<dyn Element>>,
    trailing: Option<Box<dyn Element>>,
    is_text_selectable: bool,
    has_header_above: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let element = match text {
        FormattedTextOrElement::FormattedText(formatted_text) => {
            formatted_text.set_selectable(is_text_selectable).finish()
        }
        FormattedTextOrElement::Element(element) => element,
    };

    let has_trailing = trailing.is_some();
    let mut text_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if let Some(icon) = icon {
        text_row.add_child(
            Container::new(
                ConstrainedBox::new(icon)
                    .with_width(icon_size(app))
                    .with_height(icon_size(app))
                    .finish(),
            )
            .with_margin_right(ICON_MARGIN)
            .finish(),
        );
    }

    // When a trailing element is present we use a Wrap layout (below) so it
    // flows to a second row on narrow panes. In that case we must NOT wrap the
    // text in Align, because Align always reports the full constraint width,
    // which would inflate the text row and force the trailing element to a new
    // line unconditionally.
    if has_trailing {
        text_row.add_child(Shrinkable::new(1., element).finish());
    } else {
        text_row.add_child(Shrinkable::new(1., Align::new(element).left().finish()).finish());
    }

    let content = if let Some(trailing) = trailing {
        let mut wrap = Wrap::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_run_spacing(8.);
        wrap.extend([WrapFill::new(0., text_row.finish()).finish(), trailing]);
        wrap.finish()
    } else {
        text_row.finish()
    };

    Container::new(content)
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        // The requested action row is overloaded (ported comment): it is the
        // body of a card with a header above, or the entire single-row card.
        .with_vertical_padding(if has_header_above {
            INLINE_ACTION_VERTICAL_PADDING
        } else {
            INLINE_ACTION_HEADER_VERTICAL_PADDING
        })
        .finish()
}

/// Co-ported spacing trait (`view_impl::WithContentItemSpacing`), with the
/// constants re-derived from this pane's message-row geometry (see above).
pub(super) trait WithContentItemSpacing {
    /// Standard margins for a full-width "content item" card.
    fn with_content_item_spacing(self) -> Container;

    /// Content-item spacing with additional left margin, vertically aligning
    /// the card with the transcript's text column (the way Agent Mode aligned
    /// agent output with the user query).
    fn with_agent_output_item_spacing(self) -> Container;
}

impl WithContentItemSpacing for Box<dyn Element> {
    fn with_content_item_spacing(self) -> Container {
        Container::new(self)
            .with_horizontal_margin(CONTENT_HORIZONTAL_PADDING)
            .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
    }

    fn with_agent_output_item_spacing(self) -> Container {
        let left_margin = CONTENT_HORIZONTAL_PADDING + MESSAGE_AVATAR_WIDTH + MESSAGE_AVATAR_MARGIN;
        Container::new(self)
            .with_margin_left(left_margin)
            .with_margin_right(CONTENT_HORIZONTAL_PADDING)
            .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
    }
}
