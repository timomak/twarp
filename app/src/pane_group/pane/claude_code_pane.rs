//! [`PaneContent`] wrapper that hosts [`ClaudeCodeView`] as a first-class
//! main-content pane (roadmap feature 07, sub-phase 7b).
//!
//! Modeled on [`NetworkLogPane`](super::network_log_pane) — the simplest
//! non-persisted pane: it wraps the view in a [`PaneView`], forwards the view's
//! [`PaneEvent`]s to the [`PaneGroup`], and snapshots to a non-persisted
//! [`LeafContents::ClaudeCode`] (twarp keeps no transcript store; live history
//! comes from `claude --resume`, wired in 7h). Unlike `NetworkLogPane` there is
//! no pane manager: a Claude Code pane is opened on demand by the `claude`
//! terminal trigger and has no global registry.

use std::path::PathBuf;

use claude_code::launch::LaunchOptions;
use twarpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

use crate::app_state::LeafContents;
use crate::claude_code_view::{ClaudeCodeView, ClaudeCodeViewEvent, ResumeSession};

use super::{
    view::PaneView, DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};

pub struct ClaudeCodePane {
    view: ViewHandle<PaneView<ClaudeCodeView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl ClaudeCodePane {
    pub fn from_view(claude_code_view: ViewHandle<ClaudeCodeView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = claude_code_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(claude_code_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_claude_code_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                claude_code_view,
                (),
                pane_configuration.clone(),
                ctx,
            )
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// Open a fresh Claude Code pane. `launch` is the parsed `claude [flags]
    /// [prompt]` invocation (PRODUCT §2 — recognized flags map onto the spawn
    /// options, the positional seeds the first turn); `cwd` is the originating
    /// terminal's directory (PRODUCT §4).
    pub fn new<V: View>(
        launch: LaunchOptions,
        cwd: Option<PathBuf>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view =
            ctx.add_typed_action_view(move |ctx| ClaudeCodeView::new(launch, cwd, None, ctx));
        Self::from_view(view, ctx)
    }

    /// Reopen a stored session (PRODUCT §36, 7h): the pane renders the
    /// session's on-disk history and continues it live via `claude --resume`
    /// on the next message.
    pub fn new_resume<V: View>(
        resume: ResumeSession,
        launch: LaunchOptions,
        cwd: Option<PathBuf>,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view = ctx
            .add_typed_action_view(move |ctx| ClaudeCodeView::new(launch, cwd, Some(resume), ctx));
        Self::from_view(view, ctx)
    }

    pub fn claude_code_view(&self, ctx: &AppContext) -> ViewHandle<ClaudeCodeView> {
        self.view.as_ref(ctx).child(ctx)
    }

    /// The pane's working directory — the cwd of the terminal that opened it
    /// (PRODUCT §4). Lets the pane group treat this pane like a terminal
    /// session for directory context (split inheritance, Open Changes roots).
    pub fn cwd(&self, ctx: &AppContext) -> Option<PathBuf> {
        self.claude_code_view(ctx).as_ref(ctx).cwd().cloned()
    }
}

impl PaneContent for ClaudeCodePane {
    fn id(&self) -> PaneId {
        PaneId::from_claude_code_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let claude_code_view = self.claude_code_view(ctx);
        let pane_id = self.id();

        let claude_view_for_raw_cli = claude_code_view.clone();
        ctx.subscribe_to_view(&claude_code_view, move |pane_group, _, event, ctx| {
            match event {
                ClaudeCodeViewEvent::Pane(pane_event) => {
                    pane_group.handle_pane_event(pane_id, pane_event, ctx)
                }
                // twarp 07 (7p): the tab status dot reads this view's session
                // state, but the tab bar lives in the workspace — bubble a
                // repaint request the same way terminal state changes do.
                ClaudeCodeViewEvent::TabStatusChanged => {
                    ctx.emit(crate::pane_group::Event::TerminalViewStateChanged)
                }
                // twarp 07 (7i, PRODUCT §39): the pane group creates a real
                // terminal session (it owns the session resources) and hands
                // it to the view, which embeds it in place of the chat — the
                // pane itself never changes, so the layout can't (PRODUCT
                // §39: same tab position).
                ClaudeCodeViewEvent::SwapToRawCli {
                    session_id,
                    cwd,
                    claude_binary,
                    flags,
                } => {
                    let (manager, terminal) =
                        pane_group.create_raw_claude_terminal(cwd.clone(), ctx);
                    terminal.update(ctx, |terminal, ctx| {
                        // #6: no floating "← Claude Code" overlay — the header's
                        // Chat UI / Raw CLI section toggle is the way back.
                        // Launch by ABSOLUTE path: the `claude`-at-submit
                        // trigger peels `exec` as an alias wrapper and would
                        // intercept a bare `claude` here — opening a second
                        // Claude pane instead of running the CLI (the 7i
                        // "terminal + duplicate pane" bug). A path token is
                        // never intercepted (PRODUCT §3), and a quoted full
                        // path also sidesteps shell aliases entirely — so the
                        // alias's *default flags* (effort / model / permission
                        // mode) are re-applied explicitly via `flags`, kept in
                        // lockstep with the chat UI's headless spawn (§43;
                        // otherwise the CLI falls back to its own config
                        // defaults, e.g. xhigh effort over the alias's medium).
                        // `claude_binary` is
                        // resolved by the view against the login-shell PATH —
                        // the pane only sees the launchd-minimal process PATH
                        // under a GUI launch, where a bare-`claude` fallback
                        // would be eaten by the trigger and never start (the
                        // release-only "Raw CLI shows an empty terminal" bug).
                        // Session ids are UUIDs (shell-safe); `exec` makes the
                        // PTY *be* the CLI, so the CLI exiting ends the session
                        // — the §44 auto-return signal.
                        let claude = claude_binary.clone();
                        let flags = flags.clone();
                        // `--resume <id>` only when the session is actually on
                        // disk. A fresh pane (or one whose first turn hasn't
                        // completed) has no `.jsonl` yet, so `--resume` would
                        // fail instantly and the CLI's immediate exit bounces
                        // straight back to the chat — the "View CLI breaks and
                        // reverts to UI" report. In that case start a fresh
                        // interactive session pinned to the pane's own id
                        // (`--session-id`, a real flag) so returning still
                        // re-reads the right history (§44).
                        let persisted = cwd
                            .clone()
                            .or_else(|| std::env::current_dir().ok())
                            .and_then(|cwd| claude_code::sessions::session_file(&cwd, session_id))
                            .is_some_and(|path| path.exists());
                        let session_arg = if persisted {
                            format!("--resume {session_id}")
                        } else {
                            format!("--session-id {session_id}")
                        };
                        terminal.set_pending_command(
                            &format!("exec '{claude}' {flags} {session_arg}"),
                            ctx,
                        );
                    });
                    claude_view_for_raw_cli.update(ctx, |view, ctx| {
                        view.enter_raw_mode(manager, terminal, ctx);
                    });
                    // #14: make this pane the active one as raw mode opens. The
                    // embedded terminal's focus-change can't reliably reach the
                    // pane group before its first layout, so set it explicitly —
                    // otherwise Cmd+W would still target the previously focused
                    // pane.
                    pane_group.focus_pane_by_id(pane_id, ctx);
                }
                // twarp: "Fork conversation" — the view has written the branch
                // file; open it as a resumed session in a fresh split to the
                // right, inheriting the parent pane's launch settings and cwd.
                // The forking pane and its session are untouched.
                ClaudeCodeViewEvent::ForkSession {
                    resume,
                    launch,
                    cwd,
                } => {
                    let pane = ClaudeCodePane::new_resume(
                        resume.clone(),
                        launch.clone(),
                        cwd.clone(),
                        ctx,
                    );
                    pane_group.add_pane_with_direction(
                        crate::pane_group::Direction::Right,
                        pane,
                        true, /* focus_new_pane */
                        ctx,
                    );
                }
            }
        });
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // No manager to deregister from; just drop the subscriptions. Closing
        // the pane drops `ClaudeCodeView`, which (in 7c) kills the live
        // `claude` process via `kill_on_drop`.
        let claude_code_view = self.claude_code_view(ctx);
        ctx.unsubscribe_to_view(&claude_code_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        // twarp 07: persist enough to `claude --resume` this conversation on
        // next launch (see `LeafContents::ClaudeCode`). We don't replay the live
        // process — restoration reads the `.jsonl` `claude` already wrote and
        // respawns lazily on the next message. Only record a `session_id` once
        // that file actually exists (mirrors the raw-CLI `--resume` guard): a
        // zero-state pane has a pinned id but no on-disk session, so persisting
        // it would restore an empty pane. Such a pane reports `session_id: None`
        // and is filtered by `is_persisted`.
        let view = self.claude_code_view(app);
        let view = view.as_ref(app);
        let cwd = view.cwd().cloned();
        let session_id = view.session_id().to_owned();
        let has_session = cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .and_then(|cwd| claude_code::sessions::session_file(&cwd, &session_id))
            .is_some_and(|path| path.exists());
        LeafContents::ClaudeCode(crate::app_state::ClaudeCodePaneSnapshot {
            session_id: has_session.then_some(session_id),
            cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
            provider: claude_code::driver::AgentProvider::Claude,
        })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        // #14/#13: the focused inner view (the raw-CLI terminal, or in chat
        // mode the message editor) can be missed by the layout-ancestor check
        // right after the pane opens — or, the bug this guards against, right
        // after a `Split` creates a *second* Claude pane: before the first
        // layout establishes the chain up to this pane, the layout-ancestor
        // walk (`is_self_or_child_focused` → `check_view_or_child_focused`,
        // which traverses the *presenter* tree) returns a stale answer. When
        // the global `HandleFocusChange` rescan asks every pane "are you
        // focused?" against that stale tree, none answers yes, so the
        // focused-pane state is left pointing at the *previously* focused Claude
        // pane. A later focus reconciliation (window reactivation,
        // `focus_active_tab`) then yanks UI focus back into that pane's
        // composer — the "click the other pane, it focuses for a split second,
        // then snaps back to the Claude input" report, and it's intermittent
        // because it hinges on whether layout has caught up to focus yet.
        //
        // Resolve it against the live window focus (`focused_view_id`, updated
        // synchronously in `App::focus`, independent of layout) by comparing the
        // pane's own view-handle ids directly. This is what the old comment
        // *claimed* to do but didn't — the direct handles were still queried via
        // the presenter-based `is_self_or_child_focused`.
        let view = self.claude_code_view(ctx);
        let raw_cli_view = view.as_ref(ctx).raw_cli_view();
        let editor = view.as_ref(ctx).input_editor_view();

        if let Some(focused) = ctx.focused_view_id(ctx.window_id()) {
            if focused == editor.id() || raw_cli_view.as_ref().is_some_and(|t| focused == t.id()) {
                return true;
            }
        }

        // Fall back to the layout-ancestor walk for the case the direct id
        // check can't see: focus landed on a *descendant* of the editor or
        // raw-CLI terminal (a popup/child view), whose id we don't hold.
        if self.view.is_self_or_child_focused(ctx) {
            return true;
        }
        if let Some(terminal) = raw_cli_view {
            if terminal.is_self_or_child_focused(ctx) {
                return true;
            }
        }
        editor.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.claude_code_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
