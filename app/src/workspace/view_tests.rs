use super::{
    compute_tab_bar_left_padding_value, should_render_code_review_rail, TAB_BAR_PADDING_LEFT,
};
use twarp_core::ui::tokens::spacing;

#[test]
fn tab_bar_left_padding_design_shell_sidebar_open_owns_origin() {
    assert_eq!(
        compute_tab_bar_left_padding_value(true, true, false, false, Some(64.), 36.),
        TAB_BAR_PADDING_LEFT
    );
}

#[test]
fn tab_bar_left_padding_keeps_theme_chooser_separate_from_sidebar() {
    assert_eq!(
        compute_tab_bar_left_padding_value(true, false, true, false, Some(64.), 0.),
        0.
    );
}

#[test]
fn tab_bar_left_padding_disabled_design_shell_ignores_sidebar_state() {
    assert_eq!(
        compute_tab_bar_left_padding_value(false, true, false, false, Some(64.), 0.),
        64. + spacing::LG
    );
}

#[test]
fn tab_bar_left_padding_fullscreen_without_lights_uses_plain_padding() {
    assert_eq!(
        compute_tab_bar_left_padding_value(false, false, false, true, Some(64.), 0.),
        TAB_BAR_PADDING_LEFT
    );
}

#[test]
fn tab_bar_left_padding_design_shell_closed_reserves_overlay_toggle_space() {
    assert_eq!(
        compute_tab_bar_left_padding_value(true, false, false, false, Some(64.), 36.),
        64. + spacing::LG + 36.
    );
}

#[test]
fn code_review_rail_stays_visible_during_close_animation() {
    assert!(should_render_code_review_rail(
        true, false, false, false, false, true,
    ));
}

#[test]
fn code_review_rail_is_suppressed_when_maximized_or_outside_design_shell() {
    assert!(!should_render_code_review_rail(
        true, false, false, true, true, false,
    ));
    assert!(!should_render_code_review_rail(
        false, false, false, true, false, false,
    ));
    assert!(!should_render_code_review_rail(
        true, true, false, true, false, false,
    ));
}
