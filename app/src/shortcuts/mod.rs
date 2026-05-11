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
    let text = match std::fs::read_to_string(shortcuts_file_path()) {
        Ok(t) => t,
        Err(_) => return,
    };
    let result = parse_shortcuts_yaml(&text);
    for err in &result.errors {
        log::warn!("{err}");
    }
    let has_errors = !result.errors.is_empty();
    ShortcutsModel::handle(app).update(app, |model, _| {
        model.registry = result.shortcuts;
        model.errors = result.errors;
        model.errors_pending_toast = has_errors;
    });
}

#[cfg(not(feature = "local_fs"))]
pub fn load(_app: &mut AppContext) {}
