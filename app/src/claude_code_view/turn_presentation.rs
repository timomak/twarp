//! Pure projection from the provider-neutral transcript into completed turns.
//!
//! Providers keep emitting the full chronological transcript. The pane uses
//! this projection only after a turn settles, so live work remains visible and
//! the completed state can lead with the final answer without discarding any
//! underlying detail.

use claude_code::{ToolStatus, TranscriptItem};
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct FileEditSummary {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TurnPresentation {
    pub start: usize,
    pub end: usize,
    pub final_response: Option<usize>,
    pub file_edits: Vec<usize>,
    pub hidden_work: Vec<usize>,
    pub compact: bool,
}

/// Split the transcript at user messages and decide which turns can safely use
/// the final-result presentation. A turn with live controls, running work, or a
/// terminal notice/error remains in the chronological renderer so completion
/// chrome never hides something that still needs attention.
pub(super) fn project_turns(items: &[TranscriptItem], streaming: bool) -> Vec<TurnPresentation> {
    let starts = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| matches!(item, TranscriptItem::User(_)).then_some(index))
        .collect::<Vec<_>>();

    starts
        .iter()
        .enumerate()
        .map(|(turn_index, &start)| {
            let end = starts.get(turn_index + 1).copied().unwrap_or(items.len());
            let is_live_turn = streaming && turn_index + 1 == starts.len();
            let final_response = (start + 1..end).rev().find(|&index| {
                matches!(
                    &items[index],
                    TranscriptItem::Assistant { text, done: true } if !text.trim().is_empty()
                )
            });
            let file_edits = (start + 1..end)
                .filter(|&index| is_completed_file_edit(&items[index]))
                .collect::<Vec<_>>();
            let hidden_work = (start + 1..end)
                .filter(|index| Some(*index) != final_response)
                .collect::<Vec<_>>();
            let compact = !is_live_turn
                && final_response.is_some()
                && items[start + 1..end].iter().all(is_safe_to_hide);

            TurnPresentation {
                start,
                end,
                final_response,
                file_edits,
                hidden_work,
                compact,
            }
        })
        .collect()
}

fn is_completed_file_edit(item: &TranscriptItem) -> bool {
    matches!(
        item,
        TranscriptItem::Tool {
            name,
            status: ToolStatus::Completed,
            ..
        } if matches!(name.as_str(), "Edit" | "MultiEdit" | "Write" | "NotebookEdit")
    )
}

fn is_safe_to_hide(item: &TranscriptItem) -> bool {
    match item {
        TranscriptItem::Tool {
            status, children, ..
        } => *status == ToolStatus::Completed && children.iter().all(is_safe_to_hide),
        TranscriptItem::Thinking { done, .. } | TranscriptItem::Assistant { done, .. } => *done,
        TranscriptItem::Permission { decision, .. } => decision.is_some(),
        TranscriptItem::Question { answered, .. } => *answered,
        TranscriptItem::Error(_) | TranscriptItem::Notice(_) => false,
        TranscriptItem::User(_) | TranscriptItem::Todos(_) | TranscriptItem::Metrics(_) => true,
    }
}

/// Best-effort file inventory for provider file-change payloads that cannot be
/// synthesized into an inline `DiffCard` (notably Codex app-server's
/// `{changes:[{path,diff}]}` shape). Claude's richer old/new payload continues
/// through the existing diff parser instead.
pub(super) fn file_edit_summaries(input: &Value) -> Vec<FileEditSummary> {
    let changes = input
        .as_array()
        .or_else(|| input.get("changes").and_then(Value::as_array));
    if let Some(changes) = changes {
        return changes
            .iter()
            .filter_map(|change| {
                let path = change.get("path")?.as_str()?.to_owned();
                let (added, removed) = unified_diff_counts(
                    change
                        .get("diff")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                Some(FileEditSummary {
                    path,
                    added,
                    removed,
                })
            })
            .collect();
    }

    input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .and_then(Value::as_str)
        .map(|path| {
            vec![FileEditSummary {
                path: path.to_owned(),
                added: 0,
                removed: 0,
            }]
        })
        .unwrap_or_default()
}

fn unified_diff_counts(diff: &str) -> (usize, usize) {
    let added = diff
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .count();
    let removed = diff
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count();
    (added, removed)
}

#[cfg(test)]
#[path = "turn_presentation_tests.rs"]
mod tests;
