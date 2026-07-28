//! twarp 21b/21c: the native PR detail view, rendered in-page by the Pull
//! Requests surface when a list row is clicked. Tabs: Conversation (markdown
//! description + PR-level comment/review timeline + merge box), Checks
//! (per-check CI rows), and Files (per-file diff cards with inline review
//! threads — [`crate::pull_requests::files_tab`]).
//!
//! Markdown bodies render through the read-only comment markdown editor
//! ([`crate::code::editor::comment_editor::create_readonly_comment_markdown_editor`]),
//! which also makes them selectable. Editor views are rebuilt in [`PrDetailState::sync`]
//! whenever the fetched content changes (render passes only get `&AppContext`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use twarp_core::ui::tokens::{radius, spacing, type_ramp};
use twarp_core::ui::Icon;
use twarpui::{
    elements::{
        new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig},
        Align, Border, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable, MainAxisSize, MouseStateHandle,
        ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
    },
    platform::Cursor,
    prelude::ColorU,
    units::Pixels,
    AppContext, SingletonEntity, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::automation::view::{AutomationView, AutomationViewAction};
use crate::code::editor::comment_editor::{
    create_editable_comment_markdown_editor, create_readonly_comment_markdown_editor,
};
use crate::notebooks::editor::view::RichTextEditorView;
use crate::pull_requests::files_tab::{FilesTabState, FILES_CONTENT_MAX_WIDTH};
use crate::pull_requests::list_page::{render_badge, PullRequestsPageAction, CONTENT_MAX_WIDTH};
use crate::pull_requests::review::{build_review_payload, PrReviewEvent};
use crate::pull_requests::store::{
    relative_updated_at, PrCheck, PrCiState, PrDetail, PrDetailData, PrMergeMethod, PrReviewState,
    PrTimelineItem, PrTimelineKind, PullRequestsStoreModel,
};
use crate::view_components::action_button::{
    ActionButton, NakedTheme, PrimaryTheme, SecondaryTheme,
};

/// The detail view's tabs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PrDetailTab {
    #[default]
    Conversation,
    Checks,
    Files,
}

