use std::collections::HashSet;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use regex::{Regex, RegexBuilder};

use crate::workspace::view::global_search::SearchConfig;

const CONTEXT_LINE_COUNT: usize = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceFingerprint {
    pub query: String,
    pub config: SearchConfig,
    pub roots: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ReplacePreview {
    pub fingerprint: ReplaceFingerprint,
    pub replacement: String,
    pub files: Vec<ReplaceFilePreview>,
}

impl ReplacePreview {
    pub fn included_file_count(&self) -> usize {
        self.files.iter().filter(|file| file.included).count()
    }

    pub fn included_match_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.included)
            .map(|file| file.matches.len())
            .sum()
    }

    pub fn total_file_count(&self) -> usize {
        self.files.len()
    }

    pub fn total_match_count(&self) -> usize {
        self.files.iter().map(|file| file.matches.len()).sum()
    }
}

#[derive(Clone, Debug)]
pub struct ReplaceFilePreview {
    pub path: PathBuf,
    pub included: bool,
    pub matches: Vec<ReplaceMatchPreview>,
}

#[derive(Clone, Debug)]
pub struct ReplaceMatchPreview {
    pub byte_range: Range<usize>,
    pub line_number: usize,
    pub column_number: usize,
    pub old_text: String,
    pub replacement_text: String,
    pub context_before: Vec<String>,
    pub line_text: String,
    pub context_after: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ReplaceApplySummary {
    pub applied: Vec<PathBuf>,
    pub skipped: Vec<ReplaceSkippedFile>,
    pub failed: Vec<ReplaceFailedFile>,
}

impl ReplaceApplySummary {
    pub fn changed_file_count(&self) -> usize {
        self.applied.len()
    }
}

#[derive(Clone, Debug)]
pub struct ReplaceSkippedFile {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ReplaceFailedFile {
    pub path: PathBuf,
    pub error: String,
}

pub fn make_fingerprint(
    query: String,
    config: SearchConfig,
    mut roots: Vec<PathBuf>,
) -> ReplaceFingerprint {
    roots.sort();
    ReplaceFingerprint {
        query,
        config,
        roots,
    }
}

pub async fn generate_preview(
    fingerprint: ReplaceFingerprint,
    replacement: String,
    candidate_paths: Vec<PathBuf>,
) -> Result<ReplacePreview> {
    let matcher = build_matcher(&fingerprint.query, &fingerprint.config)?;
    let mut seen_paths = HashSet::new();
    let mut files = Vec::new();

    for path in candidate_paths {
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        if !path_is_inside_roots(&path, &fingerprint.roots) {
            continue;
        }

        let content = async_fs::read_to_string(&path).await?;
        let matches = preview_matches_for_content(&content, &replacement, &matcher);
        if matches.is_empty() {
            continue;
        }

        files.push(ReplaceFilePreview {
            path,
            included: true,
            matches,
        });
    }

    Ok(ReplacePreview {
        fingerprint,
        replacement,
        files,
    })
}

pub fn preview_matches_for_content(
    content: &str,
    replacement: &str,
    matcher: &Regex,
) -> Vec<ReplaceMatchPreview> {
    let line_infos = LineInfo::collect(content);

    matcher
        .find_iter(content)
        .filter_map(|matched| {
            let byte_range = matched.start()..matched.end();
            let line_index = line_index_for_byte(&line_infos, byte_range.start)?;
            let line = &line_infos[line_index];
            let line_text = content[line.content_range.clone()].to_string();
            let column_number = content[line.content_range.start..byte_range.start]
                .chars()
                .count()
                + 1;

            let context_start = line_index.saturating_sub(CONTEXT_LINE_COUNT);
            let context_before = line_infos[context_start..line_index]
                .iter()
                .map(|info| content[info.content_range.clone()].to_string())
                .collect();

            let after_start = line_index + 1;
            let after_end = (after_start + CONTEXT_LINE_COUNT).min(line_infos.len());
            let context_after = line_infos[after_start..after_end]
                .iter()
                .map(|info| content[info.content_range.clone()].to_string())
                .collect();

            Some(ReplaceMatchPreview {
                byte_range,
                line_number: line_index + 1,
                column_number,
                old_text: matched.as_str().to_string(),
                replacement_text: replacement.to_string(),
                context_before,
                line_text,
                context_after,
            })
        })
        .collect()
}

pub async fn apply_preview_to_disk(
    preview: &ReplacePreview,
    open_paths: HashSet<PathBuf>,
) -> ReplaceApplySummary {
    let mut summary = ReplaceApplySummary::default();

    for file in &preview.files {
        if !file.included {
            summary.skipped.push(ReplaceSkippedFile {
                path: file.path.clone(),
                reason: "File excluded from preview".to_string(),
            });
            continue;
        }

        if open_paths.contains(&file.path) {
            summary.skipped.push(ReplaceSkippedFile {
                path: file.path.clone(),
                reason: "File is open in the editor; skipped to avoid overwriting editor state"
                    .to_string(),
            });
            continue;
        }

        if !path_is_inside_roots(&file.path, &preview.fingerprint.roots) {
            summary.skipped.push(ReplaceSkippedFile {
                path: file.path.clone(),
                reason: "File is outside the searched project roots".to_string(),
            });
            continue;
        }

        match apply_file_to_disk(file, &preview.replacement).await {
            Ok(ApplyFileOutcome::Applied) => summary.applied.push(file.path.clone()),
            Ok(ApplyFileOutcome::Skipped(reason)) => summary.skipped.push(ReplaceSkippedFile {
                path: file.path.clone(),
                reason,
            }),
            Err(error) => summary.failed.push(ReplaceFailedFile {
                path: file.path.clone(),
                error: error.to_string(),
            }),
        }
    }

    summary
}

enum ApplyFileOutcome {
    Applied,
    Skipped(String),
}

async fn apply_file_to_disk(
    file: &ReplaceFilePreview,
    replacement: &str,
) -> Result<ApplyFileOutcome> {
    let mut content = match async_fs::read_to_string(&file.path).await {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(ApplyFileOutcome::Skipped(
                "File no longer exists on disk".to_string(),
            ));
        }
        Err(err) => return Err(err.into()),
    };

