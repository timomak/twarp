use std::path::PathBuf;

use twarpui::App;

use super::{
    command_start_error, run_in_repo, run_in_repo_with_stdin, FetchResult, PrDetailData,
    PrStateFilter, PullRequestsStoreModel,
};

#[test]
fn page_owned_stores_keep_selection_and_filter_independent() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let beta = directory.path().join("beta");
        let first = app.add_model(PullRequestsStoreModel::new);
        let second = app.add_model(PullRequestsStoreModel::new);

        first.update(&mut app, |store, ctx| {
            store.set_projects(vec![alpha.clone()], None, false, ctx);
        });
        second.update(&mut app, |store, ctx| {
            store.set_projects(vec![beta.clone()], None, false, ctx);
            store.set_filter(PrStateFilter::All, ctx);
        });

        first.read(&app, |store, _| {
            assert_eq!(store.selected_repo(), Some(alpha.as_path()));
            assert_eq!(store.filter(), PrStateFilter::Open);
        });
        second.read(&app, |store, _| {
            assert_eq!(store.selected_repo(), Some(beta.as_path()));
            assert_eq!(store.filter(), PrStateFilter::All);
        });
    });
}

#[test]
fn rediscovery_preserves_explicit_selection_without_refetching_unchanged_context() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let beta = directory.path().join("beta");
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            store.set_projects(vec![alpha.clone(), beta.clone()], None, false, ctx);
            store.select_repo(beta.clone(), ctx);
            let generation = store.generation;

            store.set_projects(vec![beta.clone(), alpha.clone()], None, false, ctx);
            assert_eq!(store.selected_repo(), Some(beta.as_path()));
            assert_eq!(store.generation, generation);

            store.set_projects(vec![alpha, beta], None, true, ctx);
            assert_eq!(store.generation, generation + 1);
        });
    });
}

#[test]
fn changing_context_drops_detail_and_author_filter() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let beta = directory.path().join("beta");
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            store.set_projects(vec![alpha.clone()], None, false, ctx);
            store.detail = Some(((alpha, 7), PrDetailData::default()));
            store.author_filter = Some("old-repository-author".to_owned());
            let detail_generation = store.detail_generation;
            let files_generation = store.files_generation;

            store.set_projects(vec![beta.clone()], Some(beta.clone()), false, ctx);

            assert_eq!(store.selected_repo(), Some(beta.as_path()));
            assert!(store.detail.is_none());
            assert_eq!(store.author_filter(), None);
            assert_eq!(store.detail_generation, detail_generation + 1);
            assert_eq!(store.files_generation, files_generation + 1);
        });
    });
}

#[test]
fn clearing_context_rejects_an_in_flight_fetch() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let repo = directory.path().join("missing");
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            store.set_projects(vec![repo.clone()], None, false, ctx);
            let generation = store.generation;
            store.set_projects(Vec::new(), None, false, ctx);
            store.apply_fetch(
                FetchResult {
                    repo: repo.clone(),
                    generation,
                    viewer: None,
                    outcome: Ok(Vec::new()),
                    directory_unavailable: false,
                },
                ctx,
            );

            assert_eq!(store.selected_repo(), None);
            assert!(store.projects().is_empty());
            assert!(!store.data[&repo].fetched);
            assert_eq!(store.generation, generation + 1);
        });
    });
}

#[test]
fn stale_picker_action_cannot_select_a_removed_repository() {
    App::test((), |mut app| async move {
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            store.select_repo(PathBuf::from("/removed-project"), ctx);
            assert_eq!(store.selected_repo(), None);
            assert_eq!(store.generation, 0);
        });
    });
}

#[test]
fn missing_folder_is_reported_before_executable_resolution_for_reads_and_writes() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    let read_error = run_in_repo(&missing, "twarp-test-missing-executable", &[]).unwrap_err();
    let write_error =
        run_in_repo_with_stdin(&missing, "twarp-test-missing-executable", &[], "body").unwrap_err();

    assert!(read_error.contains("Project folder"));
    assert!(read_error.contains(&missing.display().to_string()));
    assert_eq!(write_error, read_error);
    assert!(!read_error.contains("PATH"));
}

#[test]
fn missing_executable_in_an_existing_folder_has_its_own_error() {
    let directory = tempfile::tempdir().unwrap();
    let error = run_in_repo(directory.path(), "twarp-test-missing-executable", &[]).unwrap_err();
    assert_eq!(
        error,
        "`twarp-test-missing-executable` was not found on your PATH."
    );
}

#[test]
fn removed_folder_after_validation_is_not_misreported_as_missing_executable() {
    let directory = tempfile::tempdir().unwrap();
    let removed = directory.path().join("removed");
    std::fs::create_dir(&removed).unwrap();
    super::validate_repo_directory(&removed).unwrap();
    std::fs::remove_dir(&removed).unwrap();
    let error = command_start_error(
        &removed,
        "git",
        std::io::Error::from(std::io::ErrorKind::NotFound),
    );
    assert!(error.contains("Project folder"));
    assert!(!error.contains("PATH"));
}

#[test]
fn ambiguous_candidates_require_a_choice_and_discovery_keeps_that_choice() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let beta = directory.path().join("beta");
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            store.set_projects(vec![alpha.clone(), beta.clone()], None, false, ctx);
            assert_eq!(store.selected_repo(), None);
            assert_eq!(store.generation, 0);
            store.select_repo(beta.clone(), ctx);
            store.set_projects(vec![alpha.clone(), beta.clone()], Some(alpha), false, ctx);
            assert_eq!(store.selected_repo(), Some(beta.as_path()));
        });
    });
}

#[test]
fn automatic_selection_follows_context_but_explicit_choice_wins() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let beta = directory.path().join("beta");
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            let projects = vec![alpha.clone(), beta.clone()];
            store.set_projects(projects.clone(), Some(alpha.clone()), false, ctx);
            assert!(!store.has_explicit_selection());
            store.set_projects(projects.clone(), Some(beta.clone()), false, ctx);
            assert_eq!(store.selected_repo(), Some(beta.as_path()));

            // Choosing the already selected project still establishes priority.
            store.select_repo(beta.clone(), ctx);
            assert!(store.has_explicit_selection());
            store.set_projects(projects, Some(alpha), false, ctx);
            assert_eq!(store.selected_repo(), Some(beta.as_path()));
        });
    });
}

#[test]
fn explicit_selection_survives_folder_relocation() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let replacement = directory.path().join("replacement");
        let other = directory.path().join("other");
        let model = app.add_model(PullRequestsStoreModel::new);
        model.update(&mut app, |store, ctx| {
            store.set_projects(vec![original.clone()], None, false, ctx);
            store.select_repo(original, ctx);
            store.set_projects(
                vec![replacement.clone(), other.clone()],
                Some(replacement.clone()),
                false,
                ctx,
            );
            assert!(store.has_explicit_selection());
            store.set_projects(
                vec![replacement.clone(), other.clone()],
                Some(other),
                false,
                ctx,
            );
            assert_eq!(store.selected_repo(), Some(replacement.as_path()));
        });
    });
}
