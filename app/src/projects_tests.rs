use std::path::PathBuf;

use chrono::{NaiveDate, NaiveDateTime};
use twarpui::{App, SingletonEntity};
use twarpui_extras::user_preferences;

use super::{DirectoryReplacements, ProjectManagementModel, DIRECTORY_REPLACEMENTS_KEY};
use crate::persistence::model::Project;

fn add_private_preferences(app: &mut twarpui::App) {
    app.add_singleton_model(|_| {
        settings::PrivatePreferences::new(
            Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
        )
    });
}

fn mapping_path(path: &str) -> PathBuf {
    std::env::temp_dir()
        .join("twarp-directory-mapping-tests")
        .join(path.trim_start_matches('/'))
}

fn timestamp(day: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, day)
        .expect("test date should be valid")
        .and_hms_opt(12, 0, 0)
        .expect("test time should be valid")
}

#[test]
fn project_paths_are_sorted_by_recency_then_path() {
    App::test((), |mut app| async move {
        add_private_preferences(&mut app);
        let projects = vec![
            Project {
                path: "/work/zeta".to_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(2)),
                name: None,
            },
            Project {
                path: "/work/beta".to_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(3)),
                name: None,
            },
            Project {
                path: "/work/alpha".to_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(3)),
                name: None,
            },
        ];
        app.add_singleton_model(|ctx| ProjectManagementModel::new(projects, None, ctx));

        let paths =
            app.update(|ctx| ProjectManagementModel::as_ref(ctx).project_paths_by_recency());
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/work/alpha"),
                PathBuf::from("/work/beta"),
                PathBuf::from("/work/zeta"),
            ]
        );
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn canonical_project_paths_collapse_to_one_library_identity() {
    App::test((), |mut app| async move {
        add_private_preferences(&mut app);
        let directory = tempfile::tempdir().expect("project directory should exist");
        let canonical_path =
            dunce::canonicalize(directory.path()).expect("project directory should canonicalize");
        std::fs::create_dir(directory.path().join("child"))
            .expect("alias child directory should exist");
        let alias_path = directory.path().join("child").join("..");
        let projects = vec![
            Project {
                path: canonical_path.to_string_lossy().into_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(2)),
                name: None,
            },
            Project {
                path: alias_path.to_string_lossy().into_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(3)),
                name: None,
            },
        ];
        app.add_singleton_model(|ctx| ProjectManagementModel::new(projects, None, ctx));

        let paths =
            app.update(|ctx| ProjectManagementModel::as_ref(ctx).project_paths_by_recency());
        assert_eq!(paths, vec![canonical_path]);
    });
}

#[test]
fn folder_replacement_preserves_suffix_and_uses_the_longest_explicit_prefix() {
    let replacements = DirectoryReplacements::default()
        .with_relocation(mapping_path("/old/repo"), mapping_path("/local/repo"))
        .unwrap()
        .with_relocation(
            mapping_path("/old/repo/nested"),
            mapping_path("/local/separate"),
        )
        .unwrap();
    assert_eq!(
        replacements
            .resolve(&mapping_path("/old/repo/src"))
            .unwrap(),
        mapping_path("/local/repo/src")
    );
    assert_eq!(
        replacements
            .resolve(&mapping_path("/old/repo/nested/src"))
            .unwrap(),
        mapping_path("/local/separate/src")
    );
    assert_eq!(
        replacements
            .resolve(&mapping_path("/old/repository"))
            .unwrap(),
        mapping_path("/old/repository")
    );
}

#[test]
fn folder_replacements_follow_explicit_chains_and_reject_cycles() {
    let replacements = DirectoryReplacements::default()
        .with_relocation(mapping_path("/first"), mapping_path("/second"))
        .unwrap()
        .with_relocation(mapping_path("/second"), mapping_path("/third"))
        .unwrap();
    assert_eq!(
        replacements.resolve(&mapping_path("/first/src")).unwrap(),
        mapping_path("/third/src")
    );
    assert!(replacements
        .with_relocation(mapping_path("/third"), mapping_path("/first"))
        .is_err());
    assert!(DirectoryReplacements::default()
        .with_relocation(mapping_path("/repo"), mapping_path("/repo/nested"))
        .is_err());
}

