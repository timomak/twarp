//! Synthesizing diffs from `Edit` / `MultiEdit` / `Write` tool inputs
//! (PRODUCT §20, sub-phase 7e).
//!
//! The stream-json `tool_use` input is all twarp ever sees of an edit — there
//! is no "before" file snapshot in the stream, and reading the file from disk
//! would race the edit itself. So a diff card diffs exactly what the tool
//! declared: `old_string` → `new_string` for an edit, nothing → `content` for
//! a `Write`. The pane renders each [`DiffEdit`] read-only through feature
//! 05's diff treatment (TECH §The chat UI: reuse `InlineDiffView`, never the
//! deleted interactive `code_diff_view.rs`).
//!
//! Headless on purpose: shapes are unit-tested here, rendering lives in the
//! app crate.

use serde_json::Value;

/// One old → new replacement. A `Write` is a single edit from empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEdit {
    pub old: String,
    pub new: String,
}

impl DiffEdit {
    /// An all-additions edit (`Write`, PRODUCT §20).
    pub fn is_creation(&self) -> bool {
        self.old.is_empty()
    }
}

/// The synthesized diff for one `Edit` / `MultiEdit` / `Write` tool call:
/// the file it touches and its edits in tool-input order (PRODUCT §20:
/// `MultiEdit` renders each edit in order against the same file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiff {
    pub file_path: String,
    pub edits: Vec<DiffEdit>,
}

impl ToolDiff {
    /// Net line delta across all edits — the `+N −M` summary the collapsed
    /// card shows (PRODUCT §19/§20).
    pub fn line_counts(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;
        for edit in &self.edits {
            removed += count_lines(&edit.old);
            added += count_lines(&edit.new);
        }
        (added, removed)
    }
}

/// Lines in a tool-input string. Empty text has no lines; a trailing newline
/// does not add a phantom line (`"a\n"` is one line, like `wc -l`).
fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Synthesize the diff for a tool call, if it is one of the file-editing
/// tools (PRODUCT §20: `Edit`, `MultiEdit`, `Write`). Returns `None` for
/// every other tool and for malformed input — the card then degrades to the
/// 7d tool card rather than rendering a broken diff (PRODUCT §29).
pub fn diff_for_tool(name: &str, input: &Value) -> Option<ToolDiff> {
    let file_path = input.get("file_path")?.as_str()?.to_owned();
    let edits = match name {
        "Write" => {
            let content = input.get("content")?.as_str()?.to_owned();
            vec![DiffEdit {
                old: String::new(),
                new: content,
            }]
        }
        "Edit" => {
            let old = input.get("old_string")?.as_str()?.to_owned();
            let new = input.get("new_string")?.as_str()?.to_owned();
            vec![DiffEdit { old, new }]
        }
        "MultiEdit" => {
            let edits = input.get("edits")?.as_array()?;
            let parsed: Vec<DiffEdit> = edits
                .iter()
                .filter_map(|edit| {
                    Some(DiffEdit {
                        old: edit.get("old_string")?.as_str()?.to_owned(),
                        new: edit.get("new_string")?.as_str()?.to_owned(),
                    })
                })
                .collect();
            // All edits malformed (or none): nothing to render as a diff.
            if parsed.is_empty() {
                return None;
            }
            parsed
        }
        _ => return None,
    };
    // An Edit replacing nothing with nothing renders no meaningful diff.
    if edits.iter().all(|e| e.old.is_empty() && e.new.is_empty()) {
        return None;
    }
    Some(ToolDiff { file_path, edits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_is_single_all_additions_edit() {
        let input = json!({"file_path": "src/new.rs", "content": "fn main() {}\n"});
        let diff = diff_for_tool("Write", &input).expect("Write synthesizes a diff");
        assert_eq!(diff.file_path, "src/new.rs");
        assert_eq!(diff.edits.len(), 1);
        assert!(diff.edits[0].is_creation());
        assert_eq!(diff.edits[0].new, "fn main() {}\n");
        assert_eq!(diff.line_counts(), (1, 0));
    }

    #[test]
    fn edit_maps_old_to_new() {
        let input = json!({
            "file_path": "a.rs",
            "old_string": "let x = 1;",
            "new_string": "let x = 2;\nlet y = 3;",
        });
        let diff = diff_for_tool("Edit", &input).expect("Edit synthesizes a diff");
        assert_eq!(diff.edits.len(), 1);
        assert!(!diff.edits[0].is_creation());
        assert_eq!(diff.line_counts(), (2, 1));
    }

    #[test]
    fn multi_edit_preserves_edit_order() {
        let input = json!({
            "file_path": "b.rs",
            "edits": [
                {"old_string": "one", "new_string": "uno"},
                {"old_string": "two", "new_string": "dos"},
            ],
        });
        let diff = diff_for_tool("MultiEdit", &input).expect("MultiEdit synthesizes a diff");
        assert_eq!(diff.edits.len(), 2);
        assert_eq!(diff.edits[0].old, "one");
        assert_eq!(diff.edits[1].new, "dos");
    }

    #[test]
    fn multi_edit_skips_malformed_entries_but_keeps_good_ones() {
        let input = json!({
            "file_path": "c.rs",
            "edits": [
                {"old_string": "keep", "new_string": "kept"},
                {"oops": true},
            ],
        });
        let diff = diff_for_tool("MultiEdit", &input).expect("good edit survives");
        assert_eq!(diff.edits.len(), 1);
    }

    #[test]
    fn non_edit_tools_and_malformed_input_yield_none() {
        // PRODUCT §29 defensive: degrade to the plain tool card.
        assert_eq!(diff_for_tool("Read", &json!({"file_path": "a"})), None);
        assert_eq!(diff_for_tool("Bash", &json!({"command": "ls"})), None);
        assert_eq!(diff_for_tool("Edit", &json!({})), None);
        assert_eq!(diff_for_tool("Edit", &Value::Null), None);
        assert_eq!(diff_for_tool("Write", &json!({"file_path": "a"})), None);
        assert_eq!(
            diff_for_tool("MultiEdit", &json!({"file_path": "a", "edits": []})),
            None
        );
        // Numeric / wrong-typed fields never panic.
        assert_eq!(
            diff_for_tool(
                "Edit",
                &json!({"file_path": 42, "old_string": "a", "new_string": "b"})
            ),
            None
        );
    }

    #[test]
    fn empty_to_empty_edit_is_no_diff() {
        let input = json!({"file_path": "a.rs", "old_string": "", "new_string": ""});
        assert_eq!(diff_for_tool("Edit", &input), None);
    }

    #[test]
    fn line_counts_ignore_trailing_newline() {
        let diff = ToolDiff {
            file_path: "x".into(),
            edits: vec![DiffEdit {
                old: "a\nb\n".into(),
                new: "a\nb\nc\n".into(),
            }],
        };
        assert_eq!(diff.line_counts(), (3, 2));
    }
}
