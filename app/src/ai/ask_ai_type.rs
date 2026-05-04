use std::{collections::HashSet, sync::Arc};

use warp_core::command::ExitCode;

use crate::{server::telemetry::OpenedWarpAISource, terminal::model::terminal_model::BlockIndex};

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum AskAIType {
    /// Covers all possible origins of text selection, including the block list terminal,
    /// the alt-screen terminal, and the input area. Not all instances will require
    /// `populate_input_box`, which determines whether we should automatically render
    /// something like "Explain the following" within the user's input box.
    FromTextSelection {
        text: Arc<String>,
        populate_input_box: bool,
    },
    /// Data about a block to inform Agent Mode.
    FromBlock {
        input: Arc<String>,
        output: Arc<String>,
        exit_code: ExitCode,
        block_index: BlockIndex,
    },
    /// Which blocks to attach to a block list AI query.
    FromBlocks {
        block_indices: HashSet<BlockIndex>,
    },
    FromAICommandSearch {
        query: Arc<String>,
    },
}

impl From<&AskAIType> for OpenedWarpAISource {
    fn from(value: &AskAIType) -> Self {
        match value {
            AskAIType::FromAICommandSearch { .. } => OpenedWarpAISource::FromAICommandSearch,
            AskAIType::FromBlock { .. } | AskAIType::FromBlocks { .. } => {
                OpenedWarpAISource::HelpWithBlock
            }
            AskAIType::FromTextSelection { .. } => OpenedWarpAISource::HelpWithTextSelection,
        }
    }
}
