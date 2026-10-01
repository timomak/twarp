use super::raw_cli_command;
use claude_code::driver::AgentProvider;
use std::path::Path;

#[test]
fn raw_cli_uses_the_pinned_transcript_instead_of_creating_a_duplicate_session() {
    let command = raw_cli_command(
        AgentProvider::Claude,
        "/path with spaces/claude",
        "--model sonnet",
        "session-uuid",
        Some(Path::new("/old history/session-uuid.jsonl")),
    );
    assert_eq!(
        shell_words::split(&command).unwrap(),
        vec![
            "exec",
            "/path with spaces/claude",
            "--model",
            "sonnet",
            "--resume",
            "/old history/session-uuid.jsonl"
        ]
    );
}

#[test]
fn fresh_raw_cli_keeps_the_uuid_when_no_transcript_exists() {
    let command = raw_cli_command(
        AgentProvider::Claude,
        "/bin/claude",
        "",
        "session-uuid",
        None,
    );
    assert_eq!(command, "exec /bin/claude --session-id session-uuid");
}
