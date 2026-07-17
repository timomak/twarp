use super::{extract_captured_env, ENV_CAPTURE_END, ENV_CAPTURE_START};

#[test]
fn extracts_clean_environment() {
    let output = format!(
        "{ENV_CAPTURE_START}\nPATH=/opt/homebrew/bin:/usr/bin\nTOKEN=secret\n{ENV_CAPTURE_END}"
    );
    assert_eq!(
        extract_captured_env(&output),
        Some("\nPATH=/opt/homebrew/bin:/usr/bin\nTOKEN=secret\n")
    );
}

#[test]
fn ignores_startup_banner_output() {
    // rc files printing to stdout (fastfetch/MOTD) before the environment.
    let output = format!(
        "ascii art line 1\nOS macOS shell zsh\n\
         {ENV_CAPTURE_START}\nPATH=/opt/homebrew/bin:/usr/bin:/bin\n{ENV_CAPTURE_END}\n"
    );
    assert_eq!(
        extract_captured_env(&output),
        Some("\nPATH=/opt/homebrew/bin:/usr/bin:/bin\n")
    );
}

#[test]
fn missing_markers_returns_none() {
    assert_eq!(
        extract_captured_env("PATH=/opt/homebrew/bin:/usr/bin"),
        None
    );
}

#[test]
fn missing_end_marker_returns_none() {
    let output = format!("{ENV_CAPTURE_START}\nPATH=/opt/homebrew/bin");
    assert_eq!(extract_captured_env(&output), None);
}

#[test]
fn empty_environment_between_markers() {
    let output = format!("{ENV_CAPTURE_START}{ENV_CAPTURE_END}");
    assert_eq!(extract_captured_env(&output), Some(""));
}

#[test]
fn preserves_values_and_surrounding_noise() {
    let env = "PATH=/a:/b:/c\nCUSTOM=value=with=equals";
    let output = format!("before{ENV_CAPTURE_START}{env}{ENV_CAPTURE_END}after");
    assert_eq!(extract_captured_env(&output), Some(env));
}
