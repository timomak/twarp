//! twarp 21c: self-contained parsing for the PR Files tab — splits `gh pr
//! diff` output into per-file patches, parses GitHub review threads out of
//! the GraphQL response, and anchors threads onto diff lines.
//!
//! The code_review unified-diff parser
//! ([`crate::code_review::diff_state::DiffStateModel::parse_diff_hunks`]) was
//! considered and rejected: it flattens all files' hunks into one vec (no
//! `diff --git` splitting) and silently drops blank context lines, which is a
//! rendering-fidelity bug for a read-only diff view. This module owns a small
//! parser instead; only the *visual* idioms are shared with code_review.

use std::collections::HashMap;

/// A parsed diff line's kind.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrDiffLineKind {
    Context,
    Add,
    Delete,
}

/// One line of a hunk, with the dual (old/new) line numbers used for display
/// and for anchoring review threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrDiffLine {
    pub kind: PrDiffLineKind,
    pub old_line: Option<u64>,
    pub new_line: Option<u64>,
    /// Line content without the leading `+`/`-`/space marker.
    pub text: String,
}

/// One `@@` hunk: the raw header line plus its lines.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrHunk {
    /// The full `@@ -a,b +c,d @@ …` line, rendered verbatim as a separator.
    pub header: String,
    pub lines: Vec<PrDiffLine>,
}

/// The change kind of one file in the PR diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrFileStatus {
    Added,
    Deleted,
    Renamed { old_path: String },
    Modified,
}

/// One file's patch, split out of the multi-file `gh pr diff` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrFileDiff {
    /// The new-side path (old-side path for deletions).
    pub path: String,
    pub status: PrFileStatus,
    pub is_binary: bool,
    pub hunks: Vec<PrHunk>,
}

impl PrFileDiff {
    pub fn additions(&self) -> u64 {
        self.count_kind(PrDiffLineKind::Add)
    }

    pub fn deletions(&self) -> u64 {
        self.count_kind(PrDiffLineKind::Delete)
    }

    fn count_kind(&self, kind: PrDiffLineKind) -> u64 {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == kind)
            .count() as u64
    }

    /// Rendered patch size: content lines plus one row per hunk header.
    pub fn patch_line_count(&self) -> u64 {
        self.hunks.iter().map(|h| h.lines.len() as u64 + 1).sum()
    }
}

