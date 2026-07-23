use std::path::PathBuf;

use chrono::{NaiveDate, NaiveDateTime};
use twarpui::{App, SingletonEntity};

use super::ProjectManagementModel;
use crate::persistence::model::Project;

fn timestamp(day: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, day)
        .expect("test date should be valid")
        .and_hms_opt(12, 0, 0)
        .expect("test time should be valid")
}

#[test]
fn project_paths_are_sorted_by_recency_then_path() {
    App::test((), |mut app| async move {
        let projects = vec![
            Project {
                path: "/work/zeta".to_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(2)),
            },
            Project {
                path: "/work/beta".to_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(3)),
            },
            Project {
                path: "/work/alpha".to_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(3)),
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
            },
            Project {
                path: alias_path.to_string_lossy().into_owned(),
                added_ts: timestamp(1),
                last_opened_ts: Some(timestamp(3)),
            },
        ];
        app.add_singleton_model(|ctx| ProjectManagementModel::new(projects, None, ctx));

        let paths =
            app.update(|ctx| ProjectManagementModel::as_ref(ctx).project_paths_by_recency());
        assert_eq!(paths, vec![canonical_path]);
    });
}
