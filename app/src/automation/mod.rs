//! twarp 20: the automation sidebar surfaces — Scheduled Tasks, Skills, and
//! MCPs — each opening a dedicated full-page main-pane view.
//!
//! 20a ships the shell: sidebar entry points, pane plumbing, and placeholder
//! page content. Later phases (20b+) fill in the real page content.

pub mod mcps_page;
pub mod pane_manager;
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

    /// Placeholder body copy shown until the real page content lands (20b+).
    pub fn placeholder_body(self) -> &'static str {
        match self {
            AutomationPage::ScheduledTasks => "No scheduled tasks yet.",
            AutomationPage::Skills => "No skills yet.",
            AutomationPage::Mcps => "No MCP servers configured yet.",
        }
    }
}
