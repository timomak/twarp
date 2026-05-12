//! Staged Changes / Changes section rendering (PRODUCT §§4, 8, 10).
//!
//! 5a renders a flat list with status glyph + path. Hover actions
//! (`[+]`, `[−]`, `[↺]`), row click → diff, and conflict-row `[Resolve…]`
//! arrive in 5b–5c.

use warpui::elements::{Container, CrossAxisAlignment, Element, Flex, MainAxisSize, ParentElement};
use warpui::ui_components::components::UiComponent;

use crate::appearance::Appearance;
use crate::open_changes::repo::FileEntry;

/// Render one collapsible section (Staged Changes or Changes). 5a omits
/// the collapse toggle and renders the section permanently expanded.
pub fn render(label: &str, files: &[FileEntry], appearance: &Appearance) -> Box<dyn Element> {
    let header_text = format!("{label}  ·  {}", files.len());
    let header = appearance.ui_builder().span(header_text).build().finish();

    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.0)
        .with_child(header);

    for file in files {
        col = col.with_child(render_row(file, appearance));
    }

    Container::new(col.finish())
        .with_padding_top(4.0)
        .with_padding_bottom(4.0)
        .finish()
}

fn render_row(file: &FileEntry, appearance: &Appearance) -> Box<dyn Element> {
    let (basename, dir) = super::split_path_for_display(&file.path);
    let glyph_label = super::status_glyph_label(file.status);

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(6.0)
        .with_child(appearance.ui_builder().span(glyph_label).build().finish())
        .with_child(appearance.ui_builder().span(basename).build().finish());

    let row = if dir.is_empty() {
        row
    } else {
        row.with_child(appearance.ui_builder().span(dir).build().finish())
    };

    Container::new(row.finish())
        .with_padding_top(2.0)
        .with_padding_bottom(2.0)
        .finish()
}
