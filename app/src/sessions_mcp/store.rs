//! twarp 26b: past-session access for the sessions MCP tools — both
//! providers' on-disk stores mapped into the same flat transcript shape live
//! sessions serve (PRODUCT P#7).

use std::path::{Path, PathBuf};

use claude_code::{
    codex,
    driver::AgentProvider,
    sessions::{self, StoredSession},
    Transcript,
};
use serde_json::Value;

use super::registry::{flatten_transcript, FlatTranscriptItem};

/// One stored session plus the cwd its store records (Claude reads it from
/// the session's own lines, best-effort; Codex's `session_meta` always
/// carries it).
pub struct PastSession {
    pub session: StoredSession,
    pub cwd: Option<PathBuf>,
}

/// Every stored session from both providers' stores, most-recent first.
/// Blocking filesystem work — call off the main thread.
pub fn all_past_sessions() -> Vec<PastSession> {
    let mut out: Vec<PastSession> = sessions::list_all_claude_sessions()
        .into_iter()
        .map(|(session, cwd)| PastSession { session, cwd })
        .collect();
    out.extend(
        codex::sessions::list_all_sessions()
            .into_iter()
            .map(|(session, cwd)| PastSession {
                session,
                cwd: Some(cwd),
            }),
    );
    out.sort_by(|a, b| b.session.timestamp.cmp(&a.session.timestamp));
    out
}

/// Locate a stored session by id across both stores (P#7: `get_transcript`
/// serves past sessions of either provider).
pub fn find_stored_session(session_id: &str) -> Option<PastSession> {
    all_past_sessions()
        .into_iter()
        .find(|past| past.session.id == session_id)
}

/// A stored session's transcript in the live projection's exact shape (P#7).
/// Claude replays the jsonl through the driver's history parser into a
/// `Transcript` and flattens it — the same pipeline a resumed pane uses;
/// Codex maps the rollout file's `event_msg` lines directly (there is no
/// replay parser for rollouts — resume gets history from `thread/resume`).
pub fn stored_transcript(session: &StoredSession) -> Vec<FlatTranscriptItem> {
    match session.provider {
        AgentProvider::Claude => {
            let mut transcript = Transcript::new();
            for event in sessions::load_history(&session.jsonl_path) {
                transcript.apply(event);
            }
            flatten_transcript(transcript.items())
        }
        AgentProvider::Codex => codex_rollout_transcript(&session.jsonl_path),
    }
}

/// Flat-project a codex rollout file: `event_msg` payloads map onto the same
/// roles the live projection uses. Best-effort like every store read —
/// unfamiliar lines are skipped.
fn codex_rollout_transcript(path: &Path) -> Vec<FlatTranscriptItem> {
    use std::io::BufRead as _;

    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let reader = std::io::BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let role = match payload.get("type").and_then(Value::as_str) {
            Some("user_message") => "user",
            Some("agent_message") => "assistant",
            Some("agent_reasoning") => "thinking",
            _ => continue,
        };
        let Some(text) = payload
            .get("message")
            .or_else(|| payload.get("text"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        out.push(FlatTranscriptItem {
            index: out.len() as u64,
            role,
            text: text.to_owned(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn stored(provider: AgentProvider, path: PathBuf) -> StoredSession {
        StoredSession {
            id: "test".to_owned(),
            title: "test".to_owned(),
            timestamp: SystemTime::UNIX_EPOCH,
            jsonl_path: path,
            provider,
        }
    }

    /// P#7: a Claude jsonl fixture and a Codex rollout fixture recording the
    /// same conversation project into identical `FlatTranscriptItem`s.
    #[test]
    fn both_stores_map_to_the_same_shape() {
        let dir = std::env::temp_dir().join("twarp-test-sessions-mcp-store");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let claude_path = dir.join("claude.jsonl");
        std::fs::write(
            &claude_path,
            concat!(
                r#"{"type":"user","message":{"role":"user","content":"fix the build"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}"#,
                "\n",
            ),
        )
        .unwrap();

        let codex_path = dir.join("rollout.jsonl");
        std::fs::write(
            &codex_path,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"test","cwd":"/proj/a"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"fix the build"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"done"}}"#,
                "\n",
                r#"{"type":"turn_context","payload":{}}"#,
                "\n",
            ),
        )
        .unwrap();

        let claude_items = stored_transcript(&stored(AgentProvider::Claude, claude_path));
        let codex_items = stored_transcript(&stored(AgentProvider::Codex, codex_path));
        assert_eq!(claude_items, codex_items, "P#7: identical shape");
        assert_eq!(claude_items.len(), 2);
        assert_eq!(claude_items[0].index, 0);
        assert_eq!(claude_items[0].role, "user");
        assert_eq!(claude_items[0].text, "fix the build");
        assert_eq!(claude_items[1].index, 1);
        assert_eq!(claude_items[1].role, "assistant");
        assert_eq!(claude_items[1].text, "done");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_reasoning_maps_to_thinking() {
        let dir = std::env::temp_dir().join("twarp-test-sessions-mcp-reasoning");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"event_msg","payload":{"type":"agent_reasoning","text":"pondering"}}"#,
                "\n",
                "garbage line\n",
            ),
        )
        .unwrap();
        let items = codex_rollout_transcript(&path);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].role, "thinking");
        assert_eq!(items[0].text, "pondering");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
