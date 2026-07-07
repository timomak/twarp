// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

#[path = "channel_config.rs"]
mod channel_config;

use anyhow::Result;
use twarp_core::{
    channel::{Channel, ChannelState},
    features,
};

// Simple wrapper around twarp::run() for feature preview channel builds.
fn main() -> Result<()> {
    ChannelState::set(
        // twarp: de-cloud (2b) — ForceLogin deleted; there is no login screen.
        ChannelState::new(Channel::Preview, channel_config::load_config!("preview"))
            .with_additional_features(features::PREVIEW_FLAGS),
    );

    twarp::run()
}
