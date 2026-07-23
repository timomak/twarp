//! Pure projection for the agent transcript's compact turn navigator.
//!
//! The rail is a view concern: providers continue to own and emit the full
//! chronological transcript.  We derive one stable entry per user turn and
//! keep the preview deliberately short so hovering a marker never becomes a
//! second transcript surface.

use claude_code::TranscriptItem;

use super::turn_presentation::project_turns;

const PREVIEW_MAX_CHARS: usize = 56;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TimelineEntry {
    pub turn_start: usize,
    pub prompt_preview: String,
    pub response_preview: Option<String>,
}

pub(super) fn project_timeline(items: &[TranscriptItem], streaming: bool) -> Vec<TimelineEntry> {
    project_turns(items, streaming)
        .into_iter()
        .filter_map(|turn| {
            let TranscriptItem::User(prompt) = &items[turn.start] else {
                return None;
            };
            let response_preview = turn.final_response.and_then(|index| match &items[index] {
                TranscriptItem::Assistant { text, .. } if !text.trim().is_empty() => {
                    Some(preview(text))
                }
                _ => None,
            });
            Some(TimelineEntry {
                turn_start: turn.start,
                prompt_preview: preview(prompt),
                response_preview,
            })
        })
        .collect()
}

/// Collapse markdown/newline-heavy content into a compact, single-line hover
/// preview. Character-based truncation keeps non-ASCII prompts safe.
fn preview(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let shown = chars.by_ref().take(PREVIEW_MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{shown}…")
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use claude_code::TranscriptItem;

    use super::{preview, project_timeline, PREVIEW_MAX_CHARS};

    fn assistant(text: &str) -> TranscriptItem {
        TranscriptItem::Assistant {
            text: text.to_owned(),
            done: true,
        }
    }

    #[test]
    fn projects_one_entry_per_user_turn_with_the_final_response() {
        let items = vec![
            TranscriptItem::User("first prompt".to_owned()),
            assistant("intermediate"),
            assistant("first final"),
            TranscriptItem::User("second prompt".to_owned()),
            assistant("second final"),
        ];

        let entries = project_timeline(&items, false);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].turn_start, 0);
        assert_eq!(entries[0].prompt_preview, "first prompt");
        assert_eq!(entries[0].response_preview.as_deref(), Some("first final"));
        assert_eq!(entries[1].turn_start, 3);
        assert_eq!(entries[1].response_preview.as_deref(), Some("second final"));
    }

    #[test]
    fn live_turn_without_a_response_still_has_a_marker() {
        let items = vec![TranscriptItem::User("waiting".to_owned())];
        let entries = project_timeline(&items, true);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].prompt_preview, "waiting");
        assert_eq!(entries[0].response_preview, None);
    }

    #[test]
    fn preview_collapses_whitespace_and_truncates_on_character_boundaries() {
        assert_eq!(preview("one\n\n two\tthree"), "one two three");

        let long = "é".repeat(PREVIEW_MAX_CHARS + 1);
        let result = preview(&long);
        assert_eq!(result.chars().count(), PREVIEW_MAX_CHARS + 1);
        assert!(result.ends_with('…'));
    }
}
