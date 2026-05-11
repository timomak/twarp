use crate::shortcuts::action::Action;
use crate::shortcuts::ShortcutId;

/// In-flight state for one custom-shortcut sequence.
///
/// Owned by `Workspace` (one runner per window), not by the singleton
/// `ShortcutsModel`. The shortcut's action list is captured by value at the
/// moment the runner starts, so a reload of `shortcuts.yaml` mid-sequence
/// (4b file-watch) does not change the actions of an already-running runner
/// (PRODUCT §24, TECH risk-mitigation "Runner dereferences live registry").
pub struct ShortcutRunner {
    pub id: ShortcutId,
    pub actions: Vec<Action>,
    pub action_idx: usize,
    pub target_tab: usize,
    pub cancelled: bool,
}

impl ShortcutRunner {
    pub fn new(id: ShortcutId, actions: Vec<Action>, target_tab: usize) -> Self {
        Self {
            id,
            actions,
            action_idx: 0,
            target_tab,
            cancelled: false,
        }
    }

    pub fn next_action(&self) -> Option<&Action> {
        self.actions.get(self.action_idx)
    }

    pub fn advance(&mut self) {
        self.action_idx = self.action_idx.saturating_add(1);
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub fn is_done(&self) -> bool {
        self.cancelled || self.action_idx >= self.actions.len()
    }
}
