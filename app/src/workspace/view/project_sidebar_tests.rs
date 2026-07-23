use std::path::{Path, PathBuf};

use super::{
    project_disambiguation, project_matches_search, project_title, resolve_project_directory,
    should_responsively_collapse_right_tool, toggle_right_tool_state,
};
use crate::app_state::RightToolKind;
use crate::workspace::view::ProjectDirectoryResolution;

#[test]
fn project_title_prefers_custom_name_then_assigned_root() {
    assert_eq!(
        project_title(
            Some("API"),
            Some(Path::new("/tmp/repository")),
            &[],
            "Terminal"
        ),
        "API"
    );
    assert_eq!(
        project_title(None, Some(Path::new("/tmp/repository")), &[], "Terminal"),
        "repository"
    );
}

#[test]
fn duplicate_project_context_uses_the_shortest_useful_parent() {
    assert_eq!(
        project_disambiguation(Some(Path::new("/work/acme/api")), 0),
        "acme/api"
    );
    assert_eq!(project_disambiguation(None, 2), "Project 3");
}

#[test]
fn project_title_uses_only_unambiguous_derived_root() {
    assert_eq!(
        project_title(None, None, &[PathBuf::from("/tmp/one")], "Terminal"),
        "one"
    );
    assert_eq!(
        project_title(
            None,
            None,
            &[PathBuf::from("/tmp/one"), PathBuf::from("/tmp/two")],
            "Terminal"
        ),
        "Terminal"
    );
}

#[test]
fn project_search_is_case_insensitive_and_preserves_empty_query() {
    assert!(project_matches_search(
        "REPO",
        ["/tmp/my-repository".to_owned()]
    ));
    assert!(project_matches_search("  ", ["anything".to_owned()]));
    assert!(!project_matches_search(
        "missing",
        ["repository".to_owned()]
    ));
}

#[test]
fn narrow_layout_collapses_the_right_tool_before_the_center() {
    assert!(should_responsively_collapse_right_tool(900., 240., 300.));
    assert!(!should_responsively_collapse_right_tool(1_200., 240., 300.));
}

#[test]
fn right_tool_host_toggles_switches_and_clears_maximize() {
    let opened = toggle_right_tool_state(RightToolKind::Files, false, RightToolKind::Files, false);
    assert!(opened.open);
    assert_eq!(opened.tool, RightToolKind::Files);

    let switched =
        toggle_right_tool_state(RightToolKind::CodeReview, true, RightToolKind::Files, true);
    assert!(switched.open);
    assert_eq!(switched.tool, RightToolKind::Files);
    assert!(switched.clear_code_review_maximize);

    let closed = toggle_right_tool_state(
        RightToolKind::CodeReview,
        true,
        RightToolKind::CodeReview,
        true,
    );
    assert!(!closed.open);
    assert!(closed.clear_code_review_maximize);
}

#[test]
fn project_directory_resolution_handles_assigned_single_and_multiple_roots() {
    let one = tempfile::tempdir().expect("first directory should exist");
    let two = tempfile::tempdir().expect("second directory should exist");

    assert_eq!(
        resolve_project_directory(Some(one.path().to_path_buf()), Vec::new()),
        ProjectDirectoryResolution::Resolved(one.path().to_path_buf())
    );
    assert_eq!(
        resolve_project_directory(None, vec![one.path().to_path_buf()]),
        ProjectDirectoryResolution::Resolved(one.path().to_path_buf())
    );
    assert_eq!(
        resolve_project_directory(
            None,
            vec![one.path().to_path_buf(), two.path().to_path_buf()]
        ),
        ProjectDirectoryResolution::ChooseFrom(vec![
            one.path().to_path_buf(),
            two.path().to_path_buf()
        ])
    );
    assert_eq!(
        resolve_project_directory(Some(one.path().join("missing")), Vec::new()),
        ProjectDirectoryResolution::Unavailable
    );
}
