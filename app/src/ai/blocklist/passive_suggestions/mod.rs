mod maa;

use warpui::ModelHandle;

pub use maa::{
    PassiveSuggestionsEvent as MaaPassiveSuggestionsEvent,
    PassiveSuggestionsModel as MaaPassiveSuggestionsModel,
};

#[derive(Clone)]
pub struct PassiveSuggestionsModels {
    pub maa: ModelHandle<MaaPassiveSuggestionsModel>,
}
