//! twarp 26c: the event plumbing behind `watch_session` and
//! `wait_for_completion` — broadcast receivers off the registry, turned into
//! either a stream of watch notifications or a single wait outcome. Kept
//! transport-free (channels in, values out) so the semantics are unit-testable
//! without an SSE connection; `bridge.rs` owns the MCP framing.

use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast::error::RecvError, mpsc};

use super::{
    registry::{SessionEvent, SessionRegistry, SessionSubscription},
    status::SessionStatus,
};

/// One notification a `watch_session` forwarder emits — already the JSON
/// payload the bridge sends as an MCP `notifications/message` (PRODUCT P#9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchNotification(pub serde_json::Value);

/// How a `wait_for_completion` resolved (PRODUCT P#10–12). Timeout is a
/// distinct outcome, not an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    Completed {
        session_id: String,
        status: SessionStatus,
        /// `Some("closed")` when the resolution was the pane closing (P#12).
        reason: Option<&'static str>,
        last_assistant_text: Option<String>,
    },
    TimedOut,
}

/// Forward a watched session's events into `sink` until the session closes
/// or the sink's receiver is dropped (= the SSE connection went away).
///
/// A lagged receiver (`RecvError::Lagged`) must not wedge the watcher: it
/// resyncs from the registry snapshot — every transcript item after the last
/// one it delivered, plus the current status — and resumes.
pub async fn forward_watch_events(
    registry: &'static SessionRegistry,
    session_id: String,
    mut subscription: SessionSubscription,
    sink: mpsc::Sender<WatchNotification>,
) {
    let mut last_index = subscription.last_index;
    loop {
        match subscription.receiver.recv().await {
            Ok(SessionEvent::Item(item)) => {
                // A resync may have already delivered this item from the
                // snapshot; never emit an index twice.
                if last_index.is_some_and(|last| item.index <= last) {
                    continue;
                }
                last_index = Some(item.index);
                if send_item(&sink, &session_id, &item).await.is_err() {
                    return;
                }
            }
            Ok(SessionEvent::Status(status)) => {
                if send_status(&sink, &session_id, status).await.is_err() {
                    return;
                }
            }
            Ok(SessionEvent::Closed) | Err(RecvError::Closed) => {
                let _ = send_closed(&sink, &session_id).await;
                return;
            }
            Err(RecvError::Lagged(_)) => {
                // Jump the receiver to the live tail first, so the stale
                // backlog is never replayed; the snapshot below covers the
                // gap, and the index guard above filters any overlap.
                subscription.receiver = subscription.receiver.resubscribe();
                let Some(items) = registry.transcript_since(&session_id, last_index) else {
                    // The session vanished while we were behind: terminal.
                    let _ = send_closed(&sink, &session_id).await;
                    return;
                };
                for item in items {
                    last_index = Some(item.index);
                    if send_item(&sink, &session_id, &item).await.is_err() {
                        return;
                    }
                }
                match registry.status_of(&session_id) {
                    Some(status) => {
                        if send_status(&sink, &session_id, status).await.is_err() {
                            return;
                        }
                    }
                    None => {
                        let _ = send_closed(&sink, &session_id).await;
                        return;
                    }
                }
            }
        }
    }
}

async fn send_item(
    sink: &mpsc::Sender<WatchNotification>,
    session_id: &str,
    item: &super::registry::FlatTranscriptItem,
) -> Result<(), mpsc::error::SendError<WatchNotification>> {
    sink.send(WatchNotification(json!({
        "type": "session_event",
        "event": "item",
        "session_id": session_id,
        "item": item,
    })))
    .await
}

async fn send_status(
    sink: &mpsc::Sender<WatchNotification>,
    session_id: &str,
    status: SessionStatus,
) -> Result<(), mpsc::error::SendError<WatchNotification>> {
    sink.send(WatchNotification(json!({
        "type": "session_event",
        "event": "status",
        "session_id": session_id,
        "status": status.as_str(),
    })))
    .await
}

async fn send_closed(
    sink: &mpsc::Sender<WatchNotification>,
    session_id: &str,
) -> Result<(), mpsc::error::SendError<WatchNotification>> {
    sink.send(WatchNotification(json!({
        "type": "session_event",
        "event": "closed",
        "session_id": session_id,
    })))
    .await
}

