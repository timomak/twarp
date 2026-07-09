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

/// The on-disk `.jsonl` path `claude` uses for `session_id` under `cwd`, when
/// `HOME` is set. Existence is *not* implied — callers that need to know
/// whether the session has been persisted yet (e.g. before `claude --resume`)
/// must probe with [`Path::exists`].
pub fn session_file(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    sessions_dir(cwd).map(|dir| dir.join(format!("{session_id}.jsonl")))
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

/// Fork a stored session at a turn boundary into a new independent session
/// ("Fork conversation" — Claude's own term, the CLI's `--fork-session`).
///
/// Writes a fresh `<new_session_id>.jsonl` under `cwd`'s session dir containing
/// the first `keep_user_turns` user turns of `parent_path` (and all the
/// assistant/thinking/tool activity those turns produced), then returns its
/// path. `claude --resume <new_session_id>` continues it as a branch that
/// diverges from the chosen point — the original session is never touched.
///
/// The `claude` CLI has no native fork-at-message flag, so the branch is
/// synthesized by truncating claude's own jsonl after the chosen user turn and
/// rewriting each retained line's `sessionId` to the new id. Retained lines are
/// a contiguous prefix, so the `uuid`/`parentUuid` chain stays internally
/// consistent and claude resumes it like any ordinary session.
///
/// Turn counting reuses the driver's defensive parser, so it classifies genuine
/// prompts versus meta / tool-result `user` lines exactly the way the panel
/// counts its `User` transcript items — the caller and this function agree on
/// what "turn N" means.
pub fn fork_session_file(
    parent_path: &Path,
    new_session_id: &str,
    keep_user_turns: usize,
    cwd: &Path,
) -> std::io::Result<PathBuf> {
    use std::io::Write as _;

    let file = std::fs::File::open(parent_path)?;
    let reader = std::io::BufReader::new(file);

    let mut kept: Vec<String> = Vec::new();
    let mut user_turns_seen = 0usize;
    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            // Unparseable line: carry it through unchanged (defensive — claude
            // writes valid JSON, and a stray line shouldn't drop the prefix).
            kept.push(line);
            continue;
        };
        let mut events = VecDeque::new();
        crate::driver::parse_history_line(&value, &mut events);
        let starts_user_turn = events
            .iter()
            .any(|e| matches!(e, TranscriptEvent::UserMessage(_)));
        if starts_user_turn && user_turns_seen >= keep_user_turns {
            // This line begins the turn *after* the fork point — stop before it.
            break;
        }
        kept.push(rewrite_session_id(value, new_session_id));
        if starts_user_turn {
            user_turns_seen += 1;
        }
    }

    let dir = sessions_dir(cwd).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no claude sessions dir (HOME unset)",
        )
    })?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{new_session_id}.jsonl"));
    let mut out = std::fs::File::create(&path)?;
    for line in &kept {
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
    }
    Ok(path)
}

/// Rewrite a parsed jsonl line's `sessionId` to `new_session_id`, leaving every
/// other field intact, and re-serialize. A line that isn't a JSON object is
/// serialized back unchanged.
fn rewrite_session_id(value: Value, new_session_id: &str) -> String {
    match value {
        Value::Object(mut map) => {
            if map.contains_key("sessionId") {
                map.insert(
                    "sessionId".to_owned(),
                    Value::String(new_session_id.to_owned()),
                );
            }
            Value::Object(map).to_string()
        }
        other => other.to_string(),
    }
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
        // Slash-command envelopes become the command line the user typed;
        // local-command stdout echoes are skipped (display_user_text → None).
        if let Some(text) = content.as_str() {
            if let Some(display) = crate::driver::display_user_text(text) {
                return Some(short_title(&display));
            }
            continue;
        }
        if let Some(arr) = content.as_array() {
            for block in arr {
                // Only text blocks contribute to the title — tool_result blocks
                // are echoed user events that aren't meaningful labels.
                if block.get("type").and_then(|v| v.as_str()) != Some("text") {
                    continue;
                }
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if let Some(display) = crate::driver::display_user_text(text) {
                        return Some(short_title(&display));
                    }
                }
            }
        }
    }
    None
}

