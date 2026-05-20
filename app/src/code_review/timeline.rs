//! twarp 05d: per-file commit Timeline for the Code Review panel.
//!
//! Spec: PRODUCT.md §§18–23. The Timeline section lists the focused
//! file's commits in reverse chronological order, paged 20 at a time.
//! Each entry shows an author-avatar, author name, relative time, and
//! commit subject, plus a small `R` badge on the rename commit and a
//! `↑` marker on commits that are ahead of the upstream.
//!
//! This module is **render-data-only**: it parses `git log` output and
//! exposes async helpers for fetching pages and the ahead-of-upstream
//! SHA set. The view layer (`code_review_view.rs`) owns the
//! `TimelineState`, drives fetches, and renders rows.
//!
//! ## Log format
//!
//! ```text
//! git log --follow --name-status \
//!         --format=COMMIT:%H<US>%an<US>%ae<US>%at<US>%s \
//!         -n <limit> --skip <offset> -- <path>
//! ```
//!
//! `<US>` is the ASCII unit separator (`\x1f`), chosen because real
//! commit metadata almost never contains it (unlike `\t` or `|`). Each
//! `COMMIT:` line is the per-entry header; the lines that follow until
//! the next `COMMIT:` are `--name-status` rows for that commit (e.g.
//! `M\tpath/to/file.rs` or `R100\told\tnew`). A line whose status is
//! `R...` and whose `new` column matches the focused path marks the
//! commit as the rename commit for that file (PRODUCT §22); the
//! corresponding `old` path becomes the `original_path`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[cfg(feature = "local_fs")]
use anyhow::Result;

#[cfg(feature = "local_fs")]
use crate::util::git::run_git_command;

/// Field separator used inside a `COMMIT:` line. ASCII unit separator
/// (`\x1f`) — extremely unlikely to appear in author names, emails, or
/// subjects, unlike `\t` or `|`.
const FIELD_SEP: char = '\x1f';

/// Header prefix that distinguishes commit metadata from `--name-status`
/// rows in the log output.
const COMMIT_PREFIX: &str = "COMMIT:";

/// Default page size for the Timeline (PRODUCT §19: "first page = 20
/// entries"; §20: "[Load more] appends the next 20").
pub const TIMELINE_PAGE_SIZE: usize = 20;

/// One commit row in the Timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEntry {
    pub sha: String,
    pub short_sha: String,
    pub author_name: String,
    pub author_email: String,
    /// Unix epoch seconds of the author timestamp.
    pub timestamp: i64,
    pub subject: String,
    /// True when this commit's `--name-status` row for the focused
    /// path was an `R<score>` entry — i.e. the commit that renamed
    /// the file to its current path. PRODUCT §22.
    pub is_rename_commit: bool,
    /// For commits older than (or equal to) the rename commit, the
    /// path the file had at that point in history. PRODUCT §22 — used
    /// for the per-row tooltip. `None` for commits where the path
    /// matched the focused path verbatim.
    pub original_path: Option<PathBuf>,
    /// `true` when the commit's SHA is in the
    /// `<upstream>..HEAD` set for the focused path. PRODUCT §23.
    pub is_local_only: bool,
}

impl TimelineEntry {
    /// First letter of the author name, used by the avatar circle.
    /// Falls back to `?` for empty names.
    pub fn avatar_letter(&self) -> char {
        self.author_name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('?')
    }
}

/// Parse `git log --follow --name-status --format=COMMIT:...` output.
///
/// `focused_path` is the path the Timeline is currently tracking;
/// `--name-status` lines are checked against it to set the rename and
/// `original_path` fields. Lines that don't parse cleanly are skipped
/// rather than failing the whole batch — partial output is better than
/// no output when, e.g., a future git release adds a new status code.
pub fn parse_log_output(text: &str, focused_path: &Path) -> Vec<TimelineEntry> {
    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut current: Option<TimelineEntry> = None;
    let focused_str = focused_path.to_string_lossy();

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(COMMIT_PREFIX) {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            current = parse_commit_header(rest);
        } else if let Some(entry) = current.as_mut() {
            apply_name_status_line(entry, line, &focused_str);
        }
    }
    if let Some(last) = current.take() {
        entries.push(last);
    }
    entries
}

fn parse_commit_header(rest: &str) -> Option<TimelineEntry> {
    // Fields: sha | author_name | author_email | timestamp | subject
    let parts: Vec<&str> = rest.splitn(5, FIELD_SEP).collect();
    if parts.len() < 5 {
        return None;
    }
    let sha = parts[0].trim();
    if sha.is_empty() {
        return None;
    }
    let timestamp = parts[3].trim().parse::<i64>().ok()?;
    let short_sha: String = sha.chars().take(7).collect();
    Some(TimelineEntry {
        sha: sha.to_string(),
        short_sha,
        author_name: parts[1].to_string(),
        author_email: parts[2].to_string(),
        timestamp,
        subject: parts[4].to_string(),
        is_rename_commit: false,
        original_path: None,
        is_local_only: false,
    })
}

