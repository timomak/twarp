//! twarp 20: the automation sidebar surfaces — Scheduled Tasks, Skills, and
//! MCPs — each opening a dedicated full-page main-pane view.
//!
//! 20a ships the shell: sidebar entry points, pane plumbing, and placeholder
//! page content. Later phases (20b+) fill in the real page content.

use twarp_core::ui::tokens::{spacing, type_ramp};
use twarp_core::ui::Icon;
use twarpui::elements::{
    ChildView, ConstrainedBox, Container, CrossAxisAlignment, Element, Flex, ParentElement, Text,
};
use twarpui::{AppContext, SingletonEntity, ViewHandle};

use crate::appearance::Appearance;
use crate::view_components::action_button::ActionButton;

pub mod mcps_page;
pub mod pane_manager;
pub mod scheduled_tasks_page;
pub mod scheduler;
pub mod skills_page;
pub mod view;

/// Which automation page a pane displays. One [`view::AutomationView`] type is
/// parameterized by this enum rather than three near-identical view types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AutomationPage {
    ScheduledTasks,
    Skills,
    Mcps,
}

impl AutomationPage {
    /// Title shown in the pane header, tab, and page heading.
    pub fn title(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "Scheduled Tasks",
            AutomationPage::Skills => "Skills",
            AutomationPage::Mcps => "MCPs",
        }
    }

    /// Stable string for the `automation_panes.page` column (20e). Paired
    /// with [`AutomationPage::from_persistence_str`]; changing a value is a
    /// persistence-format change.
    pub fn as_persistence_str(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "scheduled_tasks",
            AutomationPage::Skills => "skills",
            AutomationPage::Mcps => "mcps",
        }
    }

    /// Inverse of [`AutomationPage::as_persistence_str`]. Unknown values
    /// (from a newer build's snapshot) fall back to `None`.
    pub fn from_persistence_str(s: &str) -> Option<Self> {
        match s {
            "scheduled_tasks" => Some(AutomationPage::ScheduledTasks),
            "skills" => Some(AutomationPage::Skills),
            "mcps" => Some(AutomationPage::Mcps),
            _ => None,
        }
    }

    /// Placeholder body copy shown until the real page content lands (20b+).
    pub fn placeholder_body(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "No scheduled tasks yet.",
            AutomationPage::Skills => "No skills yet.",
            AutomationPage::Mcps => "No MCP servers configured yet.",
        }
    }
}

/// twarp 20e: the shared empty state used by all three automation pages —
/// icon, one-line hint, and a primary action, centered in the content column
/// (per the philosophy's empty-state anatomy: "one clear next action, never a
/// bare void"). `button` must be a dedicated handle (not the header's Add
/// button); one view handle cannot be mounted in two places at once.
pub(crate) fn render_empty_state(
    icon: Icon,
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
            .with_child(
                ConstrainedBox::new(icon.to_warpui_icon(sub).finish())
                    .with_width(spacing::XL)
                    .with_height(spacing::XL)
                    .finish(),
            )
            .with_child(
                Text::new_inline(hint, appearance.ui_font_family(), type_ramp::PROSE.size)
                    .with_line_height_ratio(type_ramp::PROSE.line_height)
                    .with_color(sub.into())
                    .finish(),
            )
            .with_child(ChildView::new(button).finish())
            .finish(),
    )
    .with_margin_top(spacing::XXL)
    .with_margin_bottom(spacing::XXL)
    .finish()
}