    if !spans_still_match(&content, file) {
        return Ok(ApplyFileOutcome::Skipped(
            "File changed since preview was generated".to_string(),
        ));
    }

    for matched in file.matches.iter().rev() {
        content.replace_range(matched.byte_range.clone(), replacement);
    }

    twarp_files::FileModel::write_content_for_file(file.path.clone(), content).await?;
    Ok(ApplyFileOutcome::Applied)
}

pub fn spans_still_match(content: &str, file: &ReplaceFilePreview) -> bool {
    file.matches.iter().all(|matched| {
        content
            .get(matched.byte_range.clone())
            .is_some_and(|current| current == matched.old_text)
    })
}

pub fn build_matcher(query: &str, config: &SearchConfig) -> Result<Regex> {
    let pattern = if config.use_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };

    RegexBuilder::new(&pattern)
        .case_insensitive(!config.use_case_sensitivity)
        .build()
        .map_err(|err| anyhow!("Invalid regex: {err}"))
}

fn path_is_inside_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn line_index_for_byte(lines: &[LineInfo], byte_offset: usize) -> Option<usize> {
    let index = lines.partition_point(|line| line.full_range.end <= byte_offset);
    lines.get(index).map(|_| index)
}

#[derive(Debug)]
struct LineInfo {
    full_range: Range<usize>,
    content_range: Range<usize>,
}