fn short_title(text: &str) -> String {
    // Generous safety cap only — the session list wraps the title across the
    // panel width (soft-wrap), so the full first message line is shown rather
    // than a 60-char preview. The cap just guards against a pathologically
    // long single line producing one enormous row.
    const MAX: usize = 200;
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
        let long = "a".repeat(400);
        let t = short_title(&long);
        // 200 'a's + ellipsis.
        assert!(t.ends_with('…'));
        assert_eq!(t.chars().count(), 201);
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
    fn fork_truncates_at_user_turn_and_rewrites_session_id() {
        // Two user turns; forking after the first must keep turn 1 (its user +
        // assistant lines) and drop turn 2, with every kept line re-stamped.
        let home = std::env::temp_dir().join("twarp-test-fork-home");
        let cwd = Path::new("/proj/x");
        let _ = std::fs::remove_dir_all(&home);
        std::env::set_var("HOME", &home);
        let dir = sessions_dir(cwd).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let parent = dir.join("orig.jsonl");
        std::fs::write(
            &parent,
            concat!(
                r#"{"type":"user","sessionId":"orig","message":{"role":"user","content":"one"}}"#,
                "\n",
                r#"{"type":"assistant","sessionId":"orig","message":{"role":"assistant","content":[{"type":"text","text":"a1"}]}}"#,
                "\n",
                r#"{"type":"user","sessionId":"orig","message":{"role":"user","content":[{"type":"tool_result","content":"r"}]}}"#,
                "\n",
                r#"{"type":"user","sessionId":"orig","message":{"role":"user","content":"two"}}"#,
                "\n",
                r#"{"type":"assistant","sessionId":"orig","message":{"role":"assistant","content":[{"type":"text","text":"a2"}]}}"#,
                "\n",
            ),
        )
        .unwrap();

        let new_path = fork_session_file(&parent, "fork", 1, cwd).unwrap();
        assert_eq!(new_path, dir.join("fork.jsonl"));
        let forked = std::fs::read_to_string(&new_path).unwrap();
        // Kept: user "one", assistant "a1", and the tool-result user line (which
        // isn't a genuine turn) — but NOT user "two" or its reply.
        assert!(forked.contains("\"one\""));
        assert!(forked.contains("a1"));
        assert!(forked.contains("tool_result"));
        assert!(
            !forked.contains("\"two\""),
            "turn 2 must be dropped: {forked}"
        );
        assert!(!forked.contains("a2"));
        // Every retained line is re-stamped to the new session id.
        assert!(
            !forked.contains("\"orig\""),
            "sessionId not rewritten: {forked}"
        );
        assert!(forked.contains("\"fork\""));

        // Replaying the fork yields exactly turn 1 (user + assistant).
        let events = load_history(&new_path);
        assert!(matches!(&events[0], TranscriptEvent::UserMessage(m) if m == "one"));
        assert!(events
            .iter()
            .all(|e| !matches!(e, TranscriptEvent::UserMessage(m) if m == "two")));

        fork_interrupt_marker_case(&dir, cwd);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// An interrupted turn stores "[Request interrupted by user]" as a `user`
    /// line. It must not consume a kept turn — the panel never counts it, and
    /// doing so here cut the fork a whole turn early (the reported "forks at
    /// my message, not the reply" bug). Runs inside
    /// [`fork_truncates_at_user_turn_and_rewrites_session_id`] because both
    /// scenarios mutate the process-global `HOME` and would race in parallel.
    fn fork_interrupt_marker_case(dir: &Path, cwd: &Path) {
        let parent = dir.join("orig-interrupt.jsonl");
        std::fs::write(
            &parent,
            concat!(
                r#"{"type":"user","sessionId":"orig","message":{"role":"user","content":"one"}}"#,
                "\n",
                r#"{"type":"user","sessionId":"orig","message":{"role":"user","content":"[Request interrupted by user]"}}"#,
                "\n",
                r#"{"type":"user","sessionId":"orig","message":{"role":"user","content":"two"}}"#,
                "\n",
                r#"{"type":"assistant","sessionId":"orig","message":{"role":"assistant","content":[{"type":"text","text":"a2"}]}}"#,
                "\n",
            ),
        )
        .unwrap();

        // Keep both genuine turns: "two" and its reply must survive the cut.
        let new_path = fork_session_file(&parent, "fork-interrupt", 2, cwd).unwrap();
        let forked = std::fs::read_to_string(&new_path).unwrap();
        assert!(forked.contains("\"two\""), "turn 2 dropped: {forked}");
        assert!(forked.contains("a2"), "turn 2 reply dropped: {forked}");
    }

    #[test]
    fn load_history_missing_file_is_empty() {
        let path = std::env::temp_dir().join("twarp-test-claude-history-missing.jsonl");
        let _ = std::fs::remove_file(&path);
        assert!(load_history(&path).is_empty());
    }
}
