//! twarp 21a: the Pull Requests list page — a centered full-page column with a
//! repo picker, state filter, and grouped PR rows ("Needs your review" /
//! "Yours" / "Others") with
//! CI, review-decision, draft, and conflict badges. Layout mirrors the
//! automation pages ([`crate::automation::scheduled_tasks_page`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    AppContext, SingletonEntity, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::automation::view::{AutomationView, AutomationViewAction};
use crate::pull_requests::detail_page::PrDetailState;
use crate::pull_requests::store::{
    group_prs, relative_updated_at, PrCiState, PrEntry, PrStateFilter, PullRequestsStoreModel,
};
use crate::view_components::{
    action_button::{ActionButton, SecondaryTheme},
    dropdown::DropdownItem,
    Dropdown,
};
use crate::workspace::WorkspaceAction;

/// Content column width; matches the automation pages.
pub(crate) const CONTENT_MAX_WIDTH: f32 = 720.;

/// Actions dispatched by the Pull Requests page's controls, wrapped in
/// [`AutomationViewAction::PullRequests`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestsPageAction {
    /// Select a repo by project-root path (dropdown payload).
    SelectRepo(String),
    /// Set the state filter by [`PrStateFilter::as_str`] payload.
    SetFilter(String),
    Refresh,
    /// Row click: open the native in-page detail view for this PR (21b).
    OpenDetail(u64),
    /// Open the PR in the OS browser (the row's trailing affordance).
    OpenExternal(String),
    /// Detail view: return to the list.
    BackToList,
    /// Detail view: switch tabs by `PrDetailTab::as_str` payload.
    SetDetailTab(String),
    /// Detail merge box: arm a merge strategy (`PrMergeMethod::as_str`
    /// payload) — first click of the two-click confirm.
    RequestMerge(String),
    /// Detail merge box: run the armed merge (second click).
    ConfirmMerge,
    /// Detail merge box: disarm the pending merge.
    CancelMerge,
    /// Detail merge box: take a draft PR out of draft (`gh pr ready`).
    MarkReady,
    /// Detail view: refetch the open PR's detail + timeline.
    RefreshDetail,
    /// Checks tab: open a check's details page in the built-in Browser pane.
    OpenCheck(String),
    /// Files tab: expand/collapse a file card by file index (21c).
    ToggleFileCard(u64),
    /// Files tab: expand/collapse one resolved review thread by thread index.
    ToggleFileThread(u64),
    /// Files tab: expand/collapse a file's outdated/other-threads section.
    ToggleFileThreads(u64),
    /// Conversation tab: submit the PR-level comment composer (21d).
    SubmitComment,
    /// Files tab: open/close the inline reply editor on a thread (by thread
    /// index, 21d).
    ToggleThreadReply(u64),
    /// Files tab: submit the open reply editor on a thread.
    SubmitThreadReply(u64),
    /// Files tab: resolve (true) / unresolve one review thread by index.
    SetThreadResolved(u64, bool),
    /// Files tab: open a draft-comment editor at (file, hunk, line) indices.
    StartDraftComment(u64, u64, u64),
    /// Files tab: close the open draft editor without saving.
    CancelDraftComment,
    /// Files tab: commit the open draft editor into the local drafts list.
    SaveDraftComment,
    /// Discard one pending draft by draft index.
    DiscardDraft(u64),
    /// Discard all pending drafts (two-click: first click arms).
    DiscardAllDrafts,
    /// Detail header: show/hide the review bar.
    ToggleReviewBar,
    /// Review bar: choose the verdict (`PrReviewEvent::as_str` payload).
    SetReviewEvent(String),
    /// Review bar: submit — arms a confirm for Approve / Request changes,
    /// submits directly for Comment.
    RequestSubmitReview,
    /// Review bar: run the armed submit (second click).
    ConfirmSubmitReview,
    /// Review bar: disarm the pending submit.
    CancelSubmitReview,
    /// Detail header: open a Claude pane in a new tab seeded with a review
    /// prompt for this PR (21e).
    ReviewWithClaude,
    /// Detail header: fetch the PR head and materialize it as a detached git
    /// worktree under `~/.twarp/pr-worktrees/`, then open a tab there (21e).
    CheckoutPr,
    /// Detail header: copy the PR's branch name to the clipboard (21e).
    CopyBranchName,
}

/// Per-row hover states, keyed by PR number.
#[derive(Clone, Default)]
struct RowMouseStates {
    row: MouseStateHandle,
    external: MouseStateHandle,
}

