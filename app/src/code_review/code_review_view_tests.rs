use super::{sidebar_click_opens_diff, SidebarRowId, SidebarSection, SidebarSelectionState};
use std::path::PathBuf;

fn row(section: SidebarSection, path: &str) -> SidebarRowId {
    SidebarRowId::new(section, PathBuf::from(path))
}

fn ordered_rows() -> Vec<SidebarRowId> {
    vec![
        row(SidebarSection::Staged, "a.rs"),
        row(SidebarSection::Staged, "b.rs"),
        row(SidebarSection::Changes, "c.rs"),
        row(SidebarSection::Changes, "d.rs"),
    ]
}

#[test]
fn plain_sidebar_selection_replaces_the_previous_selection() {
    let rows = ordered_rows();
    let mut selection = SidebarSelectionState::default();
    selection.begin_selection(rows[0].clone(), &rows, false, false);
    selection.begin_selection(rows[2].clone(), &rows, false, false);

    assert_eq!(selection.selected.len(), 1);
    assert!(selection.is_selected(&rows[2]));
}

#[test]
fn toggle_sidebar_selection_adds_and_removes_individual_rows() {
    let rows = ordered_rows();
    let mut selection = SidebarSelectionState::default();
    selection.begin_selection(rows[0].clone(), &rows, false, false);
    selection.begin_selection(rows[2].clone(), &rows, false, true);
    assert!(selection.is_selected(&rows[0]));
    assert!(selection.is_selected(&rows[2]));

    selection.begin_selection(rows[0].clone(), &rows, false, true);
    assert!(!selection.is_selected(&rows[0]));
    assert!(selection.is_selected(&rows[2]));
}

#[test]
fn shift_selection_spans_sections_from_the_existing_anchor() {
    let rows = ordered_rows();
    let mut selection = SidebarSelectionState::default();
    selection.begin_selection(rows[1].clone(), &rows, false, false);
    selection.begin_selection(rows[3].clone(), &rows, true, false);

    assert_eq!(selection.selected.len(), 3);
    assert!(selection.is_selected(&rows[1]));
    assert!(selection.is_selected(&rows[2]));
    assert!(selection.is_selected(&rows[3]));
}

#[test]
fn drag_selection_extends_in_both_directions() {
    let rows = ordered_rows();
    let mut selection = SidebarSelectionState::default();
    selection.begin_selection(rows[2].clone(), &rows, false, false);
    selection.drag_to(rows[0].clone(), &rows);

    assert_eq!(selection.selected.len(), 3);
    assert!(selection.is_selected(&rows[0]));
    assert!(selection.is_selected(&rows[1]));
    assert!(selection.is_selected(&rows[2]));
    assert!(!selection.is_selected(&rows[3]));
}

#[test]
fn plain_sidebar_click_opens_diff_but_multiselect_modifiers_do_not() {
    assert!(sidebar_click_opens_diff(false, false));
    assert!(!sidebar_click_opens_diff(true, false));
    assert!(!sidebar_click_opens_diff(false, true));
    assert!(!sidebar_click_opens_diff(true, true));
}
