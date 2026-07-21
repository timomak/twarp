//! twarp: sync palette data source over `claude`'s stored sessions for the
//! active working directory. Listing is a single `read_dir` plus a bounded
//! head-read per `.jsonl`, cheap enough to run synchronously per keystroke
//! (same tradeoff as `RepoDataSource`).

use std::path::PathBuf;

use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use itertools::Itertools;
use twarpui::{AppContext, Entity, SingletonEntity};

use super::ClaudeSessionSearchItem;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

/// Cap sessions considered per query — the store is per-cwd and small in
/// practice, this just bounds pathological directories.
const MAX_SESSIONS_CONSIDERED: usize = 50;

pub struct ClaudeSessionsDataSource {}

impl Default for ClaudeSessionsDataSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeSessionsDataSource {
    pub fn new() -> Self {
        Self {}
    }

    /// The working directory sessions are scoped to: the active pane's local
    /// path, same source the file search's current-folder mode uses.
    pub fn active_cwd(app: &AppContext) -> Option<PathBuf> {
        #[cfg(feature = "local_fs")]
        {
            use crate::workspace::ActiveSession;
            let active_window_id = app.windows().state().active_window;
            active_window_id
                .and_then(|window_id| ActiveSession::as_ref(app).path_if_local(window_id))
                .map(|path| path.to_path_buf())
        }
        #[cfg(not(feature = "local_fs"))]
        {
            let _ = app;
            None
        }
    }

    /// Builds palette items for `cwd`'s stored sessions matching `query_str`
    /// (empty query → all, recent-first as `list_sessions` returns them).
    pub fn matching_items(cwd: &PathBuf, query_str: &str) -> Vec<ClaudeSessionSearchItem> {
        claude_code::sessions::list_sessions(cwd)
            .into_iter()
            .take(MAX_SESSIONS_CONSIDERED)
            .filter_map(|session| {
                let match_result = if query_str.is_empty() {
                    Some(FuzzyMatchResult::no_match())
                } else {
                    match_indices_case_insensitive(session.title.as_str(), query_str)
                };
                match_result.map(|match_result| ClaudeSessionSearchItem {
                    session_id: session.id,
                    title: session.title,
                    timestamp: session.timestamp,
                    jsonl_path: session.jsonl_path,
                    cwd: cwd.clone(),
                    provider: session.provider,
                    match_result,
                })
            })
            .collect_vec()
    }
}

impl Entity for ClaudeSessionsDataSource {
    type Event = ();
}

impl SyncDataSource for ClaudeSessionsDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let Some(cwd) = Self::active_cwd(app) else {
            return Ok(vec![]);
        };

        Ok(Self::matching_items(&cwd, query.text.as_str())
            .into_iter()
            .map(QueryResult::from)
            .collect_vec())
    }
}
