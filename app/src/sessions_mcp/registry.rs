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
#[cfg(not(target_family = "wasm"))]
use tokio::sync::broadcast;

use super::{
    spawn::{SpawnOrigin, SpawnParent, SpawnRefusal, MAX_SPAWN_DEPTH},
    status::SessionStatus,
};

/// twarp 26c: one event on a live session's broadcast channel. Every watcher
/// (an SSE watch forwarder or a pending `wait_for_completion`) subscribes to
/// the same sender, so all of them receive every event (PRODUCT P#9).
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug)]
pub enum SessionEvent {
    /// A transcript item was appended (already final — the projection holds
    /// streaming items back, so an emitted item never changes).
    Item(FlatTranscriptItem),
    /// The session's status changed to this value.
    Status(SessionStatus),
    /// The pane closed / the view dropped. Terminal: no further events.
    Closed,
}

/// Watchers that fall this many events behind resync from the registry
/// snapshot (`RecvError::Lagged` handling in `events.rs`) instead of wedging.
#[cfg(not(target_family = "wasm"))]
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// A subscription to a live session's events plus the state it was taken
/// against — receiver, status, and transcript tail are captured under one
/// registry lock, so a subscriber can check "already terminal?" and then wait
/// without a gap a transition could slip through.
#[cfg(not(target_family = "wasm"))]
pub struct SessionSubscription {
    pub receiver: broadcast::Receiver<SessionEvent>,
    pub status: SessionStatus,
    pub last_index: Option<u64>,
    pub last_assistant_text: Option<String>,
}

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
    /// 26d: the spawn provenance, for sessions created via `create_chat`.
    pub origin: Option<SpawnOrigin>,
}

struct SessionRecord {
    provider: &'static str,
    cwd: Option<PathBuf>,
    title: String,
    status: SessionStatus,
    last_activity: SystemTime,
    transcript: Vec<FlatTranscriptItem>,
    #[cfg(not(target_family = "wasm"))]
    events: broadcast::Sender<SessionEvent>,
}

impl SessionRecord {
    fn last_assistant_text(&self) -> Option<String> {
        self.transcript
            .iter()
            .rev()
            .find(|item| item.role == "assistant")
            .map(|item| item.text.clone())
    }
}

/// Shared between the main thread (publisher) and the MCP runtime (reader).
/// A process-global like the codex sessions cache: every `ClaudeCodeView` and
/// every SSE service instance reaches the same map without wiring.
#[derive(Default)]
pub struct SessionRegistry {
    inner: Mutex<RegistryInner>,
}