impl PrDetailTab {
    pub const ALL: [PrDetailTab; 3] = [
        PrDetailTab::Conversation,
        PrDetailTab::Checks,
        PrDetailTab::Files,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrDetailTab::Conversation => "Conversation",
            PrDetailTab::Checks => "Checks",
            PrDetailTab::Files => "Files",
        }
    }

    /// Stable string used as the tab-switch action payload.
    pub fn as_str(self) -> &'static str {
        match self {
            PrDetailTab::Conversation => "conversation",
            PrDetailTab::Checks => "checks",
            PrDetailTab::Files => "files",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

fn action(action: PullRequestsPageAction) -> AutomationViewAction {
    AutomationViewAction::PullRequests(action)
}

/// UI state for the one open PR detail. Created on row click, dropped on
/// back-to-list; the fetched data itself lives in the store.
pub struct PrDetailState {
    number: u64,
    tab: PrDetailTab,
    scroll_state: ClippedScrollStateHandle,
    /// Fingerprint of the markdown content the editors were built from; the
    /// editors are rebuilt in [`Self::sync`] when it changes.
    content_rev: Option<u64>,
    /// Read-only markdown editor for the PR description (None while loading
    /// or when the body is empty).
    body_editor: Option<ViewHandle<RichTextEditorView>>,
    /// One read-only markdown editor per timeline item with a non-empty body,
    /// index-aligned with the store's timeline.
    timeline_editors: Vec<Option<ViewHandle<RichTextEditorView>>>,
    /// First-stage merge buttons, one per strategy.
    merge_buttons: Vec<(PrMergeMethod, ViewHandle<ActionButton>)>,
    /// Second-stage confirm button (label updated to the armed strategy).
    confirm_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    ready_button: ViewHandle<ActionButton>,
    refresh_button: ViewHandle<ActionButton>,
    /// The armed merge strategy awaiting its second (confirm) click.
    pending_merge: Option<PrMergeMethod>,
    /// True while the store reports a mutation in flight (buttons disabled).
    buttons_disabled: bool,
    back_state: MouseStateHandle,
    external_state: MouseStateHandle,
    tab_states: RefCell<HashMap<&'static str, MouseStateHandle>>,
    check_states: RefCell<HashMap<usize, MouseStateHandle>>,
    /// Hover states for the "N comments on files" timeline links (21c).
    timeline_states: RefCell<HashMap<usize, MouseStateHandle>>,
    /// Hover states for the 21d review-bar pills.
    misc_states: RefCell<HashMap<String, MouseStateHandle>>,
    /// The Files tab's UI state (21c).
    files_tab: FilesTabState,
    /// 21d: the Conversation tab's PR-comment composer.
    comment_editor: ViewHandle<RichTextEditorView>,
    comment_button: ViewHandle<ActionButton>,
    /// True while a submitted comment's mutation is in flight (the composer
    /// clears only on success, so a failure keeps the text).
    comment_submitting: bool,
    comment_error: Option<String>,
    /// 21d: the review bar (drafted inline comments + verdict + summary).
    review_bar_open: bool,
    review_summary_editor: ViewHandle<RichTextEditorView>,
    review_button: ViewHandle<ActionButton>,
    submit_review_button: ViewHandle<ActionButton>,
    confirm_review_button: ViewHandle<ActionButton>,
    cancel_review_button: ViewHandle<ActionButton>,
    /// True after the first Submit click when the verdict needs a confirm.
    pending_review_submit: bool,
    review_submitting: bool,
    review_error: Option<String>,
}

impl PrDetailState {
    pub fn new(number: u64, ctx: &mut ViewContext<AutomationView>) -> Self {
        let merge_buttons = PrMergeMethod::ALL
            .into_iter()
            .map(|method| {
                let button = ctx.add_typed_action_view(|_| {
                    ActionButton::new(method.label(), SecondaryTheme).on_click(move |ctx| {
                        ctx.dispatch_typed_action(action(PullRequestsPageAction::RequestMerge(
                            method.as_str().to_owned(),
                        )));
                    })
                });
                (method, button)
            })
            .collect();
        let confirm_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Confirm", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ConfirmMerge));
            })
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::CancelMerge));
            })
        });
        let ready_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Mark ready for review", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::MarkReady));
            })
        });
        let refresh_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Refresh", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::RefreshDetail));
            })
        });
        let comment_editor = create_editable_comment_markdown_editor(None, ctx);
        let comment_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Comment", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::SubmitComment));
            })
        });
        let review_summary_editor = create_editable_comment_markdown_editor(None, ctx);
        let review_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Review", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ToggleReviewBar));
            })
        });
        let submit_review_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Submit review", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::RequestSubmitReview));
            })
        });
        let confirm_review_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Confirm", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ConfirmSubmitReview));
            })
        });
        let cancel_review_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::CancelSubmitReview));
            })
        });
        Self {
            number,
            tab: PrDetailTab::default(),
            scroll_state: Default::default(),
            content_rev: None,
            body_editor: None,
            timeline_editors: Vec::new(),
            merge_buttons,
            confirm_button,
            cancel_button,
            ready_button,
            refresh_button,
            pending_merge: None,
            buttons_disabled: false,
            back_state: Default::default(),
            external_state: Default::default(),
            tab_states: Default::default(),
            check_states: Default::default(),
            timeline_states: Default::default(),
            misc_states: Default::default(),
            files_tab: Default::default(),
            comment_editor,
            comment_button,
            comment_submitting: false,
            comment_error: None,
            review_bar_open: false,
            review_summary_editor,
            review_button,
            submit_review_button,
            confirm_review_button,
            cancel_review_button,
            pending_review_submit: false,
            review_submitting: false,
            review_error: None,
        }
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    /// Resync from the store: rebuild the markdown editors when the fetched
    /// content changed and keep the action buttons' disabled state in step
    /// with in-flight mutations.
    pub fn sync(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let (fingerprint, body, bodies, mutating, mutation_error) = {
            let store = PullRequestsStoreModel::as_ref(ctx);
            let Some(data) = store.detail_data(self.number) else {
                return;
            };
            (
                content_fingerprint(data),
                data.detail.as_ref().map(|d| d.body.clone()),
                data.timeline
                    .iter()
                    .map(|item| item.body.clone())
                    .collect::<Vec<_>>(),
                data.mutating,
                data.mutation_error.clone(),
            )
        };

        // 21d: settle in-flight comment / review submissions. Only success
        // clears the composer / drafts — a failure keeps everything typed.
        if !mutating {
            if self.comment_submitting {
                self.comment_submitting = false;
                match &mutation_error {
                    None => {
                        self.comment_error = None;
                        self.comment_editor
                            .update(ctx, |editor, ctx| editor.reset_with_markdown("", ctx));
                    }
                    Some(error) => self.comment_error = Some(error.clone()),
                }
            }
            if self.review_submitting {
                self.review_submitting = false;
                match &mutation_error {
                    None => {
                        self.review_error = None;
                        self.review_bar_open = false;
                        self.files_tab.clear_drafts();
                        self.review_summary_editor
                            .update(ctx, |editor, ctx| editor.reset_with_markdown("", ctx));
                    }
                    Some(error) => self.review_error = Some(error.clone()),
                }
            }
        }

        // Keep the header Review button's label in step with the draft count.
        let review_label = match self.files_tab.drafts().len() {
            0 => "Review".to_owned(),
            n => format!("Review ({n})"),
        };
        self.review_button
            .update(ctx, |b, ctx| b.set_label(review_label, ctx));

        if self.content_rev != Some(fingerprint) {
            self.content_rev = Some(fingerprint);
            let max_width = Some(Pixels::new(CONTENT_MAX_WIDTH));
            self.body_editor = body
                .filter(|body| !body.trim().is_empty())
                .map(|body| create_readonly_comment_markdown_editor(&body, true, max_width, ctx));
            self.timeline_editors = bodies
                .iter()
                .map(|body| {
                    (!body.trim().is_empty()).then(|| {
                        create_readonly_comment_markdown_editor(body, true, max_width, ctx)
                    })
                })
                .collect();
        }

        self.files_tab.sync(self.number, ctx);

        if self.buttons_disabled != mutating {
            self.buttons_disabled = mutating;
            for (_, button) in &self.merge_buttons {
                button.update(ctx, |b, ctx| b.set_disabled(mutating, ctx));
            }
            for button in [
                &self.confirm_button,
                &self.ready_button,
                &self.refresh_button,
                &self.comment_button,
                &self.submit_review_button,
                &self.confirm_review_button,
            ] {
                button.update(ctx, |b, ctx| b.set_disabled(mutating, ctx));
            }
        }
    }

    /// Handle the detail-scoped page actions (tab switch, merge box). The
    /// list page handles navigation (`OpenDetail` / `BackToList`) itself.
    pub fn handle_action(
        &mut self,
        page_action: &PullRequestsPageAction,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        use PullRequestsPageAction::*;
        match page_action {
            SetDetailTab(tab) => {
                if let Some(tab) = PrDetailTab::from_str(tab) {
                    self.tab = tab;
                    self.pending_merge = None;
                    if tab == PrDetailTab::Files {
                        // First open of the Files tab kicks off the diff +
                        // review-threads fetch.
                        let number = self.number;
                        PullRequestsStoreModel::handle(ctx)
                            .update(ctx, |store, ctx| store.ensure_files(number, ctx));
                    }
                }
            }
            ToggleFileCard(index) => self.files_tab.toggle_file(*index as usize),
            ToggleFileThread(index) => self.files_tab.toggle_thread(*index as usize),
            ToggleFileThreads(index) => self.files_tab.toggle_floating(*index as usize),
            RequestMerge(method) => {
                if let Some(method) = PrMergeMethod::from_str(method) {
                    self.pending_merge = Some(method);
                    let label = format!("Confirm {}", method.label().to_lowercase());
                    self.confirm_button
                        .update(ctx, |b, ctx| b.set_label(label, ctx));
                }
            }
            ConfirmMerge => {
                if let Some(method) = self.pending_merge.take() {
                    let number = self.number;
                    PullRequestsStoreModel::handle(ctx)
                        .update(ctx, |store, ctx| store.merge_pr(number, method, ctx));
                }
            }
            CancelMerge => self.pending_merge = None,
            MarkReady => {
                let number = self.number;
                PullRequestsStoreModel::handle(ctx)
                    .update(ctx, |store, ctx| store.mark_ready(number, ctx));
            }
            RefreshDetail => {
                let number = self.number;
                let refresh_files = {
                    let store = PullRequestsStoreModel::as_ref(ctx);
                    store
                        .detail_data(number)
                        .is_some_and(|data| data.files.fetched)
                };
                PullRequestsStoreModel::handle(ctx).update(ctx, |store, ctx| {
                    store.fetch_detail(number, ctx);
                    if refresh_files {
                        store.fetch_files(number, ctx);
                    }
                });
            }
            // 21d: the review write path.
            SubmitComment => {
                let body = self
                    .comment_editor
                    .as_ref(ctx)
                    .model()
                    .as_ref(ctx)
                    .markdown(ctx);
                if body.trim().is_empty() {
                    return;
                }
                let number = self.number;
                let started = PullRequestsStoreModel::handle(ctx)
                    .update(ctx, |store, ctx| store.comment_pr(number, body, ctx));
                if started {
                    self.comment_submitting = true;
                    self.comment_error = None;
                }
            }
            ToggleThreadReply(idx) => self.files_tab.toggle_reply(*idx as usize, ctx),
            SubmitThreadReply(idx) => {
                let number = self.number;
                self.files_tab.submit_reply(number, *idx as usize, ctx);
            }
            SetThreadResolved(idx, resolved) => {
                let number = self.number;
                self.files_tab
                    .set_thread_resolved(number, *idx as usize, *resolved, ctx);
            }
            StartDraftComment(file, hunk, line) => {
                self.files_tab
                    .start_draft((*file as usize, *hunk as usize, *line as usize), ctx);
            }
            CancelDraftComment => self.files_tab.cancel_draft(),
            SaveDraftComment => {
                let number = self.number;
                self.files_tab.save_draft(number, ctx);
            }
            DiscardDraft(idx) => {
                self.files_tab.drafts_mut().discard(*idx as usize);
                self.pending_review_submit = false;
            }
            DiscardAllDrafts => {
                self.files_tab.drafts_mut().discard_all();
                self.pending_review_submit = false;
            }
            ToggleReviewBar => {
                self.review_bar_open = !self.review_bar_open;
                self.pending_review_submit = false;
                self.files_tab.drafts_mut().disarm_discard_all();
            }
            SetReviewEvent(value) => {
                if let Some(event) = PrReviewEvent::from_str(value) {
                    self.files_tab.drafts_mut().set_event(event);
                    self.pending_review_submit = false;
                }
            }
            RequestSubmitReview => {
                let event = self.files_tab.drafts().event();
                if event.needs_confirm() {
                    self.pending_review_submit = true;
                    let label = format!("Confirm: {}", event.label().to_lowercase());
                    self.confirm_review_button
                        .update(ctx, |b, ctx| b.set_label(label, ctx));
                } else {
                    self.submit_review(ctx);
                }
            }
            ConfirmSubmitReview => {
                if self.pending_review_submit {
                    self.pending_review_submit = false;
                    self.submit_review(ctx);
                }
            }
            CancelSubmitReview => self.pending_review_submit = false,
            _ => {}
        }
    }

    /// Build the review payload from the drafts + summary and hand it to the
    /// store as one atomic REST submit (21d).
    fn submit_review(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let body = self
            .review_summary_editor
            .as_ref(ctx)
            .model()
            .as_ref(ctx)
            .markdown(ctx);
        let event = self.files_tab.drafts().event();
        if event == PrReviewEvent::Comment
            && self.files_tab.drafts().is_empty()
            && body.trim().is_empty()
        {
            // Nothing to submit.
            return;
        }
        let payload = build_review_payload(event, &body, self.files_tab.drafts().drafts());
        let number = self.number;
        let started = PullRequestsStoreModel::handle(ctx)
            .update(ctx, |store, ctx| store.submit_review(number, payload, ctx));
        if started {
            self.review_submitting = true;
            self.review_error = None;
        }
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let store = PullRequestsStoreModel::as_ref(app);
        let data = store.detail_data(self.number);
        let sub = theme.sub_text_color(theme.background());

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(self.render_header(data, app));

        match data {
            Some(data) if data.error.is_some() => {
                let error = data.error.as_deref().unwrap_or_default();
                column.add_child(render_error_card(
                    &format!("Couldn't load this pull request: {error}"),
                    app,
                ));
            }
            Some(data) if data.detail.is_some() => {
                let detail = data.detail.as_ref().unwrap();
                column.add_child(self.render_tab_bar(app));
                // 21d: the review bar sits above every tab's content while
                // drafts exist (or when opened from the header).
                if self.review_bar_open || !self.files_tab.drafts().is_empty() {
                    column.add_child(self.render_review_bar(app));
                }
                match self.tab {
                    PrDetailTab::Conversation => {
                        self.render_conversation(&mut column, detail, data, app)
                    }
                    PrDetailTab::Checks => self.render_checks(&mut column, detail, app),
                    PrDetailTab::Files => self.files_tab.render(&mut column, &data.files, app),
                }
            }
            _ => {
                column.add_child(
                    Container::new(
                        Text::new_inline(
                            "Loading pull request…",
                            appearance.ui_font_family(),
                            type_ramp::UI.size,
                        )
                        .with_line_height_ratio(type_ramp::UI.line_height)
                        .with_color(sub.into())
                        .finish(),
                    )
                    .with_margin_top(spacing::LG)
                    .finish(),
                );
            }
        }

        // Diffs want more horizontal room than prose.
        let max_width = match self.tab {
            PrDetailTab::Files => FILES_CONTENT_MAX_WIDTH,
            _ => CONTENT_MAX_WIDTH,
        };
        let content = Container::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(max_width)
                .finish(),
        )
        .with_uniform_padding(spacing::XL)
        .finish();

        let centered = Align::new(content).top_center().finish();

        let scrollable = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: centered,
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            twarpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        Container::new(scrollable)
            .with_background(theme.background())
            .finish()
    }

    /// Back chevron, "#N title", state badge, refresh, and the globe.
    fn render_header(&self, data: Option<&PrDetailData>, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());
        let detail = data.and_then(|data| data.detail.as_ref());

        let back = Hoverable::new(self.back_state.clone(), move |state| {
            let color = if state.is_hovered() { main } else { sub };
            Container::new(
                ConstrainedBox::new(Icon::ChevronLeft.to_warpui_icon(color).finish())
                    .with_width(spacing::MD)
                    .with_height(spacing::MD)
                    .finish(),
            )
            .with_uniform_padding(spacing::XS)
            .finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(action(PullRequestsPageAction::BackToList)))
        .with_cursor(Cursor::PointingHand)
        .finish();

        let title = detail
            .map(|d| format!("#{} {}", d.number, d.title))
            .unwrap_or_else(|| format!("#{}", self.number));

        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(back)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new(title, appearance.ui_font_family(), type_ramp::HEADING.size)
                            .with_line_height_ratio(type_ramp::HEADING.line_height)
                            .with_color(main.into())
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );

        if let Some(detail) = detail {
            let (label, color) = state_badge(detail, theme);
            header.add_child(render_badge(label, color, appearance));
        }
        header.add_child(ChildView::new(&self.review_button).finish());
        header.add_child(ChildView::new(&self.refresh_button).finish());

        if let Some(detail) = detail {
            let url = detail.url.clone();
            let external = Hoverable::new(self.external_state.clone(), move |state| {
                let color = if state.is_hovered() { main } else { sub };
                Container::new(
                    ConstrainedBox::new(Icon::Globe.to_warpui_icon(color).finish())
                        .with_width(spacing::MD)
                        .with_height(spacing::MD)
                        .finish(),
                )
                .with_uniform_padding(spacing::XS)
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::OpenExternal(url.clone())))
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
            header.add_child(external);
        }

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header.finish());

        if let Some(detail) = detail {
            let mut meta = format!(
                "{} wants to merge {} → {}",
                detail.author, detail.head_ref, detail.base_ref
            );
            meta.push_str(&format!(
                " · +{} −{} · {} files",
                detail.additions, detail.deletions, detail.changed_files
            ));
            let created = relative_updated_at(&detail.created_at, chrono::Utc::now());
            if !created.is_empty() {
                meta.push_str(&format!(" · opened {created}"));
            }
            column.add_child(
                Container::new(
                    Text::new(meta, appearance.ui_font_family(), type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(sub.into())
                        .finish(),
                )
                .with_margin_top(spacing::XS)
                .with_margin_bottom(spacing::SM)
                .finish(),
            );
        }
        column.finish()
    }

    /// Conversation | Checks | Files.
    fn render_tab_bar(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::SM);
        for tab in PrDetailTab::ALL {
            let active = tab == self.tab;
            let state = self
                .tab_states
                .borrow_mut()
                .entry(tab.as_str())
                .or_default()
                .clone();
            let family = appearance.ui_font_family();
            row.add_child(
                Hoverable::new(state, move |hover| {
                    let color = if active || hover.is_hovered() {
                        main
                    } else {
                        sub
                    };
                    let mut container = Container::new(
                        Text::new_inline(tab.label(), family, type_ramp::UI.size)
                            .with_line_height_ratio(type_ramp::UI.line_height)
                            .with_color(color.into())
                            .finish(),
                    )
                    .with_horizontal_padding(spacing::XS)
                    .with_padding_bottom(spacing::XXS);
                    if active {
                        container = container
                            .with_border(Border::bottom(2.).with_border_fill(theme.accent()));
                    }
                    container.finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(action(PullRequestsPageAction::SetDetailTab(
                        tab.as_str().to_owned(),
                    )))
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            );
        }
        Container::new(row.finish())
            .with_margin_bottom(spacing::MD)
            .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
            .finish()
    }

    fn render_conversation(
        &self,
        column: &mut Flex,
        detail: &PrDetail,
        data: &PrDetailData,
        app: &AppContext,
    ) {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());

        // Description card.
        let created = relative_updated_at(&detail.created_at, chrono::Utc::now());
        let body: Box<dyn Element> = match &self.body_editor {
            Some(editor) => ChildView::new(editor).finish(),
            None => Text::new_inline(
                "No description provided.",
                appearance.ui_font_family(),
                type_ramp::UI.size,
            )
            .with_line_height_ratio(type_ramp::UI.line_height)
            .with_color(sub.into())
            .finish(),
        };
        column.add_child(render_timeline_card(
            &detail.author,
            "opened",
            sub.into(),
            &created,
            body,
            app,
        ));

        // Timeline items (index-aligned with `self.timeline_editors`).
        let now = chrono::Utc::now();
        for (index, item) in data.timeline.iter().enumerate() {
            column.add_child(self.render_timeline_item(index, item, now, app));
        }
        if data.timeline_truncated {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "Older conversation items aren't shown.",
                        appearance.ui_font_family(),
                        type_ramp::CAPTION.size,
                    )
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(sub.into())
                    .finish(),
                )
                .with_margin_top(spacing::SM)
                .finish(),
            );
        }

        column.add_child(self.render_composer(app));
        column.add_child(self.render_merge_box(detail, data, app));
    }

    /// 21d: the PR-level comment composer at the Conversation tab's bottom.
    fn render_composer(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(spacing::XS)
            .with_child(
                Text::new_inline(
                    "Add a comment",
                    appearance.ui_font_family(),
                    type_ramp::LABEL.size,
                )
                .with_line_height_ratio(type_ramp::LABEL.line_height)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(ChildView::new(&self.comment_editor).finish());
        if let Some(error) = &self.comment_error {
            column.add_child(
                Text::new(
                    error.clone(),
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.ui_error_color())
                .finish(),
            );
        }
        column.add_child(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(ChildView::new(&self.comment_button).finish())
                .finish(),
        );

        Container::new(column.finish())
            .with_uniform_padding(spacing::MD)
            .with_margin_top(spacing::SM)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }

    /// 21d: the review bar — pending-draft count, verdict pills, summary
    /// editor, discard-all, and the (two-click for consequential verdicts)
    /// Submit controls. Errors surface inline; drafts survive failures.
    fn render_review_bar(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());
        let family = appearance.ui_font_family();
        let drafts = self.files_tab.drafts();

        let count_label = match drafts.len() {
            0 => "Review this pull request".to_owned(),
            1 => "1 pending review comment".to_owned(),
            n => format!("{n} pending review comments"),
        };
        let mut top_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new(count_label, family.clone(), type_ramp::UI.size)
                            .with_line_height_ratio(type_ramp::UI.line_height)
                            .with_color(main.into())
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );
        if !drafts.is_empty() {
            let discard_label: &'static str = if drafts.discard_all_armed() {
                "Click again to discard all"
            } else {
                "Discard all"
            };
            let discard_color: ColorU = if drafts.discard_all_armed() {
                theme.ui_error_color()
            } else {
                sub.into()
            };
            top_row.add_child(self.review_pill(
                "discard-all",
                discard_label,
                discard_color,
                false,
                PullRequestsPageAction::DiscardAllDrafts,
                app,
            ));
        }

        let mut pills = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::XS);
        for event in PrReviewEvent::ALL {
            let selected = event == drafts.event();
            let color: ColorU = if selected {
                theme.accent().into_solid()
            } else {
                sub.into()
            };
            pills.add_child(self.review_pill(
                event.as_str(),
                event.label(),
                color,
                selected,
                PullRequestsPageAction::SetReviewEvent(event.as_str().to_owned()),
                app,
            ));
        }

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(spacing::SM)
            .with_child(top_row.finish())
            .with_child(pills.finish())
            .with_child(
                Text::new_inline("Summary (optional)", family.clone(), type_ramp::LABEL.size)
                    .with_line_height_ratio(type_ramp::LABEL.line_height)
                    .with_color(sub.into())
                    .finish(),
            )
            .with_child(ChildView::new(&self.review_summary_editor).finish());
        if let Some(error) = &self.review_error {
            column.add_child(
                Text::new(error.clone(), family, type_ramp::CAPTION.size)
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(theme.ui_error_color())
                    .finish(),
            );
        }
        let mut buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::XS);
        if self.pending_review_submit {
            buttons.add_child(ChildView::new(&self.confirm_review_button).finish());
            buttons.add_child(ChildView::new(&self.cancel_review_button).finish());
        } else {
            buttons.add_child(ChildView::new(&self.submit_review_button).finish());
        }
        column.add_child(buttons.finish());

        Container::new(column.finish())
            .with_uniform_padding(spacing::MD)
            .with_margin_bottom(spacing::MD)
            .with_border(Border::all(1.).with_border_fill(theme.ui_warning_color()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }

    /// A small outlined clickable pill for the review bar.
    fn review_pill(
        &self,
        key: &str,
        label: &'static str,
        color: ColorU,
        selected: bool,
        page_action: PullRequestsPageAction,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let main = theme.main_text_color(theme.background());
        let family = appearance.ui_font_family();
        let state = self
            .misc_states
            .borrow_mut()
            .entry(key.to_owned())
            .or_default()
            .clone();
        Hoverable::new(state, move |hover| {
            let text_color = if selected || hover.is_hovered() {
                main.into()
            } else {
                color
            };
            Container::new(
                Text::new_inline(label, family.clone(), type_ramp::CAPTION.size)
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(text_color)
                    .finish(),
            )
            .with_horizontal_padding(spacing::SM)
            .with_vertical_padding(spacing::XXS)
            .with_border(Border::all(1.).with_border_fill(color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action(page_action.clone())))
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_timeline_item(
        &self,
        index: usize,
        item: &PrTimelineItem,
        now: chrono::DateTime<chrono::Utc>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let (verb, verb_color): (&str, ColorU) = match item.kind {
            PrTimelineKind::Comment => ("commented", sub.into()),
            PrTimelineKind::Review(state) => (
                state.label(),
                match state {
                    PrReviewState::Approved => theme.ui_green_color(),
                    PrReviewState::ChangesRequested => theme.ui_error_color(),
                    PrReviewState::Commented | PrReviewState::Dismissed => sub.into(),
                },
            ),
        };
        let when = relative_updated_at(&item.created_at, now);
        let body: Box<dyn Element> = match self.timeline_editors.get(index).and_then(Option::as_ref)
        {
            Some(editor) => ChildView::new(editor).finish(),
            // A body-less review carrying line comments links to the Files
            // tab (21c) where its threads render inline.
            None if item.file_comments > 0 && matches!(item.kind, PrTimelineKind::Review(_)) => {
                let main = theme.main_text_color(theme.background());
                let label = format!(
                    "{} comment{} on files — view in the Files tab",
                    item.file_comments,
                    if item.file_comments == 1 { "" } else { "s" }
                );
                let family = appearance.ui_font_family();
                let state = self
                    .timeline_states
                    .borrow_mut()
                    .entry(index)
                    .or_default()
                    .clone();
                Hoverable::new(state, move |hover| {
                    let color = if hover.is_hovered() { main } else { sub };
                    Text::new(label.clone(), family.clone(), type_ramp::UI.size)
                        .with_line_height_ratio(type_ramp::UI.line_height)
                        .with_color(color.into())
                        .finish()
                })
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(action(PullRequestsPageAction::SetDetailTab(
                        PrDetailTab::Files.as_str().to_owned(),
                    )))
                })
                .with_cursor(Cursor::PointingHand)
                .finish()
            }
            None => Flex::row().with_main_axis_size(MainAxisSize::Min).finish(),
        };
        render_timeline_card(&item.author, verb, verb_color, &when, body, app)
    }

    fn render_checks(&self, column: &mut Flex, detail: &PrDetail, app: &AppContext) {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());

        if detail.checks.is_empty() {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "No checks reported for this pull request.",
                        appearance.ui_font_family(),
                        type_ramp::UI.size,
                    )
                    .with_line_height_ratio(type_ramp::UI.line_height)
                    .with_color(sub.into())
                    .finish(),
                )
                .with_margin_top(spacing::LG)
                .finish(),
            );
            return;
        }
        for (index, check) in detail.checks.iter().enumerate() {
            column.add_child(self.render_check_row(index, check, app));
        }
    }

    /// One check row: status dot, name, duration, click → details in browser.
    fn render_check_row(
        &self,
        index: usize,
        check: &PrCheck,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let state = self
            .check_states
            .borrow_mut()
            .entry(index)
            .or_default()
            .clone();

        let dot_color: ColorU = match check.state {
            PrCiState::Passing => theme.ui_green_color(),
            PrCiState::Failing => theme.ui_error_color().into(),
            PrCiState::Pending => theme.ui_warning_color(),
        };
        let status_label = match check.state {
            PrCiState::Passing => "passed",
            PrCiState::Failing => "failed",
            PrCiState::Pending => "running",
        };
        let mut sub_line = status_label.to_owned();
        if let Some(duration) = &check.duration {
            sub_line.push_str(&format!(" · {duration}"));
        }

        let name = check.name.clone();
        let family = appearance.ui_font_family();
        let main = theme.main_text_color(theme.background());
        let row = Hoverable::new(state, move |hover| {
            let dot = ConstrainedBox::new(
                Container::new(Flex::row().with_main_axis_size(MainAxisSize::Min).finish())
                    .with_background(dot_color)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            )
            .with_width(8.)
            .with_height(8.)
            .finish();
            let labels = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Text::new(name.clone(), family, type_ramp::UI.size)
                        .with_line_height_ratio(type_ramp::UI.line_height)
                        .with_color(main.into())
                        .finish(),
                )
                .with_child(
                    Text::new(sub_line.clone(), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(sub.into())
                        .finish(),
                )
                .finish();
            let mut container = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::SM)
                    .with_child(dot)
                    .with_child(Shrinkable::new(1., Align::new(labels).left().finish()).finish())
                    .finish(),
            )
            .with_uniform_padding(spacing::SM)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if hover.is_hovered() {
                container = container.with_background(theme.surface_overlay_2());
            }
            container.finish()
        });
        if check.details_url.is_empty() {
            return row.finish();
        }
        let url = check.details_url.clone();
        row.on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action(PullRequestsPageAction::OpenCheck(url.clone())))
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    /// Mergeability summary + Merge / Squash / Rebase (two-click confirm) or
    /// Mark-ready for drafts. GitHub has the final say, so merging stays
    /// possible (behind the confirm) even when the summary flags problems.
    fn render_merge_box(
        &self,
        detail: &PrDetail,
        data: &PrDetailData,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let (summary, ok) = merge_summary(detail);
        let summary_color = if ok {
            theme.ui_green_color()
        } else {
            theme.sub_text_color(theme.background()).into()
        };

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(spacing::SM)
            .with_child(
                Text::new(summary, appearance.ui_font_family(), type_ramp::UI.size)
                    .with_line_height_ratio(type_ramp::UI.line_height)
                    .with_color(summary_color.into())
                    .finish(),
            );

        if let Some(error) = &data.mutation_error {
            column.add_child(
                Text::new(
                    error.clone(),
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(theme.ui_error_color())
                .finish(),
            );
        }

        if detail.state == "OPEN" {
            let mut buttons = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(spacing::XS);
            if detail.is_draft {
                buttons.add_child(ChildView::new(&self.ready_button).finish());
            } else if let Some(_pending) = self.pending_merge {
                buttons.add_child(ChildView::new(&self.confirm_button).finish());
                buttons.add_child(ChildView::new(&self.cancel_button).finish());
            } else {
                for (_, button) in &self.merge_buttons {
                    buttons.add_child(ChildView::new(button).finish());
                }
            }
            column.add_child(buttons.finish());
        }

        Container::new(column.finish())
            .with_uniform_padding(spacing::LG)
            .with_margin_top(spacing::LG)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }
}