#[test]
fn relative_or_parent_traversing_folder_replacements_are_rejected() {
    for original in ["relative/repo", "/old/../repo"] {
        assert!(DirectoryReplacements::default()
            .with_relocation(PathBuf::from(original), mapping_path("/local/repo"))
            .is_err());
    }
}

#[test]
fn selecting_the_original_folder_removes_its_replacement() {
    let replacements = DirectoryReplacements::default()
        .with_relocation(mapping_path("/old"), mapping_path("/local"))
        .unwrap()
        .with_relocation(mapping_path("/old"), mapping_path("/old"))
        .unwrap();
    assert_eq!(
        replacements.resolve(&mapping_path("/old/src")).unwrap(),
        mapping_path("/old/src")
    );
}

#[test]
fn relocation_persists_privately_without_changing_project_identity() {
    App::test((), |mut app| async move {
        add_private_preferences(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("old");
        let replacement = directory.path().join("new");
        let projects = vec![Project {
            path: original.to_string_lossy().into_owned(),
            added_ts: timestamp(1),
            last_opened_ts: Some(timestamp(2)),
            name: Some("Existing project".to_owned()),
        }];
        let model = app.add_model(|ctx| ProjectManagementModel::new(projects.clone(), None, ctx));
        model.update(&mut app, |model, ctx| {
            model
                .relocate_directory(original.clone(), replacement.clone(), ctx)
                .unwrap();
            assert_eq!(model.project_paths_by_recency(), vec![original.clone()]);
            assert_eq!(
                model.project_name(&original).as_deref(),
                Some("Existing project")
            );
            assert_eq!(model.resolve_directory(&original), replacement);
        });
        app.update(|ctx| {
            assert!(settings::PrivatePreferences::as_ref(ctx)
                .read_value(DIRECTORY_REPLACEMENTS_KEY)
                .unwrap()
                .is_some());
        });
        let restored = app.add_model(|ctx| ProjectManagementModel::new(projects, None, ctx));
        restored.read(&app, |model, _| {
            assert_eq!(model.resolve_directory(&original), replacement);
            assert_eq!(model.project_paths_by_recency(), vec![original]);
        });
    });
}

#[test]
fn malformed_saved_folder_replacements_leave_original_paths_intact() {
    App::test((), |mut app| async move {
        add_private_preferences(&mut app);
        app.update(|ctx| {
            settings::PrivatePreferences::as_ref(ctx)
                .write_value(DIRECTORY_REPLACEMENTS_KEY, "{invalid".to_owned())
                .unwrap();
        });
        let model = app.add_model(|ctx| ProjectManagementModel::new(Vec::new(), None, ctx));
        model.read(&app, |model, _| {
            assert_eq!(
                model.resolve_directory(std::path::Path::new("/original")),
                PathBuf::from("/original")
            );
        });
    });
}

#[test]
fn failed_relocation_save_keeps_the_previous_directory_mapping() {
    struct RejectingPreferences;
    impl user_preferences::UserPreferences for RejectingPreferences {
        fn write_value(&self, _key: &str, _value: String) -> Result<(), user_preferences::Error> {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into())
        }
        fn read_value(&self, _key: &str) -> Result<Option<String>, user_preferences::Error> {
            Ok(None)
        }
        fn remove_value(&self, _key: &str) -> Result<(), user_preferences::Error> {
            Ok(())
        }
    }
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(Box::new(RejectingPreferences))
        });
        let model = app.add_model(|ctx| ProjectManagementModel::new(Vec::new(), None, ctx));
        let original = mapping_path("old");
        let replacement = mapping_path("new");
        model.update(&mut app, |model, ctx| {
            assert!(model
                .relocate_directory(original.clone(), replacement, ctx)
                .is_err());
            assert_eq!(model.resolve_directory(&original), original);
        });
    });
}
