// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

use anyhow::Result;
use twarp_core::{
    channel::{Channel, ChannelConfig, ChannelState, OzConfig, WarpServerConfig},
    features::FeatureFlag,
    AppId,
};

// twarp: shipped twarp features are product, not experiments. The flags that
// used to be force-enabled here (GitOperationsInCodeReview, DragTabsToWindows,
// GitBlame, DesignShellV1, ProjectSidebar, CodexAgentBackend, LocalComputerUse,
// MarkdownImages, WelcomeTab, EditableMarkdownMermaid) have had their call-site
// gating removed entirely — the enabled behavior is now unconditional product
// code. This list stays as an empty slice so future twarp-only flags have an
// obvious place to be enabled for the OSS build.
const TWARP_OSS_FLAGS: &[FeatureFlag] = &[];

// Simple wrapper around twarp::run() for Twarp OSS builds.
fn main() -> Result<()> {
    let mut state = ChannelState::new(
        Channel::Oss,
        ChannelConfig {
            app_id: AppId::new("dev", "twarp", "TwarpOss"),
            logfile_name: "twarp-oss.log".into(),
            server_config: WarpServerConfig::disabled(),
            oz_config: OzConfig::disabled(),
            mcp_static_config: None,
        },
    );
    if cfg!(debug_assertions) {
        state = state.with_additional_features(twarp_core::features::DEBUG_FLAGS);
    }
    state = state.with_additional_features(TWARP_OSS_FLAGS);
    ChannelState::set(state);

    twarp::run()
}

// If we're not using an external plist, embed the following as the Info.plist.
#[cfg(all(not(feature = "extern_plist"), target_os = "macos"))]
embed_plist::embed_info_plist_bytes!(r#"
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleDisplayName</key>
    <string>Twarp</string>
    <key>CFBundleExecutable</key>
    <string>twarp-oss</string>
    <key>CFBundleIdentifier</key>
    <string>dev.twarp.TwarpOss</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Twarp</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>UIDesignRequiresCompatibility</key>
    <true/>
    <key>CFBundleURLTypes</key>
    <array><dict><key>CFBundleURLName</key><string>Custom App</string><key>CFBundleURLSchemes</key><array><string>twarp</string></array></dict></array>
    <key>NSHumanReadableCopyright</key>
    <string>© 2026, Denver Technologies, Inc</string>
    </dict>
    </plist>
"#.as_bytes());
