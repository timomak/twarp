//! twarp 20/23: the automation sidebar surfaces — Scheduled Tasks and
//! Plugins — each opening a dedicated full-page main-pane view.
//!
//! 20a ships the shell: sidebar entry points, pane plumbing, and placeholder
//! page content. Later phases (20b+) fill in the real page content.

use twarp_core::ui::theme::Fill;
use twarp_core::ui::tokens::{radius, spacing, type_ramp};
use twarp_core::ui::Icon;
use twarpui::elements::{
    ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex,
    ParentElement, Radius, Text,
};
use twarpui::{AppContext, SingletonEntity, ViewHandle};

use crate::appearance::Appearance;
use crate::view_components::action_button::ActionButton;

pub mod pane_manager;
pub mod plugins_page;
pub mod scheduled_tasks_page;
pub mod scheduler;
pub mod view;

/// Which automation page a pane displays. One [`view::AutomationView`] type is
/// parameterized by this enum rather than three near-identical view types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AutomationPage {
    ScheduledTasks,
    /// twarp 23b: the Plugins page replaces the former Skills and MCPs pages.
    Plugins,
    /// twarp 21a: the Pull Requests page reuses the automation pane shell
    /// (pane manager, persistence, sidebar-open plumbing) wholesale.
    PullRequests,
}

impl AutomationPage {
    /// Title shown in the pane header, tab, and page heading.
    pub fn title(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "Scheduled Tasks",
            AutomationPage::Plugins => "Plugins",
            AutomationPage::PullRequests => "Pull Requests",
        }
    }

    /// Stable string for the `automation_panes.page` column (20e). Paired
    /// with [`AutomationPage::from_persistence_str`]; changing a value is a
    /// persistence-format change.
    pub fn as_persistence_str(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "scheduled_tasks",
            AutomationPage::Plugins => "plugins",
            AutomationPage::PullRequests => "pull_requests",
        }
    }

    /// Inverse of [`AutomationPage::as_persistence_str`]. Unknown values
    /// (from a newer build's snapshot) fall back to `None`. The legacy
    /// `"skills"` / `"mcps"` values (pre-23 snapshots) map to the Plugins
    /// page that replaced both, so restored panes don't dead-end.
    pub fn from_persistence_str(s: &str) -> Option<Self> {
        match s {
            "scheduled_tasks" => Some(AutomationPage::ScheduledTasks),
            "plugins" | "skills" | "mcps" => Some(AutomationPage::Plugins),
            "pull_requests" => Some(AutomationPage::PullRequests),
            _ => None,
        }
    }

    /// Placeholder body copy shown until the real page content lands (20b+).
    pub fn placeholder_body(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "No scheduled tasks yet.",
            AutomationPage::Plugins => "No plugins configured yet.",
            AutomationPage::PullRequests => "No pull requests.",
        }
    }
}

/// A square icon chip on a tinted fill — the accent that keeps empty states
/// and suggestion rows from reading as a bare gray void. `icon_size` should be
/// a spacing token; the chip adds `padding` around it.
pub(crate) fn render_icon_chip(
    icon: Icon,
    icon_color: Fill,
    tint: Fill,
    icon_size: f32,
    padding: f32,
) -> Box<dyn Element> {
    render_chip(
        icon.to_warpui_icon(icon_color).finish(),
        tint,
        icon_size,
        padding,
    )
}

/// [`render_icon_chip`] for an arbitrary icon element (e.g. an
/// [`twarp_core::ui::ExternalProductIcon`] brand mark).
pub(crate) fn render_chip(
    icon_element: Box<dyn Element>,
    tint: Fill,
    icon_size: f32,
    padding: f32,
) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(icon_element)
            .with_width(icon_size)
            .with_height(icon_size)
            .finish(),
    )
    .with_uniform_padding(padding)
    .with_background(tint)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish()
}

/// twarp 20e: the shared empty state used by the automation pages —
/// tinted icon chip, a headline, one supporting line, and a primary action,
/// centered in the content column (per the philosophy's empty-state anatomy:
/// "one clear next action, never a bare void"). `button` must be a dedicated
/// handle (not the header's Add button); one view handle cannot be mounted in
/// two places at once.
pub(crate) fn render_empty_state(
    icon: Icon,
    headline: &'static str,
    hint: &'static str,
    button: &ViewHandle<ActionButton>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sub = theme.sub_text_color(theme.background());

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::MD)
            .with_child(render_icon_chip(
                icon,
                theme.accent(),
                theme.accent_overlay(),
                spacing::XL,
                spacing::MD,
            ))
            .with_child(
                Container::new(
                    Text::new_inline(
                        headline,
                        appearance.ui_font_family(),
                        type_ramp::HEADING.size,
                    )
                    .with_line_height_ratio(type_ramp::HEADING.line_height)
                    .with_color(theme.main_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_top(spacing::XS)
                .finish(),
            )
            .with_child(
                Text::new(hint, appearance.ui_font_family(), type_ramp::PROSE.size)
                    .with_line_height_ratio(type_ramp::PROSE.line_height)
                    .with_color(sub.into())
                    .finish(),
            )
            .with_child(
                Container::new(ChildView::new(button).finish())
                    .with_margin_top(spacing::SM)
                    .finish(),
            )
            .finish(),
    )
    .with_margin_top(spacing::XXL)
    .with_margin_bottom(spacing::XXL)
    .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_str_roundtrips() {
        for page in [
            AutomationPage::ScheduledTasks,
            AutomationPage::Plugins,
            AutomationPage::PullRequests,
        ] {
            assert_eq!(
                AutomationPage::from_persistence_str(page.as_persistence_str()),
                Some(page)
            );
        }
    }

    #[test]
    fn legacy_skills_and_mcps_pages_restore_as_plugins() {
        // twarp 23b: `automation_panes.page` values written by pre-23 builds
        // must keep restoring (both surfaces merged into Plugins).
        assert_eq!(
            AutomationPage::from_persistence_str("skills"),
            Some(AutomationPage::Plugins)
        );
        assert_eq!(
            AutomationPage::from_persistence_str("mcps"),
            Some(AutomationPage::Plugins)
        );
        assert_eq!(AutomationPage::from_persistence_str("bogus"), None);
    }
}
