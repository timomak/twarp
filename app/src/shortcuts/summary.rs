//! Arrow-form summary string for a single shortcut (PRODUCT §27).
//!
//! Example: `new pane right → type "claude" → enter`.
//! Used by the list view's row rendering; pure function so it's trivially
//! testable.

use crate::shortcuts::action::{Action, KeyName};

/// Maximum on-screen length before truncating with `…`. Keeps long
/// sequences from blowing out the side panel.
const MAX_SUMMARY_LEN: usize = 80;

pub fn summarize_actions(actions: &[Action]) -> String {
    let parts: Vec<String> = actions.iter().map(format_action).collect();
    let joined = parts.join(" → ");
    if joined.chars().count() <= MAX_SUMMARY_LEN {
        return joined;
    }
    let mut truncated: String = joined.chars().take(MAX_SUMMARY_LEN - 1).collect();
    truncated.push('…');
    truncated
}

fn format_action(action: &Action) -> String {
    match action {
        Action::NewTab => "new tab".to_owned(),
        Action::NewPane(dir) => {
            use crate::pane_group::Direction::*;
            let name = match dir {
                Right => "right",
                Down => "down",
                Left => "left",
                Up => "up",
            };
            format!("new pane {name}")
        }
        Action::Type(text) => format!("type \"{text}\""),
        Action::Press(key) => key_name_short(*key).to_owned(),
        Action::Wait(d) => format!("wait {}", duration_short(*d)),
    }
}

fn key_name_short(key: KeyName) -> &'static str {
    match key {
        KeyName::Enter => "enter",
        KeyName::Tab => "tab",
        KeyName::Escape => "escape",
        KeyName::Backspace => "backspace",
        KeyName::Space => "space",
        KeyName::Up => "↑",
        KeyName::Down => "↓",
        KeyName::Left => "←",
        KeyName::Right => "→",
        KeyName::Home => "home",
        KeyName::End => "end",
        KeyName::PageUp => "pageup",
        KeyName::PageDown => "pagedown",
        KeyName::Delete => "delete",
        KeyName::Insert => "insert",
        KeyName::NumpadEnter => "numpad-enter",
        KeyName::F(_) => "f-key",
    }
}

fn duration_short(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms.is_multiple_of(60_000) {
        format!("{}m", ms / 60_000)
    } else if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane_group::Direction;
    use std::time::Duration;

    #[test]
    fn arrow_form_for_driving_example() {
        let actions = vec![
            Action::NewPane(Direction::Right),
            Action::Wait(Duration::from_millis(1500)),
            Action::Type("claude".to_owned()),
            Action::Press(KeyName::Enter),
        ];
        assert_eq!(
            summarize_actions(&actions),
            "new pane right → wait 1500ms → type \"claude\" → enter"
        );
    }

    #[test]
    fn empty_action_list() {
        assert_eq!(summarize_actions(&[]), "");
    }

    #[test]
    fn truncation_appends_ellipsis() {
        // Build an absurdly long sequence
        let actions: Vec<Action> = (0..20)
            .map(|_| Action::Type("aaaaaaaaaaaa".to_owned()))
            .collect();
        let summary = summarize_actions(&actions);
        assert!(summary.chars().count() <= MAX_SUMMARY_LEN);
        assert!(summary.ends_with('…'));
    }
}
