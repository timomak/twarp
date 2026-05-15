use warp_core::ui::theme::Fill;

use crate::render::model::{BlockItem, Decoration, RenderState, viewport::ViewportItem};

use super::{RenderableBlock, paint::RenderContext};

pub struct RenderableTemporaryBlock {
    viewport_item: ViewportItem,
    decoration: Option<Fill>,
    text_decoration: Vec<Decoration>,
}

impl RenderableTemporaryBlock {
    pub fn new(
        viewport_item: ViewportItem,
        decoration: Option<Fill>,
        text_decoration: Vec<Decoration>,
    ) -> Self {
        Self {
            viewport_item,
            decoration,
            text_decoration,
        }
    }
}

impl RenderableBlock for RenderableTemporaryBlock {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn overlay_decoration(&self) -> Option<Fill> {
        self.decoration
    }

    fn layout(
        &mut self,
        _model: &RenderState,
        _ctx: &mut warpui::LayoutContext,
        _app: &warpui::AppContext,
    ) {
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, _app: &warpui::AppContext) {
        // We cannot use `extract_block` macro here since we need to locate the viewport item by content height instead of charoffset
        // (temporary block has an offset of zero).
        let content = model.content();
        let paragraph_block = match content.block_at_height(self.viewport_item.height()) {
            Some(block) => match (&block, block.item) {
                (
                    block,
                    BlockItem::TemporaryBlock {
                        paragraph_block, ..
                    },
                ) => block.temporary_block(paragraph_block),
                other => {
                    log::warn!(
                        "Unexpected block {other:?} at {}",
                        self.viewport_item.block_offset
                    );
                    return;
                }
            },
            None => return,
        };

        let start = paragraph_block.start_char_offset;
        let paragraph_styles = &model.styles().base_text;
        // twarp 05: if this temp block is the user's current selection
        // (set by clicking on it), overlay the editor's selection fill
        // across the full content area of every paragraph. Matched by
        // height anchor — see `RenderState::temp_block_at_viewport_position`.
        let is_temp_block_selected = model
            .temp_block_selection()
            .is_some_and(|sel| sel.anchor.height == self.viewport_item.height());
        let selection_fill = model.styles().selection_fill;
        let mut decoration_index = 0;
        for paragraph in paragraph_block.paragraphs() {
            // We could draw text directly since temporary paragraph should have its own decoration and selection state.
            ctx.draw_text(
                paragraph.content_origin(),
                Default::default(),
                paragraph.item.frame(),
                paragraph_styles,
            );

            if is_temp_block_selected {
                let para_start = paragraph.start_char_offset;
                let para_end = paragraph.end_char_offset();
                paragraph.draw_highlight(
                    para_start,
                    para_end,
                    selection_fill,
                    ctx,
                    model.max_line(),
                );
            }

            let paragraph_end = paragraph.end_char_offset();
            for (idx, decoration) in self.text_decoration[decoration_index..].iter().enumerate() {
                if decoration.start + start >= paragraph_end {
                    decoration_index += idx;
                    break;
                }
                if let Some(highlight) = decoration.background {
                    paragraph.draw_highlight(
                        decoration.start + start,
                        decoration.end + start,
                        highlight.into(),
                        ctx,
                        model.max_line(),
                    );
                }
            }
        }
    }

    fn is_temporary(&self) -> bool {
        true
    }
}
