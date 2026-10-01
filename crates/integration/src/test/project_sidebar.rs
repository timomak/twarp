use std::{fs, path::PathBuf};

use pathfinder_geometry::{rect::RectF, vector::vec2f};
use twarp::features::FeatureFlag;
use twarp::integration_testing::{
    step::new_step_with_default_assertions,
    terminal::{
        execute_command_for_single_terminal_in_tab, util::ExpectedExitStatus,
        wait_until_bootstrapped_single_pane_for_tab,
    },
    view_getters::workspace_view,
};
use twarp::{workspace::WorkspaceAction, RightToolKind};
use twarpui::{
    async_assert,
    integration::{AssertionOutcome, TestStep},
    App, WindowId,
};

use crate::{util::write_all_rc_files_for_test, Builder};

use super::new_builder;

const PROJECTS_SIDEBAR_POSITION_ID: &str = "workspace_view:projects_sidebar";
const FIRST_PROJECT_ROW_POSITION_ID: &str = "workspace_view:projects_sidebar:project_row:0";
const FIRST_PROJECT_MENU_POSITION_ID: &str = "workspace_view:projects_sidebar:project_menu:0";
const FIRST_PROJECT_NEW_CHAT_POSITION_ID: &str =
    "workspace_view:projects_sidebar:project_new_chat:0";
const TAB_BAR_POSITION_ID: &str = "workspace_view:tab_bar";

const CONTEXT_FIXTURE_ENV: &str = "WARP_PROJECT_CONTEXT_FIXTURE";
const CONTEXT_MARKER_POSITION: &str = "file_tree_item:project-context-marker.txt";

fn context_directory(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(CONTEXT_FIXTURE_ENV).expect("context fixture was configured"))
        .join(name)
}

fn dispatch_workspace(app: &mut App, window_id: WindowId, action: WorkspaceAction) {
    let workspace = workspace_view(app, window_id);
    app.update(|ctx| ctx.dispatch_typed_action_for_view(window_id, workspace.id(), &action));
}

fn open_fixture_terminal() -> TestStep {
    TestStep::new("Replace the initial Welcome page with a terminal")
        .with_action(|app, window_id, _| {
            let needs_terminal = workspace_view(app, window_id).read(app, |workspace, ctx| {
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                    .is_none()
            });
            if needs_terminal {
                dispatch_workspace(
                    app,
                    window_id,
                    WorkspaceAction::AddTerminalTab {
                        hide_homepage: true,
                    },
                );
                dispatch_workspace(app, window_id, WorkspaceAction::CloseTab(0));
            }
        })
        .add_named_assertion("The fixture has one terminal tab", |app, window_id| {
            workspace_view(app, window_id).read(app, |workspace, ctx| {
                async_assert!(
                    workspace.tab_count() == 1
                        && workspace
                            .active_tab_pane_group()
                            .as_ref(ctx)
                            .active_session_view(ctx)
                            .is_some()
                )
            })
        })
}

fn project_context_builder() -> Builder {
    FeatureFlag::DesignShellV1.set_enabled(true);
    FeatureFlag::ProjectSidebar.set_enabled(true);
    new_builder()
        .set_should_run_test(|| cfg!(target_os = "macos"))
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = fs::canonicalize(utils.test_dir()).expect("test directory exists");
            for name in ["alpha", "beta"] {
                let directory = test_dir.join(name);
                fs::create_dir(&directory).expect("create local project fixture");
                fs::write(directory.join("project-context-marker.txt"), name)
                    .expect("write Files marker");
            }
            utils.set_env(
                CONTEXT_FIXTURE_ENV,
                Some(test_dir.to_string_lossy().into_owned()),
            );
            // Even a folder without a remote triggers the best-effort gh viewer
            // lookup. A fixture-only executable prevents any network or auth use.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let bin = test_dir.join("bin");
                fs::create_dir(&bin).expect("create fixture executable directory");
                let gh = bin.join("gh");
                fs::write(&gh, "#!/bin/sh\nexit 1\n").expect("write offline gh stub");
                fs::set_permissions(&gh, fs::Permissions::from_mode(0o755))
                    .expect("make offline gh stub executable");
                let mut paths = vec![bin];
                paths.extend(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                ));
                utils.set_env(
                    "PATH",
                    Some(
                        std::env::join_paths(paths)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    ),
                );
            }
            write_all_rc_files_for_test(
                &test_dir,
                "cd \"$WARP_PROJECT_CONTEXT_FIXTURE/alpha\"".to_owned(),
            );
        })
        .with_step(open_fixture_terminal())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
}

