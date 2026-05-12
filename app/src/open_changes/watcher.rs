//! File-system watcher for the Open Changes panel (PRODUCT §40).
//!
//! Watches the panel repo's working-tree top-level recursively. The
//! filter excludes high-churn `.git/` subpaths (`objects/`, `logs/`,
//! `refs/...`) but allowlists state-sentinel files (`HEAD`, `index`,
//! `MERGE_HEAD`, `REBASE_HEAD`, `CHERRY_PICK_HEAD`, `BISECT_LOG`) so the
//! panel notices terminal-driven git operations even without a separate
//! post-command hook.
//!
//! The watcher follows the [`ShortcutsWatcher`] precedent in
//! `app/src/shortcuts/watcher.rs` for debounce window and event handling.

#[cfg(not(target_family = "wasm"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

use warpui::{Entity, ModelContext, SingletonEntity};

#[cfg(not(target_family = "wasm"))]
use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
#[cfg(not(target_family = "wasm"))]
use warpui::ModelHandle;
#[cfg(not(target_family = "wasm"))]
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

#[cfg(not(target_family = "wasm"))]
use crate::open_changes::refresh;
#[cfg(not(target_family = "wasm"))]
use crate::open_changes::OpenChangesModel;

/// Debounce window for filesystem events. Matches the [`ShortcutsWatcher`]
/// default so back-to-back saves coalesce into one refresh.
#[cfg(not(target_family = "wasm"))]
const OPEN_CHANGES_WATCHER_DEBOUNCE_MS: u64 = 200;

/// `.git/` subpaths that the panel cares about (state sentinels).
/// Everything else under `.git/` is filtered out to keep the event
/// stream quiet when git is doing internal bookkeeping.
#[cfg(not(target_family = "wasm"))]
const GIT_SENTINEL_FILES: &[&str] = &[
    "HEAD",
    "index",
    "MERGE_HEAD",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "BISECT_LOG",
];

#[cfg(not(target_family = "wasm"))]
pub struct OpenChangesWatcher {
    watcher: ModelHandle<BulkFilesystemWatcher>,
    /// The repo root currently being watched. `None` until the first
    /// call to [`Self::watch_repo`].
    watched_root: Option<PathBuf>,
}

#[cfg(target_family = "wasm")]
pub struct OpenChangesWatcher;

impl Entity for OpenChangesWatcher {
    type Event = ();
}

impl SingletonEntity for OpenChangesWatcher {}

#[cfg(not(target_family = "wasm"))]
impl OpenChangesWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let watcher = ctx.add_model(|ctx| {
            BulkFilesystemWatcher::new(Duration::from_millis(OPEN_CHANGES_WATCHER_DEBOUNCE_MS), ctx)
        });
        ctx.subscribe_to_model(&watcher, Self::handle_fs_event);
        Self {
            watcher,
            watched_root: None,
        }
    }

    /// Start watching `repo_root` (and stop watching any previously
    /// registered root). Idempotent if `repo_root` is already the
    /// watched root.
    pub fn watch_repo(&mut self, repo_root: PathBuf, ctx: &mut ModelContext<Self>) {
        if self.watched_root.as_ref() == Some(&repo_root) {
            return;
        }

        // Tear down the previous registration. Awaiting isn't required
        // per the [`BulkFilesystemWatcher::unregister_path`] contract;
        // we just fire-and-forget.
        if let Some(prev) = self.watched_root.take() {
            let unreg = self.watcher.update(ctx, |w, _| w.unregister_path(&prev));
            ctx.spawn(unreg, move |_, result, _| {
                if let Err(err) = result {
                    log::debug!(
                        "open_changes: unregister of {} failed: {err}",
                        prev.display()
                    );
                }
            });
        }

        // Register the new repo root with a filter that ignores most of
        // `.git/` and accepts everything else. The filter is a path
        // predicate that runs in the watcher's background thread.
        let repo_root_for_filter = repo_root.clone();
        let filter = WatchFilter::with_filter(Arc::new(move |path: &Path| {
            path_is_relevant(&repo_root_for_filter, path)
        }));
        let registration_path = repo_root.clone();
        let registration = self.watcher.update(ctx, |w, _| {
            w.register_path(&registration_path, filter, RecursiveMode::Recursive)
        });
        let display_path = registration_path.clone();
        ctx.spawn(registration, move |_, result, _| {
            if let Err(err) = result {
                log::warn!(
                    "open_changes: failed to start watching {}: {err}",
                    display_path.display()
                );
            }
        });

        self.watched_root = Some(repo_root);
    }

    /// Stop watching the current repo, if any.
    pub fn unwatch(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(prev) = self.watched_root.take() {
            let unreg = self.watcher.update(ctx, |w, _| w.unregister_path(&prev));
            ctx.spawn(unreg, move |_, _result, _| {});
        }
    }

    fn handle_fs_event(
        &mut self,
        _event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // The watcher already filtered events per the predicate
        // installed in `watch_repo`; any event that reaches us is
        // refresh-worthy. PRODUCT §7's coalescing happens in the model.
        let Some(repo_root) = self.watched_root.clone() else {
            return;
        };
        OpenChangesModel::handle(ctx).update(ctx, |model, model_ctx| {
            refresh::schedule(model, Some(repo_root), model_ctx);
        });
    }
}