/// One conversation card: "author verb · when" header plus a markdown body.
fn render_timeline_card(
    author: &str,
    verb: &str,
    verb_color: ColorU,
    when: &str,
    body: Box<dyn Element>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sub = theme.sub_text_color(theme.background());

    let mut header = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(spacing::XS)
        .with_child(
            Text::new(
                author.to_owned(),
                appearance.ui_font_family(),
                type_ramp::UI.size,
            )
            .with_line_height_ratio(type_ramp::UI.line_height)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish(),
        )
        .with_child(
            Text::new(
                verb.to_owned(),
                appearance.ui_font_family(),
                type_ramp::UI.size,
            )
            .with_line_height_ratio(type_ramp::UI.line_height)
            .with_color(verb_color)
            .finish(),
        );
    if !when.is_empty() {
        header.add_child(
            Text::new(
                format!("· {when}"),
                appearance.ui_font_family(),
                type_ramp::CAPTION.size,
            )
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(sub.into())
            .finish(),
        );
    }

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(spacing::XS)
            .with_child(header.finish())
            .with_child(body)
            .finish(),
    )
    .with_uniform_padding(spacing::MD)
    .with_margin_bottom(spacing::SM)
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish()
}

/// Detail-page error card.
fn render_error_card(message: &str, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    Container::new(
        Text::new(
            message.to_owned(),
            appearance.ui_font_family(),
            type_ramp::UI.size,
        )
        .with_line_height_ratio(type_ramp::UI.line_height)
        .with_color(theme.ui_error_color())
        .finish(),
    )
    .with_uniform_padding(spacing::LG)
    .with_margin_top(spacing::MD)
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish()
}

