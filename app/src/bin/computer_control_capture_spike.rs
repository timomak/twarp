#[cfg(target_os = "macos")]
mod macos {
    #![allow(deprecated)]

    use anyhow::{bail, Context, Result};
    use cocoa::{
        appkit::{
            CGFloat, NSApp, NSApplication, NSApplicationActivationPolicyAccessory,
            NSBackingStoreBuffered, NSColor, NSPanel, NSScreen, NSView, NSWindow,
            NSWindowCollectionBehavior, NSWindowStyleMask,
        },
        base::{id, nil},
        foundation::{NSAutoreleasePool, NSInteger, NSPoint, NSRect, NSSize},
    };
    use computer_use::{Options, ScreenshotParams};
    use futures::executor::block_on;
    use objc::{
        class, msg_send,
        runtime::{NO, YES},
        sel, sel_impl,
    };
    use std::{
        env, fs,
        path::{Path, PathBuf},
        thread,
        time::Duration,
    };

    const NONACTIVATING_PANEL_MASK: u64 = 1 << 7;
    const NS_WINDOW_SHARING_NONE: NSInteger = 0;
    const NS_FLOATING_WINDOW_LEVEL: NSInteger = 3;
    const NS_STATUS_WINDOW_LEVEL: NSInteger = 25;

    pub fn run() -> Result<()> {
        let output_dir = output_dir()?;
        fs::create_dir_all(&output_dir).with_context(|| {
            format!(
                "failed to create spike output directory {}",
                output_dir.display()
            )
        })?;

        let _pool = unsafe { NSAutoreleasePool::new(nil) };
        let app = unsafe {
            let app = NSApp();
            if app == nil {
                NSApplication::sharedApplication(nil)
            } else {
                app
            }
        };
        unsafe {
            let _: () = msg_send![app, finishLaunching];
            app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);
        }

        let screen = unsafe { NSScreen::mainScreen(nil) };
        if screen == nil {
            bail!("no main NSScreen available for spike capture");
        }

        let screen_frame = unsafe { NSScreen::frame(screen) };
        let visible_frame = unsafe { screen.visibleFrame() };

        let overlay = unsafe { create_overlay_panel(visible_frame)? };
        let glow = unsafe { create_glow_window(screen_frame)? };
        unsafe {
            order_front_without_activating(overlay);
            order_front_without_activating(glow);
            display_window(overlay);
            display_window(glow);
        }

        // Give WindowServer a short moment to commit both windows before capture.
        thread::sleep(Duration::from_millis(3_000));

        let mut actor = computer_use::create_actor();
        let result = block_on(actor.perform_actions(
            &[],
            Options {
                screenshot_params: Some(ScreenshotParams {
                    max_long_edge_px: None,
                    max_total_px: None,
                    region: None,
                }),
            },
        ))
        .map_err(|error| anyhow::anyhow!("computer_use capture failed: {error}"))?;

        let screenshot = result
            .screenshot
            .context("computer_use actor returned no screenshot")?;
        let capture_path = output_dir.join("capture.png");
        fs::write(&capture_path, &screenshot.data)
            .with_context(|| format!("failed to write {}", capture_path.display()))?;

        let report_path = output_dir.join("report.txt");
        fs::write(
            &report_path,
            report_text(&capture_path, &screenshot, overlay, glow),
        )
        .with_context(|| format!("failed to write {}", report_path.display()))?;

        unsafe {
            close_window(overlay);
            close_window(glow);
        }