impl LineInfo {
    fn collect(content: &str) -> Vec<Self> {
        if content.is_empty() {
            return vec![LineInfo {
                full_range: 0..0,
                content_range: 0..0,
            }];
        }

        let mut lines = Vec::new();
        let mut line_start = 0;
        for segment in content.split_inclusive('\n') {
            let full_start = line_start;
            let full_end = full_start + segment.len();
            let content_end = segment
                .strip_suffix('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).len())
                .map(|line_len| full_start + line_len)
                .unwrap_or(full_end);

            lines.push(LineInfo {
                full_range: full_start..full_end,
                content_range: full_start..content_end,
            });
            line_start = full_end;
        }

        if line_start < content.len() {
            lines.push(LineInfo {
                full_range: line_start..content.len(),
                content_range: line_start..content.len(),
            });
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(use_regex: bool, use_case_sensitivity: bool) -> SearchConfig {
        SearchConfig {
            use_regex,
            use_case_sensitivity,
            includes: Vec::new(),
            excludes: Vec::new(),
        }
    }

    #[test]
    fn literal_preview_handles_multiple_matches_and_utf8_columns() {
        let matcher = build_matcher("needle", &config(false, false)).unwrap();
        let matches =
            preview_matches_for_content("pré needle\nsecond needle\n", "thread", &matcher);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 1);
        assert_eq!(matches[0].column_number, 5);
        assert_eq!(matches[0].old_text, "needle");
        assert_eq!(matches[0].replacement_text, "thread");
        assert_eq!(matches[0].context_after, vec!["second needle"]);
        assert_eq!(matches[1].context_before, vec!["pré needle"]);
    }

    #[test]
    fn regex_preview_uses_literal_replacement_text() {
        let matcher = build_matcher("need(le)", &config(true, true)).unwrap();
        let matches = preview_matches_for_content("needle", "$1", &matcher);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].old_text, "needle");
        assert_eq!(matches[0].replacement_text, "$1");
    }

    #[test]
    fn stale_span_detection_rejects_changed_content() {
        let file = ReplaceFilePreview {
            path: PathBuf::from("/tmp/example.txt"),
            included: true,
            matches: vec![ReplaceMatchPreview {
                byte_range: 0..6,
                line_number: 1,
                column_number: 1,
                old_text: "needle".to_string(),
                replacement_text: "thread".to_string(),
                context_before: Vec::new(),
                line_text: "needle".to_string(),
                context_after: Vec::new(),
            }],
        };

        assert!(spans_still_match("needle", &file));
        assert!(!spans_still_match("thread", &file));
    }

    #[test]
    fn apply_from_end_preserves_later_spans_when_lengths_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        std::fs::write(&path, "needle needle").unwrap();

        let matcher = build_matcher("needle", &config(false, true)).unwrap();
        let matches = preview_matches_for_content("needle needle", "x", &matcher);
        let preview = ReplacePreview {
            fingerprint: make_fingerprint(
                "needle".to_string(),
                config(false, true),
                vec![dir.path().to_path_buf()],
            ),
            replacement: "x".to_string(),
            files: vec![ReplaceFilePreview {
                path: path.clone(),
                included: true,
                matches,
            }],
        };

        let summary = async_io::block_on(apply_preview_to_disk(&preview, HashSet::new()));
        assert_eq!(summary.applied, vec![path.clone()]);
        assert!(summary.skipped.is_empty());
        assert!(summary.failed.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "x x");
    }

    #[test]
    fn excluded_file_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        std::fs::write(&path, "needle").unwrap();

        let matcher = build_matcher("needle", &config(false, true)).unwrap();
        let matches = preview_matches_for_content("needle", "thread", &matcher);
        let preview = ReplacePreview {
            fingerprint: make_fingerprint(
                "needle".to_string(),
                config(false, true),
                vec![dir.path().to_path_buf()],
            ),
            replacement: "thread".to_string(),
            files: vec![ReplaceFilePreview {
                path: path.clone(),
                included: false,
                matches,
            }],
        };

        let summary = async_io::block_on(apply_preview_to_disk(&preview, HashSet::new()));
        assert!(summary.applied.is_empty());
        assert_eq!(summary.skipped.len(), 1);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "needle");
    }
}
