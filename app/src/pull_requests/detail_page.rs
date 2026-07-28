//! twarp 21b: the native PR detail view, rendered in-page by the Pull
//! Requests surface when a list row is clicked. Tabs: Conversation (markdown
//! description + PR-level comment/review timeline + merge box) and Checks
//! (per-check CI rows). The Files (diff) tab is a disabled stub until 21c.
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
use crate::code::editor::comment_editor::create_readonly_comment_markdown_editor;
use crate::notebooks::editor::view::RichTextEditorView;
use crate::pull_requests::list_page::{render_badge, PullRequestsPageAction, CONTENT_MAX_WIDTH};
use crate::pull_requests::store::{
    relative_updated_at, PrCheck, PrCiState, PrDetail, PrDetailData, PrMergeMethod, PrReviewState,
    PrTimelineItem, PrTimelineKind, PullRequestsStoreModel,
};
use crate::view_components::action_button::{
    ActionButton, NakedTheme, PrimaryTheme, SecondaryTheme,
};

/// The detail view's tabs. Files (the diff view) is 21c; it renders as a
/// disabled stub for now.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PrDetailTab {
    #[default]
    Conversation,
    Checks,
}

impl PrDetailTab {
    pub const ALL: [PrDetailTab; 2] = [PrDetailTab::Conversation, PrDetailTab::Checks];

    pub fn label(self) -> &'static str {
        match self {
            PrDetailTab::Conversation => "Conversation",
            PrDetailTab::Checks => "Checks",
        }
    }

    /// Stable string used as the tab-switch action payload.
    pub fn as_str(self) -> &'static str {
        match self {
            PrDetailTab::Conversation => "conversation",
            PrDetailTab::Checks => "checks",
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
        }
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    /// Resync from the store: rebuild the markdown editors when the fetched
    /// content changed and keep the action buttons' disabled state in step
    /// with in-flight mutations.
    pub fn sync(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let (fingerprint, body, bodies, mutating) = {
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
            )
        };

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

        if self.buttons_disabled != mutating {
            self.buttons_disabled = mutating;
            for (_, button) in &self.merge_buttons {
                button.update(ctx, |b, ctx| b.set_disabled(mutating, ctx));
            }
            for button in [
                &self.confirm_button,
                &self.ready_button,
                &self.refresh_button,
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
                }
            }
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
                PullRequestsStoreModel::handle(ctx)
                    .update(ctx, |store, ctx| store.fetch_detail(number, ctx));
            }
            _ => {}
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
                match self.tab {
                    PrDetailTab::Conversation => {
                        self.render_conversation(&mut column, detail, data, app)
                    }
                    PrDetailTab::Checks => self.render_checks(&mut column, detail, app),
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

        let content = Container::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(CONTENT_MAX_WIDTH)
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

    /// Conversation | Checks | Files(disabled 21c stub).
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
        // 21c stub: the Files (diff) tab exists but is disabled.
        row.add_child(
            Container::new(
                Text::new_inline(
                    "Files (soon)",
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.outline().into_solid())
                .finish(),
            )
            .with_horizontal_padding(spacing::XS)
            .with_padding_bottom(spacing::XXS)
            .finish(),
        );

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

        column.add_child(self.render_merge_box(detail, data, app));
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
        assert_eq!(PrDetailTab::from_str("files"), None);
    }
}