fn assert_context_with_visible_files(app: &mut App, window_id: WindowId) -> AssertionOutcome {
    let expected = context_directory("alpha");
    let workspace = workspace_view(app, window_id);
    workspace.read(app, |workspace, ctx| {
        let (_, roots, store) = workspace.project_context_test_state(ctx);
        let Some(store) = store else {
            return AssertionOutcome::failure("PR page has no store".into());
        };
        let store = store.as_ref(ctx);
        let (_, tool, open) = workspace.project_sidebar_test_state();
        async_assert!(
            store.projects().contains(&expected)
                && store.selected_repo() == Some(expected.as_path())
                && roots == vec![expected]
                && open
                && tool == "files"
                && ctx
                    .element_position_by_id_at_last_frame(window_id, CONTEXT_MARKER_POSITION)
                    .is_some(),
            "PR selection and visible Files must keep the session's local folder on global pages"
        )
    })
}

pub fn test_rootless_session_pr_and_global_files_context() -> Builder {
    project_context_builder()
        .with_step(
            TestStep::new("Rootless session exposes its actual working directory")
                .add_named_assertion(
                    "Session identity is unassigned and Files has its cwd",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |workspace, ctx| {
                            let (assigned, roots, _) = workspace.project_context_test_state(ctx);
                            async_assert!(
                                assigned.is_none() && roots == vec![context_directory("alpha")]
                            )
                        })
                    },
                ),
        )
        .with_step(
            TestStep::new("Open Files at a width that fits both sidebars")
                .with_action(|app, window_id, _| {
                    app.update(|ctx| {
                        let origin = ctx.window_bounds(&window_id).unwrap().origin();
                        ctx.set_and_cache_window_bounds(
                            window_id,
                            RectF::new(origin, vec2f(1600., 1000.)),
                        );
                    });
                    let needs_open = workspace_view(app, window_id).read(app, |workspace, _| {
                        let (_, tool, open) = workspace.project_sidebar_test_state();
                        !open || tool != "files"
                    });
                    if needs_open {
                        dispatch_workspace(
                            app,
                            window_id,
                            WorkspaceAction::ToggleRightTool(RightToolKind::Files),
                        );
                    }
                })
                .add_named_assertion("The session's file is visible", |app, window_id| {
                    app.read(|ctx| {
                        async_assert!(ctx
                            .element_position_by_id_at_last_frame(
                                window_id,
                                CONTEXT_MARKER_POSITION
                            )
                            .is_some())
                    })
                }),
        )
        .with_step(
            TestStep::new("Open PRs from an unassigned local session")
                .with_action(|app, window_id, _| {
                    dispatch_workspace(app, window_id, WorkspaceAction::ShowPullRequests)
                })
                .add_named_assertion(
                    "PRs resolve the cwd and Files remains usable",
                    assert_context_with_visible_files,
                )
                .add_named_assertion(
                    "A non-git folder reports a remote error instead of no project",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |workspace, ctx| {
                            let (_, _, store) = workspace.project_context_test_state(ctx);
                            let data = store
                                .as_ref()
                                .and_then(|store| store.as_ref(ctx).selected_data());
                            async_assert!(data.is_some_and(|data| data.fetched
                                && !data.loading
                                && !data.directory_unavailable
                                && data
                                    .error
                                    .as_ref()
                                    .is_some_and(|error| error.contains("origin remote"))))
                        })
                    },
                ),
        )
        .with_step(
            TestStep::new("Navigate from PRs to Settings")
                .with_action(|app, window_id, _| {
                    dispatch_workspace(app, window_id, WorkspaceAction::ShowSettings)
                })
                .add_named_assertion(
                    "Settings retains the PR folder and visible Files",
                    assert_context_with_visible_files,
                ),
        )
        .with_step(
            TestStep::new("Return to the session without assigning its identity")
                .with_action(|app, window_id, _| {
                    dispatch_workspace(app, window_id, WorkspaceAction::ActivateTab(0))
                })
                .add_named_assertion(
                    "The original session remains unassigned",
                    |app, window_id| {
                        let workspace = workspace_view(app, window_id);
                        workspace.read(app, |workspace, ctx| {
                            let (assigned, roots, _) = workspace.project_context_test_state(ctx);
                            async_assert!(
                                assigned.is_none() && roots == vec![context_directory("alpha")]
                            )
                        })
                    },
                ),
        )
}

