//! twarp 07: the last-used Claude session settings, kept in memory and mirrored
//! to the single `claude_session_defaults` SQLite row.
//!
//! A freshly opened Claude pane reads these so it inherits the PREVIOUS
//! session's settings (model / effort / permission mode) instead of re-deriving
//! them from the user's `claude` shell alias every time. The alias is consulted
//! only to *seed* this store the first time — while [`is_seeded`] is still
//! false; afterwards the stored values are authoritative and per-invocation
//! typed flags (e.g. `claude --model haiku`) override and become the new
//! last-used.
//!
//! [`is_seeded`]: ClaudeSessionDefaultsModel::is_seeded

use claude_code::driver::PermissionMode;
use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::{ModelEvent, PersistedClaudeSessionDefaults};
use crate::GlobalResourceHandlesProvider;

/// The recorded last-used settings (`None` for a field the session didn't pin).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ClaudeSessionDefaults {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
}

pub struct ClaudeSessionDefaultsModel {
    /// `None` until a session has been recorded — the bootstrap window during
    /// which callers fall back to the `claude` alias.
    current: Option<ClaudeSessionDefaults>,
}

impl ClaudeSessionDefaultsModel {
    pub fn new(persisted: Option<PersistedClaudeSessionDefaults>) -> Self {
        Self {
            current: persisted.map(|p| ClaudeSessionDefaults {
                model: p.model,
                effort: p.effort,
                permission_mode: p
                    .permission_mode
                    .as_deref()
                    .and_then(PermissionMode::from_cli_arg),
            }),
        }
    }

    /// Whether any session has been recorded yet. While `false`, callers should
    /// bootstrap defaults from the user's `claude` alias.
    pub fn is_seeded(&self) -> bool {
        self.current.is_some()
    }

    /// The last-used settings, or `None` if nothing has been recorded yet.
    pub fn get(&self) -> Option<&ClaudeSessionDefaults> {
        self.current.as_ref()
    }

    /// Record the settings a session was (re)spawned with as the new last-used,
    /// persisting them. A no-op when nothing changed, so repeated pane opens
    /// don't churn the writer thread.
    pub fn record(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: PermissionMode,
        ctx: &mut ModelContext<Self>,
    ) {
        let next = ClaudeSessionDefaults {
            model,
            effort,
            permission_mode: Some(permission_mode),
        };
        if self.current.as_ref() == Some(&next) {
            return;
        }
        self.current = Some(next.clone());

        let handles = GlobalResourceHandlesProvider::as_ref(ctx).get();
        if let Some(sender) = &handles.model_event_sender {
            let event = ModelEvent::UpsertClaudeSessionDefaults {
                defaults: PersistedClaudeSessionDefaults {
                    model: next.model,
                    effort: next.effort,
                    permission_mode: next.permission_mode.map(|m| m.as_cli_arg().to_owned()),
                },
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to persist claude session defaults: {err}");
            }
        }
    }
}

impl Entity for ClaudeSessionDefaultsModel {
    type Event = ();
}

impl SingletonEntity for ClaudeSessionDefaultsModel {}
