use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};
use twarp_core::features::FeatureFlag;

use super::DriveSortOrder;

pub const HAS_AUTO_OPENED_WELCOME_FOLDER: &str = "HasAutoOpenedWelcomeFolder";

define_settings_group!(TwarpDriveSettings, settings: [
    sorting_choice: TwarpDriveSortingChoice {
        type: DriveSortOrder,
        default: DriveSortOrder::ByObjectType,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "twarp_drive.sorting_choice",
        description: "The sort order for items in Twarp Drive.",
    },
    sharing_onboarding_block_shown: TwarpDriveSharingOnboardingBlockShown {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    // Controls whether Warp Drive appears in the tools panel, command palette, and command search.
    enable_twarp_drive: EnableTwarpDrive {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "twarp_drive.enabled",
        description: "Whether Twarp Drive is enabled.",
    },
]);

impl TwarpDriveSettings {
    /// Returns whether Warp Drive should be considered enabled.
    /// Returns `false` when the user is anonymous or fully logged out,
    /// regardless of the user setting.
    pub fn is_twarp_drive_enabled(app: &twarpui::AppContext) -> bool {
        use twarpui::SingletonEntity as _;
        // twarp: de-cloud (2b) — SkipFirebaseAnonymousUser flag deleted;
        // logged-out is unconditional.
        let is_anonymous_or_logged_out = crate::auth::AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out();
        *Self::as_ref(app).enable_twarp_drive && !is_anonymous_or_logged_out
    }
}
