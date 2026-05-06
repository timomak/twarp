use warpui::{
    async_assert, async_assert_eq,
    integration::{AssertionCallback, AssertionWithDataCallback},
    AppContext, SingletonEntity,
};

// twarp: 2c-d — AI facts deleted; stubs.
pub struct AIFactPage;
pub struct CloudAIFactModel;
use crate::{
    cloud_object::model::{generic_string_model::GenericStringObjectId, persistence::CloudModel},
    integration_testing::view_getters::workspace_view,
    server::ids::SyncId,
};

// twarp: 2c-d — assert_rule_exists removed (AI fact / facts module deleted).
pub fn assert_rule_exists(
    _expected_id_key: impl Into<String>,
    _expected_content: impl Into<String>,
) -> AssertionWithDataCallback {
    Box::new(|_app, _window_id, _data| {})
}

/// Assert that the total number of AI facts matches the expected count
pub fn assert_rule_count(expected_count: usize) -> AssertionCallback {
    Box::new(move |app, _| {
        CloudModel::handle(app).read(app, |cloud_model, ctx| {
            let count = rule_count(cloud_model, ctx);
            async_assert_eq!(count, expected_count, "Rule count should match")
        })
    })
}

/// Helper function to count AI facts in the cloud model
pub fn rule_count(cloud_model: &CloudModel, _ctx: &AppContext) -> usize {
    cloud_model
        .get_all_objects_of_type::<GenericStringObjectId, CloudAIFactModel>()
        .count()
}

pub fn assert_rule_pane_open(key: impl Into<String>) -> AssertionWithDataCallback {
    let key = key.into();
    Box::new(move |app, window_id, data| {
        workspace_view(app, window_id).read(app, |workspace, _ctx| {
            let sync_id: &SyncId = data.get(&key).expect("No saved AI fact ID");
            workspace.ai_fact_view().read(app, |ai_fact_view, _ctx| {
                let current_page = ai_fact_view.current_page();
                async_assert_eq!(
                    current_page,
                    AIFactPage::RuleEditor {
                        sync_id: Some(*sync_id)
                    },
                    "Rule pane should be open"
                )
            })
        })
    })
}
