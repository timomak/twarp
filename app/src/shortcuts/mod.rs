//! Custom command shortcuts: declarative keybindings to action sequences.
//!
//! See `roadmap/04-command-shortcuts/PRODUCT.md` for behavior and
//! `roadmap/04-command-shortcuts/TECH.md` for the implementation plan. This
//! module covers sub-phase 4a (runtime); the side-panel GUI and file-watch
//! reload live in 4b.

pub mod action;
pub mod config;
pub mod executor;
pub mod key_to_bytes;

#[cfg(test)]
#[path = "shortcuts_tests.rs"]
mod tests;

use std::path::PathBuf;

use warpui::{AppContext, Entity, SingletonEntity};

use crate::shortcuts::config::{parse_shortcuts_yaml, Shortcut};

pub type ShortcutId = u32;

const SHORTCUTS_FILE_NAME: &str = "shortcuts.yaml";

/// First-launch default for `shortcuts.yaml`. Written to disk on first
/// startup if no file exists (PRODUCT §1) so the driving examples work out
/// of the box. Subsequent launches read whatever the user has on disk; the
/// default is never re-applied. Keep this in sync with PRODUCT §Driving
/// examples — the `default_shortcuts_yaml_parses` test enforces shape.
pub const DEFAULT_SHORTCUTS_YAML: &str = r#"# Custom command shortcuts for twarp.
#
# Each entry binds a chord to a sequence of terminal actions. v1 supports:
#   new_tab            - open a new tab (becomes the sequence's target)
#   new_pane: right    - split the active pane right (or 'down')
#   type: "text"       - write literal text into the active pane's shell
#   press: enter       - press a named key (enter, tab, escape, up, down, ...)
#   wait: 2s           - pause before the next action (e.g. 500ms, 2s, 1m)
#
# Edit this file to add your own. Restart twarp for changes to apply.
shortcuts:
  - keys: cmdorctrl-shift-D
    actions:
      - new_pane: right
      - type: "claude"
      - press: enter

  - keys: cmdorctrl-shift-A
    actions:
      - new_pane: right
      - type: "claude"
      - press: enter
      - wait: 3s
      - type: "/address-code-review-comments"
      - press: enter
"#;

#[derive(Default)]
pub struct ShortcutsModel {
    pub registry: Vec<Shortcut>,
    pub errors: Vec<String>,
    /// `true` while the parsed-but-not-yet-displayed error toast is pending.
    /// The first Workspace that observes it surfaces the toast and clears the
    /// flag. This flag never re-arms in 4a; 4b's file watcher re-arms it on
    /// each reload.
    pub errors_pending_toast: bool,
}

impl ShortcutsModel {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Entity for ShortcutsModel {
    type Event = ();
}

impl SingletonEntity for ShortcutsModel {}

#[cfg(feature = "local_fs")]
pub fn shortcuts_file_path() -> PathBuf {
    warp_core::paths::config_local_dir().join(SHORTCUTS_FILE_NAME)
}

#[cfg(not(feature = "local_fs"))]
pub fn shortcuts_file_path() -> PathBuf {
    PathBuf::from(SHORTCUTS_FILE_NAME)
}

/// Load `shortcuts.yaml` from disk, parse, populate the singleton model.
/// Called once at startup, after `keyboard::load_custom_keybindings`.
///
/// Binding registration happens in `register_shortcut_bindings`, called from
/// the same startup site.
#[cfg(feature = "local_fs")]
pub fn load(app: &mut AppContext) {
    let path = shortcuts_file_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        // PRODUCT §1: bootstrap a default file on first launch so the
        // driving examples work out of the box. `create_dir_all` first
        // because the parent may not exist on a fresh install.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(write_err) = std::fs::write(&path, DEFAULT_SHORTCUTS_YAML) {
                log::warn!(
                    "shortcuts: failed to write default shortcuts.yaml at {:?}: {write_err}",
                    path
                );
                return;
            }
            log::info!("shortcuts: wrote default shortcuts.yaml at {:?}", path);
            DEFAULT_SHORTCUTS_YAML.to_owned()
        }
        Err(e) => {
            log::warn!("shortcuts: could not read {:?}: {e}", path);
            return;
        }
    };
    let result = parse_shortcuts_yaml(&text);
    for err in &result.errors {
        log::warn!("{err}");
    }
    log::info!(
        "shortcuts: loaded {n} shortcut(s) from {:?}",
        path,
        n = result.shortcuts.len()
    );
    let has_errors = !result.errors.is_empty();
    ShortcutsModel::handle(app).update(app, |model, _| {
        model.registry = result.shortcuts;
        model.errors = result.errors;
        model.errors_pending_toast = has_errors;
    });
}

#[cfg(not(feature = "local_fs"))]
pub fn load(_app: &mut AppContext) {}
