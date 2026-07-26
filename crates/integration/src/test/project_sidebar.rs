use twarp::features::FeatureFlag;
use twarp::integration_testing::{
    step::new_step_with_default_assertions, terminal::wait_until_bootstrapped_single_pane_for_tab,
    view_getters::workspace_view,
};
use twarp::{workspace::WorkspaceAction, RightToolKind};
use twarpui::{async_assert, integration::TestStep};

use crate::Builder;

use super::new_builder;

const PROJECTS_SIDEBAR_POSITION_ID: &str = "workspace_view:projects_sidebar";
const FIRST_PROJECT_ROW_POSITION_ID: &str = "workspace_view:projects_sidebar:project_row:0";
const FIRST_PROJECT_MENU_POSITION_ID: &str = "workspace_view:projects_sidebar:project_menu:0";
const FIRST_PROJECT_NEW_CHAT_POSITION_ID: &str =
    "workspace_view:projects_sidebar:project_new_chat:0";
const TAB_BAR_POSITION_ID: &str = "workspace_view:tab_bar";

pub fn test_project_sidebar_shell_smoke() -> Builder {
    FeatureFlag::DesignShellV1.set_enabled(true);
    FeatureFlag::ProjectSidebar.set_enabled(true);

    new_builder()
        .set_should_run_test(|| cfg!(target_os = "macos"))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Projects replaces the horizontal tab strip")
                .add_named_assertion(
                    "Projects is visible and tab strip is absent",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |workspace, ctx| {
                            let (projects_open, _, _) = workspace.project_sidebar_test_state();
                            async_assert!(
                                projects_open
                                    && ctx
                                        .element_position_by_id_at_last_frame(
                                            window_id,
                                            PROJECTS_SIDEBAR_POSITION_ID,
                                        )
                                        .is_some()
                                    && ctx
                                        .element_position_by_id_at_last_frame(
                                            window_id,
                                            TAB_BAR_POSITION_ID,
                                        )
                                        .is_none(),
                                "expected Projects to replace the horizontal tab strip"
                            )
                        })
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions("Hovering a project reveals its actions")
                .with_hover_over_saved_position(FIRST_PROJECT_ROW_POSITION_ID)
                .add_named_assertion(
                    "Project menu and new-chat actions are visible",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |_, ctx| {
                            async_assert!(
                                ctx.element_position_by_id_at_last_frame(
                                    window_id,
                                    FIRST_PROJECT_MENU_POSITION_ID,
                                )
                                .is_some()
                                    && ctx
                                        .element_position_by_id_at_last_frame(
                                            window_id,
                                            FIRST_PROJECT_NEW_CHAT_POSITION_ID,
                                        )
                                        .is_some(),
                                "expected project hover to reveal both trailing actions"
                            )
                        })
                    },
                ),
        )
        .with_step(
            TestStep::new("Open Files from the right activity strip")
                .with_action(|app, window_id, _| {
                    let workspace = workspace_view(app, window_id);
                    app.update(|ctx| {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            workspace.id(),
                            &WorkspaceAction::ToggleRightTool(RightToolKind::Files),
                        );
                    });
                })
                .add_named_assertion("Files is the active right tool", |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |workspace, _| {
                        let (_, tool, open) = workspace.project_sidebar_test_state();
                        async_assert!(open && tool == "files")
                    })
                }),
        )
        .with_step(
            TestStep::new("Switch directly to Code Review")
                .with_action(|app, window_id, _| {
                    let workspace = workspace_view(app, window_id);
                    app.update(|ctx| {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            workspace.id(),
                            &WorkspaceAction::ToggleRightTool(RightToolKind::CodeReview),
                        );
                    });
                })
                .add_named_assertion("Code Review replaces Files", |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |workspace, _| {
                        let (_, tool, open) = workspace.project_sidebar_test_state();
                        async_assert!(open && tool == "code_review")
                    })
                }),
        )
        .with_step(
            TestStep::new("Close and reopen Projects")
                .with_action(|app, window_id, _| {
                    let workspace = workspace_view(app, window_id);
                    app.update(|ctx| {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            workspace.id(),
                            &WorkspaceAction::ToggleProjectsSidebar,
                        );
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            workspace.id(),
                            &WorkspaceAction::ToggleProjectsSidebar,
                        );
                    });
                })
                .add_named_assertion(
                    "Projects returns without restoring tabs",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |workspace, ctx| {
                            let (projects_open, tool, tool_open) =
                                workspace.project_sidebar_test_state();
                            async_assert!(
                                projects_open
                                    && tool_open
                                    && tool == "code_review"
                                    && ctx
                                        .element_position_by_id_at_last_frame(
                                            window_id,
                                            TAB_BAR_POSITION_ID,
                                        )
                                        .is_none(),
                                "expected Projects state to return without the tab strip"
                            )
                        })
                    },
                ),
        )
}