pub fn test_project_pr_context_is_independent_between_windows() -> Builder {
    project_context_builder()
        .with_step(TestStep::new("Open the first window's PR page")
            .with_action(|app, window_id, _| dispatch_workspace(app, window_id, WorkspaceAction::ShowPullRequests))
            .add_named_assertion("The first page selects alpha", |app, window_id| {
                let workspace = workspace_view(app, window_id);
                workspace.read(app, |workspace, ctx| {
                    let (_, _, store) = workspace.project_context_test_state(ctx);
                    async_assert!(store.is_some_and(|store| store.as_ref(ctx).selected_repo() == Some(context_directory("alpha").as_path())))
                })
            }))
        .with_step(TestStep::new("Open a second window")
            .with_action(|app, window_id, data| {
                data.insert("context_first_window", window_id);
                app.dispatch_global_action("root_view:open_new", ());
            })
            .add_named_assertion_with_data_from_prior_step("The new window becomes active", |app, _, data| {
                let first_window: WindowId = *data.get("context_first_window").expect("first window was saved");
                app.read(|ctx| async_assert!(app.window_ids().len() == 2 && ctx.windows().active_window().is_some_and(|window| window != first_window)))
            }))
        .with_step(open_fixture_terminal())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(0, "cd \"$WARP_PROJECT_CONTEXT_FIXTURE/beta\"".to_owned(), ExpectedExitStatus::Success, ()))
        .with_step(TestStep::new("Open PRs at the second window's working directory")
            .with_action(|app, window_id, _| dispatch_workspace(app, window_id, WorkspaceAction::ShowPullRequests))
            .add_named_assertion("The second page selects beta", |app, window_id| {
                let workspace = workspace_view(app, window_id);
                workspace.read(app, |workspace, ctx| {
                    let (_, roots, store) = workspace.project_context_test_state(ctx);
                    let expected = context_directory("beta");
                    async_assert!(roots == vec![expected.clone()] && store.is_some_and(|store| store.as_ref(ctx).selected_repo() == Some(expected.as_path())))
                })
            }))
        .with_step(TestStep::new("Change only the second PR page's author filter")
            .with_action(|app, window_id, _| {
                let workspace = workspace_view(app, window_id);
                let store = workspace.read(app, |workspace, ctx| workspace.project_context_test_state(ctx).2.unwrap());
                store.update(app, |store, ctx| store.set_author_filter(Some("fixture-author".to_owned()), ctx));
            })
            .add_named_assertion_with_data_from_prior_step("Both windows retain distinct stores and selections", |app, window_id, data| {
                let first_window: WindowId = *data.get("context_first_window").expect("first window was saved");
                let first = workspace_view(app, first_window);
                let second = workspace_view(app, window_id);
                let first_store = first.read(app, |workspace, ctx| workspace.project_context_test_state(ctx).2.unwrap());
                let second_store = second.read(app, |workspace, ctx| workspace.project_context_test_state(ctx).2.unwrap());
                app.read(|ctx| async_assert!(
                    first_window != window_id && first_store.id() != second_store.id()
                        && first_store.as_ref(ctx).selected_repo() == Some(context_directory("alpha").as_path())
                        && second_store.as_ref(ctx).selected_repo() == Some(context_directory("beta").as_path())
                        && first_store.as_ref(ctx).author_filter().is_none()
                        && second_store.as_ref(ctx).author_filter() == Some("fixture-author"),
                    "changing PR context or filters in one window must leave the other page unchanged"
                ))
            }))
}

pub fn test_restored_pr_requires_choice_for_multiple_projects() -> Builder {
    project_context_builder().with_step(
        TestStep::new("Restore PRs followed by two saved project sessions")
            .with_action(|app, window_id, data| {
                let workspace = workspace_view(app, window_id);
                let restored_window = workspace.update(app, |workspace, ctx| {
                    workspace.restore_project_context_test_window(
                        vec![context_directory("alpha"), context_directory("beta")],
                        ctx,
                    )
                });
                data.insert("restored_context_window", restored_window);
            })
            .add_named_assertion_with_data_from_prior_step(
                "Restored PRs do not inherit a transient restore activation",
                |app, _, data| {
                    let window_id: WindowId = *data.get("restored_context_window").unwrap();
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |workspace, ctx| {
                        let (_, roots, store) = workspace.project_context_test_state(ctx);
                        let Some(store) = store else {
                            return AssertionOutcome::failure(
                                "Restored PR page has no store".into(),
                            );
                        };
                        let store = store.as_ref(ctx);
                        async_assert!(
                            workspace.tab_count() == 3
                                && store.projects().contains(&context_directory("alpha"))
                                && store.projects().contains(&context_directory("beta"))
                                && store.selected_repo().is_none()
                                && !store.has_explicit_selection()
                                && roots.is_empty(),
                            "multiple restored projects require a deliberate selection"
                        )
                    })
                },
            ),
    )
}

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