pub struct PullRequestsPageState {
    scroll_state: ClippedScrollStateHandle,
    repo_dropdown: Option<ViewHandle<Dropdown<AutomationViewAction>>>,
    filter_dropdown: ViewHandle<Dropdown<AutomationViewAction>>,
    refresh_button: ViewHandle<ActionButton>,
    /// Dedicated handle for the empty state's CTA (a view handle cannot be
    /// mounted in two places at once).
    empty_refresh_button: ViewHandle<ActionButton>,
    rows: RefCell<HashMap<u64, RowMouseStates>>,
    /// The open in-page detail view, if a row was clicked (21b). Not
    /// persisted: after a restart the page reopens on the list.
    detail: Option<PrDetailState>,
}

impl PullRequestsPageState {
    pub fn new(ctx: &mut ViewContext<AutomationView>) -> Self {
        let refresh_button = new_refresh_button("Refresh", ctx);
        let empty_refresh_button = new_refresh_button("Refresh", ctx);
        let mut state = Self {
            scroll_state: Default::default(),
            repo_dropdown: None,
            filter_dropdown: new_filter_dropdown(PrStateFilter::default(), ctx),
            refresh_button,
            empty_refresh_button,
            rows: Default::default(),
            detail: None,
        };
        state.sync(ctx);
        state
    }

    /// Rebuild the dropdowns from the store (project list, selection, and
    /// filter all change out from under the page).
    pub fn sync(&mut self, ctx: &mut ViewContext<AutomationView>) {
        let (projects, selected, filter) = {
            let store = PullRequestsStoreModel::as_ref(ctx);
            (
                store.projects().to_vec(),
                store.selected_repo().map(Path::to_path_buf),
                store.filter(),
            )
        };
        self.repo_dropdown = (!projects.is_empty()).then(|| {
            let selected = selected.clone();
            ctx.add_typed_action_view(|ctx| {
                let mut dropdown = Dropdown::new(ctx);
                dropdown.set_items(
                    projects
                        .iter()
                        .map(|path| DropdownItem::new(project_label(path), repo_action(path)))
                        .collect(),
                    ctx,
                );
                if let Some(selected) = &selected {
                    dropdown.set_selected_by_action(repo_action(selected), ctx);
                }
                dropdown.set_top_bar_max_width(220.);
                dropdown
            })
        });
        self.filter_dropdown = new_filter_dropdown(filter, ctx);
        if let Some(mut detail) = self.detail.take() {
            detail.sync(ctx);
            self.detail = Some(detail);
        }
    }

    pub fn handle_action(
        &mut self,
        action: &PullRequestsPageAction,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        use PullRequestsPageAction::*;
        match action {
            SelectRepo(path) => {
                // Changing repos invalidates the open detail — back to list.
                self.close_detail(ctx);
                let path = PathBuf::from(path);
                PullRequestsStoreModel::handle(ctx)
                    .update(ctx, |store, ctx| store.select_repo(path, ctx));
                self.sync(ctx);
            }
            SetFilter(value) => {
                if let Some(filter) = PrStateFilter::from_str(value) {
                    PullRequestsStoreModel::handle(ctx)
                        .update(ctx, |store, ctx| store.set_filter(filter, ctx));
                    self.sync(ctx);
                }
            }
            Refresh => {
                PullRequestsStoreModel::handle(ctx).update(ctx, |store, ctx| store.refresh(ctx));
            }
            OpenDetail(number) => {
                let number = *number;
                self.detail = Some(PrDetailState::new(number, ctx));
                PullRequestsStoreModel::handle(ctx)
                    .update(ctx, |store, ctx| store.fetch_detail(number, ctx));
            }
            BackToList => self.close_detail(ctx),
            OpenExternal(url) => ctx.open_url(url),
            OpenCheck(url) => {
                ctx.dispatch_typed_action(&WorkspaceAction::OpenUrlInBrowserPane(url.clone()));
            }
            // Everything else is detail-scoped (tabs, merge box, 21d writes).
            _ => {
                if let Some(mut detail) = self.detail.take() {
                    detail.handle_action(action, ctx);
                    self.detail = Some(detail);
                }
            }
        }
        ctx.notify();
    }

