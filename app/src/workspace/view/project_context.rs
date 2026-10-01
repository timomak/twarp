//! Local tool context is separate from the project identity used to group chats.
use super::*;

#[derive(Default)]
pub(super) struct ProjectContextState {
    /// Restoration activates tabs while building them, before the saved focus is applied.
    pub(super) restoring_tabs: bool,
    last_project: Option<PathBuf>,
    recovery_original: Option<PathBuf>,
    observed_pr_stores: HashSet<EntityId>,
    /// None means a background directory check is in flight.
    directory_checks: HashMap<PathBuf, Option<Result<(), String>>>,
    canonical_directories: HashMap<PathBuf, PathBuf>,
    directory_check_generation: u64,
}

impl ProjectContextState {
    fn begin_directory_check(&mut self, path: &Path) -> Option<u64> {
        if self.directory_checks.contains_key(path) {
            return None;
        }
        self.directory_checks.insert(path.to_path_buf(), None);
        Some(self.directory_check_generation)
    }

    fn finish_directory_check(
        &mut self,
        path: PathBuf,
        generation: u64,
        result: Result<(), String>,
    ) -> bool {
        if generation != self.directory_check_generation {
            return false;
        }
        self.directory_checks.insert(path, Some(result));
        true
    }

    fn invalidate_directory_checks(&mut self) {
        self.directory_check_generation = self.directory_check_generation.wrapping_add(1);
        self.directory_checks.clear();
        self.canonical_directories.clear();
    }
}

fn requires_context_override(
    global: bool,
    assigned: bool,
    relocated: bool,
    unavailable: bool,
) -> bool {
    global || assigned || relocated || unavailable
}

/// An assigned folder is authoritative, even when unavailable. Otherwise use
/// the focused local session, or the sole unambiguous local root.
fn choose_project(
    assigned: Option<PathBuf>,
    focused: Option<PathBuf>,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    assigned
        .or(focused)
        .or_else(|| (roots.len() == 1).then(|| roots[0].clone()))
}

impl Workspace {
    /// Inspect the real workspace wiring without reconciling or changing it.
    #[cfg(all(feature = "integration_tests", feature = "local_fs"))]
    pub fn project_context_test_state(
        &self,
        app: &AppContext,
    ) -> (
        Option<PathBuf>,
        Vec<PathBuf>,
        Option<ModelHandle<crate::pull_requests::PullRequestsStoreModel>>,
    ) {
        let group = self.active_tab_pane_group();
        let directories = self
            .working_directories_model
            .as_ref(app)
            .most_recent_directories_for_pane_group(group.id())
            .map(|directories| directories.map(|directory| directory.path).collect())
            .unwrap_or_default();
        let store = self
            .tabs
            .iter()
            .find_map(|tab| tab.pane_group.as_ref(app).pull_requests_store(app));
        (
            self.tabs[self.active_tab_index].project_root.clone(),
            directories,
            store,
        )
    }