/// The header state badge: label + color for Open/Draft/Merged/Closed.
fn state_badge(
    detail: &PrDetail,
    theme: &twarp_core::ui::theme::WarpTheme,
) -> (&'static str, ColorU) {
    match detail.state.as_str() {
        "MERGED" => ("Merged", theme.accent().into_solid()),
        "CLOSED" => ("Closed", theme.ui_error_color()),
        _ if detail.is_draft => (
            "Draft",
            theme.sub_text_color(theme.background()).into_solid(),
        ),
        _ => ("Open", theme.ui_green_color()),
    }
}

/// One-line mergeability summary; `true` when merging looks clean.
pub(crate) fn merge_summary(detail: &PrDetail) -> (String, bool) {
    match detail.state.as_str() {
        "MERGED" => return ("This pull request is merged.".to_owned(), false),
        "CLOSED" => return ("This pull request is closed.".to_owned(), false),
        _ => {}
    }
    if detail.is_draft {
        return (
            "This pull request is a draft — mark it ready to enable merging.".to_owned(),
            false,
        );
    }
    if detail.mergeable == "CONFLICTING" {
        return (
            format!(
                "This branch has conflicts with {} that must be resolved.",
                detail.base_ref
            ),
            false,
        );
    }
    match detail.merge_state_status.as_str() {
        "CLEAN" | "HAS_HOOKS" => ("This branch is ready to merge.".to_owned(), true),
        "UNSTABLE" => (
            "Some checks are failing; GitHub may still allow the merge.".to_owned(),
            false,
        ),
        "BLOCKED" => (
            "Merging is blocked (required reviews or checks are missing).".to_owned(),
            false,
        ),
        "BEHIND" => (format!("This branch is behind {}.", detail.base_ref), false),
        "DIRTY" => ("This branch has merge conflicts.".to_owned(), false),
        _ => ("Mergeability is still being computed.".to_owned(), false),
    }
}

