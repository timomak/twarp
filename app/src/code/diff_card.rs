//! Shared **diff-card chrome** — the flat, collapsible disclosure row used by
//! both the Claude Code pane's tool/diff cards and the Open Changes (Code
//! Review) panel's file list.
//!
//! It started life as the Agent-Mode `inline_action` chrome ported for the
//! Claude pane (feature 07, sub-phase 7d). It was hoisted here, into the neutral
//! `code` module, so the Code Review panel can render each file change as the
//! same collapse-on-click card with a colored diff body — one component, one
//! visual contract for "a file change you can expand" everywhere twarp shows
//! one. The Claude pane keeps importing these names through
//! `claude_code_view::inline_action`, which re-exports them.
//!
//! What lives here is only the *chrome* (header glyph + label + result cluster +
//! chevron, and the bordered body box revealed on expand). The colored diff
//! *body* is the caller's: an `InlineDiffView` for the Claude pane, the
//! interactive code-review editor for Open Changes — both already colored via
//! the shared `editor::diff` palette.

use std::rc::Rc;

use twarp_core::ui::theme::AnsiColorIdentifier;
use twarpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded, Flex,
        Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
        Shrinkable, Text,
    },
    platform::Cursor,
    AppContext, Element, EventContext, SingletonEntity,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon as WarpIcon;
use crate::view_components::compactible_action_button::render_expansion_icon;

/// Margin between the cluster label and its status icon (and between a glyph and
/// its label). Ported from the AI block's `inline_action` spacing.
pub(crate) const ICON_MARGIN: f32 = 8.;

/// Left indent that lines a top-level disclosure up with the Claude transcript's
/// text column (message-row padding + avatar width + avatar margin = 14+16+12).
/// The default so the Claude pane's call sites need no change; other consumers
/// (Code Review) override it via [`Disclosure::with_outer_margins`].
const DEFAULT_LEFT_MARGIN: f32 = 14. + 16. + 12.;
/// Right margin matching the Claude transcript's message-row padding.
const DEFAULT_RIGHT_MARGIN: f32 = 14.;
/// Tight gap below each disclosure row so consecutive cards stack closely.
const DEFAULT_BOTTOM_MARGIN: f32 = 4.;

/// Returns the size for icons in the card chrome, scaled to the user's current
/// font size. (Port of `inline_action_icons::icon_size`.)
pub(crate) fn icon_size(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);
    app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    )
}

/// Status icon for a running tool (port of `agent::icons::yellow_running_icon`).
pub(crate) fn running_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::Circle.into(),
        AnsiColorIdentifier::Yellow.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Status icon for a completed tool (port of `green_check_icon`).