    /// Drop the open detail view (and its cached data in the store).
    fn close_detail(&mut self, ctx: &mut ViewContext<AutomationView>) {
        if self.detail.take().is_some() {
            PullRequestsStoreModel::handle(ctx).update(ctx, |store, ctx| store.close_detail(ctx));
        }
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if let Some(detail) = &self.detail {
            return detail.render(app);
        }
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let store = PullRequestsStoreModel::as_ref(app);
        let sub = theme.sub_text_color(theme.background());

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Header: title left; repo picker, filter, and Refresh right.
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::SM)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new_inline(
                            "Pull Requests",
                            appearance.ui_font_family(),
                            type_ramp::HEADING.size,
                        )
                        .with_line_height_ratio(type_ramp::HEADING.line_height)
                        .with_color(theme.main_text_color(theme.background()).into())
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );
        if let Some(repo_dropdown) = &self.repo_dropdown {
            header.add_child(ChildView::new(repo_dropdown).finish());
        }
        header.add_child(ChildView::new(&self.filter_dropdown).finish());
        header.add_child(ChildView::new(&self.refresh_button).finish());
        column.add_child(header.finish());

        column.add_child(
            Container::new(
                Text::new_inline(
                    "Pull requests on the selected project's GitHub origin, via the gh CLI.",
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(sub.into())
                .finish(),
            )
            .with_margin_top(spacing::XS)
            .with_margin_bottom(spacing::LG)
            .finish(),
        );

        let data = store.selected_data();
        if store.selected_repo().is_none() {
            column.add_child(crate::automation::render_empty_state(
                Icon::GitPullRequest,
                "No project open",
                "Open a project with a GitHub remote to see its pull requests here.",
                &self.empty_refresh_button,
                app,
            ));
        } else if let Some(error) = data.and_then(|data| data.error.as_deref()) {
            column.add_child(self.render_error(error, app));
        } else if data.is_none_or(|data| !data.fetched) {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "Loading pull requests…",
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
        } else if data.is_some_and(|data| data.prs.is_empty()) {
            column.add_child(crate::automation::render_empty_state(
                Icon::GitPullRequest,
                "All clear",
                "No pull requests match this filter — refresh to check for new ones.",
                &self.empty_refresh_button,
                app,
            ));
        } else if let Some(data) = data {
            // Prune hover states for PRs no longer listed so the map doesn't
            // grow monotonically across refreshes/filter changes.
            {
                let current: std::collections::HashSet<u64> =
                    data.prs.iter().map(|pr| pr.number).collect();
                self.rows
                    .borrow_mut()
                    .retain(|number, _| current.contains(number));
            }
            let (needs_review, yours, others) = group_prs(&data.prs, store.viewer());
            let now = chrono::Utc::now();
            for (label, group) in [
                ("Needs your review", needs_review),
                ("Yours", yours),
                ("Others", others),
            ] {
                if group.is_empty() {
                    continue;
                }
                column.add_child(
                    Container::new(
                        Text::new_inline(label, appearance.ui_font_family(), type_ramp::LABEL.size)
                            .with_line_height_ratio(type_ramp::LABEL.line_height)
                            .with_color(sub.into())
                            .finish(),
                    )
                    .with_margin_top(spacing::MD)
                    .with_margin_bottom(spacing::XS)
                    .finish(),
                );
                for pr in group {
                    column.add_child(self.render_row(pr, now, app));
                }
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

    /// Page-level error card: gh missing, non-GitHub origin, auth failures.
    fn render_error(&self, error: &str, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Container::new(
            Text::new(
                format!("Couldn't load pull requests: {error}"),
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

    /// One PR row: CI dot, title + author/updated/branch, badges, and a
    /// trailing open-externally affordance. The row body opens the PR in the
    /// built-in Browser pane.
    fn render_row(
        &self,
        pr: &PrEntry,
        now: chrono::DateTime<chrono::Utc>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let states = self.rows.borrow_mut().entry(pr.number).or_default().clone();

        let ci_color: ColorU = match pr.ci {
            Some(PrCiState::Passing) => theme.ui_green_color(),
            Some(PrCiState::Failing) => theme.ui_error_color().into(),
            Some(PrCiState::Pending) => theme.ui_warning_color(),
            None => theme.outline().into(),
        };
        let ci_dot = ConstrainedBox::new(
            Container::new(Flex::row().with_main_axis_size(MainAxisSize::Min).finish())
                .with_background(ci_color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
        )
        .with_width(8.)
        .with_height(8.)
        .finish();

        let mut sub_line = pr.author.clone();
        let updated = relative_updated_at(&pr.updated_at, now);
        if !updated.is_empty() {
            sub_line.push_str(&format!(" · updated {updated}"));
        }
        if !pr.head_ref.is_empty() {
            sub_line.push_str(&format!(" · {}", pr.head_ref));
        }

        let labels = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new(
                    format!("#{} {}", pr.number, pr.title),
                    appearance.ui_font_family(),
                    type_ramp::UI.size,
                )
                .with_line_height_ratio(type_ramp::UI.line_height)
                .with_color(theme.main_text_color(theme.background()).into())
                .finish(),
            )
            .with_child(
                Text::new(
                    sub_line,
                    appearance.ui_font_family(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(sub.into())
                .finish(),
            )
            .finish();

        let mut badges = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::XS);
        if pr.is_draft {
            badges.add_child(render_badge("Draft", sub.into(), appearance));
        }
        if let Some(decision) = pr.review_decision {
            let color: ColorU = match decision {
                crate::pull_requests::store::PrReviewDecision::Approved => theme.ui_green_color(),
                crate::pull_requests::store::PrReviewDecision::ChangesRequested => {
                    theme.ui_error_color().into()
                }
                crate::pull_requests::store::PrReviewDecision::ReviewRequired => {
                    theme.ui_warning_color()
                }
            };
            badges.add_child(render_badge(decision.label(), color, appearance));
        }
        if pr.conflicting {
            badges.add_child(render_badge(
                "Conflicts",
                theme.ui_error_color().into(),
                appearance,
            ));
        }

        let open_action =
            AutomationViewAction::PullRequests(PullRequestsPageAction::OpenDetail(pr.number));
        let body = Hoverable::new(states.row, move |state| {
            let mut row = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(spacing::SM)
                    .with_child(ci_dot)
                    .with_child(Shrinkable::new(1., Align::new(labels).left().finish()).finish())
                    .with_child(badges.finish())
                    .finish(),
            )
            .with_uniform_padding(spacing::SM)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if state.is_hovered() {
                row = row.with_background(theme.surface_overlay_2());
            }
            row.finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(open_action.clone()))
        .with_cursor(Cursor::PointingHand)
        .finish();

        // Trailing open-externally affordance, a SIBLING of the clickable row
        // body so the two click targets never nest.
        let external_action = AutomationViewAction::PullRequests(
            PullRequestsPageAction::OpenExternal(pr.url.clone()),
        );
        let external = Hoverable::new(states.external, move |state| {
            let color = if state.is_hovered() {
                theme.main_text_color(theme.background())
            } else {
                sub
            };
            Container::new(
                ConstrainedBox::new(Icon::Globe.to_warpui_icon(color).finish())
                    .with_width(spacing::MD)
                    .with_height(spacing::MD)
                    .finish(),
            )
            .with_uniform_padding(spacing::XS)
            .finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(external_action.clone()))
        .with_cursor(Cursor::PointingHand)
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::XS)
            .with_child(Shrinkable::new(1., body).finish())
            .with_child(external)
            .finish()
    }
}

/// A small outlined pill badge in the given accent color.
pub(crate) fn render_badge(
    text: &'static str,
    color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Container::new(
        Text::new_inline(text, appearance.ui_font_family(), type_ramp::CAPTION.size)
            .with_line_height_ratio(type_ramp::CAPTION.line_height)
            .with_color(color.into())
            .finish(),
    )
    .with_padding_left(spacing::XS)
    .with_padding_right(spacing::XS)
    .with_padding_top(1.)
    .with_padding_bottom(1.)
    .with_border(Border::all(1.).with_border_fill(color))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
    .finish()
}

/// The dropdown-item action selecting `path` (also the selection key).
fn repo_action(path: &Path) -> AutomationViewAction {
    AutomationViewAction::PullRequests(PullRequestsPageAction::SelectRepo(
        path.to_string_lossy().into_owned(),
    ))
}

/// Human label for a project root: its folder name.
fn project_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn new_filter_dropdown(
    selected: PrStateFilter,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<Dropdown<AutomationViewAction>> {
    let filter_action = |filter: PrStateFilter| {
        AutomationViewAction::PullRequests(PullRequestsPageAction::SetFilter(
            filter.as_str().to_owned(),
        ))
    };
    ctx.add_typed_action_view(|ctx| {
        let mut dropdown = Dropdown::new(ctx);
        dropdown.set_items(
            PrStateFilter::ALL
                .into_iter()
                .map(|filter| DropdownItem::new(filter.label(), filter_action(filter)))
                .collect(),
            ctx,
        );
        dropdown.set_selected_by_action(filter_action(selected), ctx);
        dropdown.set_top_bar_max_width(120.);
        dropdown
    })
}

fn new_refresh_button(
    label: &'static str,
    ctx: &mut ViewContext<AutomationView>,
) -> ViewHandle<ActionButton> {
    ctx.add_typed_action_view(|_| {
        ActionButton::new(label, SecondaryTheme).on_click(|ctx| {
            ctx.dispatch_typed_action(AutomationViewAction::PullRequests(
                PullRequestsPageAction::Refresh,
            ));
        })
    })
}
