//! twarp 26b: the live-session registry — a main-thread-published mirror of
//! every live agent pane that the MCP runtime reads without touching views.
//!
//! `ClaudeCodeView` publishes into it at the same points it repaints the tab
//! (every `TabStatusChanged` emission and every ingested transcript event),
//! so `list_sessions` status can never disagree with the tab UI (PRODUCT
//! P#4). Transcripts are a deliberately flat append-only projection (index,
//! role, text) — not the render model — so the seam survives transcript
//! churn and indices stay stable (P#8).

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use claude_code::TranscriptItem;
use serde::Serialize;

use super::status::SessionStatus;

/// One item of the flat transcript projection (PRODUCT P#6): a stable,
/// monotonically increasing `index`, a role/kind, and text content —
/// identical in shape for live and stored, Claude and Codex sessions (P#7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlatTranscriptItem {
    pub index: u64,
    pub role: &'static str,
    pub text: String,
}

/// A live session's registry row, minus its transcript.
#[derive(Clone, Debug)]
pub struct LiveSession {
    pub session_id: String,
    pub provider: &'static str,
    pub cwd: Option<PathBuf>,
    pub title: String,
    pub status: SessionStatus,
    pub last_activity: SystemTime,
}

struct SessionRecord {
    provider: &'static str,
    cwd: Option<PathBuf>,
    title: String,
    status: SessionStatus,
    last_activity: SystemTime,
    transcript: Vec<FlatTranscriptItem>,
}

/// Shared between the main thread (publisher) and the MCP runtime (reader).
/// A process-global like the codex sessions cache: every `ClaudeCodeView` and
/// every SSE service instance reaches the same map without wiring.
#[derive(Default)]
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, SessionRecord>>,
}

impl SessionRegistry {
    pub fn global() -> &'static SessionRegistry {
        static REGISTRY: OnceLock<SessionRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Default::default)
    }

    /// Upsert a live session's row. `projected` is the full flat projection
    /// of the session's transcript; the registry keeps its stored prefix and
    /// appends only the tail, so an item, once served, never changes (P#8).
    /// A projection shorter than the stored one means the pane restarted its
    /// conversation — replaced wholesale (the id normally changes with it).
    pub fn publish(
        &self,
        session_id: &str,
        provider: &'static str,
        cwd: Option<PathBuf>,
        title: String,
        status: SessionStatus,
        projected: Vec<FlatTranscriptItem>,
    ) {
        let mut sessions = self.sessions.lock().expect("session registry poisoned");
        let record = sessions
            .entry(session_id.to_owned())
            .or_insert_with(|| SessionRecord {
                provider,
                cwd: None,
                title: String::new(),
                status: SessionStatus::Idle,
                last_activity: SystemTime::now(),
                transcript: Vec::new(),
            });
        let activity = projected.len() != record.transcript.len() || status != record.status;
        if projected.len() < record.transcript.len() {
            record.transcript = projected;
        } else {
            record
                .transcript
                .extend(projected.into_iter().skip(record.transcript.len()));
        }
        record.provider = provider;
        record.cwd = cwd;
        record.title = title;
        record.status = status;
        if activity {
            record.last_activity = SystemTime::now();
        }
    }

    /// Drop a session's row (pane closed / view dropped, or the pane's id
    /// changed and the old key is stale).
    pub fn remove(&self, session_id: &str) {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .remove(session_id);
    }

    pub fn snapshot(&self) -> Vec<LiveSession> {
        self.sessions
            .lock()
            .expect("session registry poisoned")
            .iter()
            .map(|(session_id, record)| LiveSession {
                session_id: session_id.clone(),
                provider: record.provider,
                cwd: record.cwd.clone(),
                title: record.title.clone(),
                status: record.status,
                last_activity: record.last_activity,
            })
            .collect()
    }

    /// The live session's transcript items after `since_index` (all of them
    /// when `None`). `None` result = no live session with that id (the
    /// caller falls back to the stored-session stores).
    pub fn transcript_since(
        &self,
        session_id: &str,
        since_index: Option<u64>,
    ) -> Option<Vec<FlatTranscriptItem>> {
        let sessions = self.sessions.lock().expect("session registry poisoned");
        let record = sessions.get(session_id)?;
        Some(items_since(&record.transcript, since_index))
    }
}

/// Apply `since_index` (P#6: only items *after* that index; the latest index
/// yields an empty list).
pub fn items_since(
    items: &[FlatTranscriptItem],
    since_index: Option<u64>,
) -> Vec<FlatTranscriptItem> {
    match since_index {
        None => items.to_vec(),
        Some(since) => items
            .iter()
            .filter(|item| item.index > since)
            .cloned()
            .collect(),
    }
}

