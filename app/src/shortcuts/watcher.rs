//! File-watch hot reload for `shortcuts.yaml` (PRODUCT §24).
//!
//! Spawns a background `BulkFilesystemWatcher` (debounced 200ms) on the
//! parent directory of `shortcuts.yaml` (notify-rs on macOS doesn't deliver
//! events for atomic rename-into-place when watching the file directly), and
//! filters the watch to just that file. On any event, calls
//! `shortcuts::reload(ctx)`. A save-in-flight flag (`save::SAVE_IN_FLIGHT`)
//! lets the GUI's atomic save suppress its own watcher echo.
//!
//! In-flight `ShortcutRunner`s are unaffected by reload — 4a's runner
//! captures the action list by value at start.

use warpui::{Entity, ModelContext, SingletonEntity};

#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::sync::{atomic::Ordering, Arc};
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
#[cfg(not(target_family = "wasm"))]
use warpui::ModelHandle;
#[cfg(not(target_family = "wasm"))]
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

#[cfg(not(target_family = "wasm"))]
use crate::shortcuts::save::SAVE_IN_FLIGHT;
#[cfg(not(target_family = "wasm"))]
use crate::shortcuts::shortcuts_file_path;

/// Debounce duration between filesystem events for the shortcuts watcher.
/// 200ms is short enough to feel instant but long enough to coalesce the
/// typical write-and-rename atomic save pattern into one event.
#[cfg(not(target_family = "wasm"))]
const SHORTCUTS_WATCHER_DEBOUNCE_MS: u64 = 200;

#[cfg(not(target_family = "wasm"))]
pub struct ShortcutsWatcher {
    _watcher: ModelHandle<BulkFilesystemWatcher>,
}

#[cfg(target_family = "wasm")]
pub struct ShortcutsWatcher;

impl Entity for ShortcutsWatcher {
    type Event = ();
}

impl SingletonEntity for ShortcutsWatcher {}

#[cfg(not(target_family = "wasm"))]
impl ShortcutsWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let watcher = ctx.add_model(|ctx| {
            BulkFilesystemWatcher::new(Duration::from_millis(SHORTCUTS_WATCHER_DEBOUNCE_MS), ctx)
        });
        ctx.subscribe_to_model(&watcher, Self::handle_fs_event);

        let path = shortcuts_file_path();
        if let Some(parent) = path.parent() {
            let target = path.clone();
            let filter = WatchFilter::with_filter(Arc::new(move |p| p == target));
            Self::register_path(ctx, &watcher, parent.to_path_buf(), filter);
        }

        Self { _watcher: watcher }
    }

    fn register_path(
        ctx: &mut ModelContext<Self>,
        watcher: &ModelHandle<BulkFilesystemWatcher>,
        directory_path: PathBuf,
        watch_filter: WatchFilter,
    ) {
        let registration_path = directory_path.clone();
        let registration = watcher.update(ctx, |watcher, _ctx| {
            watcher.register_path(
                &registration_path,
                watch_filter,
                RecursiveMode::NonRecursive,
            )
        });
        ctx.spawn(registration, move |_, result, _ctx| {
            if let Err(err) = result {
                log::warn!(
                    "shortcuts: failed to start watching {}: {err}",
                    directory_path.display()
                );
            }
        });
    }

    fn handle_fs_event(
        &mut self,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if SAVE_IN_FLIGHT.load(Ordering::SeqCst) {
            log::debug!("shortcuts: watcher event during save; suppressing reload");
            return;
        }
        let path = shortcuts_file_path();
        let touched = event.added.contains(&path)
            || event.modified.contains(&path)
            || event.deleted.contains(&path)
            || event.moved.contains_key(&path);
        if !touched {
            return;
        }
        log::info!("shortcuts: file changed on disk; reloading");
        // ModelContext derefs to AppContext, so we can pass it straight to
        // the AppContext-taking reload.
        crate::shortcuts::reload(ctx);
    }
}

#[cfg(target_family = "wasm")]
impl ShortcutsWatcher {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }
}
