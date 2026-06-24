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
use warpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

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
        let view = ctx.add_typed_action_view(move |ctx| {
            ClaudeCodeView::new(launch, cwd, Some(resume), ctx)
        });
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
                // twarp 07 (7i, PRODUCT §39): the pane group creates a real
                // terminal session (it owns the session resources) and hands
                // it to the view, which embeds it in place of the chat — the
                // pane itself never changes, so the layout can't (PRODUCT
                // §39: same tab position).
                ClaudeCodeViewEvent::SwapToRawCli {
                    session_id,
                    cwd,
                    claude_binary,
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
                        // path also sidesteps shell aliases entirely, so raw
                        // mode runs the vanilla CLI (§43). `claude_binary` is
                        // resolved by the view against the login-shell PATH —
                        // the pane only sees the launchd-minimal process PATH
                        // under a GUI launch, where a bare-`claude` fallback
                        // would be eaten by the trigger and never start (the
                        // release-only "Raw CLI shows an empty terminal" bug).
                        // Session ids are UUIDs (shell-safe); `exec` makes the
                        // PTY *be* the CLI, so the CLI exiting ends the session
                        // — the §44 auto-return signal.
                        let claude = claude_binary.clone();
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
                            .and_then(|cwd| {
                                claude_code::sessions::session_file(&cwd, session_id)
                            })
                            .is_some_and(|path| path.exists());
                        let session_arg = if persisted {
                            format!("--resume {session_id}")
                        } else {
                            format!("--session-id {session_id}")
                        };
                        terminal.set_pending_command(
                            &format!("exec '{claude}' {session_arg}"),
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

    fn snapshot(&self, _app: &AppContext) -> LeafContents {
        // Non-persisted (see `LeafContents::is_persisted`): a restored pane
        // can't replay a live `claude` process, and twarp keeps no transcript
        // store. Session restore is `claude --resume` from the 7h session list.
        LeafContents::ClaudeCode
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        if self.view.is_self_or_child_focused(ctx) {
            return true;
        }
        // #14/#13: the focused inner view (the raw-CLI terminal, or in chat
        // mode the message editor) can be missed by the layout-ancestor check
        // above right after the pane opens, before the first layout establishes
        // the chain up to this pane. Check those handles directly
        // (layout-timing-independent) so pane activation / Cmd+W / maximize
        // target this pane whenever its chat or CLI is focused.
        let view = self.claude_code_view(ctx);
        let raw_cli_view = view.as_ref(ctx).raw_cli_view();
        if let Some(terminal) = raw_cli_view {
            if terminal.is_self_or_child_focused(ctx) {
                return true;
            }
        }
        let editor = view.as_ref(ctx).input_editor_view();
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
