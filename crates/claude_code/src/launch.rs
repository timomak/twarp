//! Mapping a `claude [flags] [prompt]` invocation's arguments onto the
//! pane's spawn options (PRODUCT §2; TECH §The trigger: "recognized flags
//! map onto `SpawnOptions`").
//!
//! This is what makes a user's `alias claude='claude --dangerously-skip-
//! permissions --effort max'` mean the same thing in the pane as in the raw
//! CLI: Warp expands the alias before the trigger sees it, so the flags
//! arrive as ordinary arguments and are translated here.
//!
//! Deliberately conservative (PRODUCT §29): only flags whose meaning is safe
//! under the pane's headless `-p --output-format stream-json` invocation are
//! honored. Everything else is **dropped, not passed through** — a flag like
//! `--remote-control` (a separate interactive mode) or an unknown future
//! flag could break the stream contract the driver depends on. The first
//! non-flag token starts the prompt; the prompt is everything from there on.

use crate::driver::PermissionMode;

/// The parsed launch request: the spawn-relevant flags plus the positional
/// prompt (PRODUCT §2: a trailing positional becomes the first user turn).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaunchOptions {
    pub prompt: Option<String>,
    /// `--permission-mode <m>`, or `--dangerously-skip-permissions` →
    /// [`PermissionMode::BypassPermissions`].
    pub permission_mode: Option<PermissionMode>,
    /// `--model <m>`.
    pub model: Option<String>,
    /// `--effort <level>` (verified a real flag on `claude` 2.1.173).
    pub effort: Option<String>,
    /// `--resume <id>` / `-r <id>`. A bare `--resume` (the CLI's interactive
    /// picker) carries no id and is ignored — the pane has no picker.
    pub resume_session_id: Option<String>,
}

/// One flag's value: either attached (`--flag=value`) or the next token.
/// Returns `None` (and consumes nothing) when the next token is another flag.
fn flag_value<'a>(
    attached: Option<&'a str>,
    tokens: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>,
) -> Option<&'a str> {
    if attached.is_some() {
        return attached;
    }
    match tokens.peek() {
        Some(next) if !next.starts_with('-') => tokens.next(),
        _ => None,
    }
}

/// Parse the tokens after the `claude` program token (PRODUCT §2).
///
/// Flag handling:
/// - `--permission-mode`, `--model`, `--effort`, `--resume`/`-r` → mapped.
/// - `--dangerously-skip-permissions` → bypass mode (what the flag means).
/// - `--remote-control [name]` → dropped; the pane is always local (#74).
///   Treated as valueless: a following bare token reads as the prompt, which
///   is the right call for a pane that cannot host the remote mode anyway.
/// - Any other flag → dropped (with its attached `=value` only; a separated
///   value token is conservatively left to start the prompt rather than
///   guessed at — claude's flags are a moving target, PRODUCT §29).
/// - First non-flag token: the prompt starts here and takes everything after.
pub fn parse_launch_args<'a>(args: impl IntoIterator<Item = &'a str>) -> LaunchOptions {
    let mut launch = LaunchOptions::default();
    let mut tokens = args.into_iter().peekable();
    let mut prompt_parts: Vec<&str> = Vec::new();

    while let Some(token) = tokens.next() {
        if !prompt_parts.is_empty() || !token.starts_with('-') {
            // The prompt has started; everything joins it verbatim.
            prompt_parts.push(token);
            continue;
        }
        let (name, attached) = match token.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (token, None),
        };
        match name {
            "--permission-mode" => {
                if let Some(value) = flag_value(attached, &mut tokens) {
                    launch.permission_mode =
                        permission_mode_from_cli(value).or(launch.permission_mode);
                }
            }
            "--dangerously-skip-permissions" => {
                launch.permission_mode = Some(PermissionMode::BypassPermissions);
            }
            "--model" => {
                if let Some(value) = flag_value(attached, &mut tokens) {
                    launch.model = Some(value.to_owned());
                }
            }
            "--effort" => {
                if let Some(value) = flag_value(attached, &mut tokens) {
                    launch.effort = Some(value.to_owned());
                }
            }
            "--resume" | "-r" => {
                if let Some(value) = flag_value(attached, &mut tokens) {
                    launch.resume_session_id = Some(value.to_owned());
                }
            }
            // Known interactive-only / no-op-here flags, dropped quietly.
            // (`--remote-control` takes an optional name; see fn docs.)
            _ => {}
        }
    }

    let prompt = prompt_parts.join(" ");
    if !prompt.trim().is_empty() {
        launch.prompt = Some(prompt);
    }
    launch
}