/// Everything spawn validation must see atomically lives under one lock
/// (PRODUCT P#28: racing `create_chat` calls can never exceed the cap).
#[derive(Default)]
struct RegistryInner {
    sessions: HashMap<String, SessionRecord>,
    /// 26d: spawn provenance per created session. An entry with no matching
    /// `sessions` row is a just-reserved spawn whose pane hasn't published
    /// yet — it counts as running for the cap.
    spawn_origins: HashMap<String, SpawnOrigin>,
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
        origin: Option<&SpawnOrigin>,
    ) {
        let mut inner = self.inner.lock().expect("session registry poisoned");
        // 26d: keep the provenance row in lockstep with the pane's view —
        // survives a session re-key because the view republishes it.
        if let Some(origin) = origin {
            inner
                .spawn_origins
                .insert(session_id.to_owned(), origin.clone());
        }
        let record = inner
            .sessions
            .entry(session_id.to_owned())
            .or_insert_with(|| SessionRecord {
                provider,
                cwd: None,
                title: String::new(),
                status: SessionStatus::Idle,
                last_activity: SystemTime::now(),
                transcript: Vec::new(),
                #[cfg(not(target_family = "wasm"))]
                events: broadcast::channel(EVENT_CHANNEL_CAPACITY).0,
            });
        let status_changed = status != record.status;
        let activity = projected.len() != record.transcript.len() || status_changed;
        if projected.len() < record.transcript.len() {
            // Wholesale replace (restarted conversation): indices reset, so
            // there is no meaningful append event to broadcast — watchers of
            // a restarted pane resync via `transcript_since` on next lag or
            // read.
            record.transcript = projected;
        } else {
            let appended = &projected[record.transcript.len()..];
            #[cfg(not(target_family = "wasm"))]
            for item in appended {
                let _ = record.events.send(SessionEvent::Item(item.clone()));
            }
            record.transcript.extend_from_slice(appended);
        }
        record.provider = provider;
        record.cwd = cwd;
        record.title = title;
        record.status = status;
        #[cfg(not(target_family = "wasm"))]
        if status_changed {
            let _ = record.events.send(SessionEvent::Status(status));
        }
        if activity {
            record.last_activity = SystemTime::now();
        }
    }

    /// Drop a session's row (pane closed / view dropped, or the pane's id
    /// changed and the old key is stale).
    pub fn remove(&self, session_id: &str) {
        let removed = {
            let mut inner = self.inner.lock().expect("session registry poisoned");
            // 26d: a closed spawned pane stops counting against the cap.
            inner.spawn_origins.remove(session_id);
            inner.sessions.remove(session_id)
        };
        // 26c: watchers get a terminal `closed` event so waits resolve
        // instead of hanging (PRODUCT P#12, P#29). Dropping the sender after
        // this also closes every receiver, which watchers treat the same.
        #[cfg(not(target_family = "wasm"))]
        if let Some(record) = removed {
            let _ = record.events.send(SessionEvent::Closed);
        }
        #[cfg(target_family = "wasm")]
        drop(removed);
    }

    pub fn snapshot(&self) -> Vec<LiveSession> {
        let inner = self.inner.lock().expect("session registry poisoned");
        inner
            .sessions
            .iter()
            .map(|(session_id, record)| LiveSession {
                session_id: session_id.clone(),
                provider: record.provider,
                cwd: record.cwd.clone(),
                title: record.title.clone(),
                status: record.status,
                last_activity: record.last_activity,
                origin: inner.spawn_origins.get(session_id).cloned(),
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
        let inner = self.inner.lock().expect("session registry poisoned");
        let record = inner.sessions.get(session_id)?;
        Some(items_since(&record.transcript, since_index))
    }

    /// Subscribe to a live session's events (26c). `None` = no live session
    /// with that id. Receiver and current state are captured atomically so
    /// callers can check the pre-subscription status without a race window.
    #[cfg(not(target_family = "wasm"))]
    pub fn subscribe(&self, session_id: &str) -> Option<SessionSubscription> {
        let inner = self.inner.lock().expect("session registry poisoned");
        let record = inner.sessions.get(session_id)?;
        Some(SessionSubscription {
            receiver: record.events.subscribe(),
            status: record.status,
            last_index: record.transcript.last().map(|item| item.index),
            last_assistant_text: record.last_assistant_text(),
        })
    }

    /// Subscribe to every live session at once (26c: `wait_for_completion`
    /// on `"any"`). Sessions created after this call are not included.
    #[cfg(not(target_family = "wasm"))]
    pub fn subscribe_all(&self) -> Vec<(String, SessionSubscription)> {
        let inner = self.inner.lock().expect("session registry poisoned");
        inner
            .sessions
            .iter()
            .map(|(session_id, record)| {
                (
                    session_id.clone(),
                    SessionSubscription {
                        receiver: record.events.subscribe(),
                        status: record.status,
                        last_index: record.transcript.last().map(|item| item.index),
                        last_assistant_text: record.last_assistant_text(),
                    },
                )
            })
            .collect()
    }

    /// The live session's current status (`None` = closed / never existed).
    /// Used by lagged watchers to resync (26c).
    pub fn status_of(&self, session_id: &str) -> Option<SessionStatus> {
        self.inner
            .lock()
            .expect("session registry poisoned")
            .sessions
            .get(session_id)
            .map(|record| record.status)
    }

    /// The live session's latest final assistant message text, if any.
    pub fn last_assistant_text(&self, session_id: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("session registry poisoned")
            .sessions
            .get(session_id)
            .and_then(SessionRecord::last_assistant_text)
    }

    /// 26d: atomically validate and reserve a `create_chat` spawn (PRODUCT
    /// P#23-24, 28). Cap and depth are checked and the origin recorded under
    /// one lock, so N racing calls admit at most `cap` spawns; the entry
    /// itself is the reservation (a reserved id with no published session row
    /// counts as running until the pane publishes). On refusal nothing is
    /// recorded; a failed spawn after reservation must call [`Self::remove`]
    /// to release its slot.
    pub fn try_reserve_spawn(
        &self,
        session_id: &str,
        parent: &SpawnParent,
        cap: usize,
    ) -> Result<SpawnOrigin, SpawnRefusal> {
        let mut inner = self.inner.lock().expect("session registry poisoned");
        let depth = match parent {
            // In-pane caller: the parent's own recorded depth (0 for a
            // user-created pane) plus one.
            SpawnParent::InPane { session_id, .. } => inner
                .spawn_origins
                .get(session_id)
                .map_or(0, |origin| origin.depth)
                .saturating_add(1),
            SpawnParent::External { .. } => 1,
        };
        if depth >= MAX_SPAWN_DEPTH {
            return Err(SpawnRefusal::DepthExceeded {
                depth,
                max: MAX_SPAWN_DEPTH,
            });
        }
        let running_spawned = inner
            .spawn_origins
            .keys()
            .filter(|id| {
                match inner.sessions.get(*id) {
                    // Published: counts while the session is actually running.
                    Some(record) => record.status == SessionStatus::Running,
                    // Reserved but not yet published: about to run.
                    None => true,
                }
            })
            .count();
        if running_spawned >= cap {
            return Err(SpawnRefusal::AtCapacity { limit: cap });
        }
        let origin = SpawnOrigin {
            parent_session_id: match parent {
                SpawnParent::InPane { session_id, .. } => Some(session_id.clone()),
                SpawnParent::External { .. } => None,
            },
            parent_label: match parent {
                SpawnParent::InPane { title, .. } => title.clone(),
                SpawnParent::External { label } => label.clone(),
            },
            depth,
        };
        inner
            .spawn_origins
            .insert(session_id.to_owned(), origin.clone());
        Ok(origin)
    }

    /// The recorded spawn origin for a live/reserved session, if any.
    pub fn spawn_origin(&self, session_id: &str) -> Option<SpawnOrigin> {
        self.inner
            .lock()
            .expect("session registry poisoned")
            .spawn_origins
            .get(session_id)
            .cloned()
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
            None,
        );
        registry.publish(
            "s1",
            "claude",
            None,
            "t".to_owned(),
            SessionStatus::DoneOk,
            vec![flat(0, "user", "hello"), flat(1, "assistant", "hi")],
            None,
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

    /// P#28: N concurrent spawn reservations racing a cap of 4 admit exactly
    /// 4 — never 5 — because count + insert happen under one lock.
    #[test]
    fn concurrent_spawn_reservations_never_exceed_cap() {
        let registry = std::sync::Arc::new(SessionRegistry::default());
        let admitted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let registry = registry.clone();
                let admitted = admitted.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = registry.try_reserve_spawn(
                        &format!("spawn-{i}"),
                        &SpawnParent::External {
                            label: "test".to_owned(),
                        },
                        4,
                    );
                    match result {
                        Ok(_) => {
                            admitted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        Err(SpawnRefusal::AtCapacity { limit }) => assert_eq!(limit, 4),
                        Err(other) => panic!("unexpected refusal: {other:?}"),
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(admitted.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    /// A spawned session that finished (or a closed one) frees its cap slot;
    /// a still-reserved (unpublished) one does not.
    #[test]
    fn cap_counts_only_running_or_reserved_spawns() {
        let registry = SessionRegistry::default();
        let external = SpawnParent::External {
            label: "t".to_owned(),
        };
        for i in 0..4 {
            registry
                .try_reserve_spawn(&format!("s{i}"), &external, 4)
                .unwrap();
        }
        assert!(matches!(
            registry.try_reserve_spawn("s4", &external, 4),
            Err(SpawnRefusal::AtCapacity { limit: 4 })
        ));
        // s0 publishes as done: it no longer counts as running.
        registry.publish(
            "s0",
            "claude",
            None,
            "t".to_owned(),
            SessionStatus::DoneOk,
            Vec::new(),
            None,
        );
        registry.try_reserve_spawn("s4", &external, 4).unwrap();
        // s1 closes entirely: another slot opens.
        registry.remove("s1");
        registry.try_reserve_spawn("s5", &external, 4).unwrap();
        assert!(matches!(
            registry.try_reserve_spawn("s6", &external, 4),
            Err(SpawnRefusal::AtCapacity { .. })
        ));
    }

    /// P#24: depth = parent depth + 1; the chain refuses at MAX_SPAWN_DEPTH
    /// with a distinct depth-exceeded error even when the cap has room.
    #[test]
    fn spawn_depth_chain_refuses_at_max() {
        let registry = SessionRegistry::default();
        let depth1 = registry
            .try_reserve_spawn(
                "a",
                &SpawnParent::InPane {
                    session_id: "user-pane".to_owned(),
                    title: "root".to_owned(),
                },
                8,
            )
            .unwrap();
        assert_eq!(depth1.depth, 1);
        let depth2 = registry
            .try_reserve_spawn(
                "b",
                &SpawnParent::InPane {
                    session_id: "a".to_owned(),
                    title: "a".to_owned(),
                },
                8,
            )
            .unwrap();
        assert_eq!(depth2.depth, 2);
        assert_eq!(
            registry.try_reserve_spawn(
                "c",
                &SpawnParent::InPane {
                    session_id: "b".to_owned(),
                    title: "b".to_owned(),
                },
                8,
            ),
            Err(SpawnRefusal::DepthExceeded {
                depth: 3,
                max: MAX_SPAWN_DEPTH
            })
        );
    }
}
