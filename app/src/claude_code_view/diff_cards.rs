//! Diff cards for the Claude Code pane (feature 07, sub-phase 7e; PRODUCT
//! §20–§21).
//!
//! An `Edit` / `MultiEdit` / `Write` tool call renders as a **diff card**: the
//! file path in a ported `inline_action` header, and the old → new change as
//! a real diff — rendered by **feature 05's** diff stack ([`InlineDiffView`]
//! wrapping a [`CodeEditorView`] seeded with the old text, the change applied
//! as [`DiffDelta`]s), exactly the construction the deleted Agent Mode
//! `code_diff_view` used and the Open Changes panel uses today. The +/- line
//! tinting, hunk gutters, and monospace treatment therefore match every other
//! diff twarp shows (§21), from the active theme.
//!
//! Read-only on purpose: no file is registered with `FileModel`, which makes
//! the embedded editor selection-only (`InlineDiffView` docs) — no
//! staging/accept/revert affordances here (§21).
//!
//! The headless half (parsing the tool input into a [`ToolDiff`]) lives in
//! `claude_code::diff` where it is unit-tested; this module owns the views.

use claude_code::diff::ToolDiff;
use claude_code::{ToolOutput, ToolStatus};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::standardized_path::StandardizedPath;
use warpui::{
    elements::{ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisSize, ParentElement},
    presenter::ChildView,
    AppContext, Element, SingletonEntity, ViewContext, ViewHandle,
};

use super::inline_action::{render_requested_action_body_text, Disclosure, RightCluster};
use super::tool_cards::{status_icon, verb_title_row, ToolCardUi};
use super::{ClaudeCodeView, ClaudeCodeViewAction};
use crate::app_state::{DiffDelta, DiffType};
use crate::appearance::Appearance;
use crate::code::diff_viewer::{DiffViewer, DisplayMode};
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code::inline_diff::InlineDiffView;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as WarpIcon;

/// Height bound for one embedded diff. Kept very large on purpose: a diff that
/// scrolls *internally* traps the mouse wheel, so scrolling over a code edit
/// never scrolls the transcript behind it (the "scroll the page too" report).
/// Letting the diff grow to its full content height instead means there's no
/// inner vertical scroll to capture — wheel events flow straight to the
/// transcript — while a finite bound still satisfies the editor's flex layout
/// (it needs a bounded vertical constraint or its inner flex panics). Practical
/// diffs render in full; only a pathologically huge `Write` would clip here.
const DIFF_MAX_HEIGHT: f32 = 100_000.;

/// The live diff views backing one tool call's card, one per edit
/// (PRODUCT §20: `MultiEdit` renders each edit in order).
pub(super) struct DiffCard {
    pub file_path: String,
    pub viewers: Vec<ViewHandle<InlineDiffView>>,
    pub added: usize,
    pub removed: usize,
}

/// `apply_diffs` replaces whole base lines; a partial trailing line in the
/// tool input would silently merge into the next line's render.
fn ensure_trailing_newline(mut text: String) -> String {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Build the feature-05 diff views for one synthesized [`ToolDiff`]
/// (TECH §The chat UI: editor seeded with the old text, the new text applied
/// as a delta — the `code_diff_view` construction, minus its interactivity).
pub(super) fn build_diff_card(diff: ToolDiff, ctx: &mut ViewContext<ClaudeCodeView>) -> DiffCard {
    let (added, removed) = diff.line_counts();
    let viewers = diff
        .edits
        .into_iter()
        .map(|edit| {
            let editor = ctx.add_typed_action_view(|ctx| {
                CodeEditorView::new(
                    None,
                    None,
                    CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight)
                        .lazy_layout(),
                    ctx,
                )
            });
            let old = ensure_trailing_newline(edit.old);
            let new = ensure_trailing_newline(edit.new);
            let old_lines = old.lines().count();
            editor.update(ctx, |editor_view, ctx| {
                // Syntax highlighting + the base ("old") content the deltas
                // diff against.
                editor_view.set_language_with_path(Path::new(&diff.file_path), ctx);
                editor_view.reset(InitialBufferState::plain_text(&old), ctx);
            });
            let delta = DiffDelta {
                replacement_line_range: 0..old_lines,
                insertion: new,
            };
            let diff_type = if old_lines == 0 {
                // A `Write` (all-additions, PRODUCT §20).
                DiffType::Create { delta }
            } else {
                DiffType::Update {
                    deltas: vec![delta],
                    rename: None,
                }
            };
            let standardized_path = StandardizedPath::try_new(&diff.file_path).ok();
            ctx.add_typed_action_view(|ctx| {
                // No `register_file` → read-only (selection-only editor), §21.
                InlineDiffView::new(
                    editor,
                    Some(diff_type),
                    Some(DisplayMode::Embedded {
                        max_height: DIFF_MAX_HEIGHT,
                    }),
                    standardized_path,
                    ctx,
                )
            })
        })
        .collect();
    DiffCard {
        file_path: diff.file_path,
        viewers,
        added,
        removed,
    }
}