    /// Build a synthetic saved window through the production restoration loop.
    /// The inert provider panes never launch a child process.
    #[cfg(all(feature = "integration_tests", feature = "local_fs"))]
    pub fn restore_project_context_test_window(
        &self,
        roots: Vec<PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) -> WindowId {
        let mut snapshot = self.snapshot(ctx.window_id(), false, ctx);
        let tab = |contents, project_root| TabSnapshot {
            custom_title: None,
            project_root,
            project_root_initialized: true,
            root: PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents,
            }),
            default_directory_color: None,
            selected_color: Default::default(),
            left_panel: None,
            right_panel: None,
        };
        snapshot.tabs = vec![tab(
            LeafContents::Automation(AutomationPage::PullRequests),
            None,
        )];
        snapshot
            .tabs
            .extend(roots.into_iter().enumerate().map(|(index, root)| {
                tab(
                    LeafContents::ClaudeCode(crate::app_state::ClaudeCodePaneSnapshot {
                        session_id: Some(format!("project-context-fixture-{index}")),
                        history_path: None,
                        cwd: Some(root.to_string_lossy().into_owned()),
                        provider: claude_code::driver::AgentProvider::Claude,
                        spawn_origin: None,
                    }),
                    Some(root),
                )
            }));
        snapshot.active_tab_index = 0;
        crate::root_view::open_new_with_workspace_source(
            NewWorkspaceSource::Restored {
                window_snapshot: snapshot,
                block_lists: Arc::new(HashMap::new()),
            },
            ctx,
        )
        .0
    }

    #[cfg(feature = "local_fs")]
    fn session_project_context(
        &self,
        tab: &TabData,
        ctx: &AppContext,
    ) -> (Option<PathBuf>, Vec<PathBuf>) {
        let group = tab.pane_group.as_ref(ctx);
        let focused = group
            .focused_claude_code_view_id(ctx)
            .or_else(|| group.focused_session_view(ctx).map(|view| view.id()));
        // Keep missing folders as recovery candidates. The usual terminal
        // directory accessor probes is_dir(), which both drops them and blocks
        // the UI once per terminal while reconciling the project picker.
        let directories: Vec<_> = group
            .terminal_views(ctx)
            .into_iter()
            .filter_map(|view| {
                let terminal = view.as_ref(ctx);
                if terminal.active_session_is_local(ctx) != Some(true)
                    || terminal.has_pending_ssh_command()
                {
                    return None;
                }
                let session = terminal
                    .active_block_session_id()
                    .and_then(|id| terminal.sessions_model().as_ref(ctx).get(id))?;
                if session.is_wsl() {
                    return None;
                }
                let path = session
                    .launch_data()?
                    .maybe_convert_absolute_path(&terminal.pwd()?)?;
                Some((view.id(), path))
            })
            .chain(
                group
                    .claude_code_view_cwds(ctx)
                    .filter_map(|(id, path)| path.map(|path| (id, PathBuf::from(path)))),
            )
            .collect();
        let mut roots = Vec::new();
        for (_, path) in &directories {
            if !roots.contains(path) {
                roots.push(path.clone());
            }
        }
        // File-only tabs also carry useful local context, without a shell.
        for (_, path) in group
            .code_view_local_paths(ctx)
            .chain(group.code_diff_view_local_paths(ctx))
            .chain(group.file_notebook_local_paths(ctx))
        {
            if let Some(path) =
                path.and_then(|path| PathBuf::from(path).parent().map(Path::to_path_buf))
            {
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
        }
        let focused = directories
            .iter()
            .find(|(id, _)| Some(*id) == focused)
            .map(|(_, path)| path.clone());
        (
            choose_project(tab.project_root.clone(), focused, &roots),
            roots,
        )
    }

    #[cfg(feature = "local_fs")]
    fn local_tool_directory(&self, original: &Path, ctx: &AppContext) -> PathBuf {
        let local = ProjectManagementModel::as_ref(ctx).resolve_directory(original);
        // Validate the actual assigned/session path before collapsing it to an
        // ancestor repository, so a missing subfolder cannot silently disappear.
        let Some(canonical) = self.project_context.canonical_directories.get(&local) else {
            return local;
        };
        DetectedRepositories::as_ref(ctx)
            .get_cached_root_for_path(canonical)
            .unwrap_or_else(|| canonical.clone())
    }

    /// Reconcile all existing PR pages, including restored pages, without
    /// replacing an explicit picker choice or moving a session between projects.
    pub(super) fn sync_project_context(&mut self, refresh: bool, ctx: &mut ViewContext<Self>) {
        if self.project_context.restoring_tabs {
            return;
        }
        #[cfg(feature = "local_fs")]
        self.sync_local_project_context(refresh, ctx);
        #[cfg(not(feature = "local_fs"))]
        let _ = (refresh, ctx);
    }

    #[cfg(feature = "local_fs")]
    fn sync_local_project_context(&mut self, refresh: bool, ctx: &mut ViewContext<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab_index) else {
            return;
        };
        let assigned = tab.project_root.is_some();
        let group = tab.pane_group.clone();
        let global =
            group.as_ref(ctx).has_automation_panes() || group.as_ref(ctx).has_settings_panes();
        let active_store = group.as_ref(ctx).pull_requests_store(ctx);
        let (session_choice, _) = self.session_project_context(tab, ctx);
        if !global {
            // Do not carry a previous session's folder into a rootless session.
            self.project_context.last_project = session_choice.clone();
        }
        let preferred_original =
            session_choice.or_else(|| self.project_context.last_project.clone());
        let mut originals = Vec::new();
        for tab in &self.tabs {
            let (preferred, roots) = self.session_project_context(tab, ctx);
            originals.extend(preferred);
            originals.extend(roots);
        }
        originals.extend(ProjectManagementModel::as_ref(ctx).project_paths_by_recency());
        originals.extend(preferred_original.clone());
        let stores: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| tab.pane_group.as_ref(ctx).pull_requests_store(ctx))
            .collect();
        // Keep a picker selection available if its session was closed. A saved
        // replacement redirects that selection on the next reconciliation.
        for store in &stores {
            originals.extend(store.as_ref(ctx).selected_repo().map(Path::to_path_buf));
        }
        let mut candidates = Vec::new();
        let mut candidate_originals = HashMap::new();
        for original in originals {
            let local = self.local_tool_directory(&original, ctx);
            candidate_originals.entry(local.clone()).or_insert(original);
            if !candidates.contains(&local) {
                candidates.push(local);
            }
        }
        for store in stores {
            if self.project_context.observed_pr_stores.insert(store.id()) {
                ctx.subscribe_to_model(&store, |me, _, event, ctx| {
                    if matches!(
                        event,
                        crate::pull_requests::PullRequestsEvent::SelectionChanged(_)
                    ) {
                        me.sync_project_context(false, ctx);
                    }
                });
            }
            let selection = store.as_ref(ctx).selected_repo().map(Path::to_path_buf);
            let current = selection
                .as_ref()
                .map(|path| self.local_tool_directory(path, ctx));
            let contextual = preferred_original
                .as_ref()
                .map(|path| self.local_tool_directory(path, ctx));
            let preferred = if store.as_ref(ctx).has_explicit_selection() {
                current.or(contextual)
            } else {
                contextual.or(current)
            };
            store.update(ctx, |store, ctx| {
                store.set_projects(candidates.clone(), preferred, refresh, ctx)
            });
        }
        let selected = if let Some(store) = active_store {
            store.as_ref(ctx).selected_repo().map(Path::to_path_buf)
        } else if global {
            self.project_context
                .last_project
                .as_ref()
                .map(|path| self.local_tool_directory(path, ctx))
        } else {
            preferred_original
                .as_ref()
                .map(|path| self.local_tool_directory(path, ctx))
        };
        let original = selected.as_ref().map(|path| {
            candidate_originals
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.clone())
        });
        if global && selected.is_some() {
            self.project_context.last_project = original.clone();
        }
        let relocated = original.as_ref().is_some_and(|path| {
            ProjectManagementModel::as_ref(ctx).resolve_directory(path) != *path
        });
        self.project_context.recovery_original = original;

        // Preserve the remote and pending-SSH state machine. A remote session's
        // path must never be interpreted as a folder on this machine.
        let native_session_context =
            group
                .as_ref(ctx)
                .active_session_view(ctx)
                .is_some_and(|terminal| {
                    let terminal = terminal.as_ref(ctx);
                    terminal.active_session_is_local(ctx) == Some(false)
                        || terminal.has_pending_ssh_command()
                        || terminal
                            .active_block_session_id()
                            .and_then(|id| terminal.sessions_model().as_ref(ctx).get(id))
                            .is_some_and(|session| session.is_wsl())
                });
        if native_session_context && !global {
            self.working_directories_model.update(ctx, |model, ctx| {
                model.set_project_context(group.id(), None, ctx)
            });
            self.left_panel_view
                .update(ctx, |panel, ctx| panel.set_project_context(None, None, ctx));
            return;
        }

        if let Some(path) = &selected {
            self.check_project_directory(path.clone(), ctx);
        }
        let error = selected
            .as_ref()
            .and_then(|path| self.project_context.directory_checks.get(path))
            .and_then(|result| result.as_ref())
            .and_then(|result| result.as_ref().err())
            .cloned();
        // A chosen folder being checked is loading, not an empty selection.
        // The explorer's own background load reports filesystem failures too.
        let available = selected.clone().filter(|_| error.is_none());
        if !requires_context_override(global, assigned, relocated, error.is_some()) {
            // Keep every native root and the session's existing enablement for
            // ordinary unassigned tabs, including split panes in different repos.
            self.working_directories_model.update(ctx, |model, ctx| {
                model.set_project_context(group.id(), None, ctx);
            });
            self.left_panel_view.update(ctx, |panel, ctx| {
                panel.set_project_context(None, None, ctx);
            });
            return;
        }
        let repository = available
            .as_ref()
            .filter(|path| {
                matches!(
                    self.project_context.directory_checks.get(*path),
                    Some(Some(Ok(())))
                )
            })
            .and_then(|path| DetectedRepositories::as_ref(ctx).get_cached_root_for_path(path));
        self.working_directories_model.update(ctx, |model, ctx| {
            model.set_project_context(group.id(), Some((available, repository)), ctx);
        });
        let label = selected.map(|path| path.display().to_string());
        self.left_panel_view.update(ctx, |panel, ctx| {
            panel.update_coding_panel_enablement(CodingPanelEnablementState::Enabled, ctx);
            panel.set_project_context(label, error, ctx);
        });
    }

    #[cfg(feature = "local_fs")]
    fn check_project_directory(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let Some(generation) = self.project_context.begin_directory_check(&path) else {
            return;
        };
        let checked = path.clone();
        ctx.spawn(
            async move {
                dunce::canonicalize(&checked)
                    .and_then(|canonical| {
                        std::fs::read_dir(&canonical)?;
                        Ok(canonical)
                    })
                    .map_err(|error| {
                        format!(
                        "Cannot open {}: {error}. Locate its folder on this computer, or retry.",
                        checked.display()
                    )
                    })
            },
            move |me, result, ctx| {
                let canonical = result.as_ref().ok().cloned();
                if !me.project_context.finish_directory_check(
                    path.clone(),
                    generation,
                    result.map(|_| ()),
                ) {
                    return;
                }
                if let Some(canonical) = &canonical {
                    me.project_context
                        .canonical_directories
                        .insert(path.clone(), canonical.clone());
                }
                me.sync_project_context(false, ctx);
                if let Some(canonical) = canonical {
                    let future = DetectedRepositories::handle(ctx).update(ctx, |model, ctx| {
                        model.detect_possible_git_repo(
                            &canonical.to_string_lossy(),
                            RepoDetectionSource::TerminalNavigation,
                            ctx,
                        )
                    });
                    ctx.spawn(future, |me, _, ctx| me.sync_project_context(false, ctx));
                }
            },
        );
    }

    pub(super) fn retry_project_directory(&mut self, ctx: &mut ViewContext<Self>) {
        self.project_context.invalidate_directory_checks();
        self.sync_project_context(true, ctx);
    }

    pub(super) fn locate_project_directory(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        {
            let Some(original) = self.project_context.recovery_original.clone() else {
                self.open_project_folder_picker(ctx);
                return;
            };
            ctx.open_file_picker(
                move |result, ctx| {
                    let Ok(paths) = result else {
                        return;
                    };
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    if let Some(handle) = ctx.handle().upgrade(ctx) {
                        handle.update(ctx, |workspace, ctx| {
                            workspace.validate_replacement_directory(original, path.into(), ctx);
                        });
                    }
                },
                FilePickerConfiguration::new().folders_only(),
            );
        }
        #[cfg(not(feature = "local_fs"))]
        let _ = ctx;
    }

    pub(super) fn locate_project_directory_at(
        &mut self,
        path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        self.project_context.recovery_original = Some(path);
        self.locate_project_directory(ctx);
    }

    #[cfg(feature = "local_fs")]
    fn validate_replacement_directory(
        &mut self,
        original: PathBuf,
        replacement: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.spawn(
            async move {
                let path = std::fs::canonicalize(replacement)?;
                std::fs::read_dir(&path)?;
                Ok::<_, std::io::Error>(path)
            },
            move |me, result, ctx| {
                let result = result
                    .map_err(|error| error.to_string())
                    .and_then(|replacement| {
                        ProjectManagementModel::handle(ctx).update(ctx, |projects, ctx| {
                            projects.relocate_directory(original, replacement, ctx)
                        })
                    });
                if let Err(error) = result {
                    me.toast_stack.update(ctx, |toasts, ctx| {
                        toasts.add_ephemeral_toast(DismissibleToast::error(error), ctx);
                    });
                } else {
                    me.retry_project_directory(ctx);
                }
            },
        );
    }
}

#[cfg(test)]
#[path = "project_context_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "project_context_audit_tests.rs"]
mod audit_tests;
