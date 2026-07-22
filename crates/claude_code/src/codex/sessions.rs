//! Reading the on-disk rollout store `codex` itself maintains under
//! `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-<ts>-<uuid>.jsonl`.
//!
//! The Codex counterpart of [`crate::sessions`], with one structural
//! difference: there is no per-cwd directory. Every session lives in one
//! dated tree and records its owning cwd in the file's first `session_meta`
//! line, so listing scans each file's head and filters on that cwd.
//!
//! Resume never reads these files: history is replayed by the app-server's
//! `thread/resume` response (18b), so a listed session only needs its id,
//! title, and timestamp — the rollout path is carried for diagnostics.
//! Reads are best-effort like the Claude store's: a corrupt or unfamiliar
//! file is silently skipped.

use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde_json::Value;

use crate::driver::AgentProvider;
use crate::sessions::StoredSession;

/// How many rollout lines to scan for the first genuine user message. The
/// preamble (permissions/AGENTS.md instructions, turn context) is a handful of
/// lines; a session whose head has no `user_message` within this window is
/// treated as never-used and skipped.
const TITLE_SCAN_LINES: usize = 200;

/// The root of codex's rollout store. `None` when `HOME` is not set.
pub fn sessions_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".codex").join("sessions"))
}

/// Whether any stored codex session exists for this cwd. Backed by the same
/// per-cwd cache as [`list_sessions`], so the probes running in UI event
/// handlers (sidebar visibility on every directory change / group switch)
/// cost one metadata-only tree walk unless the store actually changed.
pub fn has_sessions(cwd: &Path) -> bool {
    !list_sessions(cwd).is_empty()
}

/// Per-cwd listing cache. Unlike Claude's store the rollout tree is not
/// cwd-partitioned, so a listing must open every file's head — too much for
/// the sync UI paths that re-probe on every directory change. The cache key
/// is a stamp over the tree's directory+file mtimes (metadata only, no file
/// opens): unchanged stamp → cached result; head parsing happens only when a
/// rollout file appears, grows, or disappears.
type SessionsCache = Mutex<HashMap<PathBuf, (SystemTime, Vec<StoredSession>)>>;
fn cache() -> &'static SessionsCache {
    static CACHE: OnceLock<SessionsCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// List stored codex sessions for a given cwd, most-recent first. Sessions
/// with no user turn (codex writes a rollout file on every launch, used or
/// not) are skipped — they resume to an empty thread and only clutter the
/// list.
pub fn list_sessions(cwd: &Path) -> Vec<StoredSession> {
    let Some(root) = sessions_root() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut stamp = SystemTime::UNIX_EPOCH;
    collect_jsonl_files(&root, 0, &mut files, &mut stamp);

    let mut cache = cache().lock().expect("codex sessions cache poisoned");
    if let Some((cached_stamp, cached)) = cache.get(cwd) {
        if *cached_stamp == stamp {
            return cached.clone();
        }
    }

    let mut sessions = Vec::new();
    for path in files {
        let Some(session) = read_session_head(&path, cwd) else {
            continue;
        };
        sessions.push(session);
    }
    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    cache.insert(cwd.to_owned(), (stamp, sessions.clone()));
    sessions
}

/// Recursively collect `.jsonl` files under `dir` (the store nests
/// year/month/day; the depth cap just guards against a pathological tree),
/// folding every directory and file mtime into `stamp` so callers can detect
/// "nothing changed" without opening a single file.
fn collect_jsonl_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, stamp: &mut SystemTime) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            *stamp = (*stamp).max(modified);
        }
        if path.is_dir() {
            collect_jsonl_files(&path, depth + 1, out, stamp);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// Parse only a rollout file's first `session_meta` line, returning the
/// session id when the file belongs to `cwd`. The one-line read both probes
/// (`has_sessions`) and the full listing share.
fn head_meta(path: &Path, cwd: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;
    let meta: Value = serde_json::from_str(&first_line).ok()?;
    if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = meta.get("payload")?;
    if payload.get("cwd").and_then(Value::as_str).map(Path::new) != Some(cwd) {
        return None;
    }
    payload.get("id").and_then(Value::as_str).map(str::to_owned)
}

/// Parse a rollout file's head: the `session_meta` first line gives the id
/// and owning cwd; the first `user_message` event gives the title. Returns
/// `None` when the file belongs to another cwd, has no meta line, or was
/// never actually used.
fn read_session_head(path: &Path, cwd: &Path) -> Option<StoredSession> {
    let id = head_meta(path, cwd)?;
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let lines = reader.lines().map_while(Result::ok).skip(1);

    let title = lines.take(TITLE_SCAN_LINES).find_map(|line| {
        let value: Value = serde_json::from_str(&line).ok()?;
        if value.get("type").and_then(Value::as_str) != Some("event_msg") {
            return None;
        }
        let payload = value.get("payload")?;
        if payload.get("type").and_then(Value::as_str) != Some("user_message") {
            return None;
        }
        let message = payload.get("message").and_then(Value::as_str)?;
        let trimmed = message.trim();
        (!trimmed.is_empty()).then(|| crate::sessions::short_title(trimmed))
    })?;

    let timestamp = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Some(StoredSession {
        id,
        title,
        timestamp,
        jsonl_path: path.to_owned(),
        provider: AgentProvider::Codex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_rollout(dir: &Path, name: &str, cwd: &str, with_user_message: bool) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut body = format!(
            "{{\"timestamp\":\"2026-07-01T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"{cwd}\"}}}}\n",
            name.trim_end_matches(".jsonl"),
        );
        if with_user_message {
            body.push_str(
                "{\"timestamp\":\"2026-07-01T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"fix the build\\nplease\"}}\n",
            );
        }
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn lists_only_matching_cwd_with_user_turns() {
        let root = std::env::temp_dir().join("twarp-test-codex-sessions");
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join("2026/07/01");
        write_rollout(&day, "match.jsonl", "/proj/a", true);
        write_rollout(&day, "other-cwd.jsonl", "/proj/b", true);
        write_rollout(&day, "unused.jsonl", "/proj/a", false);

        let mut files = Vec::new();
        let mut stamp = SystemTime::UNIX_EPOCH;
        collect_jsonl_files(&root, 0, &mut files, &mut stamp);
        assert_eq!(files.len(), 3);
        assert_ne!(stamp, SystemTime::UNIX_EPOCH);

        let sessions: Vec<_> = files
            .iter()
            .filter_map(|path| read_session_head(path, Path::new("/proj/a")))
            .collect();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "match");
        assert_eq!(sessions[0].title, "fix the build");
        assert_eq!(sessions[0].provider, AgentProvider::Codex);
        let _ = std::fs::remove_dir_all(&root);
    }
}