pub(crate) fn green_check_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::Check.into(),
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Status icon for a failed tool (port of `red_x_icon`).
pub(crate) fn red_x_icon(appearance: &Appearance) -> twarpui::elements::Icon {
    twarpui::elements::Icon::new(
        WarpIcon::X.into(),
        AnsiColorIdentifier::Red.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub(crate) type OnToggleExpandedCallback = Rc<dyn Fn(&mut EventContext) + 'static>;

/// The muted right-hand cluster of a card row: an optional result label (e.g.
/// `+5 −3`) and an optional status icon. (The right-label + icon slot the
/// sibling leaf `search_results_common.rs` rendered.)
#[derive(Default)]
pub(crate) struct RightCluster {
    pub label: Option<String>,
    pub icon: Option<twarpui::elements::Icon>,
}

impl RightCluster {
    /// Render the cluster's label + icon, each separated by the ported 8px
    /// margin (`search_results_common` spacing).
    pub(crate) fn render(self, appearance: &Appearance, app: &AppContext) -> Vec<Box<dyn Element>> {
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

/// A flat, borderless disclosure row: a leading glyph + label + (optional)
/// trailing action + chevron, with the result/status cluster and a bordered
/// content box revealed **only when expanded**. The one collapsible-card
/// component twarp uses for tool calls, diff/thinking cards, the task list, and
/// each file change in the Open Changes panel — so they all read the same.
pub(crate) struct Disclosure {
    title: Box<dyn Element>,
    glyph: Option<twarpui::elements::Icon>,
    expanded: bool,
    expandable: bool,
    /// Result label + status icon — rendered in the row's trailing cluster only
    /// while expanded.
    cluster: RightCluster,
    /// A persistent trailing element (e.g. an "open file" button) rendered
    /// before the chevron, in both states. `None` for the Claude pane's cards.
    trailing_action: Option<Box<dyn Element>>,
    mouse_state: MouseStateHandle,
    on_toggle: Option<OnToggleExpandedCallback>,
    /// The content shown below the label when expanded.
    body: Option<Box<dyn Element>>,
    /// When true (the default) the body is wrapped in a padded, bordered,
    /// rounded box on the lower surface. Consumers whose body brings its own
    /// chrome (the code-review editor) set this false.
    body_boxed: bool,
    /// A child rendered inside a parent's box (a `Task`'s sub-calls): no
    /// outer indent, just a tight bottom margin.
    nested: bool,
    left_margin: f32,
    right_margin: f32,
    bottom_margin: f32,
}

impl Disclosure {
    pub(crate) fn new(title: Box<dyn Element>) -> Self {
        Self {
            title,
            glyph: None,
            expanded: false,
            expandable: false,
            cluster: RightCluster::default(),
            trailing_action: None,
            mouse_state: MouseStateHandle::default(),
            on_toggle: None,
            body: None,
            body_boxed: true,
            nested: false,
            left_margin: DEFAULT_LEFT_MARGIN,
            right_margin: DEFAULT_RIGHT_MARGIN,
            bottom_margin: DEFAULT_BOTTOM_MARGIN,
        }
    }

    pub(crate) fn with_glyph(mut self, glyph: twarpui::elements::Icon) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub(crate) fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub(crate) fn expandable(mut self, expandable: bool) -> Self {
        self.expandable = expandable;
        self
    }

    pub(crate) fn with_cluster(mut self, cluster: RightCluster) -> Self {
        self.cluster = cluster;
        self
    }

    pub(crate) fn with_trailing_action(mut self, action: Box<dyn Element>) -> Self {
        self.trailing_action = Some(action);
        self
    }

    pub(crate) fn with_mouse_state(mut self, mouse_state: MouseStateHandle) -> Self {
        self.mouse_state = mouse_state;
        self
    }

    pub(crate) fn on_toggle<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut EventContext) + 'static,
    {
        self.on_toggle = Some(Rc::new(callback));
        self
    }

    pub(crate) fn with_body(mut self, body: Box<dyn Element>) -> Self {
        self.body = Some(body);
        self
    }

    /// Render the expanded body raw, without the default bordered box (for a
    /// body that brings its own chrome, like the code-review editor).
    pub(crate) fn with_unboxed_body(mut self) -> Self {
        self.body_boxed = false;
        self
    }

    pub(crate) fn nested(mut self, nested: bool) -> Self {
        self.nested = nested;
        self
    }

    /// Override the outer margins (Code Review aligns the card to the panel's
    /// own padding rather than the Claude transcript column).
    pub(crate) fn with_outer_margins(mut self, left: f32, right: f32, bottom: f32) -> Self {
        self.left_margin = left;
        self.right_margin = right;
        self.bottom_margin = bottom;
        self
    }

    pub(crate) fn render(self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut left = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(glyph) = self.glyph {
            left.add_child(
                Container::new(
                    ConstrainedBox::new(glyph.finish())
                        .with_width(icon_size(app))
                        .with_height(icon_size(app))
                        .finish(),
                )
                .with_margin_right(ICON_MARGIN)
                .finish(),
            );
        }
        left.add_child(Expanded::new(1., self.title).finish());

        let mut row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., left.finish()).finish());

        let mut trailing = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        // The cluster (result label + status icon) is part of the reveal: it
        // only appears once the row is expanded.
        if self.expanded {
            for child in self.cluster.render(appearance, app) {
                trailing.add_child(child);
            }
        }
        if let Some(action) = self.trailing_action {
            trailing.add_child(
                Container::new(action)
                    .with_margin_right(ICON_MARGIN)
                    .finish(),
            );
        }
        if self.expandable {
            trailing.add_child(
                ConstrainedBox::new(render_expansion_icon(self.expanded, false, appearance, app))
                    .with_width(icon_size(app))
                    .with_height(icon_size(app))
                    .finish(),
            );
        }
        row.add_child(trailing.finish());

        let header = Container::new(row.finish())
            .with_horizontal_padding(2.)
            .with_vertical_padding(5.)
            .finish();

        let header = match (self.expandable, self.on_toggle) {
            (true, Some(callback)) => Hoverable::new(self.mouse_state, |_| header)
                .on_click(move |ctx, _, _| callback(ctx))
                .with_cursor(Cursor::PointingHand)
                .finish(),
            _ => header,
        };

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(header);

        if self.expanded {
            if let Some(body) = self.body {
                let body = if self.body_boxed {
                    // A bordered, rounded box on the lower surface — the
                    // boundary shown below the label only on expand.
                    Container::new(body)
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(8.)
                        .with_background(theme.surface_1())
                        .with_border(Border::all(1.).with_border_fill(theme.surface_2()))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                        .finish()
                } else {
                    body
                };
                column.add_child(Container::new(body).with_margin_top(6.).finish());
            }
        }

        let content = column.finish();
        if self.nested {
            Container::new(content)
                .with_margin_bottom(self.bottom_margin)
                .finish()
        } else {
            Container::new(content)
                .with_margin_left(self.left_margin)
                .with_margin_right(self.right_margin)
                .with_margin_bottom(self.bottom_margin)
                .finish()
        }
    }
}