/// Block until one of `targets` resolves the wait (status flips to a
/// `resolves_wait` state, or the session closes), or `timeout` elapses.
///
/// Subscriptions were captured atomically against the registry, so a target
/// already in a resolving state returns immediately — and because the
/// registry only flips to `done_*` when the view fires its own (deferred)
/// completion, "still running background scripts" never resolves early
/// (PRODUCT P#11). Nothing here can hang past `timeout` (P#29).
pub async fn wait_for_completion(
    registry: &'static SessionRegistry,
    targets: Vec<(String, SessionSubscription)>,
    timeout: Duration,
) -> WaitOutcome {
    // Already resolved at subscription time?
    for (session_id, subscription) in &targets {
        if subscription.status.resolves_wait() {
            return WaitOutcome::Completed {
                session_id: session_id.clone(),
                status: subscription.status,
                reason: None,
                last_assistant_text: subscription.last_assistant_text.clone(),
            };
        }
    }

    // Fan every target's events into one channel; each task sends exactly one
    // resolution then exits.
    let (tx, mut rx) = mpsc::channel::<WaitOutcome>(targets.len().max(1));
    for (session_id, subscription) in targets {
        let tx = tx.clone();
        tokio::spawn(watch_one_for_completion(
            registry,
            session_id,
            subscription,
            tx,
        ));
    }
    drop(tx);

    tokio::select! {
        outcome = rx.recv() => match outcome {
            Some(outcome) => outcome,
            // Unreachable (every task sends exactly one outcome before
            // exiting), but never hang on it (P#29).
            None => WaitOutcome::TimedOut,
        },
        _ = tokio::time::sleep(timeout) => WaitOutcome::TimedOut,
    }
}

