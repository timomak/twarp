use super::is_twarp_bundle;

#[test]
fn is_twarp_bundle_recognises_twarp_channels() {
    assert!(is_twarp_bundle("dev.twarp.Twarp"));
    assert!(is_twarp_bundle("dev.twarp.TwarpDev"));
    assert!(is_twarp_bundle("dev.twarp.TwarpPreview"));
    assert!(is_twarp_bundle("dev.twarp.TwarpOss"));
}

#[test]
fn is_twarp_bundle_rejects_other_apps() {
    assert!(!is_twarp_bundle("com.microsoft.VSCode"));
    assert!(!is_twarp_bundle("com.apple.TextEdit"));
    assert!(!is_twarp_bundle("dev.zed.Zed"));
    assert!(!is_twarp_bundle("invalid"));
    assert!(!is_twarp_bundle(""));
}