/// Diff cards open showing their diff — the diff *is* the card's content
/// (PRODUCT §20). The user's collapse choice still wins (handled by the
/// caller via [`ToolCardUi::expanded_override`]).
pub(super) fn default_expanded() -> bool {
    true
}

/// The collapsed result label: the `+N −M` summary (§19/§20-flavored), or
/// "failed" so a broken edit reads at a glance.
fn cluster_label(card: &DiffCard, status: ToolStatus) -> String {
    match status {
        ToolStatus::Failed => "failed".to_owned(),
        _ => format!("+{} \u{2212}{}", card.added, card.removed),
    }
}

/// Render one diff card (PRODUCT §20–§21, UI cleanup): a flat `pencil + "Edited
/// file.rs"` disclosure with the `+N −M` cluster and status icon revealed on
/// expand, over the stacked feature-05 diff views; a failed call appends the
/// error verbatim below the diff. `name` selects the verb (`Write` → "Wrote").
pub(super) fn render_diff_card(
    id: &str,
    name: &str,
    status: ToolStatus,
    output: Option<&ToolOutput>,
    card: &DiffCard,
    ui: &HashMap<String, ToolCardUi>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let card_ui = ui.get(id);
    let mouse_state = card_ui.map(|ui| ui.mouse_state.clone()).unwrap_or_default();
    let expanded = card_ui
        .and_then(|ui| ui.expanded_override)
        .unwrap_or_else(default_expanded);
    let failed = status == ToolStatus::Failed;

    let verb = if name == "Write" { "Wrote" } else { "Edited" };
    let arg = Path::new(&card.file_path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned);
    let title = verb_title_row(verb, arg.as_deref(), failed, appearance);
    let glyph =
        warpui::elements::Icon::new(WarpIcon::Pencil.into(), blended_colors::neutral_7(theme));

    let toggle_id = id.to_owned();
    let mut disclosure = Disclosure::new(title)
        .with_glyph(glyph)
        .expandable(true)
        .expanded(expanded)
        .with_cluster(RightCluster {
            label: Some(cluster_label(card, status)),
            icon: Some(status_icon(status, appearance)),
        })
        .with_mouse_state(mouse_state)
        .on_toggle(move |ctx| {
            ctx.dispatch_typed_action(ClaudeCodeViewAction::ToggleToolCard(toggle_id.clone()));
        });

    if expanded {
        let mut body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);
        for (idx, viewer) in card.viewers.iter().enumerate() {
            // The embedded editor uses `GrowToMaxHeight`, which produces a
            // *flexible* child and so needs a bounded height constraint — but the
            // transcript renders items in a `MainAxisSize::Min` column (natural
            // height), which hands each child an *unbounded* vertical constraint.
            // `InlineDiffView` never applies `DisplayMode::Embedded { max_height }`
            // as a layout bound (its `render` emits a bare `ChildView`), so without
            // an explicit cap the editor's inner flex panics ("infinite constraint
            // along the flex axis"). Bound it here — the same fix the composer
            // editor uses — so the diff grows up to `DIFF_MAX_HEIGHT` and then
            // scrolls internally (§19).
            let bounded = ConstrainedBox::new(ChildView::new(viewer).finish())
                .with_max_height(DIFF_MAX_HEIGHT)
                .finish();
            let mut diff_container = Container::new(bounded);
            if idx + 1 < card.viewers.len() {
                // MultiEdit: breathing room between consecutive edits (§20).
                diff_container = diff_container.with_margin_bottom(8.);
            }
            body.add_child(diff_container.finish());
        }
        if failed {
            // The edit failed — the diff above shows what was *attempted*; the
            // error below says why it didn't land (PRODUCT §19/§30 verbatim).
            let error_text = output
                .map(|o| o.text.trim().to_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "The edit failed with no error output.".to_owned());
            body.add_child(
                Container::new(
                    render_requested_action_body_text(
                        Cow::Owned(error_text),
                        appearance.monospace_font_family(),
                        app,
                    )
                    .finish(),
                )
                .with_padding_top(6.)
                .finish(),
            );
        }
        disclosure = disclosure.with_body(body.finish());
    }

    disclosure.render(app)
}
