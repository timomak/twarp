use super::{requires_context_override, ProjectContextState};
use std::path::PathBuf;

#[test]
fn retry_ignores_the_previous_directory_check_completion() {
    let mut state = ProjectContextState::default();
    let path = PathBuf::from("/project");
    let previous = state.begin_directory_check(&path).unwrap();
    assert_eq!(state.begin_directory_check(&path), None);

    state.invalidate_directory_checks();
    let current = state.begin_directory_check(&path).unwrap();
    assert!(state.finish_directory_check(path.clone(), current, Ok(())));
    assert!(!state.finish_directory_check(path.clone(), previous, Err("stale".into())));
    assert_eq!(state.directory_checks.get(&path), Some(&Some(Ok(()))));
}

#[test]
fn retry_keeps_a_new_check_pending_when_an_old_success_arrives() {
    let mut state = ProjectContextState::default();
    let path = PathBuf::from("/project");
    let previous = state.begin_directory_check(&path).unwrap();
    state.invalidate_directory_checks();
    let current = state.begin_directory_check(&path).unwrap();

    assert!(!state.finish_directory_check(path.clone(), previous, Ok(())));
    assert_eq!(state.directory_checks.get(&path), Some(&None));
    assert!(state.finish_directory_check(path.clone(), current, Err("missing".into())));
    assert_eq!(
        state.directory_checks.get(&path),
        Some(&Some(Err("missing".into())))
    );
}

#[test]
fn ordinary_session_preserves_all_native_tool_roots() {
    assert!(!requires_context_override(false, false, false, false));
    assert!(requires_context_override(true, false, false, false));
    assert!(requires_context_override(false, true, false, false));
    assert!(requires_context_override(false, false, true, false));
    assert!(requires_context_override(false, false, false, true));
}
