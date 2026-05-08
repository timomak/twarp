use warpui::integration::TestStep;

// twarp: 2c-d — AI facts deleted; stubs.
pub struct AIFactPage;
pub struct AIMemory;

// twarp: 2c-d — AI rule/fact integration test helpers removed (AI deleted).
pub fn create_a_personal_rule(
    _key: impl Into<String>,
    _name: impl Into<String>,
    _content: impl Into<String>,
) -> TestStep {
    TestStep::new("Create a personal rule (no-op)")
}

pub fn open_rule_pane(_window_key: impl Into<String>, _key: impl Into<String>) -> TestStep {
    TestStep::new("Open rule pane (no-op)")
}

pub fn update_rule_content(
    _fact_key: impl Into<String>,
    _new_content: impl Into<String>,
) -> TestStep {
    TestStep::new("Update rule content (no-op)")
}
