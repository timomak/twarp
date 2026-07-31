use std::sync::mpsc::SyncSender;
use std::{
    collections::{hash_map::Entry, HashMap},
    path::PathBuf,
};

use chrono::Utc;
use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::{model::Project, ModelEvent};

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

pub struct ProjectManagementModel {
    projects: HashMap<PathBuf, Project>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
}

impl Entity for ProjectManagementModel {
    type Event = ProjectEvent;
}

impl SingletonEntity for ProjectManagementModel {}

fn project_identity(path: PathBuf) -> PathBuf {
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
        _ctx: &mut ModelContext<Self>,
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

        Self {
            projects,
            model_event_sender,
        }
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
