use std::sync::mpsc::SyncSender;
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap, HashSet},
    path::{Component, Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use twarpui::{Entity, ModelContext, SingletonEntity};

pub(crate) use crate::persistence::model::Project;
use crate::persistence::ModelEvent;

#[derive(Debug)]
pub enum ProjectEvent {
    Added {
        #[expect(unused, reason = "TODO(jparker): #pod-code-mode wip")]
        path: PathBuf,
    },
    Removed {
        #[expect(unused, reason = "listeners re-read the model on notify")]
        path: PathBuf,
    },
    Updated {
        #[expect(unused, reason = "listeners re-read the model on notify")]
        path: PathBuf,
    },
}

const DIRECTORY_REPLACEMENTS_KEY: &str = "ProjectDirectoryReplacements";

/// Explicit mappings belong to this machine's private preferences, separate
/// from project identities and provider session/history paths.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct DirectoryReplacements(BTreeMap<PathBuf, PathBuf>);

impl DirectoryReplacements {
    fn resolve(&self, path: &Path) -> Result<PathBuf, String> {
        let mut resolved = path.to_path_buf();
        let mut visited = HashSet::new();
        loop {
            let Some((original, replacement)) = self
                .0
                .iter()
                .filter(|(original, _)| resolved.starts_with(original))
                .max_by_key(|(original, _)| original.components().count())
            else {
                return Ok(resolved);
            };
            if !visited.insert(original) {
                return Err(
                    "Folder replacements cannot form a cycle or contain themselves.".to_owned(),
                );
            }
            let suffix = resolved
                .strip_prefix(original)
                .expect("matched directory prefix");
            resolved = replacement.join(suffix);
        }
    }

    fn validate(&self) -> Result<(), String> {
        for (original, replacement) in &self.0 {
            if !original.is_absolute()
                || !replacement.is_absolute()
                || original
                    .components()
                    .chain(replacement.components())
                    .any(|part| matches!(part, Component::ParentDir))
            {
                return Err(
                    "Folder replacements require absolute paths without parent components."
                        .to_owned(),
                );
            }
            self.resolve(original)?;
        }
        Ok(())
    }

    fn with_relocation(&self, original: PathBuf, replacement: PathBuf) -> Result<Self, String> {
        let mut updated = self.clone();
        if original == replacement {
            updated.0.remove(&original);
        } else {
            updated.0.insert(original, replacement);
        }
        updated.validate()?;
        Ok(updated)
    }
}

pub struct ProjectManagementModel {
    projects: HashMap<PathBuf, Project>,
    directory_replacements: DirectoryReplacements,
    model_event_sender: Option<SyncSender<ModelEvent>>,
}

impl Entity for ProjectManagementModel {
    type Event = ProjectEvent;
}

impl SingletonEntity for ProjectManagementModel {}

pub(crate) fn project_identity(path: PathBuf) -> PathBuf {
    #[cfg(feature = "local_fs")]
    {
        dunce::canonicalize(&path).unwrap_or(path)
    }
    #[cfg(not(feature = "local_fs"))]
    {
        path
    }
}

impl ProjectManagementModel {
    /// Create a new Projects model with persisted data
    pub fn new(
        persisted_projects: Vec<Project>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        log::debug!("Loading {} persisted projects", persisted_projects.len());

        let mut projects = HashMap::new();
        for mut project in persisted_projects {
            let path = project_identity(PathBuf::from(&project.path));
            project.path = path.to_string_lossy().into_owned();
            match projects.entry(path) {
                Entry::Vacant(entry) => {
                    entry.insert(project);
                }
                Entry::Occupied(mut entry)
                    if project.last_used_at() > entry.get().last_used_at() =>
                {
                    entry.insert(project);
                }
                Entry::Occupied(_) => {}
            }
        }

        let directory_replacements = settings::PrivatePreferences::as_ref(ctx)
            .read_value(DIRECTORY_REPLACEMENTS_KEY)
            .map_err(|error| error.to_string())
            .and_then(|value| {
                let replacements: DirectoryReplacements = value
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .unwrap_or_default();
                replacements.validate()?;
                Ok(replacements)
            })
            .unwrap_or_else(|error| {
                log::warn!("Could not load local folder replacements: {error}");
                DirectoryReplacements::default()
            });
        Self {
            projects,
            directory_replacements,
            model_event_sender,
        }
    }

    /// Resolve only explicit replacements. This is lexical and performs no
    /// filesystem work, so it is safe while rendering or reconciling context.
    pub fn resolve_directory(&self, path: &Path) -> PathBuf {
        self.directory_replacements
            .resolve(path)
            .unwrap_or_else(|_| path.to_path_buf())
    }

    /// Remember a user-selected local replacement after the caller validates
    /// it off the UI thread. Project identity and session history are unchanged.
    pub fn relocate_directory(
        &mut self,
        original: PathBuf,
        replacement: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        let updated = self
            .directory_replacements
            .with_relocation(original.clone(), replacement)?;
        let serialized = serde_json::to_string(&updated)
            .map_err(|error| format!("Could not save the folder replacement: {error}"))?;
        settings::PrivatePreferences::as_ref(ctx)
            .write_value(DIRECTORY_REPLACEMENTS_KEY, serialized)
            .map_err(|error| format!("Could not save the folder replacement: {error}"))?;
        self.directory_replacements = updated;
        ctx.emit(ProjectEvent::Updated { path: original });
        ctx.notify();
        Ok(())
    }

    /// Add a project to the list. If it already exists, update the last_opened_ts.
    pub fn upsert_project(&mut self, path: PathBuf, ctx: &mut ModelContext<Self>) {
        let path = project_identity(path);
        let now = Utc::now().naive_utc();

        let project = if let Some(existing_project) = self.projects.get_mut(&path) {
            // Update existing project's last opened time
            existing_project.last_opened_ts = Some(now);
            existing_project.clone()
        } else {
            // Create new project
            let project = Project {
                path: path.to_string_lossy().to_string(),
                added_ts: now,
                last_opened_ts: Some(now),
                name: None,
            };
            self.projects.insert(path.clone(), project.clone());
            project
        };
        self.save_project(project);
        ctx.emit(ProjectEvent::Added { path });
    }

    /// The user-chosen display name for a project, if one was set via
    /// "Rename project" in the sidebar.
    pub fn project_name(&self, path: &PathBuf) -> Option<String> {
        let path = project_identity(path.clone());
        self.projects
            .get(&path)
            .and_then(|project| project.name.clone())
            .filter(|name| !name.trim().is_empty())
    }

    /// Sets (or clears, with `None` / blank) the custom display name for a
    /// project. Upserts the project row so renaming a live project that was
    /// never opened through the library still persists.
    pub fn set_project_name(
        &mut self,
        path: PathBuf,
        name: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let path = project_identity(path);
        let name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        let now = Utc::now().naive_utc();
        let project = match self.projects.get_mut(&path) {
            Some(existing_project) => {
                if existing_project.name == name {
                    return;
                }
                existing_project.name = name;
                existing_project.clone()
            }
            None => {
                let project = Project {
                    path: path.to_string_lossy().to_string(),
                    added_ts: now,
                    last_opened_ts: Some(now),
                    name,
                };
                self.projects.insert(path.clone(), project.clone());
                project
            }
        };
        self.save_project(project);
        ctx.emit(ProjectEvent::Updated { path });
    }

    /// Removes a project from the library and deletes its row from the
    /// database. Sessions under the project are untouched — only the library
    /// entry (and its custom name/metadata) is dropped.
    pub fn remove_project(&mut self, path: PathBuf, ctx: &mut ModelContext<Self>) {
        let path = project_identity(path);
        if self.projects.remove(&path).is_none() {
            return;
        }
        if let Some(sender) = &self.model_event_sender {
            let event = ModelEvent::DeleteProject {
                path: path.to_string_lossy().into_owned(),
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to delete project from database: {err}");
            }
        }
        ctx.emit(ProjectEvent::Removed { path });
    }

    pub fn all_projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    /// Returns project directories in stable most-recently-used order.
    ///
    /// This is the app-wide project library presented by every Projects sidebar.
    /// Cloning the paths keeps view code from holding model borrows while it builds
    /// rows or dispatches actions.
    pub fn project_paths_by_recency(&self) -> Vec<PathBuf> {
        let mut projects: Vec<_> = self.projects.iter().collect();
        projects.sort_by(|(left_path, left), (right_path, right)| {
            right
                .last_used_at()
                .cmp(&left.last_used_at())
                .then_with(|| left_path.cmp(right_path))
        });
        projects.into_iter().map(|(path, _)| path.clone()).collect()
    }

    /// Save a project to the database
    fn save_project(&self, project: Project) {
        if let Some(sender) = &self.model_event_sender {
            let event = ModelEvent::UpsertProject { project };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to save project to database: {err}");
            }
        }
    }
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