/// Split a multi-file unified diff (as printed by `gh pr diff`) into per-file
/// patches. Unparseable stretches are skipped rather than failing the whole
/// diff — this is a read-only viewer.
pub fn parse_pr_diff(diff: &str) -> Vec<PrFileDiff> {
    let mut files: Vec<PrFileDiff> = Vec::new();
    let mut current: Option<PrFileDiff> = None;
    // (old counter, new counter) for the hunk being filled.
    let mut counters: Option<(u64, u64)> = None;
    // Explicit statuses seen in the extended headers of the current file.
    let mut rename_from: Option<String> = None;
    let mut new_file = false;
    let mut deleted_file = false;

    let finalize = |file: Option<PrFileDiff>,
                    rename_from: &mut Option<String>,
                    new_file: &mut bool,
                    deleted_file: &mut bool,
                    files: &mut Vec<PrFileDiff>| {
        if let Some(mut file) = file {
            file.status = if let Some(old) = rename_from.take() {
                PrFileStatus::Renamed { old_path: old }
            } else if *new_file {
                PrFileStatus::Added
            } else if *deleted_file {
                PrFileStatus::Deleted
            } else {
                PrFileStatus::Modified
            };
            files.push(file);
        }
        *new_file = false;
        *deleted_file = false;
    };

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            finalize(
                current.take(),
                &mut rename_from,
                &mut new_file,
                &mut deleted_file,
                &mut files,
            );
            counters = None;
            current = Some(PrFileDiff {
                path: path_from_git_header(rest),
                status: PrFileStatus::Modified,
                is_binary: false,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue; // Preamble before the first file header.
        };

        if let Some(header) = parse_hunk_header(line) {
            counters = Some(header.1);
            file.hunks.push(PrHunk {
                header: header.0,
                lines: Vec::new(),
            });
            continue;
        }

        match counters {
            Some((ref mut old, ref mut new)) if is_hunk_content(line) => {
                let hunk = file.hunks.last_mut().expect("counters imply a hunk");
                // An entirely empty line inside a hunk is a context line whose
                // trailing space was stripped somewhere in transit.
                let (kind, text) = if line.is_empty() {
                    (PrDiffLineKind::Context, "")
                } else {
                    match line.as_bytes()[0] {
                        b'+' => (PrDiffLineKind::Add, &line[1..]),
                        b'-' => (PrDiffLineKind::Delete, &line[1..]),
                        b'\\' => continue, // "\ No newline at end of file"
                        _ => (PrDiffLineKind::Context, &line[1..]),
                    }
                };
                let (old_line, new_line) = match kind {
                    PrDiffLineKind::Context => {
                        *old += 1;
                        *new += 1;
                        (Some(*old - 1), Some(*new - 1))
                    }
                    PrDiffLineKind::Add => {
                        *new += 1;
                        (None, Some(*new - 1))
                    }
                    PrDiffLineKind::Delete => {
                        *old += 1;
                        (Some(*old - 1), None)
                    }
                };
                hunk.lines.push(PrDiffLine {
                    kind,
                    old_line,
                    new_line,
                    text: text.to_owned(),
                });
                continue;
            }
            _ => counters = None, // Extended header territory.
        }

        // Extended headers between `diff --git` and the first `@@`.
        if line.starts_with("new file mode") {
            new_file = true;
        } else if line.starts_with("deleted file mode") {
            deleted_file = true;
        } else if let Some(old) = line.strip_prefix("rename from ") {
            rename_from = Some(old.to_owned());
        } else if let Some(new) = line.strip_prefix("rename to ") {
            file.path = new.to_owned();
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            file.is_binary = true;
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            file.path = path.to_owned();
        } else if let Some(path) = line.strip_prefix("--- a/") {
            // Keep the old-side path for deletions (`+++` is /dev/null then).
            if file.path.is_empty() || deleted_file {
                file.path = path.to_owned();
            }
        }
    }
    finalize(
        current.take(),
        &mut rename_from,
        &mut new_file,
        &mut deleted_file,
        &mut files,
    );
    files
}

/// True when a line can be hunk content (we're inside a hunk and this is not
/// the start of the next file's headers).
fn is_hunk_content(line: &str) -> bool {
    line.is_empty() || matches!(line.as_bytes()[0], b'+' | b'-' | b' ' | b'\\')
}

/// Best-effort path out of the `diff --git a/x b/y` remainder. Overridden by
/// `+++ b/` / `rename to` when those headers follow. Paths containing " b/"
/// can mislead this; the later headers correct it.
fn path_from_git_header(rest: &str) -> String {
    let rest = rest.strip_prefix("a/").unwrap_or(rest);
    match rest.find(" b/") {
        Some(idx) => rest[idx + 3..].to_owned(),
        None => rest.to_owned(),
    }
}

/// Parse an `@@ -a[,b] +c[,d] @@ …` header into (full header line, (old
/// start, new start)).
fn parse_hunk_header(line: &str) -> Option<(String, (u64, u64))> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_part, rest) = rest.split_once(" +")?;
    let (new_part, _) = rest.split_once(" @@")?;
    let start = |part: &str| -> Option<u64> { part.split(',').next()?.parse().ok() };
    Some((line.to_owned(), (start(old_part)?, start(new_part)?)))
}

/// Which side of the diff a review thread is anchored to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrDiffSide {
    Left,
    Right,
}

/// One comment inside a review thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrThreadComment {
    pub author: String,
    /// Raw RFC 3339 timestamp.
    pub created_at: String,
    /// Markdown body.
    pub body: String,
}

/// One line-anchored GitHub review thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrReviewThread {
    /// The GraphQL node id, used by the 21d reply/resolve mutations.
    pub id: String,
    pub path: String,
    pub line: Option<u64>,
    pub start_line: Option<u64>,
    pub side: PrDiffSide,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub comments: Vec<PrThreadComment>,
}

