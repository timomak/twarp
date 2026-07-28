//! twarp 21d: local draft state for a batched PR review, plus the JSON
//! payload builders backing the write-path mutations.
//!
//! Route choice: the batched review submit goes through the REST endpoint
//! (`gh api repos/{slug}/pulls/{n}/reviews --input -`) because it accepts the
//! whole review (event + body + line comments) as one JSON document piped via
//! stdin — no GraphQL node-id lookup and no nested-input quoting through
//! `gh api graphql -f`. Thread replies and resolve/unresolve use GraphQL
//! (`addPullRequestReviewThreadReply` / `resolveReviewThread` /
//! `unresolveReviewThread`) since those key off thread node ids that the 21c
//! reviewThreads query already fetches from the origin-slug repo.

use crate::pull_requests::diff::PrDiffSide;

impl PrDiffSide {
    /// The REST/GraphQL `side` value.
    pub fn api_str(self) -> &'static str {
        match self {
            PrDiffSide::Left => "LEFT",
            PrDiffSide::Right => "RIGHT",
        }
    }
}

/// The review verdict submitted with a batched review.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PrReviewEvent {
    Approve,
    RequestChanges,
    #[default]
    Comment,
}

impl PrReviewEvent {
    pub const ALL: [PrReviewEvent; 3] = [
        PrReviewEvent::Approve,
        PrReviewEvent::RequestChanges,
        PrReviewEvent::Comment,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrReviewEvent::Approve => "Approve",
            PrReviewEvent::RequestChanges => "Request changes",
            PrReviewEvent::Comment => "Comment",
        }
    }

    /// Stable string used as the action payload.
    pub fn as_str(self) -> &'static str {
        match self {
            PrReviewEvent::Approve => "approve",
            PrReviewEvent::RequestChanges => "request_changes",
            PrReviewEvent::Comment => "comment",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == s)
    }

    /// The REST `event` value.
    pub fn api_str(self) -> &'static str {
        match self {
            PrReviewEvent::Approve => "APPROVE",
            PrReviewEvent::RequestChanges => "REQUEST_CHANGES",
            PrReviewEvent::Comment => "COMMENT",
        }
    }

    /// Approve / Request changes are consequential enough for a two-click
    /// confirm; a plain Comment review submits directly.
    pub fn needs_confirm(self) -> bool {
        !matches!(self, PrReviewEvent::Comment)
    }
}

/// One locally drafted inline comment, anchored both by API coordinates
/// (path + line + side, what gets submitted) and by diff position
/// (file/hunk/line indices, where the pending card renders).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrDraftComment {
    pub path: String,
    /// File line number on `side` (RIGHT = new file, LEFT = old file).
    pub line: u64,
    pub side: PrDiffSide,
    /// (file index, hunk index, line index) into the parsed diff, for
    /// rendering the pending card inline. May go stale across refetches; the
    /// API coordinates above are what's submitted.
    pub position: (usize, usize, usize),
    pub body: String,
}

/// The accumulating local review: pending inline drafts plus the chosen
/// verdict. Purely local until submitted.
#[derive(Clone, Debug, Default)]
pub struct ReviewDrafts {
    drafts: Vec<PrDraftComment>,
    event: PrReviewEvent,
    /// True after the first "Discard all" click; the second click clears.
    discard_all_armed: bool,
}

impl ReviewDrafts {
    pub fn drafts(&self) -> &[PrDraftComment] {
        &self.drafts
    }

    pub fn is_empty(&self) -> bool {
        self.drafts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.drafts.len()
    }

    pub fn event(&self) -> PrReviewEvent {
        self.event
    }

    pub fn set_event(&mut self, event: PrReviewEvent) {
        self.event = event;
    }

    pub fn discard_all_armed(&self) -> bool {
        self.discard_all_armed
    }

    /// Any interaction other than the second "Discard all" click disarms it.
    pub fn disarm_discard_all(&mut self) {
        self.discard_all_armed = false;
    }

    pub fn add(&mut self, draft: PrDraftComment) {
        self.discard_all_armed = false;
        self.drafts.push(draft);
    }

    pub fn discard(&mut self, index: usize) {
        self.discard_all_armed = false;
        if index < self.drafts.len() {
            self.drafts.remove(index);
        }
    }

    /// Two-click discard-all: the first call arms, the second clears.
    /// Returns true when the drafts were actually cleared.
    pub fn discard_all(&mut self) -> bool {
        if self.discard_all_armed {
            self.discard_all_armed = false;
            self.drafts.clear();
            true
        } else {
            self.discard_all_armed = true;
            false
        }
    }

    /// Clear after a successful submit.
    pub fn clear(&mut self) {
        self.drafts.clear();
        self.discard_all_armed = false;
        self.event = PrReviewEvent::default();
    }