/// The CLI's `--permission-mode` values → the pane's four spec'd modes
/// (PRODUCT §25). Unknown values (2.1.173 added `auto`/`dontAsk`) are
/// ignored rather than guessed.
fn permission_mode_from_cli(value: &str) -> Option<PermissionMode> {
    match value {
        "default" => Some(PermissionMode::Default),
        "acceptEdits" => Some(PermissionMode::AcceptEdits),
        "plan" => Some(PermissionMode::Plan),
        "bypassPermissions" => Some(PermissionMode::BypassPermissions),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> LaunchOptions {
        parse_launch_args(s.split_whitespace())
    }

    #[test]
    fn the_alias_case_maps_skip_permissions_and_effort_and_drops_remote_control() {
        // The exact alias shape this exists for:
        // alias claude='claude --dangerously-skip-permissions --remote-control --effort max'
        let launch = parse("--dangerously-skip-permissions --remote-control --effort max");
        assert_eq!(
            launch.permission_mode,
            Some(PermissionMode::BypassPermissions)
        );
        assert_eq!(launch.effort.as_deref(), Some("max"));
        assert_eq!(launch.prompt, None);
        assert_eq!(launch.model, None);
        assert_eq!(launch.resume_session_id, None);
    }

    #[test]
    fn prompt_survives_leading_flags() {
        let launch = parse("--dangerously-skip-permissions --effort max fix the failing test");
        assert_eq!(launch.prompt.as_deref(), Some("fix the failing test"));
        assert_eq!(launch.effort.as_deref(), Some("max"));
    }

    #[test]
    fn bare_prompt_has_no_flags() {
        let launch = parse("explain this repo");
        assert_eq!(launch.prompt.as_deref(), Some("explain this repo"));
        assert_eq!(
            launch,
            LaunchOptions {
                prompt: Some("explain this repo".to_owned()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn flag_like_tokens_after_the_prompt_starts_stay_prompt() {
        let launch = parse("fix the --weird flag handling");
        assert_eq!(
            launch.prompt.as_deref(),
            Some("fix the --weird flag handling")
        );
        assert_eq!(launch.effort, None);
    }

    #[test]
    fn equals_and_separate_value_forms_both_parse() {
        assert_eq!(parse("--model=opus").model.as_deref(), Some("opus"));
        assert_eq!(parse("--model opus").model.as_deref(), Some("opus"));
        assert_eq!(
            parse("--permission-mode=plan").permission_mode,
            Some(PermissionMode::Plan)
        );
        assert_eq!(
            parse("--permission-mode acceptEdits").permission_mode,
            Some(PermissionMode::AcceptEdits)
        );
    }

    #[test]
    fn resume_maps_with_id_and_is_ignored_bare() {
        assert_eq!(
            parse("--resume abc-123").resume_session_id.as_deref(),
            Some("abc-123")
        );
        assert_eq!(
            parse("-r abc-123").resume_session_id.as_deref(),
            Some("abc-123")
        );
        // Bare --resume is the CLI's interactive picker; the pane has none.
        let bare = parse("--resume --effort max");
        assert_eq!(bare.resume_session_id, None);
        assert_eq!(bare.effort.as_deref(), Some("max"));
    }

    #[test]
    fn unknown_permission_modes_are_ignored() {
        assert_eq!(parse("--permission-mode dontAsk").permission_mode, None);
        assert_eq!(parse("--permission-mode auto").permission_mode, None);
    }

    #[test]
    fn unknown_flags_are_dropped_not_passed_through() {
        let launch = parse("--include-partial-messages --chrome");
        assert_eq!(launch, LaunchOptions::default());
    }

    #[test]
    fn empty_input_is_default() {
        assert_eq!(parse(""), LaunchOptions::default());
        assert_eq!(parse("   "), LaunchOptions::default());
    }
}