#[cfg(target_family = "wasm")]
impl OpenChangesWatcher {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }

    pub fn watch_repo(&mut self, _repo_root: std::path::PathBuf, _ctx: &mut ModelContext<Self>) {}

    pub fn unwatch(&mut self, _ctx: &mut ModelContext<Self>) {}
}

/// Returns true if a filesystem event on `path` (relative or absolute)
/// should trigger a panel refresh. PRODUCT §40.
#[cfg(not(target_family = "wasm"))]
fn path_is_relevant(repo_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(repo_root) else {
        // Path is outside the watched root (shouldn't happen with the
        // current registration shape, but be defensive).
        return false;
    };

    let mut components = relative.components();
    let first = components
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());

    match first.as_deref() {
        // Working-tree paths: always relevant.
        Some(seg) if seg != ".git" => true,
        // `.git/<sentinel>`: relevant.
        Some(".git") => {
            let rest: PathBuf = components.collect();
            let head = rest.iter().next().map(|c| c.to_string_lossy().into_owned());
            match head.as_deref() {
                Some(name) if GIT_SENTINEL_FILES.contains(&name) => true,
                Some("rebase-merge") | Some("rebase-apply") => true,
                _ => false,
            }
        }
        // The repo root itself or an empty relative path: not interesting.
        _ => false,
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod watcher_tests {
    use super::*;

    #[test]
    fn working_tree_path_is_relevant() {
        let root = Path::new("/tmp/repo");
        assert!(path_is_relevant(root, &root.join("src/main.rs")));
        assert!(path_is_relevant(root, &root.join("Cargo.toml")));
    }

    #[test]
    fn git_object_paths_are_filtered() {
        let root = Path::new("/tmp/repo");
        assert!(!path_is_relevant(root, &root.join(".git/objects/ab/cdef")));
        assert!(!path_is_relevant(root, &root.join(".git/logs/HEAD")));
        assert!(!path_is_relevant(root, &root.join(".git/refs/heads/main")));
    }

    #[test]
    fn git_sentinel_paths_are_relevant() {
        let root = Path::new("/tmp/repo");
        assert!(path_is_relevant(root, &root.join(".git/HEAD")));
        assert!(path_is_relevant(root, &root.join(".git/index")));
        assert!(path_is_relevant(root, &root.join(".git/MERGE_HEAD")));
        assert!(path_is_relevant(
            root,
            &root.join(".git/rebase-merge/head-name")
        ));
    }

    #[test]
    fn paths_outside_repo_are_ignored() {
        let root = Path::new("/tmp/repo");
        assert!(!path_is_relevant(root, Path::new("/etc/passwd")));
    }
}
