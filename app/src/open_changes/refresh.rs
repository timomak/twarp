//! Refresh coordinator for the Open Changes panel.
//!
//! Refreshes are debounced and coalesced (PRODUCT §7): a burst of N
//! `schedule()` calls produces at most one in-flight refresh plus one
//! pending follow-up. The follow-up fires exactly once when the in-flight
//! refresh completes; further schedules during the in-flight window are
//! collapsed into that single follow-up.
//!
//! Refreshes are spawned from a [`ModelContext<OpenChangesModel>`] so the
//! result lands back on the main thread and can `ctx.notify()` the view
//! handles that read the model.

use std::path::PathBuf;

use warpui::ModelContext;

use crate::open_changes::git;
use crate::open_changes::repo::find_repo_root;
use crate::open_changes::OpenChangesModel;

/// Schedule a refresh of the Open Changes panel for `pane_cwd`.
///
/// Called from inside an `OpenChangesModel::handle(ctx).update(...)`
/// callback — `model_ctx` is the model's own `ModelContext`, and the
/// in-flight bookkeeping fields are mutated on `model`.
///
/// - If `pane_cwd` is `None` or not inside a git repo, the model is
///   cleared to the no-repo state (PRODUCT §3).
/// - If a refresh is already in flight, marks one pending and returns.
/// - Otherwise spawns an async task that fetches the status and updates
///   the model on completion (PRODUCT §7).
pub fn schedule(
    model: &mut OpenChangesModel,
    pane_cwd: Option<PathBuf>,
    model_ctx: &mut ModelContext<OpenChangesModel>,
) {
    let target_root = pane_cwd.as_deref().and_then(find_repo_root);

    // No-repo branch: clear state and stop.
    let Some(repo_root) = target_root else {
        if model.state.is_some() {
            model.state = None;
            model_ctx.notify();
        }
        model.refresh_in_flight = false;
        model.refresh_pending = false;
        return;
    };

    if model.refresh_in_flight {
        model.refresh_pending = true;
        return;
    }

    model.refresh_in_flight = true;
    spawn_refresh(repo_root, model_ctx);
}

fn spawn_refresh(repo_root: PathBuf, model_ctx: &mut ModelContext<OpenChangesModel>) {
    let fut = async move { git::fetch_status(&repo_root).await };

    model_ctx.spawn(fut, move |model, result, model_ctx| {
        match result {
            Ok(state) => {
                model.state = Some(state);
            }
            Err(err) => {
                log::warn!("open_changes: refresh failed: {err}");
                if let Some(existing) = model.state.as_mut() {
                    existing.errors.push(format!("{err}"));
                }
            }
        }
        model.refresh_in_flight = false;
        model_ctx.notify();

        // PRODUCT §7: if a refresh was scheduled while one was in flight,
        // fire exactly one more.
        if model.refresh_pending {
            model.refresh_pending = false;
            let cwd = model.state.as_ref().map(|s| s.root.clone());
            schedule(model, cwd, model_ctx);
        }
    });
}
