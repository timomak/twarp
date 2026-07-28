//! twarp 21c: the PR detail view's Files tab — per-file collapsible diff
//! cards with GitHub review threads rendered inline at their anchored lines.
//! Read-only; drafting/replying to threads is 21d.
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
use crate::code::editor::comment_editor::create_readonly_comment_markdown_editor;
use crate::code::editor::{add_color, add_overlay_color, remove_color, remove_overlay_color};
use crate::notebooks::editor::view::RichTextEditorView;
use crate::pull_requests::diff::{
    anchor_threads, default_expanded, total_line_count, FileThreadAnchors, PrDiffLine,
    PrDiffLineKind, PrFileDiff, PrFileStatus, PrReviewThread, BIG_PR_TOTAL_LINE_THRESHOLD,
};
use crate::pull_requests::list_page::PullRequestsPageAction;
use crate::pull_requests::store::{relative_updated_at, PrFilesData, PullRequestsStoreModel};

/// The Files tab renders wider than the conversation column.
pub(crate) const FILES_CONTENT_MAX_WIDTH: f32 = 1080.;

/// Max width of thread comment markdown bodies.
const THREAD_EDITOR_MAX_WIDTH: f32 = 720.;

/// Width of one line-number gutter column.
const LINE_NUMBER_WIDTH: f32 = 44.;

/// Width of the `+`/`−` marker column.
const MARKER_WIDTH: f32 = 16.;

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
                    card.add_child(diff_line_row(line, app));
                    let Some(anchored) = anchors.and_then(|a| a.inline.get(&(hunk_idx, line_idx)))
                    else {
                        continue;
                    };
                    for &thread_idx in anchored {
                        if let Some(thread) = threads.get(thread_idx) {
                            card.add_child(self.render_thread(thread_idx, thread, true, app));
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
        let indent = if indented {
            LINE_NUMBER_WIDTH * 2. + MARKER_WIDTH
        } else {
            spacing::SM
        };

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

/// One diff line: dual line-number gutter, marker, monospace content, and an
/// add/delete background overlay matching the Open Changes palette.
fn diff_line_row(line: &PrDiffLine, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sub = theme.sub_text_color(theme.background());
    let main = theme.main_text_color(theme.background());
    let mono = appearance.monospace_font_family();
    let size = appearance.monospace_font_size();

    let (marker, background): (&str, Option<ColorU>) = match line.kind {
        PrDiffLineKind::Add => ("+", Some(add_overlay_color(appearance))),
        PrDiffLineKind::Delete => ("−", Some(remove_overlay_color(appearance))),
        PrDiffLineKind::Context => ("", None),
    };
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

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(number(line.old_line))
        .with_child(number(line.new_line))
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
                Text::new(line.text.clone(), mono, size)
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