/// Parse the review-threads GraphQL response into threads plus a truncation
/// flag (more threads exist than one page holds).
pub fn parse_review_threads(json: &str) -> Result<(Vec<PrReviewThread>, bool), String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| format!("Could not parse the review-threads response: {err}"))?;
    let connection = value
        .pointer("/data/repository/pullRequest/reviewThreads")
        .ok_or_else(|| "The review-threads response has no pull request.".to_owned())?;
    let nodes = connection
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let total = connection
        .get("totalCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let truncated = total > nodes.len() as u64;

    let threads = nodes
        .iter()
        .map(|node| {
            let bool_of = |key: &str| node.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
            let comments = node
                .pointer("/comments/nodes")
                .and_then(|v| v.as_array())
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|c| PrThreadComment {
                    author: c
                        .pointer("/author/login")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    created_at: c
                        .get("createdAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    body: c
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect();
            PrReviewThread {
                id: node
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                path: node
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                line: node.get("line").and_then(|v| v.as_u64()),
                start_line: node.get("startLine").and_then(|v| v.as_u64()),
                side: match node.get("diffSide").and_then(|v| v.as_str()) {
                    Some("LEFT") => PrDiffSide::Left,
                    _ => PrDiffSide::Right,
                },
                is_resolved: bool_of("isResolved"),
                is_outdated: bool_of("isOutdated"),
                comments,
            }
        })
        .collect();
    Ok((threads, truncated))
}

/// Where each of one file's threads landed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileThreadAnchors {
    /// (hunk index, line index) → indices into the threads slice, shown
    /// inline right after that diff line.
    pub inline: HashMap<(usize, usize), Vec<usize>>,
    /// Thread indices that are outdated or could not be matched to a line,
    /// shown in the file card's collapsed "Outdated / other threads" section.
    pub floating: Vec<usize>,
}

/// Map review threads onto diff lines. Returns one [`FileThreadAnchors`] per
/// file (index-aligned with `files`). A thread anchors to the first line of
/// the file whose side-appropriate line number matches `thread.line`;
/// outdated threads and misses go to the file's floating bucket. Threads
/// whose path matches no file in the diff are dropped (known limitation:
/// GitHub can hold threads on files no longer touched by the PR).
pub fn anchor_threads(files: &[PrFileDiff], threads: &[PrReviewThread]) -> Vec<FileThreadAnchors> {
    let mut anchors: Vec<FileThreadAnchors> = vec![FileThreadAnchors::default(); files.len()];
    for (thread_idx, thread) in threads.iter().enumerate() {
        let Some(file_idx) = files.iter().position(|f| f.path == thread.path) else {
            continue;
        };
        let target = &mut anchors[file_idx];
        let anchor = (!thread.is_outdated)
            .then(|| thread.line)
            .flatten()
            .and_then(|line| find_line(&files[file_idx], line, thread.side));
        match anchor {
            Some(position) => target.inline.entry(position).or_default().push(thread_idx),
            None => target.floating.push(thread_idx),
        }
    }
    anchors
}

/// Find the (hunk, line) position matching a thread's line number on the
/// given side. Right-side threads match new-file numbering on added/context
/// lines; left-side threads match old-file numbering on deleted/context
/// lines.
fn find_line(file: &PrFileDiff, line: u64, side: PrDiffSide) -> Option<(usize, usize)> {
    for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
        for (line_idx, diff_line) in hunk.lines.iter().enumerate() {
            let matched = match side {
                PrDiffSide::Right => {
                    diff_line.kind != PrDiffLineKind::Delete && diff_line.new_line == Some(line)
                }
                PrDiffSide::Left => {
                    diff_line.kind != PrDiffLineKind::Add && diff_line.old_line == Some(line)
                }
            };
            if matched {
                return Some((hunk_idx, line_idx));
            }
        }
    }
    None
}

/// Above this many total rendered diff lines, every file card starts
/// collapsed and the header notes why.
pub const BIG_PR_TOTAL_LINE_THRESHOLD: u64 = 10_000;

/// Only the first this-many files auto-expand.
pub const AUTO_EXPAND_FILE_COUNT: usize = 5;

/// Files whose patch exceeds this many lines never auto-expand.
pub const AUTO_EXPAND_MAX_PATCH_LINES: u64 = 300;

