//! Background loading for the explorer. Visibility changes release watchers,
//! while a small cache preserves expanded directories for the next activation.

use super::*;
use std::collections::VecDeque;

const MAX_CACHED_ROOTS: usize = 8;

#[derive(Default)]
pub(super) struct LazyLoading {
    next_request: u64,
    roots: HashMap<StandardizedPath, u64>,
    pub(super) epochs: HashMap<StandardizedPath, u64>,
    directories: HashMap<(StandardizedPath, StandardizedPath), u64>,
    errors: HashMap<(StandardizedPath, StandardizedPath), String>,
    cached: VecDeque<(StandardizedPath, FileTreeEntry)>,
}

impl LazyLoading {
    pub(super) fn has_pending_root(&self, root: &StandardizedPath) -> bool {
        self.roots.contains_key(root)
    }

    fn next_request(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    pub(super) fn cancel(&mut self, root: &StandardizedPath) {
        self.roots.remove(root);
        self.epochs.remove(root);
        self.directories.retain(|(path, _), _| path != root);
        self.errors.retain(|(path, _), _| path != root);
    }

    pub(super) fn cache(&mut self, root: StandardizedPath, entry: FileTreeEntry) {
        self.cached.retain(|(path, _)| path != &root);
        self.cached.push_back((root, entry));
        while self.cached.len() > MAX_CACHED_ROOTS {
            self.cached.pop_front();
        }
    }

    fn take_cached(&mut self, root: &StandardizedPath) -> Option<FileTreeEntry> {
        let index = self.cached.iter().position(|(path, _)| path == root)?;
        self.cached.remove(index).map(|(_, entry)| entry)
    }
}

/// Re-read the root and previously loaded directories without retaining stale
/// children. All filesystem access, including gitignore parsing, happens here.
fn read_root(
    path: &StandardizedPath,
    cached: Option<FileTreeEntry>,
) -> Result<(FileTreeEntry, Vec<Gitignore>), RepoMetadataError> {
    let local = path
        .to_local_path()
        .ok_or_else(|| RepoMetadataError::PathEncodingMismatch(path.clone()))?;
    let metadata = std::fs::metadata(&local)
        .map_err(|error| RepoMetadataError::InvalidPath(format!("{path}: {error}")))?;
    if !metadata.is_dir() {
        return Err(RepoMetadataError::InvalidPath(format!(
            "{path} is not a directory"
        )));
    }
    let mut gitignores = gitignores_for_directory(&local);
    let mut file_limit = MAX_FILES_PER_REPO;
    let root = Entry::build_tree(
        &local,
        &mut Vec::new(),
        &mut gitignores,
        Some(&mut file_limit),
        1,
        0,
        &IgnoredPathStrategy::Include,
    )
    .map_err(RepoMetadataError::BuildTree)?;
    let mut entry = FileTreeEntry::from(root);
    if let Some(cached) = cached {
        let mut parents = vec![path.clone()];
        while let Some(parent) = parents.pop() {
            for child in cached.child_paths(&parent) {
                if matches!(cached.get(child), Some(FileTreeEntryState::Directory(dir)) if dir.loaded)
                    && matches!(entry.get(child), Some(FileTreeEntryState::Directory(_)))
                {
                    // A child removed or made unreadable while hidden should
                    // not discard the rest of the usable root.
                    if entry.load_at_path(child, &mut gitignores).is_ok() {
                        parents.push((**child).clone());
                    }
                }
            }
        }
    }
    Ok((entry, gitignores))
}

impl LocalRepoMetadataModel {
    #[cfg(any(test, feature = "test-util"))]
    pub fn has_pending_explorer_loads(&self) -> bool {
        !self.lazy_loading.roots.is_empty() || !self.lazy_loading.directories.is_empty()
    }

