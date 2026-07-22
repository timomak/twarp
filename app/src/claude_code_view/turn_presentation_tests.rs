use claude_code::{ToolOutput, ToolStatus, TranscriptItem, TurnMetrics};
use serde_json::json;

use super::{file_edit_summaries, project_turns};

fn assistant(text: &str) -> TranscriptItem {
    TranscriptItem::Assistant {
        text: text.to_owned(),
        done: true,
    }
}

fn tool(name: &str, status: ToolStatus) -> TranscriptItem {
    TranscriptItem::Tool {
        id: format!("{name}-id"),
        name: name.to_owned(),
        input: json!({}),
        status,
        output: Some(ToolOutput {
            text: String::new(),
            summary: None,
        }),
        children: Vec::new(),
    }
}

#[test]
fn completed_turn_keeps_only_last_assistant_message_as_final_response() {
    let items = vec![
        TranscriptItem::User("do it".to_owned()),
        assistant("I will inspect the repository."),
        tool("Read", ToolStatus::Completed),
        assistant("Done.\n\nEverything is ready."),
        TranscriptItem::Metrics(TurnMetrics {
            duration_ms: Some(2_000),
            ..TurnMetrics::default()
        }),
    ];

    let turns = project_turns(&items, false);

    assert_eq!(turns.len(), 1);
    assert!(turns[0].compact);
    assert_eq!(turns[0].final_response, Some(3));
    assert_eq!(turns[0].hidden_work, vec![1, 2, 4]);
}

#[test]
fn running_turn_stays_chronological() {
    let items = vec![
        TranscriptItem::User("do it".to_owned()),
        assistant("Starting."),
        tool("Bash", ToolStatus::Running),
    ];

    let turns = project_turns(&items, true);

    assert_eq!(turns.len(), 1);
    assert!(!turns[0].compact);
}

#[test]
fn unresolved_or_failed_turn_stays_chronological() {
    let unresolved = vec![
        TranscriptItem::User("do it".to_owned()),
        assistant("I need approval."),
        TranscriptItem::Permission {
            id: "permission".to_owned(),
            tool: "Bash".to_owned(),
            input: json!({}),
            decision: None,
        },
    ];
    let failed = vec![
        TranscriptItem::User("do it".to_owned()),
        assistant("I could not finish."),
        tool("Bash", ToolStatus::Failed),
    ];

    assert!(!project_turns(&unresolved, false)[0].compact);
    assert!(!project_turns(&failed, false)[0].compact);
}

#[test]
fn completed_file_edits_become_artifacts() {
    let items = vec![
        TranscriptItem::User("edit it".to_owned()),
        tool("Edit", ToolStatus::Completed),
        tool("Write", ToolStatus::Failed),
        assistant("Done."),
    ];

    let turns = project_turns(&items, false);

    assert_eq!(turns[0].file_edits, vec![1]);
}

#[test]
fn earlier_turn_compacts_while_latest_turn_streams() {
    let items = vec![
        TranscriptItem::User("first".to_owned()),
        tool("Read", ToolStatus::Completed),
        assistant("First done."),
        TranscriptItem::User("second".to_owned()),
        assistant("Working on it."),
    ];

    let turns = project_turns(&items, true);

    assert!(turns[0].compact);
    assert!(!turns[1].compact);
}

#[test]
fn codex_file_changes_produce_paths_and_real_diff_counts() {
    let input = json!([
        {
            "path": "src/main.rs",
            "diff": "--- a/src/main.rs\n+++ b/src/main.rs\n-old\n+new\n+second"
        },
        {"path": "README.md", "kind": {"type": "update"}}
    ]);

    let summaries = file_edit_summaries(&input);

    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].path, "src/main.rs");
    assert_eq!((summaries[0].added, summaries[0].removed), (2, 1));
    assert_eq!((summaries[1].added, summaries[1].removed), (0, 0));
}
