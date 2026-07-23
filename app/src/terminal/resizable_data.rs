use std::collections::HashMap;

use twarpui::elements::resizable_state_handle;
use twarpui::elements::ResizableStateHandle;
use twarpui::Entity;
use twarpui::SingletonEntity;
use twarpui::WindowId;

use crate::app_state::WindowSnapshot;

pub const DEFAULT_UNIVERSAL_SEARCH_WIDTH: f32 = 700.;
pub const DEFAULT_WARP_AI_WIDTH: f32 = 410.;
pub const DEFAULT_VOLTRON_WIDTH: f32 = 700.;
pub const DEFAULT_TWARP_DRIVE_INDEX_WIDTH: f32 = 300.;
pub const DEFAULT_SETTINGS_PANEL_WIDTH: f32 = 194.;
pub const DEFAULT_LEFT_PANEL_WIDTH: f32 = 240.;
pub const DEFAULT_RIGHT_PANEL_WIDTH: f32 = 480.;
pub const DEFAULT_PROJECTS_SIDEBAR_WIDTH: f32 = 240.;
pub const DEFAULT_FILES_TOOL_WIDTH: f32 = 300.;
pub const DEFAULT_CODE_REVIEW_TOOL_WIDTH: f32 = 480.;
/// A naming system for the ResizableStateHandles
pub enum ModalType {
    UniversalSearchWidth,
    WarpAIWidth,
    VoltronWidth,
    TwarpDriveIndexWidth,
    SettingsPanelWidth,
    LeftPanelWidth,
    RightPanelWidth,
    ProjectsSidebarWidth,
    FilesToolWidth,
    CodeReviewToolWidth,
}

/// A grouping of state handles for the resizables that should be stored and loaded as a part
/// of session restoration.
pub struct ModalSizes {
    pub universal_search_width: ResizableStateHandle,
    pub warp_ai_width: ResizableStateHandle,
    pub voltron_width: ResizableStateHandle,
    pub twarp_drive_index_width: ResizableStateHandle,
    pub settings_panel_width: ResizableStateHandle,
    pub left_panel_width: ResizableStateHandle,
    pub right_panel_width: ResizableStateHandle,
    pub projects_sidebar_width: ResizableStateHandle,
    pub files_tool_width: ResizableStateHandle,
    pub code_review_tool_width: ResizableStateHandle,
}

impl ModalSizes {
    /// Constructs a ModalSizes struct using a loaded-in WindowSnapshot
    pub fn from_restored(
        window_snapshot: &WindowSnapshot,
        left_panel_size: f32,
        right_panel_size: f32,
    ) -> Self {
        let universal_search_width = window_snapshot
            .universal_search_width
            .unwrap_or(DEFAULT_UNIVERSAL_SEARCH_WIDTH);
        let warp_ai_width = window_snapshot
            .warp_ai_width
            .unwrap_or(DEFAULT_WARP_AI_WIDTH);
        let voltron_width = window_snapshot
            .voltron_width
            .unwrap_or(DEFAULT_VOLTRON_WIDTH);
        let twarp_drive_index_width = window_snapshot
            .twarp_drive_index_width
            .unwrap_or(DEFAULT_TWARP_DRIVE_INDEX_WIDTH);
        let settings_panel_width = DEFAULT_SETTINGS_PANEL_WIDTH;
        let left_panel_width = window_snapshot.left_panel_width.unwrap_or(left_panel_size);
        let right_panel_width = window_snapshot
            .right_panel_width
            .unwrap_or(right_panel_size);
        let projects_sidebar_width = window_snapshot
            .projects_sidebar_width
            .or(window_snapshot.left_panel_width)
            .unwrap_or(DEFAULT_PROJECTS_SIDEBAR_WIDTH);
        let files_tool_width = window_snapshot
            .files_tool_width
            .or(window_snapshot.left_panel_width)
            .unwrap_or(DEFAULT_FILES_TOOL_WIDTH);
        let code_review_tool_width = window_snapshot
            .code_review_tool_width
            .or(window_snapshot.right_panel_width)
            .unwrap_or(DEFAULT_CODE_REVIEW_TOOL_WIDTH);

        Self {
            universal_search_width: resizable_state_handle(universal_search_width),
            warp_ai_width: resizable_state_handle(warp_ai_width),
            voltron_width: resizable_state_handle(voltron_width),
            twarp_drive_index_width: resizable_state_handle(twarp_drive_index_width),
            settings_panel_width: resizable_state_handle(settings_panel_width),
            left_panel_width: resizable_state_handle(left_panel_width),
            right_panel_width: resizable_state_handle(right_panel_width),
            projects_sidebar_width: resizable_state_handle(projects_sidebar_width),
            files_tool_width: resizable_state_handle(files_tool_width),
            code_review_tool_width: resizable_state_handle(code_review_tool_width),
        }
    }

