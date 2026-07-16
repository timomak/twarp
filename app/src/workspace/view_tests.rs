use super::{compute_tab_bar_left_padding_value, TAB_BAR_PADDING_LEFT};
use twarp_core::ui::tokens::spacing;

#[test]
fn tab_bar_left_padding_design_shell_sidebar_open_owns_origin() {
    assert_eq!(
        compute_tab_bar_left_padding_value(true, true, false, false, Some(64.)),
        TAB_BAR_PADDING_LEFT
    );
}

#[test]
fn tab_bar_left_padding_keeps_theme_chooser_separate_from_sidebar() {
    assert_eq!(
        compute_tab_bar_left_padding_value(true, false, true, false, Some(64.)),
        0.
    );
}

#[test]
fn tab_bar_left_padding_disabled_design_shell_ignores_sidebar_state() {
    assert_eq!(
        compute_tab_bar_left_padding_value(false, true, false, false, Some(64.)),
        64. + spacing::LG
    );
}

#[test]
fn tab_bar_left_padding_fullscreen_without_lights_uses_plain_padding() {
    assert_eq!(
        compute_tab_bar_left_padding_value(false, false, false, true, Some(64.)),
        TAB_BAR_PADDING_LEFT
    );
}
