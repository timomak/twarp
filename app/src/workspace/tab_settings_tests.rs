use super::*;
use crate::test_util::settings::initialize_settings_for_tests;
use settings::Setting;
use twarpui::{App, SingletonEntity};

#[test]
fn color_for_directory_uses_longest_prefix_color_match() {
    let colors = DirectoryTabColors(
        [
            (
                "/a".to_string(),
                DirectoryTabColor::Color(AnsiColorIdentifier::Red),
            ),
            (
                "/a/b".to_string(),
                DirectoryTabColor::Color(AnsiColorIdentifier::Green),
            ),
        ]
        .into_iter()
        .collect(),
    );

    assert_eq!(
        colors.color_for_directory(Path::new("/a/b/c")),
        Some(DirectoryTabColor::Color(AnsiColorIdentifier::Green))
    );
}

#[test]
fn color_for_directory_suppressed_shadows_broader_prefix() {
    let colors = DirectoryTabColors(
        [
            (
                "/a".to_string(),
                DirectoryTabColor::Color(AnsiColorIdentifier::Red),
            ),
            ("/a/b".to_string(), DirectoryTabColor::Suppressed),
        ]
        .into_iter()
        .collect(),
    );

    assert_eq!(colors.color_for_directory(Path::new("/a/b/c")), None);
}

#[test]
fn color_for_directory_broader_prefix_still_applies_outside_suppressed_subtree() {
    let colors = DirectoryTabColors(
        [
            (
                "/a".to_string(),
                DirectoryTabColor::Color(AnsiColorIdentifier::Red),
            ),
            ("/a/b".to_string(), DirectoryTabColor::Suppressed),
        ]
        .into_iter()
        .collect(),
    );

    assert_eq!(
        colors.color_for_directory(Path::new("/a/x")),
        Some(DirectoryTabColor::Color(AnsiColorIdentifier::Red))
    );
}

#[test]
fn color_for_directory_returns_none_without_configured_prefix() {
    let colors = DirectoryTabColors(
        [(
            "/b".to_string(),
            DirectoryTabColor::Color(AnsiColorIdentifier::Blue),
        )]
        .into_iter()
        .collect(),
    );

    assert_eq!(colors.color_for_directory(Path::new("/a/b/c")), None);
}

#[test]
fn use_latest_user_prompt_as_conversation_title_in_tab_names_defaults_to_false() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        TabSettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(!*settings.use_latest_user_prompt_as_conversation_title_in_tab_names);
        });
    });
}

#[test]
fn use_latest_user_prompt_as_conversation_title_in_tab_names_uses_vertical_tabs_path() {
    assert_eq!(
        UseLatestUserPromptAsConversationTitleInTabNames::toml_path(),
        Some("appearance.vertical_tabs.use_latest_prompt_as_title")
    );
    assert_eq!(
        UseLatestUserPromptAsConversationTitleInTabNames::hierarchy(),
        Some("appearance.vertical_tabs")
    );
    assert_eq!(
        UseLatestUserPromptAsConversationTitleInTabNames::toml_key(),
        "use_latest_prompt_as_title"
    );
}

#[test]
fn show_vertical_tab_panel_in_restored_windows_defaults_to_false() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        TabSettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(!*settings.show_vertical_tab_panel_in_restored_windows);
        });
    });
}

#[test]
fn show_vertical_tab_panel_in_restored_windows_uses_vertical_tabs_path() {
    assert_eq!(
        ShowVerticalTabPanelInRestoredWindows::toml_path(),
        Some("appearance.vertical_tabs.show_panel_in_restored_windows")
    );
    assert_eq!(
        ShowVerticalTabPanelInRestoredWindows::hierarchy(),
        Some("appearance.vertical_tabs")
    );
    assert_eq!(
        ShowVerticalTabPanelInRestoredWindows::toml_key(),
        "show_panel_in_restored_windows"
    );
}