    /// Draft indices anchored at one diff position, in draft order.
    pub fn at_position(&self, position: (usize, usize, usize)) -> Vec<usize> {
        self.drafts
            .iter()
            .enumerate()
            .filter(|(_, d)| d.position == position)
            .map(|(i, _)| i)
            .collect()
    }
}

/// The REST request body for `POST /repos/{slug}/pulls/{n}/reviews`: event +
/// optional summary body + the drafted line comments.
pub fn build_review_payload(event: PrReviewEvent, body: &str, drafts: &[PrDraftComment]) -> String {
    let comments: Vec<serde_json::Value> = drafts
        .iter()
        .map(|d| {
            serde_json::json!({
                "path": d.path,
                "line": d.line,
                "side": d.side.api_str(),
                "body": d.body,
            })
        })
        .collect();
    let mut payload = serde_json::json!({
        "event": event.api_str(),
        "comments": comments,
    });
    let trimmed = body.trim();
    if !trimmed.is_empty() {
        payload["body"] = serde_json::Value::String(trimmed.to_owned());
    }
    payload.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(path: &str, line: u64, side: PrDiffSide, body: &str) -> PrDraftComment {
        PrDraftComment {
            path: path.into(),
            line,
            side,
            position: (0, 0, line as usize),
            body: body.into(),
        }
    }

    #[test]
    fn accumulates_and_discards_drafts() {
        let mut review = ReviewDrafts::default();
        assert!(review.is_empty());
        review.add(draft("a.rs", 1, PrDiffSide::Right, "one"));
        review.add(draft("a.rs", 2, PrDiffSide::Right, "two"));
        review.add(draft("b.rs", 9, PrDiffSide::Left, "three"));
        assert_eq!(review.len(), 3);
        assert_eq!(review.at_position((0, 0, 2)), vec![1]);

        review.discard(1);
        assert_eq!(review.len(), 2);
        assert_eq!(review.drafts()[1].body, "three");
        // Out-of-range discard is a no-op.
        review.discard(99);
        assert_eq!(review.len(), 2);
    }

    #[test]
    fn discard_all_is_two_click() {
        let mut review = ReviewDrafts::default();
        review.add(draft("a.rs", 1, PrDiffSide::Right, "one"));
        assert!(!review.discard_all()); // Arms.
        assert!(review.discard_all_armed());
        assert_eq!(review.len(), 1);
        assert!(review.discard_all()); // Clears.
        assert!(review.is_empty());
        assert!(!review.discard_all_armed());

        // Another interaction between the two clicks disarms.
        review.add(draft("a.rs", 1, PrDiffSide::Right, "one"));
        assert!(!review.discard_all());
        review.add(draft("a.rs", 2, PrDiffSide::Right, "two"));
        assert!(!review.discard_all_armed());
        assert!(!review.discard_all()); // Re-arms instead of clearing.
        assert_eq!(review.len(), 2);
    }

    #[test]
    fn clear_resets_event_and_drafts() {
        let mut review = ReviewDrafts::default();
        review.add(draft("a.rs", 1, PrDiffSide::Right, "one"));
        review.set_event(PrReviewEvent::Approve);
        review.clear();
        assert!(review.is_empty());
        assert_eq!(review.event(), PrReviewEvent::default());
    }

    #[test]
    fn builds_review_payload() {
        let drafts = vec![
            draft("src/a.rs", 12, PrDiffSide::Right, "use a helper"),
            draft("src/b.rs", 3, PrDiffSide::Left, "why removed?"),
        ];
        let payload = build_review_payload(PrReviewEvent::RequestChanges, " overall\n", &drafts);
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["event"], "REQUEST_CHANGES");
        assert_eq!(value["body"], "overall");
        let comments = value["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0]["path"], "src/a.rs");
        assert_eq!(comments[0]["line"], 12);
        assert_eq!(comments[0]["side"], "RIGHT");
        assert_eq!(comments[0]["body"], "use a helper");
        assert_eq!(comments[1]["side"], "LEFT");
    }

    #[test]
    fn payload_omits_empty_body() {
        let payload = build_review_payload(PrReviewEvent::Approve, "  ", &[]);
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["event"], "APPROVE");
        assert!(value.get("body").is_none());
        assert_eq!(value["comments"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn review_event_round_trip() {
        for event in PrReviewEvent::ALL {
            assert_eq!(PrReviewEvent::from_str(event.as_str()), Some(event));
        }
        assert!(PrReviewEvent::Approve.needs_confirm());
        assert!(PrReviewEvent::RequestChanges.needs_confirm());
        assert!(!PrReviewEvent::Comment.needs_confirm());
        assert_eq!(PrReviewEvent::from_str("bogus"), None);
    }
}
