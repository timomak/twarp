// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

use anyhow::Result;
use twarp_core::{
    channel::{Channel, ChannelConfig, ChannelState, OzConfig, WarpServerConfig},
    features::FeatureFlag,
    AppId,
};

// twarp: feature 05 (Open Changes rework) builds on top of the right-side
// Code Review panel layout that upstream gates behind a Preview flag. The
// rework IS the canonical layout for twarp, so enable the flag in OSS by
// default — otherwise `cargo run` (which defaults to twarp-oss) hides the
// reworked sidebar entirely.
//
// twarp: feature 08 (macOS UI overhaul, sub-phase 8b/8c) gates drag-a-tab-out
// / drag-between-windows behind DragTabsToWindows, which upstream ships only in
// DOGFOOD_FLAGS. twarp-oss (the default `./script/run` binary) never enables the
// dogfood set, so without this the tab drag axis stays locked to horizontal and
// detach-to-new-window never fires. Force-enable it for the OSS build.
//
// twarp: feature 11a (Git blame gutter) is smoke-tested through the default
// twarp-oss binary. Keep the feature flag boundary in code, but enable it for
// this fork's dev binary so the gutter path is live in fleet UX gates.
//
// twarp: feature 19b (Codex shell) is likewise gated in code so the layout can
// be A/B'd, but the default OSS worker build should expose it for UX gates.
const TWARP_OSS_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::GitOperationsInCodeReview,
    FeatureFlag::DragTabsToWindows,
    FeatureFlag::GitBlame,
    FeatureFlag::DesignShellV1,
];

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
    <string>TwarpOss</string>
    <key>CFBundleExecutable</key>
    <string>twarp-oss</string>
    <key>CFBundleIdentifier</key>
    <string>dev.twarp.TwarpOss</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>TwarpOss</string>
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