    pub fn default_with_panel_defaults(left_default: f32, right_default: f32) -> Self {
        ModalSizes {
            universal_search_width: resizable_state_handle(DEFAULT_UNIVERSAL_SEARCH_WIDTH),
            warp_ai_width: resizable_state_handle(DEFAULT_WARP_AI_WIDTH),
            voltron_width: resizable_state_handle(DEFAULT_VOLTRON_WIDTH),
            twarp_drive_index_width: resizable_state_handle(DEFAULT_TWARP_DRIVE_INDEX_WIDTH),
            settings_panel_width: resizable_state_handle(DEFAULT_SETTINGS_PANEL_WIDTH),
            left_panel_width: resizable_state_handle(left_default),
            right_panel_width: resizable_state_handle(right_default),
            projects_sidebar_width: resizable_state_handle(DEFAULT_PROJECTS_SIDEBAR_WIDTH),
            files_tool_width: resizable_state_handle(DEFAULT_FILES_TOOL_WIDTH),
            code_review_tool_width: resizable_state_handle(DEFAULT_CODE_REVIEW_TOOL_WIDTH),
        }
    }

    /// Given a type (e.g., "universal search width"), returns the corresponding state handle
    pub fn get_resizable_state_handle(&self, modal: ModalType) -> ResizableStateHandle {
        match modal {
            ModalType::UniversalSearchWidth => self.universal_search_width.clone(),
            ModalType::WarpAIWidth => self.warp_ai_width.clone(),
            ModalType::VoltronWidth => self.voltron_width.clone(),
            ModalType::TwarpDriveIndexWidth => self.twarp_drive_index_width.clone(),
            ModalType::SettingsPanelWidth => self.settings_panel_width.clone(),
            ModalType::LeftPanelWidth => self.left_panel_width.clone(),
            ModalType::RightPanelWidth => self.right_panel_width.clone(),
            ModalType::ProjectsSidebarWidth => self.projects_sidebar_width.clone(),
            ModalType::FilesToolWidth => self.files_tool_width.clone(),
            ModalType::CodeReviewToolWidth => self.code_review_tool_width.clone(),
        }
    }
}

impl Default for ModalSizes {
    /// Constructs a ModalSizes struct using all default values
    fn default() -> Self {
        Self {
            universal_search_width: resizable_state_handle(DEFAULT_UNIVERSAL_SEARCH_WIDTH),
            warp_ai_width: resizable_state_handle(DEFAULT_WARP_AI_WIDTH),
            voltron_width: resizable_state_handle(DEFAULT_VOLTRON_WIDTH),
            twarp_drive_index_width: resizable_state_handle(DEFAULT_TWARP_DRIVE_INDEX_WIDTH),
            settings_panel_width: resizable_state_handle(DEFAULT_SETTINGS_PANEL_WIDTH),
            left_panel_width: resizable_state_handle(DEFAULT_LEFT_PANEL_WIDTH),
            right_panel_width: resizable_state_handle(DEFAULT_RIGHT_PANEL_WIDTH),
            projects_sidebar_width: resizable_state_handle(DEFAULT_PROJECTS_SIDEBAR_WIDTH),
            files_tool_width: resizable_state_handle(DEFAULT_FILES_TOOL_WIDTH),
            code_review_tool_width: resizable_state_handle(DEFAULT_CODE_REVIEW_TOOL_WIDTH),
        }
    }
}

/// The purpose of this model is to persist the dimensions of resized panels.
///
/// This works as a singleton model that tracks all the heights and widths of modals that should remain
/// across app startup, via the session restoration process. This has to track modal sizes on a
/// per-window basis, and the underlying HashMap is keyed by the windowId.
///
/// Restored windows should read in their modal sizes from a window snapshot. Fresh windows should
/// create a ModalSizes of all default values.
#[derive(Default)]
pub struct ResizableData {
    sizes_per_window: HashMap<WindowId, ModalSizes>,
}

pub enum ResizableDataEvent {}

impl ResizableData {
    /// Accepts a new window and stores the corresponding ModalSizes
    pub fn insert(&mut self, window_id: WindowId, modal_sizes: ModalSizes) {
        self.sizes_per_window.insert(window_id, modal_sizes);
    }

    /// Takes a window_id and modal type and returns the corresponding state handle.
    /// This is None if the window_id could not be found.
    pub fn get_handle(
        &self,
        window_id: WindowId,
        modal: ModalType,
    ) -> Option<ResizableStateHandle> {
        self.sizes_per_window
            .get(&window_id)
            .map(|modal_sizes| modal_sizes.get_resizable_state_handle(modal))
    }

    /// Returns all ResizableStateHandles we're storing for a given window.
    pub fn get_all_handles(&self, window_id: WindowId) -> Option<&ModalSizes> {
        self.sizes_per_window.get(&window_id)
    }
}

impl Entity for ResizableData {
    type Event = ResizableDataEvent;
}

impl SingletonEntity for ResizableData {}