/// Total rendered line count across all files.
pub fn total_line_count(files: &[PrFileDiff]) -> u64 {
    files.iter().map(PrFileDiff::patch_line_count).sum()
}

/// The default expanded/collapsed state per file card (index-aligned).
pub fn default_expanded(files: &[PrFileDiff]) -> Vec<bool> {
    if total_line_count(files) > BIG_PR_TOTAL_LINE_THRESHOLD {
        return vec![false; files.len()];
    }
    files
        .iter()
        .enumerate()
        .map(|(idx, file)| {
            idx < AUTO_EXPAND_FILE_COUNT
                && !file.is_binary
                && file.patch_line_count() <= AUTO_EXPAND_MAX_PATCH_LINES
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,5 @@ mod header
 fn main() {
-    old();
+    new();
+    extra();
 }

diff --git a/old_name.rs b/new_name.rs
similarity index 90%
rename from old_name.rs
rename to new_name.rs
index 333..444 100644
--- a/old_name.rs
+++ b/new_name.rs
@@ -10 +10 @@
-a
+b
diff --git a/added.txt b/added.txt
new file mode 100644
index 000..555
--- /dev/null
+++ b/added.txt
@@ -0,0 +1,2 @@
+hello
+world
\\ No newline at end of file
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index 666..000
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/img.png b/img.png
index 777..888 100644
Binary files a/img.png and b/img.png differ
";

    #[test]
    fn splits_files_and_statuses() {
        let files = parse_pr_diff(SAMPLE_DIFF);
        assert_eq!(files.len(), 5);
        assert_eq!(files[0].path, "src/lib.rs");
        assert_eq!(files[0].status, PrFileStatus::Modified);
        assert_eq!(
            files[1].status,
            PrFileStatus::Renamed {
                old_path: "old_name.rs".into()
            }
        );
        assert_eq!(files[1].path, "new_name.rs");
        assert_eq!(files[2].status, PrFileStatus::Added);
        assert_eq!(files[2].path, "added.txt");
        assert_eq!(files[3].status, PrFileStatus::Deleted);
        assert_eq!(files[3].path, "gone.txt");
        assert!(files[4].is_binary);
        assert!(files[4].hunks.is_empty());
    }

    #[test]
    fn line_numbers_and_counts() {
        let files = parse_pr_diff(SAMPLE_DIFF);
        let first = &files[0];
        assert_eq!((first.additions(), first.deletions()), (2, 1));
        let lines = &first.hunks[0].lines;
        assert_eq!(first.hunks[0].header, "@@ -1,4 +1,5 @@ mod header");
        // Context line keeps both numbers.
        assert_eq!(lines[0].kind, PrDiffLineKind::Context);
        assert_eq!((lines[0].old_line, lines[0].new_line), (Some(1), Some(1)));
        // Deleted line has only an old number.
        assert_eq!(lines[1].kind, PrDiffLineKind::Delete);
        assert_eq!((lines[1].old_line, lines[1].new_line), (Some(2), None));
        // Added lines advance only the new counter.
        assert_eq!(lines[2].kind, PrDiffLineKind::Add);
        assert_eq!((lines[2].old_line, lines[2].new_line), (None, Some(2)));
        assert_eq!(lines[3].new_line, Some(3));
        // The closing brace context line resumes both counters.
        assert_eq!(lines[4].kind, PrDiffLineKind::Context);
        assert_eq!((lines[4].old_line, lines[4].new_line), (Some(3), Some(4)));
        // The fully blank line was kept as an empty context line.
        assert_eq!(lines[5].text, "");
        assert_eq!(lines[5].kind, PrDiffLineKind::Context);

        // Added file: 2 additions; "\ No newline" marker dropped.
        assert_eq!(files[2].additions(), 2);
        assert_eq!(files[2].hunks[0].lines.len(), 2);
        assert_eq!(files[3].deletions(), 1);
    }

    #[test]
    fn tolerates_garbage_and_empty_input() {
        assert!(parse_pr_diff("").is_empty());
        assert!(parse_pr_diff("gh: could not resolve").is_empty());
    }

    fn thread(path: &str, line: Option<u64>, side: PrDiffSide, outdated: bool) -> PrReviewThread {
        PrReviewThread {
            id: String::new(),
            path: path.into(),
            line,
            start_line: None,
            side,
            is_resolved: false,
            is_outdated: outdated,
            comments: Vec::new(),
        }
    }

    #[test]
    fn anchors_threads_to_lines() {
        let files = parse_pr_diff(SAMPLE_DIFF);
        let threads = vec![
            // Right side, new line 2 of src/lib.rs = the `new();` addition.
            thread("src/lib.rs", Some(2), PrDiffSide::Right, false),
            // Left side, old line 2 = the deleted `old();`.
            thread("src/lib.rs", Some(2), PrDiffSide::Left, false),
            // Outdated → floating even though the line exists.
            thread("src/lib.rs", Some(1), PrDiffSide::Right, true),
            // Line outside every hunk → floating.
            thread("src/lib.rs", Some(999), PrDiffSide::Right, false),
            // Unknown path → dropped.
            thread("nowhere.rs", Some(1), PrDiffSide::Right, false),
        ];
        let anchors = anchor_threads(&files, &threads);
        assert_eq!(anchors.len(), files.len());
        let first = &anchors[0];
        assert_eq!(first.inline.get(&(0, 2)), Some(&vec![0]));
        assert_eq!(first.inline.get(&(0, 1)), Some(&vec![1]));
        assert_eq!(first.floating, vec![2, 3]);
        assert!(anchors[1].inline.is_empty() && anchors[1].floating.is_empty());
    }

    #[test]
    fn parses_review_threads_json() {
        let json = r#"{
            "data": {"repository": {"pullRequest": {"reviewThreads": {
                "totalCount": 2,
                "nodes": [{
                    "id": "PRRT_node1",
                    "isResolved": true,
                    "isOutdated": false,
                    "path": "src/lib.rs",
                    "line": 2,
                    "startLine": null,
                    "diffSide": "RIGHT",
                    "comments": {"nodes": [
                        {"author": {"login": "alice"}, "createdAt": "2026-07-20T00:00:00Z", "body": "nit"},
                        {"author": {"login": "bob"}, "createdAt": "2026-07-21T00:00:00Z", "body": "done"}
                    ]}
                }]
            }}}}
        }"#;
        let (threads, truncated) = parse_review_threads(json).unwrap();
        assert!(truncated); // totalCount 2, one node returned.
        assert_eq!(threads.len(), 1);
        let t = &threads[0];
        assert_eq!(t.id, "PRRT_node1");
        assert!(t.is_resolved);
        assert!(!t.is_outdated);
        assert_eq!(t.path, "src/lib.rs");
        assert_eq!(t.line, Some(2));
        assert_eq!(t.side, PrDiffSide::Right);
        assert_eq!(t.comments.len(), 2);
        assert_eq!(t.comments[0].author, "alice");
        assert_eq!(t.comments[1].body, "done");

        assert!(parse_review_threads("nope").is_err());
        assert!(parse_review_threads(r#"{"data": null}"#).is_err());
    }

    #[test]
    fn big_pr_threshold_and_default_expansion() {
        let files = parse_pr_diff(SAMPLE_DIFF);
        let expanded = default_expanded(&files);
        // Small PR: first files expand, binary never does.
        assert_eq!(expanded, vec![true, true, true, true, false]);

        // A file bigger than the auto-expand cap stays collapsed.
        let mut big_file = files[0].clone();
        big_file.hunks[0].lines = (0..(AUTO_EXPAND_MAX_PATCH_LINES + 1))
            .map(|i| PrDiffLine {
                kind: PrDiffLineKind::Add,
                old_line: None,
                new_line: Some(i + 1),
                text: String::new(),
            })
            .collect();
        assert_eq!(default_expanded(&[big_file.clone()]), vec![false]);

        // Past the big-PR threshold everything collapses.
        let copies = (BIG_PR_TOTAL_LINE_THRESHOLD / big_file.patch_line_count()) + 1;
        let many: Vec<PrFileDiff> = (0..copies).map(|_| big_file.clone()).collect();
        assert!(total_line_count(&many) > BIG_PR_TOTAL_LINE_THRESHOLD);
        assert!(default_expanded(&many).iter().all(|e| !e));
    }
}