/// Fingerprint of the markdown content backing the editors.
fn content_fingerprint(data: &PrDetailData) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Some(detail) = &data.detail {
        detail.body.hash(&mut hasher);
    }
    for item in &data.timeline {
        item.author.hash(&mut hasher);
        item.created_at.hash(&mut hasher);
        item.body.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pull_requests::store::PrDetail;

    fn detail() -> PrDetail {
        PrDetail {
            number: 1,
            title: "t".into(),
            body: String::new(),
            author: "a".into(),
            state: "OPEN".into(),
            is_draft: false,
            mergeable: "MERGEABLE".into(),
            merge_state_status: "CLEAN".into(),
            review_decision: None,
            base_ref: "master".into(),
            head_ref: "feat".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            url: String::new(),
            created_at: String::new(),
            checks: Vec::new(),
        }
    }

    #[test]
    fn merge_summary_states() {
        assert!(merge_summary(&detail()).1);

        let mut conflicting = detail();
        conflicting.mergeable = "CONFLICTING".into();
        let (text, ok) = merge_summary(&conflicting);
        assert!(!ok);
        assert!(text.contains("conflicts with master"));

        let mut draft = detail();
        draft.is_draft = true;
        assert!(merge_summary(&draft).0.contains("draft"));

        let mut merged = detail();
        merged.state = "MERGED".into();
        assert!(merge_summary(&merged).0.contains("merged"));

        let mut blocked = detail();
        blocked.merge_state_status = "BLOCKED".into();
        assert!(!merge_summary(&blocked).1);
    }

    #[test]
    fn tab_round_trip() {
        for tab in PrDetailTab::ALL {
            assert_eq!(PrDetailTab::from_str(tab.as_str()), Some(tab));
        }
        assert_eq!(PrDetailTab::from_str("files"), Some(PrDetailTab::Files));
        assert_eq!(PrDetailTab::from_str("bogus"), None);
    }
}