        println!("capture={}", capture_path.display());
        println!("report={}", report_path.display());
        println!("self_exclusion_gate=manual_inspection_required");
        Ok(())
    }

    fn output_dir() -> Result<PathBuf> {
        if let Some(path) = env::args_os().nth(1) {
            Ok(PathBuf::from(path))
        } else {
            Ok(env::current_dir()?.join("target/computer-control-capture-spike"))
        }
    }

    unsafe fn create_overlay_panel(visible_frame: NSRect) -> Result<id> {
        let size = NSSize::new(300.0, 112.0);
        let margin = 18.0;
        let rect = NSRect::new(
            NSPoint::new(
                visible_frame.origin.x + visible_frame.size.width - size.width - margin,
                visible_frame.origin.y + visible_frame.size.height - size.height - margin,
            ),
            size,
        );

        let style = NSWindowStyleMask::NSTitledWindowMask
            | NSWindowStyleMask::NSFullSizeContentViewWindowMask
            | NSWindowStyleMask::from_bits_truncate(NONACTIVATING_PANEL_MASK);
        let panel = NSPanel::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            rect,
            style,
            NSBackingStoreBuffered,
            NO,
        );
        if panel == nil {
            bail!("failed to allocate overlay NSPanel");
        }

        panel.setOpaque_(NO);
        panel.setBackgroundColor_(NSColor::colorWithCalibratedRed_green_blue_alpha_(
            nil, 0.07, 0.08, 0.10, 0.92,
        ));
        panel.setHasShadow_(YES);
        panel.setHidesOnDeactivate_(NO);
        panel.setCanHide_(NO);
        panel.setCollectionBehavior_(
            NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle,
        );
        let _: () = msg_send![panel, setLevel: NS_STATUS_WINDOW_LEVEL];
        let _: () = msg_send![panel, setSharingType: NS_WINDOW_SHARING_NONE];
        let _: () = msg_send![panel, setReleasedWhenClosed: YES];
        let _: () = msg_send![panel, setTitleVisibility: 1u64];
        let _: () = msg_send![panel, setTitlebarAppearsTransparent: YES];
        let _: () = msg_send![panel, setMovable: NO];
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: YES];

        let content = NSView::alloc(nil).initWithFrame_(NSRect::new(NSPoint::new(0.0, 0.0), size));
        content.setWantsLayer(YES);
        let content_layer: id = content.layer();
        let _: () = msg_send![content_layer, setCornerRadius: 10.0 as CGFloat];
        let _: () = msg_send![content_layer, setMasksToBounds: YES];
        panel.setContentView_(content);

        add_label(
            content,
            "Claude control spike",
            16.0,
            70.0,
            268.0,
            24.0,
            14.0,
            true,
        )?;
        add_label(
            content,
            "Overlay/glow are set to sharingType = none",
            16.0,
            45.0,
            268.0,
            18.0,
            11.0,
            false,
        )?;
        add_label(
            content,
            "Inspect target/computer-control-capture-spike/capture.png",
            16.0,
            23.0,
            268.0,
            18.0,
            11.0,
            false,
        )?;

        Ok(panel)
    }

    unsafe fn create_glow_window(screen_frame: NSRect) -> Result<id> {
        let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            screen_frame,
            NSWindowStyleMask::NSBorderlessWindowMask,
            NSBackingStoreBuffered,
            NO,
        );
        if window == nil {
            bail!("failed to allocate glow NSWindow");
        }

        window.setOpaque_(NO);
        window.setBackgroundColor_(NSColor::clearColor(nil));
        window.setHasShadow_(NO);
        window.setHidesOnDeactivate_(NO);
        window.setCanHide_(NO);
        window.setCollectionBehavior_(
            NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle,
        );
        let _: () = msg_send![window, setLevel: NS_FLOATING_WINDOW_LEVEL];
        let _: () = msg_send![window, setIgnoresMouseEvents: YES];
        let _: () = msg_send![window, setSharingType: NS_WINDOW_SHARING_NONE];
        let _: () = msg_send![window, setReleasedWhenClosed: YES];

        let content = NSView::alloc(nil)
            .initWithFrame_(NSRect::new(NSPoint::new(0.0, 0.0), screen_frame.size));
        content.setWantsLayer(YES);
        let layer: id = content.layer();
        let color: id = msg_send![
            class!(NSColor),
            colorWithCalibratedRed: 0.15 as CGFloat
            green: 0.63 as CGFloat
            blue: 1.0 as CGFloat
            alpha: 0.95 as CGFloat
        ];
        let cg_color: id = msg_send![color, CGColor];
        let _: () = msg_send![layer, setBorderColor: cg_color];
        let _: () = msg_send![layer, setBorderWidth: 7.0 as CGFloat];
        let _: () = msg_send![layer, setCornerRadius: 0.0 as CGFloat];
        let clear_color: id = msg_send![NSColor::clearColor(nil), CGColor];
        let _: () = msg_send![layer, setBackgroundColor: clear_color];
        window.setContentView_(content);

        Ok(window)
    }

    unsafe fn add_label(
        content: id,
        text: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        font_size: f64,
        bold: bool,
    ) -> Result<()> {
        let field: id = msg_send![class!(NSTextField), alloc];
        let field: id = msg_send![
            field,
            initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
        ];
        if field == nil {
            bail!("failed to allocate overlay label");
        }

        let ns_text = ns_string(text);
        let _: () = msg_send![field, setStringValue: ns_text];
        let _: () = msg_send![field, setBezeled: NO];
        let _: () = msg_send![field, setDrawsBackground: NO];
        let _: () = msg_send![field, setEditable: NO];
        let _: () = msg_send![field, setSelectable: NO];
        let _: () = msg_send![
            field,
            setTextColor: NSColor::colorWithCalibratedRed_green_blue_alpha_(nil, 0.93, 0.95, 0.98, 1.0)
        ];
        let font: id = if bold {
            msg_send![class!(NSFont), boldSystemFontOfSize: font_size as CGFloat]
        } else {
            msg_send![class!(NSFont), systemFontOfSize: font_size as CGFloat]
        };
        let _: () = msg_send![field, setFont: font];
        let _: () = msg_send![content, addSubview: field];
        Ok(())
    }

    unsafe fn order_front_without_activating(window: id) {
        let _: () = msg_send![window, orderFrontRegardless];
    }

    unsafe fn display_window(window: id) {
        let _: () = msg_send![window, displayIfNeeded];
    }

    unsafe fn close_window(window: id) {
        let _: () = msg_send![window, orderOut: nil];
        let _: () = msg_send![window, close];
    }

    unsafe fn ns_string(value: &str) -> id {
        use cocoa::foundation::NSString;
        NSString::alloc(nil).init_str(value).autorelease()
    }

    fn report_text(
        capture_path: &Path,
        screenshot: &computer_use::Screenshot,
        overlay: id,
        glow: id,
    ) -> String {
        format!(
            "\
Computer control self-excluding capture spike

Capture path: {capture_path}
Capture backend: computer_use mac actor using /usr/sbin/screencapture
Capture size: {}x{} px (original {}x{} px)
Overlay windowNumber: {}
Glow windowNumber: {}
Exclusion requested: NSWindow.sharingType = NSWindowSharingNone on overlay and glow windows

Manual gate:
1. Open the capture image while the spike output is fresh.
2. Verify the corner overlay panel is absent.
3. Verify the blue glow border is absent.
4. Place a normal Warp/twarp window under the main-display capture area and repeat if needed.

Honest scope:
This spike proves only whether the spike overlay NSPanel and glow NSWindow are excluded by the current screencapture backend on this macOS version.
It does not claim that all Warp/twarp app windows are excluded.
If the overlay or glow appears in capture.png, later computer-control phases must stay blocked until a ScreenCaptureKit exclusion-filter backend replaces this capture path.
",
            screenshot.width,
            screenshot.height,
            screenshot.original_width,
            screenshot.original_height,
            unsafe { window_number(overlay) },
            unsafe { window_number(glow) },
            capture_path = capture_path.display(),
        )
    }

    unsafe fn window_number(window: id) -> NSInteger {
        msg_send![window, windowNumber]
    }
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    macos::run()
}

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("computer_control_capture_spike is only supported on macOS")
}
