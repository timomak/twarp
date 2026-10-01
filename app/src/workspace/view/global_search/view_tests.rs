use super::*;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use twarp_core::telemetry::{TelemetryContextModel, TelemetryContextProvider};
use twarpui::{platform::WindowStyle, App};

struct SearchTestTelemetry;

impl TelemetryContextProvider for SearchTestTelemetry {
    fn user_id(&self, _: &AppContext) -> Option<String> {
        None
    }

    fn anonymous_id(&self, _: &AppContext) -> String {
        "global-search-test".to_owned()
    }
}

fn create_search(app: &mut App) -> ViewHandle<GlobalSearchView> {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| -> TelemetryContextModel { Box::new(SearchTestTelemetry) });
    let (_, search) = app.add_window(WindowStyle::NotStealFocus, GlobalSearchView::new);
    search
}

fn sample_match(root: &Path) -> RipgrepMatch {
    RipgrepMatch {
        file_path: root.join("file.rs"),
        line_number: 1,
        line_text: "old result".to_owned(),
        submatches: Vec::new(),
    }
}

fn sample_preview(root: &Path) -> ReplacePreview {
    ReplacePreview {
        fingerprint: make_fingerprint(
            "old".to_owned(),
            SearchConfig {
                use_regex: false,
                use_case_sensitivity: false,
                includes: Vec::new(),
                excludes: Vec::new(),
            },
            vec![root.to_owned()],
        ),
        replacement: "new".to_owned(),
        files: Vec::new(),
    }
}

#[test]
fn project_switch_discards_results_and_late_search_and_preview_completions() {
    App::test((), |mut app| async move {
        let search = create_search(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let beta = directory.path().join("beta");
        search.update(&mut app, |search, ctx| {
            search.set_root_directories(vec![alpha.clone()], ctx);
            search.current_search_id = Some(42);
            search.is_search_in_progress = true;
            search.apply_progress_item(sample_match(&alpha), ctx);
            search.selected_row = Some(RowIndex {
                directory_index: 0,
                index_type: RowIndexType::DirectoryHeader,
            });
            search.replace_preview = Some(sample_preview(&alpha));
            search.replace_is_generating_preview = true;
            let old_generation = search.replace_generation;

            search.set_root_directories(vec![beta.clone()], ctx);
            assert_eq!(search.root_directories, vec![beta]);
            assert!(search.current_search_id.is_none());
            assert!(!search.is_search_in_progress);
            assert!(search.directory_entries.is_empty());
            assert!(search.directory_path_to_directory_index_entry.is_empty());
            assert!(search.selected_row.is_none());
            assert_eq!(search.total_match_count, 0);
            assert!(search.replace_preview.is_none());
            assert!(!search.replace_is_generating_preview);

            search.handle_find_model_event(
                &GlobalSearchEvent::Progress {
                    search_id: 42,
                    result: sample_match(&alpha),
                },
                ctx,
            );
            search.handle_find_model_event(
                &GlobalSearchEvent::Completed {
                    search_id: 42,
                    total_match_count: 20,
                },
                ctx,
            );
            search.finish_replace_preview(old_generation, Ok(sample_preview(&alpha)), ctx);
            assert!(search.directory_entries.is_empty());
            assert_eq!(search.total_match_count, 0);
            assert!(search.replace_preview.is_none());
        });
    });
}

#[test]
fn unchanged_roots_preserve_active_search_and_preview() {
    App::test((), |mut app| async move {
        let search = create_search(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_owned();
        search.update(&mut app, |search, ctx| {
            search.set_root_directories(vec![root.clone()], ctx);
            search.current_search_id = Some(42);
            search.is_search_in_progress = true;
            search.apply_progress_item(sample_match(&root), ctx);
            search.replace_preview = Some(sample_preview(&root));
            let generation = search.replace_generation;

            search.set_root_directories(vec![root], ctx);
            assert_eq!(search.current_search_id, Some(42));
            assert!(search.is_search_in_progress);
            assert_eq!(search.total_match_count, 1);
            assert!(search.replace_preview.is_some());
            assert_eq!(search.replace_generation, generation);
        });
    });
}

#[test]
fn clearing_roots_clears_search_and_replacement_state() {
    App::test((), |mut app| async move {
        let search = create_search(&mut app);
        let directory = tempfile::tempdir().unwrap();
        search.update(&mut app, |search, ctx| {
            search.set_root_directories(vec![directory.path().to_owned()], ctx);
            search.apply_progress_item(sample_match(directory.path()), ctx);
            search.replace_summary = Some(ReplaceApplySummary::default());
            search.search_results_stale = true;

            search.set_root_directories(Vec::new(), ctx);
            assert!(search.search_roots.is_empty());
            assert!(search.directory_entries.is_empty());
            assert!(search.replace_summary.is_none());
            assert!(!search.search_results_stale);
            assert!(!search.can_generate_replace_preview());
        });
    });
}

#[test]
fn project_switch_reruns_retained_query_even_from_results_focus() {
    App::test((), |mut app| async move {
        let search = create_search(&mut app);
        let directory = tempfile::tempdir().unwrap();
        search.update(&mut app, |search, ctx| {
            search.set_root_directories(vec![directory.path().join("alpha")], ctx);
            // Invalid regex fails synchronously, without invoking a search subprocess.
            search.regex_search_enabled = true;
            search.query_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("[", ctx);
            });
            search.last_searched_pattern = Some("[".to_owned());
            search.focus_mode = FocusMode::ResultsList;
            search.set_root_directories(vec![directory.path().join("beta")], ctx);
            assert_eq!(search.last_searched_pattern.as_deref(), Some("["));
            assert_eq!(search.query_editor.as_ref(ctx).buffer_text(ctx), "[");
        });
        search.read(&app, |search, _| {
            assert!(search.current_search_id.is_some());
            assert!(search
                .last_error
                .as_deref()
                .unwrap()
                .contains("Invalid regex"));
        });
    });
}

#[test]
fn clearing_roots_rejects_search_start_queued_in_the_same_update() {
    App::test((), |mut app| async move {
        let search = create_search(&mut app);
        let directory = tempfile::tempdir().unwrap();
        search.update(&mut app, |search, ctx| {
            // Invalid regex queues Started and Failed without a subprocess.
            search.regex_search_enabled = true;
            search.query_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("[", ctx);
            });
            search.set_root_directories(vec![directory.path().to_owned()], ctx);
            assert!(search.current_search_id.is_some());
            // Both events are still queued until this outer update completes.
            search.set_root_directories(Vec::new(), ctx);
            assert!(search.current_search_id.is_none());
        });
        search.read(&app, |search, _| {
            assert!(search.current_search_id.is_none());
            assert!(!search.is_search_in_progress);
            assert!(search.last_error.is_none());
            assert!(search.directory_entries.is_empty());
        });
    });
}