/// Project the render transcript into the flat MCP shape. Items still
/// streaming (an open assistant/thinking block) end the projection — they are
/// held back until final so a served index's content never changes (P#8);
/// tool items are included immediately because their projected text (name +
/// input) is fixed at creation. Todos and per-turn metrics are rendering
/// furniture, not conversation, and are skipped.
pub fn flatten_transcript(items: &[TranscriptItem]) -> Vec<FlatTranscriptItem> {
    let mut out = Vec::new();
    for item in items {
        let entry: Option<(&'static str, String)> = match item {
            TranscriptItem::User(text) => Some(("user", text.clone())),
            TranscriptItem::Assistant { text, done } => {
                if !done {
                    break;
                }
                Some(("assistant", text.clone()))
            }
            TranscriptItem::Thinking { text, done, .. } => {
                if !done {
                    break;
                }
                Some(("thinking", text.clone()))
            }
            TranscriptItem::Tool { name, input, .. } => Some(("tool", tool_text(name, input))),
            TranscriptItem::Permission { tool, .. } => Some(("permission", tool.clone())),
            TranscriptItem::Question { dialog_kind, .. } => Some(("question", dialog_kind.clone())),
            TranscriptItem::Notice(text) => Some(("notice", text.clone())),
            TranscriptItem::Error(text) => Some(("error", text.clone())),
            TranscriptItem::Todos(_) | TranscriptItem::Metrics(_) => None,
        };
        if let Some((role, text)) = entry {
            out.push(FlatTranscriptItem {
                index: out.len() as u64,
                role,
                text,
            });
        }
    }
    out
}

/// A tool item's one-line text: the tool name plus its input, capped so a
/// giant Write payload doesn't balloon the transcript. Stable at creation —
/// status/output changes never alter it, keeping the projection append-only.
fn tool_text(name: &str, input: &serde_json::Value) -> String {
    const MAX_CHARS: usize = 200;
    let text = format!("{name} {input}");
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let mut clipped: String = text.chars().take(MAX_CHARS).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(index: u64, role: &'static str, text: &str) -> FlatTranscriptItem {
        FlatTranscriptItem {
            index,
            role,
            text: text.to_owned(),
        }
    }

    /// P#8: publishing a longer projection appends without touching served
    /// prefixes, and indices never change across publishes.
    #[test]
    fn transcript_indices_are_stable_across_publishes() {
        let registry = SessionRegistry::default();
        registry.publish(
            "s1",
            "claude",
            None,
            "t".to_owned(),
            SessionStatus::Running,
            vec![flat(0, "user", "hello")],
        );
        registry.publish(
            "s1",
            "claude",
            None,
            "t".to_owned(),
            SessionStatus::DoneOk,
            vec![flat(0, "user", "hello"), flat(1, "assistant", "hi")],
        );
        let all = registry.transcript_since("s1", None).unwrap();
        assert_eq!(
            all,
            vec![flat(0, "user", "hello"), flat(1, "assistant", "hi")]
        );
        // Polling with since_index never misses nor duplicates (P#8): the
        // latest index yields an empty tail.
        assert_eq!(
            registry.transcript_since("s1", Some(0)).unwrap(),
            vec![flat(1, "assistant", "hi")]
        );
        assert!(registry.transcript_since("s1", Some(1)).unwrap().is_empty());
        assert!(registry.transcript_since("missing", None).is_none());
    }

    #[test]
    fn open_streaming_items_are_held_back_until_done() {
        let items = vec![
            TranscriptItem::User("q".to_owned()),
            TranscriptItem::Assistant {
                text: "partial".to_owned(),
                done: false,
            },
        ];
        assert_eq!(flatten_transcript(&items), vec![flat(0, "user", "q")]);

        let items = vec![
            TranscriptItem::User("q".to_owned()),
            TranscriptItem::Assistant {
                text: "full".to_owned(),
                done: true,
            },
        ];
        assert_eq!(
            flatten_transcript(&items),
            vec![flat(0, "user", "q"), flat(1, "assistant", "full")]
        );
    }

    #[test]
    fn tool_items_project_name_and_input_and_skip_furniture() {
        let items = vec![
            TranscriptItem::Tool {
                id: "t1".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({ "command": "ls" }),
                status: claude_code::ToolStatus::Running,
                output: None,
                children: Vec::new(),
            },
            TranscriptItem::Todos(Vec::new()),
        ];
        let projected = flatten_transcript(&items);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].role, "tool");
        assert!(projected[0].text.starts_with("Bash "));
        assert!(projected[0].text.contains("ls"));
    }
}
