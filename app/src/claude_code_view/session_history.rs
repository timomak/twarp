use std::path::{Path, PathBuf};

/// A recorded transcript is authoritative across later checkout relocations.
/// Old snapshots have no recorded path, so consider only their original and
/// explicitly mapped locations, preferring the most recently written file.
pub(super) fn existing_path(
    recorded: Option<&Path>,
    original: Option<PathBuf>,
    mapped: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(recorded) = recorded.filter(|path| path.is_file()) {
        return Some(recorded.to_path_buf());
    }
    original
        .into_iter()
        .chain(mapped)
        .filter_map(|path| {
            let metadata = path.metadata().ok()?;
            metadata.is_file().then(|| (path, metadata.modified().ok()))
        })
        .max_by_key(|(_, modified)| *modified)
        .map(|(path, _)| path)
}

#[cfg(test)]
#[path = "session_history_tests.rs"]
mod tests;
