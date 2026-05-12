//! Render entry point for the Open Changes side panel.
//!
//! 5a renders the read-only scaffold: repo header, Staged Changes
//! section, Changes section, no-repo state, and clean-tree state.
//! Commit area, diff view, hover actions, and Timeline ship in 5b–5e.

use std::path::Path;

use warpui::{
    elements::{
        Container, CrossAxisAlignment, Element, Flex, MainAxisSize, ParentElement, Shrinkable,
    },
    ui_components::components::UiComponent,
    AppContext, SingletonEntity,
};

use crate::appearance::Appearance;
use crate::open_changes::repo::{BranchState, FileEntry, FileStatus, InProgressOp, RepoState};
use crate::open_changes::OpenChangesModel;

pub mod header;
pub mod sections;

/// Top-level render for the Open Changes panel. Called from
/// `LeftPanelView::render` via the `ToolPanelView::OpenChanges` arm.
pub fn render(app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let state = OpenChangesModel::handle(app).read(app, |m, _| m.state.clone());

    let body: Box<dyn Element> = match state {
        None => render_no_repo(appearance),
        Some(state) => render_with_repo(&state, appearance),
    };

    Shrinkable::new(
        1.0,
        Container::new(body)
            .with_padding_left(10.0)
            .with_padding_right(10.0)
            .with_padding_top(8.0)
            .finish(),
    )
    .finish()
}

/// PRODUCT §3: shown when the focused pane is not inside a git repo, or
/// when no pane is focused.
fn render_no_repo(appearance: &Appearance) -> Box<dyn Element> {
    let title = appearance
        .ui_builder()
        .span("No git repo in the focused pane.")
        .with_soft_wrap()
        .build()
        .finish();
    let hint = appearance
        .ui_builder()
        .span("Open Changes follows the focused pane's working directory.")
        .with_soft_wrap()
        .build()
        .finish();

    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(6.0)
        .with_child(title)
        .with_child(hint)
        .finish()
}

fn render_with_repo(state: &RepoState, appearance: &Appearance) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(10.0)
        .with_child(header::render(state, appearance));

    if let Some(op) = state.op_in_progress.as_ref() {
        column = column.with_child(render_op_banner(op, appearance));
    }

    column = column
        .with_child(sections::render(
            "Staged Changes",
            state.staged.as_slice(),
            appearance,
        ))
        .with_child(sections::render(
            "Changes",
            state.changes.as_slice(),
            appearance,
        ));

    if state.staged.is_empty() && state.changes.is_empty() {
        column = column.with_child(
            appearance
                .ui_builder()
                .span("Working tree is clean.")
                .with_soft_wrap()
                .build()
                .finish(),
        );
    }

    column.finish()
}

fn render_op_banner(op: &InProgressOp, appearance: &Appearance) -> Box<dyn Element> {
    // PRODUCT §22: simple text banner in 5a; 5c upgrades this to
    // `InlineBannerStyle::Recommendation` once the inline-banner host
    // is wired into the panel.
    let abort_cmd = match op {
        InProgressOp::Merging => "git merge --abort",
        InProgressOp::Rebasing => "git rebase --abort",
        InProgressOp::CherryPicking => "git cherry-pick --abort",
        InProgressOp::Bisecting => "git bisect reset",
    };
    let msg = format!(
        "{} in progress — resolve conflicts then commit, or run `{}` in the terminal.",
        op.label(),
        abort_cmd
    );
    Container::new(
        appearance
            .ui_builder()
            .span(msg)
            .with_soft_wrap()
            .build()
            .finish(),
    )
    .with_padding_top(6.0)
    .with_padding_bottom(6.0)
    .finish()
}

/// Returns a 2-character "M ", "A ", "?  " style label rendered with the
/// glyph color from PRODUCT §8. The trailing space is monospace padding
/// so glyphs line up across rows.
pub(super) fn status_glyph_label(status: FileStatus) -> String {
    format!("{}  ", status.glyph())
}

/// Path-display formatter used by row rendering. Returns
/// `(basename, dir_or_empty)` so the caller can render the basename in
/// normal weight and the dir in a dim shade. PRODUCT §8.
pub(super) fn split_path_for_display(path: &Path) -> (String, String) {
    let basename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| {
            let mut display = p.to_string_lossy().to_string();
            if !display.ends_with('/') {
                display.push('/');
            }
            display
        })
        .unwrap_or_default();

    (basename, dir)
}

/// Branch label for the repo header (PRODUCT §4). Currently just the
/// branch display string; 5d adds the upstream tracking tooltip.
pub(super) fn branch_label(branch: &BranchState) -> String {
    branch.display_label()
}

/// The list shown by `sections::render`. Used by tests in 5b+.
pub(super) fn row_label(file: &FileEntry) -> String {
    let (basename, dir) = split_path_for_display(&file.path);
    if dir.is_empty() {
        basename
    } else {
        format!("{basename}  {dir}")
    }
}
