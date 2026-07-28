//! twarp 21c/21d: the PR detail view's Files tab — per-file collapsible diff
//! cards with GitHub review threads rendered inline at their anchored lines,
//! plus the 21d write path: thread replies, resolve/unresolve, and locally
//! drafted inline comments (amber pending cards) batched into one review.
//!
//! Diff line colors reuse the code_review/Open Changes palette
//! ([`crate::code::editor::add_overlay_color`] etc). Long lines clip instead
//! of wrapping (matching the code_review panel); text selection inside diff
//! lines is not wired (SelectableArea plumbing is out of scope here).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use twarp_core::ui::tokens::{radius, spacing, type_ramp};
use twarp_core::ui::Icon;
use twarpui::{
    elements::{
        Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
        Flex, Hoverable, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
    },
    platform::Cursor,
    prelude::ColorU,
    text_layout::ClipConfig,
    units::Pixels,
    AppContext, SingletonEntity, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::automation::view::{AutomationView, AutomationViewAction};
use crate::code::editor::comment_editor::{
    create_editable_comment_markdown_editor, create_readonly_comment_markdown_editor,
};
use crate::code::editor::{add_color, add_overlay_color, remove_color, remove_overlay_color};
use crate::notebooks::editor::view::RichTextEditorView;
use crate::pull_requests::diff::{
    anchor_threads, default_expanded, total_line_count, FileThreadAnchors, PrDiffLine,
    PrDiffLineKind, PrDiffSide, PrFileDiff, PrFileStatus, PrReviewThread,
    BIG_PR_TOTAL_LINE_THRESHOLD,
};
use crate::pull_requests::list_page::PullRequestsPageAction;
use crate::pull_requests::review::{PrDraftComment, ReviewDrafts};
use crate::pull_requests::store::{relative_updated_at, PrFilesData, PullRequestsStoreModel};

/// The Files tab renders wider than the conversation column.
pub(crate) const FILES_CONTENT_MAX_WIDTH: f32 = 1080.;

/// Max width of thread comment markdown bodies.
const THREAD_EDITOR_MAX_WIDTH: f32 = 720.;

/// Width of one line-number gutter column.
const LINE_NUMBER_WIDTH: f32 = 44.;

/// Width of the `+`/`−` marker column.
const MARKER_WIDTH: f32 = 16.;

/// Width of the leading draft-comment ("+" on hover) gutter column (21d).
const DRAFT_COL_WIDTH: f32 = 18.;

/// Left indent of inline thread / draft cards, past the gutters.
const THREAD_INDENT: f32 = DRAFT_COL_WIDTH + LINE_NUMBER_WIDTH * 2. + MARKER_WIDTH;

fn action(action: PullRequestsPageAction) -> AutomationViewAction {
    AutomationViewAction::PullRequests(action)
}

/// UI state for the Files tab: expansion overrides, thread anchors, and the
/// per-comment markdown editors (built in [`Self::sync`], render passes only
/// get `&AppContext`).
#[derive(Default)]
pub struct FilesTabState {
    /// Fingerprint of the files data the derived state was built from.
    files_rev: Option<u64>,
    /// Index-aligned default expansion (first files expand unless big).
    default_expanded: Vec<bool>,
    /// User toggles overriding the defaults, keyed by file index.
    expanded_overrides: HashMap<usize, bool>,
    /// Expanded resolved threads, keyed by thread index (collapsed default).
    thread_expanded: HashMap<usize, bool>,
    /// Expanded "Outdated / other threads" sections, keyed by file index.
    floating_expanded: HashMap<usize, bool>,
    /// Index-aligned thread anchors per file.
    anchors: Vec<FileThreadAnchors>,
    /// One markdown editor per thread comment, aligned with
    /// `threads[i].comments[j]`.
    comment_editors: Vec<Vec<ViewHandle<RichTextEditorView>>>,
    file_states: RefCell<HashMap<usize, MouseStateHandle>>,
    misc_states: RefCell<HashMap<String, MouseStateHandle>>,
    /// The accumulating local review drafts (21d).
    drafts: ReviewDrafts,
    /// The one open inline draft editor, anchored at (file, hunk, line).
    draft_editor: Option<((usize, usize, usize), ViewHandle<RichTextEditorView>)>,
    /// Open reply editors, keyed by thread index.
    reply_editors: HashMap<usize, ViewHandle<RichTextEditorView>>,
    /// An in-flight thread mutation: (thread index, true for a reply / false
    /// for resolve-unresolve). The reply editor closes on success.
    thread_op: Option<(usize, bool)>,
    /// The last reply/resolve failure, shown under the affected thread.
    thread_error: Option<(usize, String)>,
}

impl FilesTabState {
    /// Rebuild the derived state (anchors, default expansion, comment
    /// editors) when the store's files data changed.
    pub fn sync(&mut self, number: u64, ctx: &mut ViewContext<AutomationView>) {
        let (fingerprint, files, threads) = {
            let store = PullRequestsStoreModel::as_ref(ctx);
            let Some(data) = store.detail_data(number) else {
                return;
            };
            // Settle an in-flight thread mutation (21d): close the reply
            // editor on success, surface the error under the thread on
            // failure. The store refetches the files data on success.
            if let Some((thread_idx, is_reply)) = self.thread_op {
                if !data.mutating {
                    self.thread_op = None;
                    match &data.mutation_error {
                        None => {
                            if is_reply {
                                self.reply_editors.remove(&thread_idx);
                            }
                            self.thread_error = None;
                        }
                        Some(error) => self.thread_error = Some((thread_idx, error.clone())),
                    }
                }
            }
            if !data.files.fetched {
                return;
            }
            (
                files_fingerprint(&data.files),
                data.files.files.clone(),
                data.files.threads.clone(),
            )
        };
        if self.files_rev == Some(fingerprint) {
            return;
        }
        self.files_rev = Some(fingerprint);
        self.expanded_overrides.clear();
        self.thread_expanded.clear();
        self.floating_expanded.clear();
        // Thread indices shift on refetch: drop index-keyed editor state. The
        // accumulated review drafts survive (they anchor by path/line/side).
        self.reply_editors.clear();
        self.draft_editor = None;
        self.default_expanded = default_expanded(&files);
        self.anchors = anchor_threads(&files, &threads);
        let max_width = Some(Pixels::new(THREAD_EDITOR_MAX_WIDTH));
        self.comment_editors = threads
            .iter()
            .map(|thread| {
                thread
                    .comments
                    .iter()
                    .map(|comment| {
                        create_readonly_comment_markdown_editor(&comment.body, true, max_width, ctx)
                    })
                    .collect()
            })
            .collect();
    }

    pub fn toggle_file(&mut self, index: usize) {
        let current = self.is_file_expanded(index);
        self.expanded_overrides.insert(index, !current);
    }

    pub fn toggle_thread(&mut self, index: usize) {
        let current = self.thread_expanded.get(&index).copied().unwrap_or(false);
        self.thread_expanded.insert(index, !current);
    }

    pub fn toggle_floating(&mut self, file_index: usize) {
        let current = self
            .floating_expanded
            .get(&file_index)
            .copied()
            .unwrap_or(false);
        self.floating_expanded.insert(file_index, !current);
    }

    pub fn drafts(&self) -> &ReviewDrafts {
        &self.drafts
    }

    pub fn drafts_mut(&mut self) -> &mut ReviewDrafts {
        &mut self.drafts
    }

    /// Clear the drafts after a successful review submit.
    pub fn clear_drafts(&mut self) {
        self.drafts.clear();
        self.draft_editor = None;
    }

    /// Open/close the inline reply editor on one thread (21d).
    pub fn toggle_reply(&mut self, thread_idx: usize, ctx: &mut ViewContext<AutomationView>) {
        if self.reply_editors.remove(&thread_idx).is_none() {
            self.reply_editors.insert(
                thread_idx,
                create_editable_comment_markdown_editor(None, ctx),
            );
        }
    }

    /// Submit the open reply editor on one thread via the store (21d).
    pub fn submit_reply(
        &mut self,
        number: u64,
        thread_idx: usize,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        let Some(editor) = self.reply_editors.get(&thread_idx) else {
            return;
        };
        let body = editor.as_ref(ctx).model().as_ref(ctx).markdown(ctx);
        if body.trim().is_empty() {
            return;
        }
        let Some(thread_id) = self.thread_id(number, thread_idx, ctx) else {
            return;
        };
        let started = PullRequestsStoreModel::handle(ctx).update(ctx, |store, ctx| {
            store.reply_thread(number, thread_id, body, ctx)
        });
        if started {
            self.thread_op = Some((thread_idx, true));
            self.thread_error = None;
        }
    }

    /// Resolve / unresolve one thread via the store (21d).
    pub fn set_thread_resolved(
        &mut self,
        number: u64,
        thread_idx: usize,
        resolved: bool,
        ctx: &mut ViewContext<AutomationView>,
    ) {
        let Some(thread_id) = self.thread_id(number, thread_idx, ctx) else {
            return;
        };
        let started = PullRequestsStoreModel::handle(ctx).update(ctx, |store, ctx| {
            store.set_thread_resolved(number, thread_id, resolved, ctx)
        });
        if started {
            self.thread_op = Some((thread_idx, false));
            self.thread_error = None;
        }
    }

    fn thread_id(
        &self,
        number: u64,
        thread_idx: usize,
        ctx: &ViewContext<AutomationView>,
    ) -> Option<String> {
        let store = PullRequestsStoreModel::as_ref(ctx);
        let id = &store.detail_data(number)?.files.threads.get(thread_idx)?.id;
        (!id.is_empty()).then(|| id.clone())
    }

    /// Open the inline draft editor at one diff position (21d). Reuses the
    /// open editor's position slot — only one draft is edited at a time.
    pub fn start_draft(
        &mut self,
        position: (usize, usize, usize),
        ctx: &mut ViewContext<AutomationView>,
    ) {
        match &mut self.draft_editor {
            Some((existing, _)) => *existing = position,
            None => {
                self.draft_editor =
                    Some((position, create_editable_comment_markdown_editor(None, ctx)));
            }
        }
    }

    pub fn cancel_draft(&mut self) {
        self.draft_editor = None;
    }

    /// Commit the open draft editor into the local drafts list, mapping the
    /// diff position to GitHub's (path, line, side) coordinates: RIGHT/new
    /// numbering for add+context lines, LEFT/old numbering for deletions.
    pub fn save_draft(&mut self, number: u64, ctx: &mut ViewContext<AutomationView>) {
        let Some((position, editor)) = &self.draft_editor else {
            return;
        };
        let position = *position;
        let body = editor.as_ref(ctx).model().as_ref(ctx).markdown(ctx);
        if body.trim().is_empty() {
            return;
        }
        let coords = {
            let store = PullRequestsStoreModel::as_ref(ctx);
            store.detail_data(number).and_then(|data| {
                let file = data.files.files.get(position.0)?;
                let line = file.hunks.get(position.1)?.lines.get(position.2)?;
                let (side, number) = match line.kind {
                    PrDiffLineKind::Delete => (PrDiffSide::Left, line.old_line?),
                    _ => (PrDiffSide::Right, line.new_line?),
                };
                Some((file.path.clone(), number, side))
            })
        };
        let Some((path, line, side)) = coords else {
            return;
        };
        self.drafts.add(PrDraftComment {
            path,
            line,
            side,
            position,
            body,
        });
        self.draft_editor = None;
    }

    fn is_file_expanded(&self, index: usize) -> bool {
        self.expanded_overrides
            .get(&index)
            .copied()
            .or_else(|| self.default_expanded.get(index).copied())
            .unwrap_or(false)
    }

    /// Render the whole tab into the detail page's column.
    pub fn render(&self, column: &mut Flex, files_data: &PrFilesData, app: &AppContext) {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());

        if let Some(error) = &files_data.error {
            column.add_child(note_row(
                &format!("Couldn't load the diff: {error}"),
                theme.ui_error_color(),
                app,
            ));
            return;
        }
        if !files_data.fetched {
            column.add_child(note_row("Loading diff…", sub.into(), app));
            return;
        }

        let files = &files_data.files;
        let additions: u64 = files.iter().map(PrFileDiff::additions).sum();
        let deletions: u64 = files.iter().map(PrFileDiff::deletions).sum();
        let mut summary = format!(
            "{} file{} changed, +{additions} −{deletions}",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
        );
        if total_line_count(files) > BIG_PR_TOTAL_LINE_THRESHOLD {
            summary.push_str(" · large diff — files start collapsed");
        }
        column.add_child(
            Container::new(
                Text::new(summary, appearance.ui_font_family(), type_ramp::UI.size)
                    .with_line_height_ratio(type_ramp::UI.line_height)
                    .with_color(sub.into())
                    .finish(),
            )
            .with_margin_bottom(spacing::SM)
            .finish(),
        );
        if files_data.threads_truncated {
            column.add_child(note_row(
                "Only the first 100 review threads are shown.",
                sub.into(),
                app,
            ));
        }
        if files.is_empty() {
            column.add_child(note_row("This pull request has no diff.", sub.into(), app));
            return;
        }

        for (index, file) in files.iter().enumerate() {
            column.add_child(self.render_file_card(index, file, &files_data.threads, app));
        }
    }

    /// One collapsible file card: header row + (when expanded) hunks with
    /// inline threads and the floating-threads section.
    fn render_file_card(
        &self,
        index: usize,
        file: &PrFileDiff,
        threads: &[PrReviewThread],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let expanded = self.is_file_expanded(index);

        let mut card = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        card.add_child(self.render_file_header(index, file, expanded, app));

        if expanded {
            let anchors = self.anchors.get(index);
            if file.is_binary {
                card.add_child(note_row(
                    "Binary file not shown.",
                    theme.sub_text_color(theme.background()).into(),
                    app,
                ));
            } else if file.hunks.is_empty() {
                card.add_child(note_row(
                    "No content changes (mode or rename only).",
                    theme.sub_text_color(theme.background()).into(),
                    app,
                ));
            }
            for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                card.add_child(hunk_header_row(&hunk.header, app));
                for (line_idx, line) in hunk.lines.iter().enumerate() {
                    let position = (index, hunk_idx, line_idx);
                    card.add_child(self.diff_line_row(position, line, app));
                    for draft_idx in self.drafts.at_position(position) {
                        if let Some(draft) = self.drafts.drafts().get(draft_idx) {
                            card.add_child(self.render_pending_draft(draft_idx, draft, app));
                        }
                    }
                    if let Some((open, editor)) = &self.draft_editor {
                        if *open == position {
                            card.add_child(self.render_draft_editor(editor, app));
                        }
                    }
                    if let Some(anchored) =
                        anchors.and_then(|a| a.inline.get(&(hunk_idx, line_idx)))
                    {
                        for &thread_idx in anchored {
                            if let Some(thread) = threads.get(thread_idx) {
                                card.add_child(self.render_thread(thread_idx, thread, true, app));
                            }
                        }
                    }
                }
            }
            if let Some(anchors) = anchors {
                if !anchors.floating.is_empty() {
                    card.add_child(self.render_floating_section(index, anchors, threads, app));
                }
            }
        }

        Container::new(card.finish())
            .with_margin_bottom(spacing::SM)
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
            .finish()
    }

    /// Chevron, path (rename as "old → new"), +/− counts.
    fn render_file_header(
        &self,
        index: usize,
        file: &PrFileDiff,
        expanded: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let main = theme.main_text_color(theme.background());
        let sub = theme.sub_text_color(theme.background());
        let state = self
            .file_states
            .borrow_mut()
            .entry(index)
            .or_default()
            .clone();

        let title = match &file.status {
            PrFileStatus::Renamed { old_path } => format!("{old_path} → {}", file.path),
            _ => file.path.clone(),
        };
        let status_label = match &file.status {
            PrFileStatus::Added => Some(("added", ColorU::from(add_color(appearance)))),
            PrFileStatus::Deleted => Some(("deleted", ColorU::from(remove_color(appearance)))),
            _ => None,
        };
        let (additions, deletions) = (file.additions(), file.deletions());
        let is_binary = file.is_binary;
        let family = appearance.ui_font_family();
        let add_c: ColorU = add_color(appearance);
        let del_c: ColorU = remove_color(appearance);

        let header = Hoverable::new(state, move |hover| {
            let chevron = if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            let chevron_color = if hover.is_hovered() { main } else { sub };
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(spacing::XS)
                .with_child(
                    ConstrainedBox::new(chevron.to_warpui_icon(chevron_color).finish())
                        .with_width(spacing::MD)
                        .with_height(spacing::MD)
                        .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Text::new(title.clone(), family, type_ramp::UI.size)
                            .with_line_height_ratio(type_ramp::UI.line_height)
                            .with_color(main.into())
                            .soft_wrap(false)
                            .with_clip(ClipConfig::start())
                            .finish(),
                    )
                    .finish(),
                );
            if let Some((label, color)) = status_label {
                row.add_child(
                    Text::new_inline(label, family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(color)
                        .finish(),
                );
            }
            if is_binary {
                row.add_child(
                    Text::new_inline("binary", family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(sub.into())
                        .finish(),
                );
            } else {
                row.add_child(
                    Text::new(format!("+{additions}"), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(add_c)
                        .finish(),
                );
                row.add_child(
                    Text::new(format!("−{deletions}"), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(del_c)
                        .finish(),
                );
            }
            let mut container = Container::new(row.finish())
                .with_uniform_padding(spacing::SM)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)));
            if hover.is_hovered() {
                container = container.with_background(theme.surface_overlay_2());
            }
            container.finish()
        });
        header
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ToggleFileCard(
                    index as u64,
                )))
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    /// One review thread card. Resolved threads render collapsed as a
    /// one-line "Resolved thread — expand" row until toggled.
    fn render_thread(
        &self,
        thread_idx: usize,
        thread: &PrReviewThread,
        indented: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());
        let family = appearance.ui_font_family();
        let indent = if indented { THREAD_INDENT } else { spacing::SM };

        let expanded = !thread.is_resolved
            || self
                .thread_expanded
                .get(&thread_idx)
                .copied()
                .unwrap_or(false);
        let state = self
            .misc_states
            .borrow_mut()
            .entry(format!("thread-{thread_idx}"))
            .or_default()
            .clone();

        if !expanded {
            let count = thread.comments.len();
            let label = format!(
                "Resolved thread — {count} comment{} · expand",
                if count == 1 { "" } else { "s" }
            );
            let collapsed = Hoverable::new(state, move |hover| {
                let color = if hover.is_hovered() { main } else { sub };
                Container::new(
                    Text::new(label.clone(), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(color.into())
                        .finish(),
                )
                .with_uniform_padding(spacing::XS)
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ToggleFileThread(
                    thread_idx as u64,
                )))
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
            return Container::new(collapsed)
                .with_margin_left(indent)
                .with_margin_right(spacing::SM)
                .with_margin_top(spacing::XXS)
                .with_margin_bottom(spacing::XXS)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
                .finish();
        }

        let mut body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(spacing::XS);
        if thread.is_resolved {
            let collapse = Hoverable::new(state, move |hover| {
                let color = if hover.is_hovered() { main } else { sub };
                Text::new_inline("Resolved · collapse", family, type_ramp::CAPTION.size)
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(color.into())
                    .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ToggleFileThread(
                    thread_idx as u64,
                )))
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
            body.add_child(collapse);
        }
        let now = chrono::Utc::now();
        for (comment_idx, comment) in thread.comments.iter().enumerate() {
            let when = relative_updated_at(&comment.created_at, now);
            let mut header = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(spacing::XS)
                .with_child(
                    Text::new(comment.author.clone(), family, type_ramp::UI.size)
                        .with_line_height_ratio(type_ramp::UI.line_height)
                        .with_color(main.into())
                        .finish(),
                );
            if !when.is_empty() {
                header.add_child(
                    Text::new(format!("· {when}"), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(sub.into())
                        .finish(),
                );
            }
            body.add_child(header.finish());
            if let Some(editor) = self
                .comment_editors
                .get(thread_idx)
                .and_then(|comments| comments.get(comment_idx))
            {
                body.add_child(ChildView::new(editor).finish());
            }
        }

        // 21d: reply / resolve write affordances.
        if let Some((error_idx, error)) = &self.thread_error {
            if *error_idx == thread_idx {
                body.add_child(
                    Text::new(error.clone(), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(theme.ui_error_color())
                        .finish(),
                );
            }
        }
        if let Some(editor) = self.reply_editors.get(&thread_idx) {
            body.add_child(ChildView::new(editor).finish());
            body.add_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(spacing::SM)
                    .with_child(self.text_affordance(
                        &format!("reply-send-{thread_idx}"),
                        "Send reply",
                        PullRequestsPageAction::SubmitThreadReply(thread_idx as u64),
                        app,
                    ))
                    .with_child(self.text_affordance(
                        &format!("reply-cancel-{thread_idx}"),
                        "Cancel",
                        PullRequestsPageAction::ToggleThreadReply(thread_idx as u64),
                        app,
                    ))
                    .finish(),
            );
        } else {
            let mut footer = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(spacing::SM)
                .with_child(self.text_affordance(
                    &format!("reply-{thread_idx}"),
                    "Reply",
                    PullRequestsPageAction::ToggleThreadReply(thread_idx as u64),
                    app,
                ));
            if !thread.id.is_empty() {
                let (label, resolve): (&'static str, bool) = if thread.is_resolved {
                    ("Unresolve", false)
                } else {
                    ("Resolve", true)
                };
                footer.add_child(self.text_affordance(
                    &format!("resolve-{thread_idx}"),
                    label,
                    PullRequestsPageAction::SetThreadResolved(thread_idx as u64, resolve),
                    app,
                ));
            }
            body.add_child(footer.finish());
        }

        Container::new(
            Container::new(body.finish())
                .with_uniform_padding(spacing::SM)
                .finish(),
        )
        .with_margin_left(indent)
        .with_margin_right(spacing::SM)
        .with_margin_top(spacing::XXS)
        .with_margin_bottom(spacing::XXS)
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .finish()
    }

    /// The collapsed "Outdated / other threads" section at a card's bottom.
    fn render_floating_section(
        &self,
        file_index: usize,
        anchors: &FileThreadAnchors,
        threads: &[PrReviewThread],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());
        let family = appearance.ui_font_family();
        let expanded = self
            .floating_expanded
            .get(&file_index)
            .copied()
            .unwrap_or(false);
        let count = anchors.floating.len();
        let state = self
            .misc_states
            .borrow_mut()
            .entry(format!("floating-{file_index}"))
            .or_default()
            .clone();

        let label = format!(
            "{count} outdated / other thread{} — {}",
            if count == 1 { "" } else { "s" },
            if expanded { "collapse" } else { "expand" }
        );
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        column.add_child(
            Hoverable::new(state, move |hover| {
                let color = if hover.is_hovered() { main } else { sub };
                Container::new(
                    Text::new(label.clone(), family, type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(color.into())
                        .finish(),
                )
                .with_uniform_padding(spacing::SM)
                .finish()
            })
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action(PullRequestsPageAction::ToggleFileThreads(
                    file_index as u64,
                )))
            })
            .with_cursor(Cursor::PointingHand)
            .finish(),
        );
        if expanded {
            for &thread_idx in &anchors.floating {
                if let Some(thread) = threads.get(thread_idx) {
                    column.add_child(self.render_thread(thread_idx, thread, false, app));
                }
            }
        }
        Container::new(column.finish())
            .with_border(Border::top(1.).with_border_fill(theme.outline()))
            .finish()
    }
}

