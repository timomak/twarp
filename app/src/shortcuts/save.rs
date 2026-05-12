//! Atomic save of the in-memory shortcut registry back to `shortcuts.yaml`
//! (PRODUCT §36). Used by the side-panel GUI's create/edit/delete flows.
//!
//! Writes are done temp-file + rename so the file is never observed in a
//! half-written state by either the file watcher or a concurrent hand-edit.
//! The `SAVE_IN_FLIGHT` flag suppresses the watcher's reload while a save is
//! happening, avoiding a double-reload (watcher's reload would race the
//! explicit reload that `save_and_reload` performs after a successful write).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::shortcuts::config::{serialize_shortcuts, Shortcut};
use crate::shortcuts::shortcuts_file_path;

/// Set while `save_and_reload` is writing the file. The watcher consults this
/// before reacting to a filesystem event.
pub static SAVE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "i/o error writing shortcuts.yaml: {e}"),
        }
    }
}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        SaveError::Io(e)
    }
}

/// Serialize `shortcuts` and atomically write to `shortcuts.yaml`.
///
/// Returns the path on success so callers can log it. The watcher
/// suppression toggles around the write window; if the write fails, the
/// flag is still cleared.
#[cfg(feature = "local_fs")]
pub fn save_to_disk(shortcuts: &[Shortcut]) -> Result<PathBuf, SaveError> {
    let path = shortcuts_file_path();
    let yaml = serialize_shortcuts(shortcuts);
    SAVE_IN_FLIGHT.store(true, Ordering::SeqCst);
    let result = (|| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("yaml.tmp");
        std::fs::write(&tmp, yaml.as_bytes())?;
        std::fs::rename(&tmp, &path)?;
        Ok(path.clone())
    })();
    SAVE_IN_FLIGHT.store(false, Ordering::SeqCst);
    result
}

#[cfg(not(feature = "local_fs"))]
pub fn save_to_disk(_shortcuts: &[Shortcut]) -> Result<PathBuf, SaveError> {
    Err(SaveError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "local_fs feature disabled",
    )))
}
