//! twarp 26b: the single status projection shared by the tab indicator and
//! the sessions MCP registry (PRODUCT P#4: the two must never disagree — they
//! derive from the same pure function over the same inputs).

use crate::app_state::ConversationStatus;

/// The view state [`conversation_status`] projects from. Built by
/// `ClaudeCodeView::status_inputs` — the one place that knows the live
/// signals — and consumed by both the renderer and the registry publisher.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusInputs {
    pub streaming: bool,
    /// A pending permission/question card is parking the turn on the user.
    pub blocked_on_user: bool,
    /// Background scripts or sub-agents launched by the chat are still
    /// running (gated on a live process by the caller — once the agent is
    /// gone no task notification can ever arrive, so a Running entry in a
    /// dead/restored transcript must not count as in-flight work forever).
    pub has_active_background_work: bool,
    /// The last turn's outcome (`Some(succeeded)`), or `None` before any
    /// turn completed / after the attention was cleared.
    pub tab_attention: Option<bool>,
    /// Whether the user has visited the pane since `tab_attention` was set.
    pub tab_attention_seen: bool,
}

/// The session state the tab indicator shows (7p): blocked-on-the-user
/// outranks working, and an idle pane shows the last turn's outcome until the
/// user comes back to it. `None` renders no indicator (a fresh or revisited
/// idle pane). A turn can end while background work is still running — until
/// the last script/agent retires the tab keeps the working spinner rather
/// than declaring the turn complete.
pub fn conversation_status(inputs: StatusInputs) -> Option<ConversationStatus> {
    if inputs.streaming && inputs.blocked_on_user {
        return Some(ConversationStatus::Blocked {});
    }
    if inputs.streaming || inputs.has_active_background_work {
        return Some(ConversationStatus::InProgress);
    }
    inputs.tab_attention.map(|succeeded| {
        if succeeded {
            if inputs.tab_attention_seen {
                ConversationStatus::Success
            } else {
                ConversationStatus::SuccessUnseen
            }
        } else {
            ConversationStatus::Error
        }
    })
}

/// The status vocabulary `list_sessions` speaks (PRODUCT P#3): exactly
/// `running`, `needs_input`, `done_ok`, `done_error`, `idle`. Past (non-live)
/// sessions always report `idle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    NeedsInput,
    DoneOk,
    DoneError,
    Idle,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NeedsInput => "needs_input",
            Self::DoneOk => "done_ok",
            Self::DoneError => "done_error",
            Self::Idle => "idle",
        }
    }

    /// Whether this status resolves a `wait_for_completion` (26c, PRODUCT
    /// P#10): the done states plus `needs_input` — a session parked on a
    /// permission/question needs its caller's attention just as much as a
    /// finished one.
    pub fn resolves_wait(self) -> bool {
        matches!(self, Self::DoneOk | Self::DoneError | Self::NeedsInput)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "needs_input" => Some(Self::NeedsInput),
            "done_ok" => Some(Self::DoneOk),
            "done_error" => Some(Self::DoneError),
            "idle" => Some(Self::Idle),
            _ => None,
        }
    }
}

/// Fold the tab's [`ConversationStatus`] into the MCP status enum. Going
/// through [`conversation_status`] (not a parallel computation) is what
/// guarantees P#4 by construction.
pub fn session_status(inputs: StatusInputs) -> SessionStatus {
    match conversation_status(inputs) {
        Some(ConversationStatus::Blocked {}) => SessionStatus::NeedsInput,
        Some(ConversationStatus::InProgress) => SessionStatus::Running,
        Some(
            ConversationStatus::Success
            | ConversationStatus::SuccessUnseen
            | ConversationStatus::Done,
        ) => SessionStatus::DoneOk,
        Some(
            ConversationStatus::Error | ConversationStatus::Failed | ConversationStatus::Cancelled,
        ) => SessionStatus::DoneError,
        Some(ConversationStatus::Other) | None => SessionStatus::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        streaming: bool,
        blocked_on_user: bool,
        background: bool,
        attention: Option<bool>,
        seen: bool,
    ) -> StatusInputs {
        StatusInputs {
            streaming,
            blocked_on_user,
            has_active_background_work: background,
            tab_attention: attention,
            tab_attention_seen: seen,
        }
    }

    /// P#3–4: the full projection table, pairing what the tab shows with what
    /// `list_sessions` reports for every reachable input combination.
    #[test]
    fn status_projection_table() {
        let cases: &[(StatusInputs, Option<ConversationStatus>, SessionStatus)] = &[
            // Fresh idle pane: no indicator, MCP says idle.
            (
                inputs(false, false, false, None, false),
                None,
                SessionStatus::Idle,
            ),
            // Streaming turn.
            (
                inputs(true, false, false, None, false),
                Some(ConversationStatus::InProgress),
                SessionStatus::Running,
            ),
            // Streaming and parked on a permission/question card.
            (
                inputs(true, true, false, None, false),
                Some(ConversationStatus::Blocked {}),
                SessionStatus::NeedsInput,
            ),
            // A stale blocked flag without streaming is not blocking.
            (
                inputs(false, true, false, None, false),
                None,
                SessionStatus::Idle,
            ),
            // Turn ended but background scripts/agents still running.
            (
                inputs(false, false, true, Some(true), false),
                Some(ConversationStatus::InProgress),
                SessionStatus::Running,
            ),
            // Completed, unreviewed.
            (
                inputs(false, false, false, Some(true), false),
                Some(ConversationStatus::SuccessUnseen),
                SessionStatus::DoneOk,
            ),
            // Completed, reviewed.
            (
                inputs(false, false, false, Some(true), true),
                Some(ConversationStatus::Success),
                SessionStatus::DoneOk,
            ),
            // Failed turn (seen or not — same MCP status).
            (
                inputs(false, false, false, Some(false), false),
                Some(ConversationStatus::Error),
                SessionStatus::DoneError,
            ),
            (
                inputs(false, false, false, Some(false), true),
                Some(ConversationStatus::Error),
                SessionStatus::DoneError,
            ),
            // A new streaming turn masks the previous outcome.
            (
                inputs(true, false, false, Some(false), true),
                Some(ConversationStatus::InProgress),
                SessionStatus::Running,
            ),
        ];
        for (inputs, expected_tab, expected_mcp) in cases {
            assert_eq!(
                conversation_status(*inputs),
                *expected_tab,
                "tab status for {inputs:?}"
            );
            assert_eq!(
                session_status(*inputs),
                *expected_mcp,
                "mcp status for {inputs:?}"
            );
        }
    }

    #[test]
    fn status_strings_round_trip() {
        for status in [
            SessionStatus::Running,
            SessionStatus::NeedsInput,
            SessionStatus::DoneOk,
            SessionStatus::DoneError,
            SessionStatus::Idle,
        ] {
            assert_eq!(SessionStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(SessionStatus::parse("bogus"), None);
    }
}
