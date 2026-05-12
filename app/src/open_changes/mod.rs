//! Open Changes side-panel — VS Code Source Control-style git review.
//!
//! See `roadmap/05-open-changes/PRODUCT.md` for behavior and
//! `roadmap/05-open-changes/TECH.md` for the implementation plan.
//!
//! This module covers sub-phase **5a** (panel scaffold + working/staged file
//! lists). Diff view, staging actions, commit input, and Timeline ship in
//! 5b–5e.

#[cfg(feature = "local_fs")]
pub mod git;
#[cfg(feature = "local_fs")]
pub mod refresh;
pub mod repo;
#[cfg(feature = "local_fs")]
pub mod view;
#[cfg(feature = "local_fs")]
pub mod watcher;

#[cfg(test)]
#[path = "open_changes_tests.rs"]
mod tests;

use std::path::PathBuf;

use warpui::{AppContext, Entity, SingletonEntity};

pub use repo::{BranchState, FileEntry, FileStatus, InProgressOp, RepoState, UpstreamTracking};

/// Singleton model holding the panel's current view of the panel repo.
///
/// `state == None` is the "no git repo in the focused pane" condition
/// (PRODUCT §3). Populated by [`refresh`] tasks dispatched whenever the
/// focused-pane cwd changes or the file watcher fires.
#[derive(Default)]
pub struct OpenChangesModel {
    /// Current `RepoState` for the panel repo, or `None` when the focused
    /// pane is not inside a git repo (PRODUCT §3).
    pub state: Option<RepoState>,
    /// The path (relative to the repo root) of the most recently clicked
    /// row, used by 5b's diff view and 5e's Timeline. None in 5a.
    pub focused_file: Option<PathBuf>,
    /// True while a refresh is currently fetching git state. Used by
    /// [`refresh::RefreshCoordinator`] for coalescing.
    pub refresh_in_flight: bool,
    /// True if a refresh was requested while one was already in flight.
    /// The in-flight refresh fires one more time on completion.
    pub refresh_pending: bool,
    /// Path of the repo root that owns the active watcher. Used to detect
    /// when the panel repo has changed so the watcher can be torn down
    /// and re-registered for the new repo.
    pub watched_root: Option<PathBuf>,
}

impl OpenChangesModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total count of distinct paths across `staged` and `changes`. A path
    /// that appears in both sections (partial staging) counts once.
    /// PRODUCT §5 / §42.
    pub fn unique_change_count(&self) -> usize {
        let Some(state) = self.state.as_ref() else {
            return 0;
        };
        let mut paths = std::collections::HashSet::new();
        for f in state.staged.iter().chain(state.changes.iter()) {
            paths.insert(f.path.as_path());
        }
        paths.len()
    }
}

impl Entity for OpenChangesModel {
    type Event = ();
}

impl SingletonEntity for OpenChangesModel {}

/// Initialize the Open Changes panel. Called once at startup, after the
/// singleton model and watcher are registered.
///
/// v1 has no on-disk config to read — the model is populated by the first
/// refresh, which fires when the user focuses a pane inside a git repo.
/// This function exists for symmetry with [`crate::shortcuts::load`] and to
/// give 5b–5e a hook for any startup-time work they need to add.
pub fn load(_ctx: &mut AppContext) {}