/// One "@@ …" hunk-separator row.
fn hunk_header_row(header: &str, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sub = theme.sub_text_color(theme.background());
    Container::new(
        Text::new(
            header.to_owned(),
            appearance.monospace_font_family(),
            type_ramp::CAPTION.size,
        )
        .with_line_height_ratio(type_ramp::CAPTION.line_height)
        .with_color(sub.into())
        .soft_wrap(false)
        .with_clip(ClipConfig::end())
        .finish(),
    )
    .with_horizontal_padding(spacing::SM)
    .with_vertical_padding(spacing::XXS)
    .with_background(theme.surface_overlay_1())
    .finish()
}

impl FilesTabState {
    /// One diff line: draft-"+" gutter (hover affordance, 21d), dual
    /// line-number gutter, marker, monospace content, and an add/delete
    /// background overlay matching the Open Changes palette.
    fn diff_line_row(
        &self,
        position: (usize, usize, usize),
        line: &PrDiffLine,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let sub = theme.sub_text_color(theme.background());
        let main = theme.main_text_color(theme.background());
        let mono = appearance.monospace_font_family();
        let size = appearance.monospace_font_size();
        let (file_idx, hunk_idx, line_idx) = position;

        let (marker, background): (&str, Option<ColorU>) = match line.kind {
            PrDiffLineKind::Add => ("+", Some(add_overlay_color(appearance))),
            PrDiffLineKind::Delete => ("−", Some(remove_overlay_color(appearance))),
            PrDiffLineKind::Context => ("", None),
        };
        // Lines carrying a side-appropriate number can take a draft comment.
        let can_draft = match line.kind {
            PrDiffLineKind::Delete => line.old_line.is_some(),
            _ => line.new_line.is_some(),
        };
        let row_state = self
            .misc_states
            .borrow_mut()
            .entry(format!("line-{file_idx}-{hunk_idx}-{line_idx}"))
            .or_default()
            .clone();
        let plus_state = self
            .misc_states
            .borrow_mut()
            .entry(format!("line-plus-{file_idx}-{hunk_idx}-{line_idx}"))
            .or_default()
            .clone();
        let text = line.text.clone();
        let (old_line, new_line) = (line.old_line, line.new_line);
        let accent = theme.accent().into_solid();

        Hoverable::new(row_state, move |row_hover| {
            let number = |n: Option<u64>| {
                ConstrainedBox::new(
                    Text::new(
                        n.map(|n| n.to_string()).unwrap_or_default(),
                        mono.clone(),
                        type_ramp::CAPTION.size,
                    )
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(sub.into())
                    .soft_wrap(false)
                    .finish(),
                )
                .with_width(LINE_NUMBER_WIDTH)
                .finish()
            };
            let plus: Box<dyn Element> = if can_draft && row_hover.is_hovered() {
                Hoverable::new(plus_state.clone(), move |hover| {
                    let color: ColorU = if hover.is_hovered() {
                        accent
                    } else {
                        main.into()
                    };
                    ConstrainedBox::new(
                        Text::new_inline("+", mono.clone(), size)
                            .with_color(color)
                            .finish(),
                    )
                    .with_width(DRAFT_COL_WIDTH)
                    .finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(action(PullRequestsPageAction::StartDraftComment(
                        file_idx as u64,
                        hunk_idx as u64,
                        line_idx as u64,
                    )))
                })
                .with_cursor(Cursor::PointingHand)
                .finish()
            } else {
                ConstrainedBox::new(Flex::row().with_main_axis_size(MainAxisSize::Min).finish())
                    .with_width(DRAFT_COL_WIDTH)
                    .finish()
            };
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(plus)
                .with_child(number(old_line))
                .with_child(number(new_line))
                .with_child(
                    ConstrainedBox::new(
                        Text::new_inline(marker, mono.clone(), size)
                            .with_color(main.into())
                            .finish(),
                    )
                    .with_width(MARKER_WIDTH)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Text::new(text.clone(), mono.clone(), size)
                            .with_color(main.into())
                            .soft_wrap(false)
                            .with_clip(ClipConfig::end())
                            .finish(),
                    )
                    .finish(),
                )
                .finish();
            let mut container = Container::new(row).with_horizontal_padding(spacing::XS);
            if let Some(background) = background {
                container = container.with_background(background);
            }
            container.finish()
        })
        .finish()
    }

    /// One amber "pending" card for a locally drafted comment (21d), with a
    /// per-draft discard affordance.
    fn render_pending_draft(
        &self,
        draft_idx: usize,
        draft: &PrDraftComment,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let amber = theme.ui_warning_color();
        let main = theme.main_text_color(theme.background());
        let sub = theme.sub_text_color(theme.background());
        let family = appearance.ui_font_family();
        let discard_state = self
            .misc_states
            .borrow_mut()
            .entry(format!("draft-x-{draft_idx}"))
            .or_default()
            .clone();

        let header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(spacing::XS)
            .with_child(
                Text::new_inline("Pending", family.clone(), type_ramp::CAPTION.size)
                    .with_line_height_ratio(type_ramp::CAPTION.line_height)
                    .with_color(amber)
                    .finish(),
            )
            .with_child(
                Text::new_inline(
                    "part of your unsubmitted review",
                    family.clone(),
                    type_ramp::CAPTION.size,
                )
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(sub.into())
                .finish(),
            )
            .with_child(Shrinkable::new(1., Flex::row().finish()).finish())
            .with_child(
                Hoverable::new(discard_state, move |hover| {
                    let color = if hover.is_hovered() { main } else { sub };
                    Text::new_inline("Discard", family.clone(), type_ramp::CAPTION.size)
                        .with_line_height_ratio(type_ramp::CAPTION.line_height)
                        .with_color(color.into())
                        .finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(action(PullRequestsPageAction::DiscardDraft(
                        draft_idx as u64,
                    )))
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .finish();

        let body = Text::new(
            draft.body.clone(),
            appearance.ui_font_family(),
            type_ramp::UI.size,
        )
        .with_line_height_ratio(type_ramp::UI.line_height)
        .with_color(main.into())
        .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(spacing::XS)
                .with_child(header)
                .with_child(body)
                .finish(),
        )
        .with_uniform_padding(spacing::SM)
        .with_margin_left(THREAD_INDENT)
        .with_margin_right(spacing::SM)
        .with_margin_top(spacing::XXS)
        .with_margin_bottom(spacing::XXS)
        .with_border(Border::all(1.).with_border_fill(amber))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .finish()
    }

    /// The inline draft editor card (21d): editable markdown editor plus
    /// "Add review comment" / "Cancel" affordances.
    fn render_draft_editor(
        &self,
        editor: &ViewHandle<RichTextEditorView>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        let buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(spacing::SM)
            .with_child(self.text_affordance(
                "draft-save",
                "Add review comment",
                PullRequestsPageAction::SaveDraftComment,
                app,
            ))
            .with_child(self.text_affordance(
                "draft-cancel",
                "Cancel",
                PullRequestsPageAction::CancelDraftComment,
                app,
            ))
            .finish();
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(spacing::XS)
                .with_child(ChildView::new(editor).finish())
                .with_child(buttons)
                .finish(),
        )
        .with_uniform_padding(spacing::SM)
        .with_margin_left(THREAD_INDENT)
        .with_margin_right(spacing::SM)
        .with_margin_top(spacing::XXS)
        .with_margin_bottom(spacing::XXS)
        .with_border(Border::all(1.).with_border_fill(theme.ui_warning_color()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius::CARD)))
        .finish()
    }

    /// A small hover-highlighted clickable text label dispatching one action.
    fn text_affordance(
        &self,
        key: &str,
        label: &'static str,
        page_action: PullRequestsPageAction,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let main = theme.main_text_color(theme.background());
        let sub = theme.sub_text_color(theme.background());
        let family = appearance.ui_font_family();
        let state = self
            .misc_states
            .borrow_mut()
            .entry(key.to_owned())
            .or_default()
            .clone();
        Hoverable::new(state, move |hover| {
            let color = if hover.is_hovered() { main } else { sub };
            Text::new_inline(label, family.clone(), type_ramp::CAPTION.size)
                .with_line_height_ratio(type_ramp::CAPTION.line_height)
                .with_color(color.into())
                .finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action(page_action.clone())))
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

/// A padded single-line note inside a card or the tab body.
fn note_row(text: &str, color: ColorU, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Container::new(
        Text::new(
            text.to_owned(),
            appearance.ui_font_family(),
            type_ramp::UI.size,
        )
        .with_line_height_ratio(type_ramp::UI.line_height)
        .with_color(color)
        .finish(),
    )
    .with_uniform_padding(spacing::SM)
    .finish()
}

/// Fingerprint of the files data the derived state depends on.
fn files_fingerprint(data: &PrFilesData) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for file in &data.files {
        file.path.hash(&mut hasher);
        file.patch_line_count().hash(&mut hasher);
        file.additions().hash(&mut hasher);
        file.deletions().hash(&mut hasher);
    }
    for thread in &data.threads {
        thread.path.hash(&mut hasher);
        thread.line.hash(&mut hasher);
        thread.is_resolved.hash(&mut hasher);
        thread.is_outdated.hash(&mut hasher);
        for comment in &thread.comments {
            comment.author.hash(&mut hasher);
            comment.created_at.hash(&mut hasher);
            comment.body.hash(&mut hasher);
        }
    }
    hasher.finish()
}
