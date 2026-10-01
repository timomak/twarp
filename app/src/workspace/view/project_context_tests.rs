use super::choose_project;
use std::path::PathBuf;

#[test]
fn assigned_missing_folder_never_falls_back_to_another_session() {
    assert_eq!(
        choose_project(
            Some(PathBuf::from("/missing")),
            Some(PathBuf::from("/other")),
            &[PathBuf::from("/other")]
        ),
        Some(PathBuf::from("/missing")),
    );
}

#[test]
fn rootless_session_prefers_focused_local_directory() {
    assert_eq!(
        choose_project(
            None,
            Some(PathBuf::from("/focused")),
            &[PathBuf::from("/one"), PathBuf::from("/focused")]
        ),
        Some(PathBuf::from("/focused")),
    );
}

#[test]
fn ambiguous_unfocused_roots_require_a_selection() {
    assert_eq!(
        choose_project(None, None, &[PathBuf::from("/one"), PathBuf::from("/two")]),
        None
    );
    assert_eq!(choose_project(None, None, &[]), None);
    assert_eq!(
        choose_project(None, None, &[PathBuf::from("/one")]),
        Some(PathBuf::from("/one"))
    );
}