/// Inspect a `--name-status` row to set rename / original-path fields
/// on the current entry. Rows we don't recognize are silently ignored —
/// the entry still shows up in the Timeline list with no badge.
fn apply_name_status_line(entry: &mut TimelineEntry, line: &str, focused_path: &str) {
    let mut parts = line.split('\t');
    let Some(status) = parts.next() else { return };
    if status.starts_with('R') || status.starts_with('C') {
        let Some(old) = parts.next() else { return };
        let Some(new) = parts.next() else { return };
        if paths_match(new, focused_path) {
            // Only mark as rename for actual R<score>; copies (C<score>)
            // share the same on-disk shape but don't get the badge.
            entry.is_rename_commit = status.starts_with('R');
            entry.original_path = Some(PathBuf::from(old));
        }
    }
}

fn paths_match(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
}

/// Apply a precomputed `<upstream>..HEAD` SHA set to a freshly-fetched
/// page of entries. Called by the view after every page load so that
/// already-loaded entries' markers stay in sync with new pushes.
pub fn mark_local_only(entries: &mut [TimelineEntry], local_only: &HashSet<String>) {
    for entry in entries.iter_mut() {
        entry.is_local_only = local_only.contains(&entry.sha);
    }
}

/// Fetch a page of Timeline entries for `file_path`.
///
/// The caller computes `offset` (entries already loaded) and `limit`
/// ([`TIMELINE_PAGE_SIZE`] for both the first page and "Load more"
/// pages). The returned `Vec` may be shorter than `limit` when the
/// file's history has fewer remaining commits — the view uses that
/// signal to hide the `[Load more]` link.
#[cfg(feature = "local_fs")]
pub async fn fetch_log_page(
    repo_path: &Path,
    file_path: &Path,
    offset: usize,
    limit: usize,
) -> Result<Vec<TimelineEntry>> {
    let path_str = file_path.to_string_lossy();
    let format_arg = format!(
        "--format={COMMIT_PREFIX}%H{FIELD_SEP}%an{FIELD_SEP}%ae{FIELD_SEP}%at{FIELD_SEP}%s"
    );
    let offset_str = offset.to_string();
    let limit_str = limit.to_string();
    let output = run_git_command(
        repo_path,
        &[
            "log",
            "--follow",
            "--name-status",
            &format_arg,
            "-n",
            &limit_str,
            "--skip",
            &offset_str,
            "--",
            path_str.as_ref(),
        ],
    )
    .await?;
    Ok(parse_log_output(&output, file_path))
}