    /// Acquire a standalone root immediately, then read it on the executor.
    /// Shared consumers join the same request and retain independent refcounts.
    pub fn request_index_lazy_loaded_path(
        &mut self,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let already_registered = if let Some(count) = self.lazy_loaded_paths.get_mut(path) {
            *count += 1;
            if !matches!(
                self.repositories.get(path),
                Some(IndexedRepoState::Failed(_))
            ) {
                return;
            }
            true
        } else {
            false
        };
        if !already_registered {
            if let Some(request) = self.lazy_loading.roots.get(path).copied() {
                // The previous consumer hid the panel while this read was in
                // flight. Join it instead of starting another filesystem scan.
                self.lazy_loaded_paths.insert(path.clone(), 1);
                self.lazy_loading.epochs.insert(path.clone(), request);
                self.repositories
                    .insert(path.clone(), IndexedRepoState::Pending);
                return;
            }
        }
        if !already_registered
            && matches!(
                self.repositories.get(path),
                Some(IndexedRepoState::Indexed(_) | IndexedRepoState::Pending)
            )
        {
            return;
        }
        if !already_registered {
            self.lazy_loaded_paths.insert(path.clone(), 1);
        }
        self.repositories
            .insert(path.clone(), IndexedRepoState::Pending);
        let request = self.lazy_loading.next_request();
        self.lazy_loading.roots.insert(path.clone(), request);
        self.lazy_loading.epochs.insert(path.clone(), request);
        let cached = self.lazy_loading.take_cached(path);
        if let Some(entry) = &cached {
            // Keep the bounded cache until the request completes so a second
            // hide/show during the scan does not lose its expanded directories.
            self.lazy_loading.cache(path.clone(), entry.clone());
        }
        let path = path.clone();
        ctx.spawn(
            async move {
                let result = read_root(&path, cached);
                (path, result)
            },
            move |model, (path, result), ctx| {
                // Explicit removal, replacement, or promotion to a git
                // repository supersedes this result. A visibility-only
                // hide/show can still share the original request.
                if model.lazy_loading.roots.get(&path) != Some(&request) {
                    return;
                }
                model.lazy_loading.roots.remove(&path);
                if !model.lazy_loaded_paths.contains_key(&path) {
                    // A hidden panel needs no watcher. Keep only its bounded
                    // snapshot cache, never resurrect the released model entry.
                    if let Ok((entry, _)) = result {
                        model.lazy_loading.cache(path, entry);
                    }
                    return;
                }
                match result {
                    Ok((entry, gitignores)) => {
                        let _ = model.lazy_loading.take_cached(&path);
                        if let Some(watcher) = &model.watcher {
                            let local = path.to_local_path_lossy();
                            let filter =
                                crate::entry::repo_watch_filter(gitignores.clone(), Vec::new());
                            watcher.update(ctx, |watcher, _| {
                                std::mem::drop(watcher.register_path(
                                    &local,
                                    filter,
                                    notify_debouncer_full::notify::RecursiveMode::Recursive,
                                ));
                            });
                        }
                        let mut state = FileTreeState::from_file_tree_entry(entry);
                        state.gitignores = gitignores;
                        model
                            .repositories
                            .insert(path.clone(), IndexedRepoState::Indexed(state));
                        ctx.emit(RepositoryMetadataEvent::RepositoryUpdated { path });
                    }
                    Err(error) => {
                        model
                            .repositories
                            .insert(path.clone(), IndexedRepoState::Failed(error));
                        ctx.emit(RepositoryMetadataEvent::UpdatingRepositoryFailed { path });
                    }
                }
            },
        );
    }

    /// Load one expanded directory without blocking the UI. Requests for the
    /// same directory coalesce. A concurrent model mutation causes a fresh read
    /// instead of overwriting newer watcher data with an old snapshot.
    pub fn request_load_directory(
        &mut self,
        root: &StandardizedPath,
        path: &StandardizedPath,
        ctx: &mut ModelContext<Self>,
    ) {
        let key = (root.clone(), path.clone());
        if self.lazy_loading.directories.contains_key(&key)
            || self.lazy_loading.errors.contains_key(&key)
        {
            return;
        }
        let Some(IndexedRepoState::Indexed(state)) = self.repositories.get(root) else {
            return;
        };
        if !matches!(state.entry.get(path), Some(FileTreeEntryState::Directory(dir)) if !dir.loaded)
        {
            return;
        }
        let original = state.entry.clone();
        let mut gitignores = state.gitignores.clone();
        let ignored = original.ignored(path);
        let request = self.lazy_loading.next_request();
        self.lazy_loading.directories.insert(key.clone(), request);
        let path = path.clone();
        ctx.spawn(
            async move {
                let mut entry = Entry::Directory(crate::entry::DirectoryEntry {
                    path: path.clone(),
                    children: Vec::new(),
                    ignored,
                    loaded: false,
                });
                let result = entry.load(&mut gitignores).map(|()| {
                    let mut updated = original.clone();
                    updated.insert_entry_at_path(Arc::new(path), entry);
                    updated
                });
                (original, result)
            },
            move |model, (original, result), ctx| {
                if model.lazy_loading.directories.get(&key) != Some(&request) {
                    return;
                }
                model.lazy_loading.directories.remove(&key);
                let (root, path) = &key;
                let Some(IndexedRepoState::Indexed(state)) = model.repositories.get_mut(root)
                else {
                    return;
                };
                if !state.entry.shares_storage_with(&original) {
                    model.request_load_directory(root, path, ctx);
                    return;
                }
                match result {
                    Ok(entry) => state.entry = entry,
                    Err(error) => {
                        model
                            .lazy_loading
                            .errors
                            .insert(key.clone(), error.to_string());
                    }
                }
                ctx.emit(RepositoryMetadataEvent::FileTreeEntryUpdated { path: root.clone() });
            },
        );
    }

    pub fn directory_load_error(
        &self,
        root: &StandardizedPath,
        path: &StandardizedPath,
    ) -> Option<&str> {
        self.lazy_loading
            .errors
            .get(&(root.clone(), path.clone()))
            .map(String::as_str)
    }

    pub fn clear_directory_load_errors(&mut self, root: &StandardizedPath) {
        self.lazy_loading.errors.retain(|(path, _), _| path != root);
    }

    pub(super) fn release_pending_lazy_root(&mut self, path: &StandardizedPath) -> bool {
        if self.lazy_loading.roots.contains_key(path) {
            self.lazy_loading.epochs.remove(path);
            self.repositories.remove(path);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
#[path = "local_loading_tests.rs"]
mod tests;
