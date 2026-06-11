//! Reading the on-disk session store `claude` itself maintains under
//! `~/.claude/projects/<encoded-cwd>/*.jsonl` (PRODUCT §46–§51).
//!
//! twarp keeps **no parallel database** — every session listed here belongs
//! to `claude`, and resume is a thin wrapper over `claude --resume <id>`. The
//! file format is undocumented and may evolve, so reads are deliberately
//! best-effort: a corrupt or unfamiliar entry falls back to an untitled
//! session with just a timestamp.

use std::collections::VecDeque;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::TranscriptEvent;

/// One stored session entry for a given cwd.
#[derive(Clone, Debug)]
pub struct StoredSession {
    /// `claude`'s session id (file stem of the `.jsonl`).
    pub id: String,
    /// Best-effort title — the first user message, truncated. Falls back to
    /// "Untitled session" when nothing parseable is found.
    pub title: String,
    /// File modification time, used to sort recent-first.
    pub timestamp: SystemTime,
    /// The session's `.jsonl` file path (for diagnostics; resume doesn't read
    /// this — `claude --resume <id>` does that itself).
    pub jsonl_path: PathBuf,
}

/// Encode a working directory the way `claude` does on disk: every
/// non-alphanumeric character → `-`. Matches the convention TECH §panel /
/// 7h calls out.
pub fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Where `claude` stores sessions for a given cwd. Returns `None` when `HOME`
/// is not set (no point listing).
pub fn sessions_dir(cwd: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".claude")
            .join("projects")
            .join(encode_cwd(cwd)),
    )
}

/// Whether any stored session exists for this cwd — an existence-only probe
/// (one `read_dir`, no per-file parsing) cheap enough to gate the sidebar
/// entry's visibility on every directory change (PRODUCT §35).
pub fn has_sessions(cwd: &Path) -> bool {
    let Some(dir) = sessions_dir(cwd) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_str()
            .is_some_and(|n| n.ends_with(".jsonl"))
    })
}

/// List stored sessions for a given cwd, most-recent first.
///
/// Never returns an error: an unreadable directory or unparseable entry is
/// silently skipped (logged at `debug`). The panel can call this on every
/// open without worrying about cascading failures (PRODUCT §50).
pub fn list_sessions(cwd: &Path) -> Vec<StoredSession> {
    let Some(dir) = sessions_dir(cwd) else {
        return Vec::new();
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) => {
            log::debug!("claude: no sessions directory at {} ({err})", dir.display());
            return Vec::new();
        }
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Some(id) = name_str.strip_suffix(".jsonl") else {
            continue;
        };
        let title = best_effort_title(&path).unwrap_or_else(|| "Untitled session".to_owned());
        let timestamp = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        sessions.push(StoredSession {
            id: id.to_owned(),
            title,
            timestamp,
            jsonl_path: path,
        });
    }
    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sessions
}

/// Replay a stored session's history as transcript events (PRODUCT §36: a
/// resumed pane renders its existing history before continuing live).
///
/// Best-effort like everything else here: an unreadable file yields an empty
/// history (the pane still opens — `claude --resume` is the source of truth
/// for the conversation itself), and unfamiliar lines are skipped via the
/// driver's defensive line parser.
pub fn load_history(path: &Path) -> Vec<TranscriptEvent> {
    let Ok(file) = std::fs::File::open(path) else {
        log::debug!("claude: cannot open session file {}", path.display());
        return Vec::new();
    };
    let reader = std::io::BufReader::new(file);
    let mut out = VecDeque::new();
    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        crate::driver::parse_history_line(&value, &mut out);
    }
    out.into()
}

fn best_effort_title(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(20).filter_map(|l| l.ok()) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        let Some(content) = value.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        if let Some(text) = content.as_str() {
            return Some(short_title(text));
        }
        if let Some(arr) = content.as_array() {
            for block in arr {
                // Only text blocks contribute to the title — tool_result blocks
                // are echoed user events that aren't meaningful labels.
                if block.get("type").and_then(|v| v.as_str()) != Some("text") {
                    continue;
                }
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    return Some(short_title(text));
                }
            }
        }
    }
    None
}

fn short_title(text: &str) -> String {
    const MAX: usize = 60;
    let head = text
        .trim()
        .split('\n')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    if head.chars().count() > MAX {
        let truncated: String = head.chars().take(MAX).collect();
        format!("{truncated}…")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_cwd_predictably() {
        assert_eq!(
            encode_cwd(Path::new("/Users/foo/Development/twarp")),
            "-Users-foo-Development-twarp"
        );
        assert_eq!(
            encode_cwd(Path::new("/tmp/some.project_dir!")),
            "-tmp-some-project-dir-"
        );
    }

    #[test]
    fn short_title_truncates_long_input() {
        let long = "a".repeat(200);
        let t = short_title(&long);
        // 60 'a's + ellipsis.
        assert!(t.ends_with('…'));
        assert_eq!(t.chars().count(), 61);
    }

    #[test]
    fn short_title_uses_only_first_line() {
        assert_eq!(short_title("first\nsecond"), "first");
    }

    #[test]
    fn list_sessions_returns_empty_on_missing_dir() {
        let tmp = std::env::temp_dir().join("twarp-test-no-such-claude");
        let _ = std::fs::remove_dir_all(&tmp);
        let sessions = list_sessions(&tmp);
        assert!(sessions.is_empty());
    }

    #[test]
    fn load_history_replays_turns_and_skips_bookkeeping() {
        let dir = std::env::temp_dir().join("twarp-test-claude-history");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"mode","mode":"default"}"#,
                "\n",
                r#"{"type":"user","message":{"role":"user","content":"hello"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi there"}]}}"#,
                "\n",
                r#"not json at all"#,
                "\n",
                r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"meta"}]}}"#,
                "\n",
            ),
        )
        .unwrap();
        let events = load_history(&path);
        assert_eq!(events.len(), 3, "user + delta + done, got {events:?}");
        assert!(matches!(&events[0], TranscriptEvent::UserMessage(m) if m == "hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_history_missing_file_is_empty() {
        let path = std::env::temp_dir().join("twarp-test-claude-history-missing.jsonl");
        let _ = std::fs::remove_file(&path);
        assert!(load_history(&path).is_empty());
    }
}