/// Watch one session's events until they resolve the wait, then report.
async fn watch_one_for_completion(
    registry: &'static SessionRegistry,
    session_id: String,
    mut subscription: SessionSubscription,
    tx: mpsc::Sender<WaitOutcome>,
) {
    let mut last_assistant_text = subscription.last_assistant_text.take();
    let outcome = loop {
        match subscription.receiver.recv().await {
            Ok(SessionEvent::Item(item)) => {
                if item.role == "assistant" {
                    last_assistant_text = Some(item.text);
                }
            }
            Ok(SessionEvent::Status(status)) if status.resolves_wait() => {
                break WaitOutcome::Completed {
                    session_id,
                    status,
                    reason: None,
                    last_assistant_text,
                };
            }
            Ok(SessionEvent::Status(_)) => {}
            // Pane closed / process exited: resolve as done_error(closed)
            // rather than hanging (PRODUCT P#12).
            Ok(SessionEvent::Closed) | Err(RecvError::Closed) => {
                break WaitOutcome::Completed {
                    session_id,
                    status: SessionStatus::DoneError,
                    reason: Some("closed"),
                    last_assistant_text,
                };
            }
            Err(RecvError::Lagged(_)) => {
                // Resync from the registry instead of wedging: skip the stale
                // backlog (a replayed old done-status must not resolve a wait
                // on a session that has since resumed), refresh the last
                // assistant text, and re-check the current status.
                subscription.receiver = subscription.receiver.resubscribe();
                if let Some(text) = registry.last_assistant_text(&session_id) {
                    last_assistant_text = Some(text);
                }
                match registry.status_of(&session_id) {
                    Some(status) if status.resolves_wait() => {
                        break WaitOutcome::Completed {
                            session_id,
                            status,
                            reason: None,
                            last_assistant_text,
                        };
                    }
                    Some(_) => {}
                    None => {
                        break WaitOutcome::Completed {
                            session_id,
                            status: SessionStatus::DoneError,
                            reason: Some("closed"),
                            last_assistant_text,
                        };
                    }
                }
            }
        }
    };
    let _ = tx.send(outcome).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions_mcp::registry::FlatTranscriptItem;

    fn leaked_registry() -> &'static SessionRegistry {
        Box::leak(Box::new(SessionRegistry::default()))
    }

    fn flat(index: u64, role: &'static str, text: &str) -> FlatTranscriptItem {
        FlatTranscriptItem {
            index,
            role,
            text: text.to_owned(),
        }
    }

    fn publish(
        registry: &SessionRegistry,
        id: &str,
        status: SessionStatus,
        items: Vec<FlatTranscriptItem>,
    ) {
        registry.publish(id, "claude", None, "t".to_owned(), status, items, None);
    }

    /// P#10/#11: the wait resolves exactly when the registry status flips to
    /// a done state — which the view only publishes once deferred completion
    /// (background scripts/agents) has fired — and carries the last
    /// assistant message.
    #[tokio::test]
    async fn wait_resolves_on_status_flip_with_last_assistant_text() {
        let registry = leaked_registry();
        publish(registry, "s1", SessionStatus::Running, vec![]);
        let subscription = registry.subscribe("s1").unwrap();

        let wait = tokio::spawn(wait_for_completion(
            registry,
            vec![("s1".to_owned(), subscription)],
            Duration::from_secs(5),
        ));
        // Still running (deferred completion not fired yet): no resolution.
        publish(
            registry,
            "s1",
            SessionStatus::Running,
            vec![flat(0, "user", "q"), flat(1, "assistant", "answer")],
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!wait.is_finished());
        publish(
            registry,
            "s1",
            SessionStatus::DoneOk,
            vec![flat(0, "user", "q"), flat(1, "assistant", "answer")],
        );
        assert_eq!(
            wait.await.unwrap(),
            WaitOutcome::Completed {
                session_id: "s1".to_owned(),
                status: SessionStatus::DoneOk,
                reason: None,
                last_assistant_text: Some("answer".to_owned()),
            }
        );
    }

    /// P#10: a flip to needs_input resolves the wait too.
    #[tokio::test]
    async fn wait_resolves_on_needs_input() {
        let registry = leaked_registry();
        publish(registry, "s1", SessionStatus::Running, vec![]);
        let subscription = registry.subscribe("s1").unwrap();
        let wait = tokio::spawn(wait_for_completion(
            registry,
            vec![("s1".to_owned(), subscription)],
            Duration::from_secs(5),
        ));
        publish(registry, "s1", SessionStatus::NeedsInput, vec![]);
        assert!(matches!(
            wait.await.unwrap(),
            WaitOutcome::Completed {
                status: SessionStatus::NeedsInput,
                reason: None,
                ..
            }
        ));
    }

    /// A target already in a resolving state returns immediately.
    #[tokio::test]
    async fn wait_returns_immediately_when_already_done() {
        let registry = leaked_registry();
        publish(
            registry,
            "s1",
            SessionStatus::DoneOk,
            vec![flat(0, "assistant", "done")],
        );
        let subscription = registry.subscribe("s1").unwrap();
        let outcome = wait_for_completion(
            registry,
            vec![("s1".to_owned(), subscription)],
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(
            outcome,
            WaitOutcome::Completed {
                session_id: "s1".to_owned(),
                status: SessionStatus::DoneOk,
                reason: None,
                last_assistant_text: Some("done".to_owned()),
            }
        );
    }

    /// P#10: timeout is a distinct outcome, not an error.
    #[tokio::test]
    async fn wait_times_out_distinctly() {
        let registry = leaked_registry();
        publish(registry, "s1", SessionStatus::Running, vec![]);
        let subscription = registry.subscribe("s1").unwrap();
        let outcome = wait_for_completion(
            registry,
            vec![("s1".to_owned(), subscription)],
            Duration::from_millis(20),
        )
        .await;
        assert_eq!(outcome, WaitOutcome::TimedOut);
    }

    /// P#12/#29: closing the pane (registry removal) resolves the wait as
    /// done_error(reason: closed) instead of hanging.
    #[tokio::test]
    async fn wait_resolves_closed_on_removal() {
        let registry = leaked_registry();
        publish(
            registry,
            "s1",
            SessionStatus::Running,
            vec![flat(0, "assistant", "partial")],
        );
        let subscription = registry.subscribe("s1").unwrap();
        let wait = tokio::spawn(wait_for_completion(
            registry,
            vec![("s1".to_owned(), subscription)],
            Duration::from_secs(5),
        ));
        registry.remove("s1");
        assert_eq!(
            wait.await.unwrap(),
            WaitOutcome::Completed {
                session_id: "s1".to_owned(),
                status: SessionStatus::DoneError,
                reason: Some("closed"),
                last_assistant_text: Some("partial".to_owned()),
            }
        );
    }

    /// P#9: multiple watchers each get every event; the forwarder emits
    /// items, statuses, and a terminal closed notification.
    #[tokio::test]
    async fn watch_forwards_items_status_and_closed_to_every_watcher() {
        let registry = leaked_registry();
        publish(registry, "s1", SessionStatus::Running, vec![]);
        let mut sinks = Vec::new();
        for _ in 0..2 {
            let subscription = registry.subscribe("s1").unwrap();
            let (tx, rx) = mpsc::channel(16);
            tokio::spawn(forward_watch_events(
                registry,
                "s1".to_owned(),
                subscription,
                tx,
            ));
            sinks.push(rx);
        }
        publish(
            registry,
            "s1",
            SessionStatus::DoneOk,
            vec![flat(0, "assistant", "hi")],
        );
        registry.remove("s1");
        for rx in &mut sinks {
            let mut events = Vec::new();
            while let Some(WatchNotification(value)) = rx.recv().await {
                events.push(value);
            }
            assert_eq!(events.len(), 3, "item + status + closed: {events:?}");
            assert_eq!(events[0]["event"], "item");
            assert_eq!(events[0]["item"]["text"], "hi");
            assert_eq!(events[1]["event"], "status");
            assert_eq!(events[1]["status"], "done_ok");
            assert_eq!(events[2]["event"], "closed");
        }
    }

    /// A lagged watcher resyncs from the registry snapshot instead of
    /// wedging: it delivers every missed item exactly once, then the current
    /// status.
    #[tokio::test]
    async fn lagged_watcher_resyncs_from_snapshot() {
        let registry = leaked_registry();
        publish(registry, "s1", SessionStatus::Running, vec![]);
        let subscription = registry.subscribe("s1").unwrap();

        // Overflow the broadcast buffer before the forwarder ever polls.
        let mut items = Vec::new();
        for index in 0..600u64 {
            items.push(flat(index, "user", &format!("m{index}")));
            publish(registry, "s1", SessionStatus::Running, items.clone());
        }
        publish(registry, "s1", SessionStatus::DoneOk, items.clone());

        let (tx, mut rx) = mpsc::channel(2048);
        tokio::spawn(forward_watch_events(
            registry,
            "s1".to_owned(),
            subscription,
            tx,
        ));
        // Let the forwarder hit the lag and resync before closing the
        // session (removal during the resync is the vanished-session path,
        // which is a different — also terminal — outcome).
        tokio::time::sleep(Duration::from_millis(50)).await;
        registry.remove("s1");

        let mut seen_indices = Vec::new();
        let mut saw_status = false;
        let mut saw_closed = false;
        while let Some(WatchNotification(value)) = rx.recv().await {
            match value["event"].as_str().unwrap() {
                "item" => seen_indices.push(value["item"]["index"].as_u64().unwrap()),
                "status" => saw_status = true,
                "closed" => saw_closed = true,
                other => panic!("unexpected event {other}"),
            }
        }
        // No gaps, no duplicates, in order — the resync served what the
        // broadcast buffer dropped (P#8's polling guarantee, kept for push).
        assert_eq!(seen_indices, (0..600).collect::<Vec<u64>>());
        assert!(saw_status);
        assert!(saw_closed);
    }

    /// A lagged completion-wait resyncs too: the status flip it missed is
    /// picked up from the registry.
    #[tokio::test]
    async fn lagged_wait_resyncs_and_resolves() {
        let registry = leaked_registry();
        publish(registry, "s1", SessionStatus::Running, vec![]);
        let subscription = registry.subscribe("s1").unwrap();

        // Bury the DoneOk flip under enough later events to overflow the
        // buffer (flip to DoneOk, then keep appending in that state).
        let mut items = Vec::new();
        items.push(flat(0, "assistant", "final answer"));
        publish(registry, "s1", SessionStatus::DoneOk, items.clone());
        for index in 1..600u64 {
            items.push(flat(index, "notice", "spam"));
            publish(registry, "s1", SessionStatus::DoneOk, items.clone());
        }

        let outcome = wait_for_completion(
            registry,
            vec![("s1".to_owned(), subscription)],
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(
            outcome,
            WaitOutcome::Completed {
                session_id: "s1".to_owned(),
                status: SessionStatus::DoneOk,
                reason: None,
                last_assistant_text: Some("final answer".to_owned()),
            }
        );
    }
}
