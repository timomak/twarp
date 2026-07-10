use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use twarp_util::content_version::ContentVersion;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlameCacheKey {
    pub repo_root: PathBuf,
    pub relative_path: PathBuf,
    pub content_version: ContentVersion,
    pub head_oid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameCommitRef {
    pub full_sha: String,
    pub short_sha: String,
    pub author_name: String,
    pub author_email: Option<String>,
    pub author_timestamp: Option<i64>,
    pub author_timezone: Option<String>,
    pub summary: Option<String>,
    pub original_filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlameLineState {
    Committed(BlameCommitRef),
    Uncommitted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub line_index: usize,
    pub state: BlameLineState,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedBlame {
    pub lines: Vec<Option<BlameLine>>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlameGutterAnnotationKind {
    Committed { sha: String },
    Uncommitted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameGutterAnnotation {
    pub line_index: usize,
    pub text: String,
    pub kind: BlameGutterAnnotationKind,
}

#[derive(Debug, Clone)]
pub struct FetchedBlame {
    pub key: BlameCacheKey,
    pub parsed: ParsedBlame,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitDetailKey {
    pub repo_root: PathBuf,
    pub relative_path: PathBuf,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    pub full_sha: String,
    pub author_name: String,
    pub author_email: Option<String>,
    pub absolute_author_date: Option<String>,
    pub relative_author_date: Option<String>,
    pub message: String,
    pub relative_path: PathBuf,
    pub patch: String,
    pub patch_truncated: bool,
    pub github_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FetchedCommitDetail {
    pub key: CommitDetailKey,
    pub detail: CommitDetail,
}

#[derive(Debug, Default, Clone)]
struct CommitMetadata {
    author_name: Option<String>,
    author_email: Option<String>,
    author_timestamp: Option<i64>,
    author_timezone: Option<String>,
    summary: Option<String>,
    original_filename: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingRecord {
    sha: String,
    final_line: usize,
    remaining_lines: usize,
    metadata: CommitMetadata,
}

pub fn parse_git_blame_porcelain(input: &str) -> ParsedBlame {
    let mut parsed = ParsedBlame::default();
    let mut commits: HashMap<String, CommitMetadata> = HashMap::new();
    let mut pending: Option<PendingRecord> = None;

    for line in input.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            let Some(record) = pending.as_mut() else {
                parsed
                    .diagnostics
                    .push("content line without blame header".to_string());
                let _ = content;
                continue;
            };

            insert_record(&mut parsed, &mut commits, record);
            if record.remaining_lines > 1 {
                record.remaining_lines -= 1;
                record.final_line += 1;
            } else {
                pending = None;
            }
            continue;
        }

        if let Some(record) = parse_header(line) {
            if pending.is_some() {
                parsed
                    .diagnostics
                    .push("blame header replaced incomplete record".to_string());
            }
            pending = Some(record);
            continue;
        }

        let Some(record) = pending.as_mut() else {
            parsed
                .diagnostics
                .push(format!("metadata without blame header: {line}"));
            continue;
        };

        if let Some(value) = line.strip_prefix("author ") {
            record.metadata.author_name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("author-mail ") {
            record.metadata.author_email = Some(trim_git_mail(value));
        } else if let Some(value) = line.strip_prefix("author-time ") {
            match value.parse::<i64>() {
                Ok(timestamp) => record.metadata.author_timestamp = Some(timestamp),
                Err(_) => parsed
                    .diagnostics
                    .push(format!("malformed author-time: {value}")),
            }
        } else if let Some(value) = line.strip_prefix("author-tz ") {
            record.metadata.author_timezone = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("summary ") {
            record.metadata.summary = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("filename ") {
            record.metadata.original_filename = Some(value.to_string());
        }
    }

    if pending.is_some() {
        parsed
            .diagnostics
            .push("trailing blame header without content".to_string());
    }

    parsed
}

pub fn committed_annotation(line_index: usize, commit: &BlameCommitRef) -> BlameGutterAnnotation {
    let date = commit
        .author_timestamp
        .map(compact_relative_date)
        .unwrap_or_else(|| "unknown".to_string());
    BlameGutterAnnotation {
        line_index,
        text: format!("{} {} {}", commit.author_name, commit.short_sha, date),
        kind: BlameGutterAnnotationKind::Committed {
            sha: commit.full_sha.clone(),
        },
    }
}

pub fn uncommitted_annotation(line_index: usize) -> BlameGutterAnnotation {
    BlameGutterAnnotation {
        line_index,
        text: "(uncommitted)".to_string(),
        kind: BlameGutterAnnotationKind::Uncommitted,
    }
}

fn parse_header(line: &str) -> Option<PendingRecord> {
    let mut parts = line.split_whitespace();
    let sha = parts.next()?;
    if !is_blame_sha(sha) {
        return None;
    }
    let _original_line = parts.next()?.parse::<usize>().ok()?;
    let final_line = parts.next()?.parse::<usize>().ok()?;
    let remaining_lines = parts
        .next()
        .and_then(|group_size| group_size.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);

    Some(PendingRecord {
        sha: sha.to_string(),
        final_line: final_line.saturating_sub(1),
        remaining_lines,
        metadata: CommitMetadata::default(),
    })
}

fn insert_record(
    parsed: &mut ParsedBlame,
    commits: &mut HashMap<String, CommitMetadata>,
    record: &PendingRecord,
) {
    let line_index = record.final_line;
    if parsed.lines.len() <= line_index {
        parsed.lines.resize(line_index + 1, None);
    }

    let state = if is_zero_sha(&record.sha) {
        BlameLineState::Uncommitted
    } else {
        if has_commit_metadata(&record.metadata) {
            commits.insert(record.sha.clone(), record.metadata.clone());
        }
        let metadata = commits
            .get(&record.sha)
            .cloned()
            .unwrap_or_else(|| record.metadata.clone());
        BlameLineState::Committed(commit_ref(&record.sha, metadata))
    };

    parsed.lines[line_index] = Some(BlameLine { line_index, state });
}

fn commit_ref(sha: &str, metadata: CommitMetadata) -> BlameCommitRef {
    BlameCommitRef {
        full_sha: sha.to_string(),
        short_sha: sha.chars().take(7).collect(),
        author_name: metadata
            .author_name
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Unknown".to_string()),
        author_email: metadata.author_email,
        author_timestamp: metadata.author_timestamp,
        author_timezone: metadata.author_timezone,
        summary: metadata.summary,
        original_filename: metadata.original_filename,
    }
}

fn has_commit_metadata(metadata: &CommitMetadata) -> bool {
    metadata.author_name.is_some()
        || metadata.author_email.is_some()
        || metadata.author_timestamp.is_some()
        || metadata.author_timezone.is_some()
        || metadata.summary.is_some()
        || metadata.original_filename.is_some()
}

fn trim_git_mail(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string()
}

fn is_blame_sha(value: &str) -> bool {
    value.len() >= 7 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_zero_sha(value: &str) -> bool {
    value.chars().all(|c| c == '0')
}

pub fn compact_relative_date(timestamp: i64) -> String {
    let Some(datetime) = DateTime::<Utc>::from_timestamp(timestamp, 0) else {
        return "unknown".to_string();
    };
    let duration = Utc::now().signed_duration_since(datetime);
    let seconds = duration.num_seconds().max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let months = days / 30;
    let years = days / 365;

    if years >= 1 {
        format!("{years}y ago")
    } else if months >= 1 {
        format!("{months}mo ago")
    } else if days >= 1 {
        format!("{days}d ago")
    } else if hours >= 1 {
        format!("{hours}h ago")
    } else if minutes >= 1 {
        format!("{minutes}m ago")
    } else {
        "now".to_string()
    }
}

fn parse_git_show_commit_metadata(input: &str) -> Result<CommitDetailMetadata> {
    let mut parts = input.splitn(5, '\0');
    let full_sha = parts
        .next()
        .filter(|sha| !sha.trim().is_empty())
        .ok_or_else(|| anyhow!("missing commit hash"))?
        .trim()
        .to_string();
    let author_name = parts.next().unwrap_or("Unknown").trim().to_string();
    let author_email = parts
        .next()
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(ToOwned::to_owned);
    let raw_date = parts
        .next()
        .map(str::trim)
        .filter(|date| !date.is_empty())
        .map(ToOwned::to_owned);
    let message = parts.next().unwrap_or_default().trim().to_string();

    let (absolute_author_date, relative_author_date) = match raw_date {
        Some(date) => match DateTime::parse_from_rfc3339(&date) {
            Ok(parsed) => (
                Some(parsed.format("%Y-%m-%d %H:%M:%S %:z").to_string()),
                Some(compact_relative_date(parsed.timestamp())),
            ),
            Err(_) => (Some(date), Some("unknown".to_string())),
        },
        None => (None, None),
    };

    Ok(CommitDetailMetadata {
        full_sha,
        author_name: if author_name.is_empty() {
            "Unknown".to_string()
        } else {
            author_name
        },
        author_email,
        absolute_author_date,
        relative_author_date,
        message,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitDetailMetadata {
    full_sha: String,
    author_name: String,
    author_email: Option<String>,
    absolute_author_date: Option<String>,
    relative_author_date: Option<String>,
    message: String,
}

const MAX_COMMIT_DETAIL_PATCH_CHARS: usize = 20_000;

fn truncate_patch(patch: String) -> (String, bool) {
    if patch.chars().count() <= MAX_COMMIT_DETAIL_PATCH_CHARS {
        return (patch, false);
    }

    (
        patch.chars().take(MAX_COMMIT_DETAIL_PATCH_CHARS).collect(),
        true,
    )
}

fn github_commit_url(origin: &str, sha: &str) -> Option<String> {
    let (owner, repo) = crate::code_review::github_author::parse_github_origin(origin)?;
    Some(format!("https://github.com/{owner}/{repo}/commit/{sha}"))
}

#[cfg(feature = "local_fs")]
pub async fn fetch_commit_detail(
    repo_root: PathBuf,
    relative_path: PathBuf,
    sha: String,
) -> Result<FetchedCommitDetail> {
    use crate::util::git::run_git_command;

    let metadata_output = run_git_command(
        &repo_root,
        &[
            "show",
            "-s",
            "--format=%H%x00%an%x00%ae%x00%aI%x00%B",
            sha.as_str(),
        ],
    )
    .await?;
    let metadata = parse_git_show_commit_metadata(&metadata_output)?;

    let relative_path_arg = relative_path.to_string_lossy().to_string();
    let patch = run_git_command(
        &repo_root,
        &[
            "show",
            "--format=",
            "--patch",
            "--find-renames",
            "--find-copies",
            sha.as_str(),
            "--",
            relative_path_arg.as_str(),
        ],
    )
    .await?;
    let (patch, patch_truncated) = truncate_patch(patch);

    let github_url = run_git_command(&repo_root, &["remote", "get-url", "origin"])
        .await
        .ok()
        .and_then(|origin| github_commit_url(&origin, metadata.full_sha.as_str()));

    let key = CommitDetailKey {
        repo_root,
        relative_path: relative_path.clone(),
        sha,
    };

    Ok(FetchedCommitDetail {
        key,
        detail: CommitDetail {
            full_sha: metadata.full_sha,
            author_name: metadata.author_name,
            author_email: metadata.author_email,
            absolute_author_date: metadata.absolute_author_date,
            relative_author_date: metadata.relative_author_date,
            message: metadata.message,
            relative_path,
            patch,
            patch_truncated,
            github_url,
        },
    })
}

#[cfg(not(feature = "local_fs"))]
pub async fn fetch_commit_detail(
    repo_root: PathBuf,
    relative_path: PathBuf,
    sha: String,
) -> Result<FetchedCommitDetail> {
    Err(anyhow!(
        "git commit details are not supported on this platform: {} {} {}",
        repo_root.display(),
        relative_path.display(),
        sha
    ))
}

#[cfg(feature = "local_fs")]
pub async fn fetch_git_blame(
    repo_root: PathBuf,
    relative_path: PathBuf,
    content_version: ContentVersion,
) -> Result<FetchedBlame> {
    use crate::util::git::run_git_command;

    let head_oid = match run_git_command(&repo_root, &["rev-parse", "HEAD"]).await {
        Ok(output) => Some(output.trim().to_string()),
        Err(error) => {
            log::debug!("git blame skipped because HEAD could not be resolved: {error:#}");
            None
        }
    };

    let parsed = if head_oid.is_some() {
        let relative_path_arg = relative_path.to_string_lossy().to_string();
        match run_git_command(
            &repo_root,
            &["blame", "--porcelain", "--", relative_path_arg.as_str()],
        )
        .await
        {
            Ok(output) => parse_git_blame_porcelain(&output),
            Err(error) => {
                log::debug!(
                    "git blame failed for {}: {error:#}",
                    relative_path.display()
                );
                ParsedBlame::default()
            }
        }
    } else {
        ParsedBlame::default()
    };

    Ok(FetchedBlame {
        key: BlameCacheKey {
            repo_root,
            relative_path,
            content_version,
            head_oid,
        },
        parsed,
    })
}

#[cfg(not(feature = "local_fs"))]
pub async fn fetch_git_blame(
    repo_root: PathBuf,
    relative_path: PathBuf,
    content_version: ContentVersion,
) -> Result<FetchedBlame> {
    Ok(FetchedBlame {
        key: BlameCacheKey {
            repo_root,
            relative_path,
            content_version,
            head_oid: None,
        },
        parsed: ParsedBlame::default(),
    })
}

pub fn path_is_in_repo(path: &Path, repo_root: &Path) -> Option<PathBuf> {
    path.strip_prefix(repo_root).ok().map(Path::to_path_buf)
}

#[cfg(test)]
#[path = "git_blame_tests.rs"]
mod tests;