/// Fetch the set of SHAs that are ahead of the upstream for
/// `file_path`. `upstream_ref` is something like `"origin/main"`.
///
/// Returns an empty set when no upstream is configured — PRODUCT §23:
/// "No marker when there's no upstream configured."
#[cfg(feature = "local_fs")]
pub async fn fetch_local_only_shas(
    repo_path: &Path,
    file_path: &Path,
    upstream_ref: Option<&str>,
) -> Result<HashSet<String>> {
    let Some(upstream) = upstream_ref.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(HashSet::new());
    };
    let path_str = file_path.to_string_lossy();
    let range = format!("{upstream}..HEAD");
    let output = run_git_command(
        repo_path,
        &["log", &range, "--format=%H", "--", path_str.as_ref()],
    )
    .await?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(sha: &str, name: &str, email: &str, ts: i64, subject: &str) -> String {
        format!("{COMMIT_PREFIX}{sha}{FIELD_SEP}{name}{FIELD_SEP}{email}{FIELD_SEP}{ts}{FIELD_SEP}{subject}")
    }

    #[test]
    fn parses_single_entry_no_name_status() {
        let text = header(
            "abcdef0123456789",
            "Ada",
            "ada@example.com",
            1700000000,
            "fix bug",
        );
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.sha, "abcdef0123456789");
        assert_eq!(e.short_sha, "abcdef0");
        assert_eq!(e.author_name, "Ada");
        assert_eq!(e.author_email, "ada@example.com");
        assert_eq!(e.timestamp, 1700000000);
        assert_eq!(e.subject, "fix bug");
        assert!(!e.is_rename_commit);
        assert!(e.original_path.is_none());
        assert!(!e.is_local_only);
    }

    #[test]
    fn parses_multiple_entries() {
        let text = format!(
            "{h1}\nM\tsrc/main.rs\n{h2}\nM\tsrc/main.rs\n",
            h1 = header("aaa1111", "Ada", "a@x", 1, "one"),
            h2 = header("bbb2222", "Bob", "b@x", 2, "two"),
        );
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].subject, "one");
        assert_eq!(entries[1].subject, "two");
    }

    #[test]
    fn rename_commit_sets_badge_and_original_path() {
        let text = format!(
            "{h}\nR100\tsrc/old.rs\tsrc/new.rs\n",
            h = header("ccc3333", "Cara", "c@x", 3, "rename old to new"),
        );
        let entries = parse_log_output(&text, Path::new("src/new.rs"));
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_rename_commit);
        assert_eq!(entries[0].original_path, Some(PathBuf::from("src/old.rs")));
    }

    #[test]
    fn rename_to_unrelated_path_is_ignored() {
        let text = format!(
            "{h}\nR100\tsrc/other_a.rs\tsrc/other_b.rs\n",
            h = header("ddd4444", "Dan", "d@x", 4, "unrelated rename"),
        );
        let entries = parse_log_output(&text, Path::new("src/new.rs"));
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_rename_commit);
        assert!(entries[0].original_path.is_none());
    }

    #[test]
    fn copy_sets_original_path_but_not_rename_badge() {
        let text = format!(
            "{h}\nC75\tsrc/source.rs\tsrc/dest.rs\n",
            h = header("eee5555", "Eve", "e@x", 5, "copy file"),
        );
        let entries = parse_log_output(&text, Path::new("src/dest.rs"));
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_rename_commit);
        assert_eq!(
            entries[0].original_path,
            Some(PathBuf::from("src/source.rs"))
        );
    }

    #[test]
    fn ignores_blank_lines_and_unknown_status() {
        let text = format!(
            "{h}\n\nX\tsrc/main.rs\n",
            h = header("fff6666", "Fay", "f@x", 6, "weird"),
        );
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_rename_commit);
    }

    #[test]
    fn ignores_malformed_header() {
        let text = "COMMIT:not\x1fenough\x1ffields\n";
        let entries = parse_log_output(text, Path::new("src/main.rs"));
        assert!(entries.is_empty());
    }

    #[test]
    fn ignores_non_numeric_timestamp() {
        let text = format!(
            "{prefix}badsha{sep}Ada{sep}a@x{sep}notanumber{sep}subject\n",
            prefix = COMMIT_PREFIX,
            sep = FIELD_SEP,
        );
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert!(entries.is_empty());
    }

    #[test]
    fn short_sha_for_sub_seven_char_sha_stays_short() {
        let text = header("abc12", "Ada", "a@x", 1, "short sha");
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].short_sha, "abc12");
    }

    #[test]
    fn name_status_before_any_commit_is_dropped() {
        let text = format!(
            "M\tsrc/main.rs\nA\tsrc/new.rs\n{h}\n",
            h = header("ggg7777", "Gus", "g@x", 7, "ok"),
        );
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sha, "ggg7777");
    }

    #[test]
    fn subject_with_internal_separators_preserved() {
        // `splitn(5, FIELD_SEP)` means an FS inside the subject would
        // bleed into other fields — but git's %s never produces FS, so
        // we test that a non-FS subject with other separators (`:`,
        // `|`, `\t`) survives intact.
        let text = header("hhh8888", "Han", "h@x", 8, "fix: thing | edge-case\tnotes");
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].subject, "fix: thing | edge-case\tnotes");
    }

    #[test]
    fn avatar_letter_uppercases_first_alphanumeric() {
        let mut entry = TimelineEntry {
            sha: "abc".to_string(),
            short_sha: "abc".to_string(),
            author_name: "  ada lovelace".to_string(),
            author_email: "ada@x".to_string(),
            timestamp: 0,
            subject: "s".to_string(),
            is_rename_commit: false,
            original_path: None,
            is_local_only: false,
        };
        assert_eq!(entry.avatar_letter(), 'A');
        entry.author_name = "9-machines".to_string();
        assert_eq!(entry.avatar_letter(), '9');
        entry.author_name = "".to_string();
        assert_eq!(entry.avatar_letter(), '?');
    }

    #[test]
    fn mark_local_only_flags_matching_shas() {
        let mut entries = vec![
            TimelineEntry {
                sha: "aaa".to_string(),
                short_sha: "aaa".to_string(),
                author_name: "n".to_string(),
                author_email: "e".to_string(),
                timestamp: 0,
                subject: "s".to_string(),
                is_rename_commit: false,
                original_path: None,
                is_local_only: false,
            },
            TimelineEntry {
                sha: "bbb".to_string(),
                short_sha: "bbb".to_string(),
                author_name: "n".to_string(),
                author_email: "e".to_string(),
                timestamp: 0,
                subject: "s".to_string(),
                is_rename_commit: false,
                original_path: None,
                is_local_only: false,
            },
        ];
        let mut set = HashSet::new();
        set.insert("bbb".to_string());
        mark_local_only(&mut entries, &set);
        assert!(!entries[0].is_local_only);
        assert!(entries[1].is_local_only);
    }

    #[test]
    fn empty_input_yields_no_entries() {
        let entries = parse_log_output("", Path::new("any"));
        assert!(entries.is_empty());
    }

    #[test]
    fn handles_crlf_line_endings() {
        let text = format!(
            "{h}\r\nM\tsrc/main.rs\r\n",
            h = header("iii9999", "Ida", "i@x", 9, "crlf")
        );
        let entries = parse_log_output(&text, Path::new("src/main.rs"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sha, "iii9999");
    }
}
