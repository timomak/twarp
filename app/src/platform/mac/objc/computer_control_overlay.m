#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <QuartzCore/QuartzCore.h>

#import "computer_control_overlay.h"

static const NSUInteger TwarpNonactivatingPanelMask = 1 << 7;
static const NSWindowSharingType TwarpWindowSharingNone = 0;
static const NSInteger TwarpFloatingWindowLevel = 3;
static const NSInteger TwarpStatusWindowLevel = 25;

static NSColor *TwarpColor(TwarpComputerControlColor color) {
    return [NSColor colorWithCalibratedRed:(CGFloat)color.r / 255.0
                                     green:(CGFloat)color.g / 255.0
                                      blue:(CGFloat)color.b / 255.0
                                     alpha:(CGFloat)color.a / 255.0];
}

static NSTextField *TwarpLabel(NSRect frame, NSString *value, CGFloat fontSize, BOOL bold, NSColor *color) {
    NSTextField *field = [[[NSTextField alloc] initWithFrame:frame] autorelease];
    [field setStringValue:value ?: @""];
    [field setBezeled:NO];
    [field setDrawsBackground:NO];
    [field setEditable:NO];
    [field setSelectable:NO];
    [field setLineBreakMode:NSLineBreakByTruncatingMiddle];
    [field setTextColor:color];
    [field setFont:bold ? [NSFont boldSystemFontOfSize:fontSize] : [NSFont systemFontOfSize:fontSize]];
    return field;
}

static NSString *TwarpStringFromCString(const char *value);

static NSString *TwarpPermissionSettingsURL(TwarpComputerControlPermissionState state, BOOL screenRecording) {
    if (state == TwarpComputerControlPermissionGranted ||
        state == TwarpComputerControlPermissionRestartRequired) {
        return nil;
    }

    return screenRecording
        ? @"x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        : @"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
}

static NSString *TwarpPermissionStatusText(
    NSString *name,
    NSString *purpose,
    TwarpComputerControlPermissionState state) {
    switch (state) {
        case TwarpComputerControlPermissionGranted:
            return [NSString stringWithFormat:@"%@: granted", name];
        case TwarpComputerControlPermissionMissing:
            return [NSString stringWithFormat:@"%@: required for %@", name, purpose];
        case TwarpComputerControlPermissionRestartRequired:
            return [NSString stringWithFormat:@"%@: granted; restart twarp to use it", name];
        case TwarpComputerControlPermissionDeniedOrUnknown:
        default:
            return [NSString stringWithFormat:@"%@: blocked or unknown", name];
    }
}

static NSString *TwarpPermissionSummary(
    TwarpComputerControlPermissionState screenRecordingState,
    TwarpComputerControlPermissionState accessibilityState) {
    BOOL screenBlocked = screenRecordingState != TwarpComputerControlPermissionGranted;
    BOOL accessibilityBlocked = accessibilityState != TwarpComputerControlPermissionGranted;
    BOOL restartNeeded =
        screenRecordingState == TwarpComputerControlPermissionRestartRequired ||
        accessibilityState == TwarpComputerControlPermissionRestartRequired;

    if (restartNeeded) {
        return @"Restart twarp before starting computer control. Permissions granted in System Settings are not usable by this running process.";
    }
    if (screenBlocked && accessibilityBlocked) {
        return @"Grant Screen Recording for screenshots and Accessibility for mouse and keyboard control.";
    }
    if (screenBlocked) {
        return @"Grant Screen Recording so Claude can receive screenshots.";
    }
    if (accessibilityBlocked) {
        return @"Grant Accessibility so Claude can send mouse and keyboard input.";
    }
    return @"Permissions are ready. Retry computer control.";
}

static void TwarpOpenSettingsURL(NSString *value) {
    if (!value) {
        return;
    }

    NSURL *url = [NSURL URLWithString:value];
    if (url) {
        [[NSWorkspace sharedWorkspace] openURL:url];
    }
}

@interface TwarpComputerControlPermissionsPanelHost : NSObject {
    NSPanel *_panel;
    NSView *_content;
    NSTextField *_sessionLabel;
    NSTextField *_summaryLabel;
    NSTextField *_screenRecordingLabel;
    NSTextField *_accessibilityLabel;
    NSButton *_screenRecordingButton;
    NSButton *_accessibilityButton;
    NSButton *_retryButton;
    NSButton *_dismissButton;
    TwarpComputerControlPermissionState _screenRecordingState;
    TwarpComputerControlPermissionState _accessibilityState;
    TwarpComputerControlPermissionCallback _retryCallback;
    void *_retryContext;
    TwarpComputerControlPermissionCallback _dismissCallback;
    void *_dismissContext;
    BOOL _closed;
}

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
                screenRecordingState:(TwarpComputerControlPermissionState)screenRecordingState
                   accessibilityState:(TwarpComputerControlPermissionState)accessibilityState
                           panelColor:(TwarpComputerControlColor)panelColor
                            textColor:(TwarpComputerControlColor)textColor
                       mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                          accentColor:(TwarpComputerControlColor)accentColor
                        retryCallback:(TwarpComputerControlPermissionCallback)retryCallback
                         retryContext:(void *)retryContext
                      dismissCallback:(TwarpComputerControlPermissionCallback)dismissCallback
                       dismissContext:(void *)dismissContext;
- (void)updateWithSessionLabel:(NSString *)sessionLabel
          screenRecordingState:(TwarpComputerControlPermissionState)screenRecordingState
             accessibilityState:(TwarpComputerControlPermissionState)accessibilityState
                     panelColor:(TwarpComputerControlColor)panelColor
                      textColor:(TwarpComputerControlColor)textColor
                 mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                    accentColor:(TwarpComputerControlColor)accentColor;
- (void)closePanel;

@end

@implementation TwarpComputerControlPermissionsPanelHost

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
                screenRecordingState:(TwarpComputerControlPermissionState)screenRecordingState
                   accessibilityState:(TwarpComputerControlPermissionState)accessibilityState
                           panelColor:(TwarpComputerControlColor)panelColor
                            textColor:(TwarpComputerControlColor)textColor
                       mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                          accentColor:(TwarpComputerControlColor)accentColor
                        retryCallback:(TwarpComputerControlPermissionCallback)retryCallback
                         retryContext:(void *)retryContext
                      dismissCallback:(TwarpComputerControlPermissionCallback)dismissCallback
                       dismissContext:(void *)dismissContext {
    self = [super init];
    if (!self) {
        return nil;
    }

    _retryCallback = retryCallback;
    _retryContext = retryContext;
    _dismissCallback = dismissCallback;
    _dismissContext = dismissContext;

    NSScreen *screen = [NSScreen mainScreen];
    if (!screen) {
        [self release];
        return nil;
    }

    NSRect visibleFrame = [screen visibleFrame];
    NSSize panelSize = NSMakeSize(398.0, 252.0);
    CGFloat margin = 18.0;
    NSRect panelRect = NSMakeRect(
        NSMaxX(visibleFrame) - panelSize.width - margin,
        NSMaxY(visibleFrame) - panelSize.height - margin,
        panelSize.width,
        panelSize.height);

    NSWindowStyleMask panelStyle =
        NSWindowStyleMaskTitled |
        NSWindowStyleMaskFullSizeContentView |
        TwarpNonactivatingPanelMask;
    _panel = [[NSPanel alloc] initWithContentRect:panelRect
                                       styleMask:panelStyle
                                         backing:NSBackingStoreBuffered
                                           defer:NO];
    if (!_panel) {
        [self release];
        return nil;
    }

    [_panel setOpaque:NO];
    [_panel setBackgroundColor:TwarpColor(panelColor)];
    [_panel setHasShadow:YES];
    [_panel setHidesOnDeactivate:NO];
    [_panel setCanHide:NO];
    [_panel setReleasedWhenClosed:NO];
    [_panel setTitleVisibility:NSWindowTitleHidden];
    [_panel setTitlebarAppearsTransparent:YES];
    [_panel setMovable:NO];
    [_panel setBecomesKeyOnlyIfNeeded:YES];
    [_panel setCollectionBehavior:
        NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary |
        NSWindowCollectionBehaviorIgnoresCycle];
    [_panel setLevel:TwarpStatusWindowLevel];
    [_panel setSharingType:TwarpWindowSharingNone];

    _content = [[[NSView alloc] initWithFrame:NSMakeRect(0.0, 0.0, panelSize.width, panelSize.height)] autorelease];
    [_content setWantsLayer:YES];
    [_content layer].cornerRadius = 10.0;
    [_content layer].masksToBounds = YES;
    [_panel setContentView:_content];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);

    NSTextField *titleLabel = TwarpLabel(
        NSMakeRect(16.0, 218.0, 270.0, 20.0),
        @"Computer control blocked",
        13.0,
        YES,
        text);
    [_content addSubview:titleLabel];

    _sessionLabel = TwarpLabel(
        NSMakeRect(16.0, 198.0, 366.0, 18.0),
        sessionLabel,
        11.0,
        NO,
        muted);
    [_content addSubview:_sessionLabel];

    _summaryLabel = TwarpLabel(
        NSMakeRect(16.0, 159.0, 366.0, 34.0),
        @"",
        11.0,
        NO,
        text);
    [_summaryLabel setLineBreakMode:NSLineBreakByWordWrapping];
    [_summaryLabel setUsesSingleLineMode:NO];
    [_content addSubview:_summaryLabel];

    _screenRecordingLabel = TwarpLabel(NSMakeRect(16.0, 126.0, 226.0, 18.0), @"", 11.0, NO, muted);
    [_content addSubview:_screenRecordingLabel];

    _screenRecordingButton = [NSButton buttonWithTitle:@"Open Screen Recording" target:self action:@selector(openScreenRecording:)];
    [_screenRecordingButton setFrame:NSMakeRect(248.0, 119.0, 134.0, 28.0)];
    [_screenRecordingButton setBezelStyle:NSBezelStyleRounded];
    [_screenRecordingButton setFont:[NSFont systemFontOfSize:11.0 weight:NSFontWeightMedium]];
    [_content addSubview:_screenRecordingButton];

    _accessibilityLabel = TwarpLabel(NSMakeRect(16.0, 88.0, 226.0, 18.0), @"", 11.0, NO, muted);
    [_content addSubview:_accessibilityLabel];

    _accessibilityButton = [NSButton buttonWithTitle:@"Open Accessibility" target:self action:@selector(openAccessibility:)];
    [_accessibilityButton setFrame:NSMakeRect(248.0, 81.0, 134.0, 28.0)];
    [_accessibilityButton setBezelStyle:NSBezelStyleRounded];
    [_accessibilityButton setFont:[NSFont systemFontOfSize:11.0 weight:NSFontWeightMedium]];
    [_content addSubview:_accessibilityButton];

    NSTextField *footerLabel = TwarpLabel(
        NSMakeRect(16.0, 49.0, 366.0, 18.0),
        @"After changing settings, return here and retry.",
        11.0,
        NO,
        muted);
    [_content addSubview:footerLabel];

    _retryButton = [NSButton buttonWithTitle:@"Retry" target:self action:@selector(retryPressed:)];
    [_retryButton setFrame:NSMakeRect(237.0, 14.0, 68.0, 28.0)];
    [_retryButton setBezelStyle:NSBezelStyleRounded];
    [_retryButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightSemibold]];
    [_content addSubview:_retryButton];

    _dismissButton = [NSButton buttonWithTitle:@"Dismiss" target:self action:@selector(dismissPressed:)];
    [_dismissButton setFrame:NSMakeRect(312.0, 14.0, 70.0, 28.0)];
    [_dismissButton setBezelStyle:NSBezelStyleRounded];
    [_dismissButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightRegular]];
    [_content addSubview:_dismissButton];

    [self updateWithSessionLabel:sessionLabel
            screenRecordingState:screenRecordingState
               accessibilityState:accessibilityState
                       panelColor:panelColor
                        textColor:textColor
                   mutedTextColor:mutedTextColor
                      accentColor:accentColor];

    [_panel orderFrontRegardless];
    [_panel displayIfNeeded];

    return self;
}

- (void)updateWithSessionLabel:(NSString *)sessionLabel
          screenRecordingState:(TwarpComputerControlPermissionState)screenRecordingState
             accessibilityState:(TwarpComputerControlPermissionState)accessibilityState
                     panelColor:(TwarpComputerControlColor)panelColor
                      textColor:(TwarpComputerControlColor)textColor
                 mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                    accentColor:(TwarpComputerControlColor)accentColor {
    if (_closed) {
        return;
    }

    _screenRecordingState = screenRecordingState;
    _accessibilityState = accessibilityState;

    NSColor *panelColorValue = TwarpColor(panelColor);
    [_panel setBackgroundColor:panelColorValue];
    [_content layer].backgroundColor = [panelColorValue CGColor];
    [_content layer].borderWidth = 1.0;
    [_content layer].borderColor = [TwarpColor(accentColor) CGColor];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);
    [_sessionLabel setStringValue:sessionLabel ?: @""];
    [_sessionLabel setTextColor:muted];
    [_summaryLabel setTextColor:text];
    [_screenRecordingLabel setTextColor:muted];
    [_accessibilityLabel setTextColor:muted];

    [_summaryLabel setStringValue:TwarpPermissionSummary(screenRecordingState, accessibilityState)];
    [_screenRecordingLabel setStringValue:TwarpPermissionStatusText(
        @"Screen Recording",
        @"screenshots",
        screenRecordingState)];
    [_accessibilityLabel setStringValue:TwarpPermissionStatusText(
        @"Accessibility",
        @"mouse and keyboard control",
        accessibilityState)];

    [_screenRecordingButton setHidden:TwarpPermissionSettingsURL(screenRecordingState, YES) == nil];
    [_accessibilityButton setHidden:TwarpPermissionSettingsURL(accessibilityState, NO) == nil];
}

- (void)openScreenRecording:(id)sender {
    TwarpOpenSettingsURL(TwarpPermissionSettingsURL(_screenRecordingState, YES));
}

- (void)openAccessibility:(id)sender {
    TwarpOpenSettingsURL(TwarpPermissionSettingsURL(_accessibilityState, NO));
}

- (void)retryPressed:(id)sender {
    if (_retryCallback) {
        _retryCallback(_retryContext);
    }
}

- (void)dismissPressed:(id)sender {
    [self closePanel];
    if (_dismissCallback) {
        _dismissCallback(_dismissContext);
    }
}

- (void)closePanel {
    if (_closed) {
        return;
    }
    _closed = YES;

    if (_panel) {
        [_panel orderOut:nil];
        [_panel close];
        [_panel release];
        _panel = nil;
    }
}

- (void)dealloc {
    [self closePanel];
    [super dealloc];
}

@end

@interface TwarpComputerControlOverlayHost : NSObject {
    NSPanel *_panel;
    NSWindow *_glowWindow;
    NSView *_panelContent;
    CALayer *_glowLayer;
    NSTextField *_sessionLabel;
    NSTextField *_modeLabel;
    NSTextField *_statusLabel;
    NSButton *_stopButton;
    TwarpComputerControlStopCallback _stopCallback;
    void *_stopContext;
    BOOL _closed;
}

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
                          statusLabel:(NSString *)statusLabel
                          panelColor:(TwarpComputerControlColor)panelColor
                           textColor:(TwarpComputerControlColor)textColor
                      mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                           glowColor:(TwarpComputerControlColor)glowColor
                         stopCallback:(TwarpComputerControlStopCallback)stopCallback
                           stopContext:(void *)stopContext;
- (void)updateWithSessionLabel:(NSString *)sessionLabel
                    statusLabel:(NSString *)statusLabel
                    panelColor:(TwarpComputerControlColor)panelColor
                     textColor:(TwarpComputerControlColor)textColor
                mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                     glowColor:(TwarpComputerControlColor)glowColor;
- (void)closeWindows;

@end

@implementation TwarpComputerControlOverlayHost

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
                          statusLabel:(NSString *)statusLabel
                          panelColor:(TwarpComputerControlColor)panelColor
                           textColor:(TwarpComputerControlColor)textColor
                      mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                           glowColor:(TwarpComputerControlColor)glowColor
                         stopCallback:(TwarpComputerControlStopCallback)stopCallback
                           stopContext:(void *)stopContext {
    self = [super init];
    if (!self) {
        return nil;
    }

    _stopCallback = stopCallback;
    _stopContext = stopContext;

    NSScreen *screen = [NSScreen mainScreen];
    if (!screen) {
        [self release];
        return nil;
    }

    NSRect screenFrame = [screen frame];
    NSRect visibleFrame = [screen visibleFrame];
    NSSize panelSize = NSMakeSize(318.0, 132.0);
    CGFloat margin = 18.0;
    NSRect panelRect = NSMakeRect(
        NSMaxX(visibleFrame) - panelSize.width - margin,
        NSMaxY(visibleFrame) - panelSize.height - margin,
        panelSize.width,
        panelSize.height);

    NSWindowStyleMask panelStyle =
        NSWindowStyleMaskTitled |
        NSWindowStyleMaskFullSizeContentView |
        TwarpNonactivatingPanelMask;
    _panel = [[NSPanel alloc] initWithContentRect:panelRect
                                       styleMask:panelStyle
                                         backing:NSBackingStoreBuffered
                                           defer:NO];
    if (!_panel) {
        [self release];
        return nil;
    }

    [_panel setOpaque:NO];
    [_panel setBackgroundColor:TwarpColor(panelColor)];
    [_panel setHasShadow:YES];
    [_panel setHidesOnDeactivate:NO];
    [_panel setCanHide:NO];
    [_panel setReleasedWhenClosed:NO];
    [_panel setTitleVisibility:NSWindowTitleHidden];
    [_panel setTitlebarAppearsTransparent:YES];
    [_panel setMovable:NO];
    [_panel setBecomesKeyOnlyIfNeeded:YES];
    [_panel setCollectionBehavior:
        NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary |
        NSWindowCollectionBehaviorIgnoresCycle];
    [_panel setLevel:TwarpStatusWindowLevel];
    [_panel setSharingType:TwarpWindowSharingNone];

    _panelContent = [[[NSView alloc] initWithFrame:NSMakeRect(0.0, 0.0, panelSize.width, panelSize.height)] autorelease];
    [_panelContent setWantsLayer:YES];
    [_panelContent layer].cornerRadius = 10.0;
    [_panelContent layer].masksToBounds = YES;
    [_panelContent layer].backgroundColor = [TwarpColor(panelColor) CGColor];
    [_panel setContentView:_panelContent];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);

    NSTextField *titleLabel = TwarpLabel(
        NSMakeRect(16.0, 94.0, 214.0, 20.0),
        @"Claude control live",
        13.0,
        YES,
        text);
    [_panelContent addSubview:titleLabel];

    _sessionLabel = TwarpLabel(
        NSMakeRect(16.0, 73.0, 286.0, 18.0),
        sessionLabel,
        11.0,
        NO,
        muted);
    [_panelContent addSubview:_sessionLabel];

    _modeLabel = TwarpLabel(
        NSMakeRect(16.0, 48.0, 286.0, 18.0),
        @"Mode: confirm before act",
        11.0,
        NO,
        text);
    [_panelContent addSubview:_modeLabel];

    _statusLabel = TwarpLabel(
        NSMakeRect(16.0, 27.0, 286.0, 18.0),
        statusLabel ?: @"Latest: no actions yet",
        11.0,
        NO,
        muted);
    [_panelContent addSubview:_statusLabel];

    _stopButton = [NSButton buttonWithTitle:@"Stop" target:self action:@selector(stopPressed:)];
    [_stopButton setFrame:NSMakeRect(238.0, 89.0, 64.0, 28.0)];
    [_stopButton setBezelStyle:NSBezelStyleRounded];
    [_stopButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightSemibold]];
    if ([_stopButton respondsToSelector:@selector(setContentTintColor:)]) {
        [_stopButton setContentTintColor:text];
    }
    [_panelContent addSubview:_stopButton];

    _glowWindow = [[NSWindow alloc] initWithContentRect:screenFrame
                                              styleMask:NSWindowStyleMaskBorderless
                                                backing:NSBackingStoreBuffered
                                                  defer:NO];
    if (!_glowWindow) {
        [self release];
        return nil;
    }

    [_glowWindow setOpaque:NO];
    [_glowWindow setBackgroundColor:[NSColor clearColor]];
    [_glowWindow setHasShadow:NO];
    [_glowWindow setHidesOnDeactivate:NO];
    [_glowWindow setCanHide:NO];
    [_glowWindow setReleasedWhenClosed:NO];
    [_glowWindow setIgnoresMouseEvents:YES];
    [_glowWindow setCollectionBehavior:
        NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary |
        NSWindowCollectionBehaviorIgnoresCycle];
    [_glowWindow setLevel:TwarpFloatingWindowLevel];
    [_glowWindow setSharingType:TwarpWindowSharingNone];

    NSView *glowContent = [[[NSView alloc] initWithFrame:NSMakeRect(0.0, 0.0, screenFrame.size.width, screenFrame.size.height)] autorelease];
    [glowContent setWantsLayer:YES];
    _glowLayer = [glowContent layer];
    _glowLayer.borderWidth = 7.0;
    _glowLayer.borderColor = [TwarpColor(glowColor) CGColor];
    _glowLayer.backgroundColor = [[NSColor clearColor] CGColor];
    [_glowWindow setContentView:glowContent];

    [_glowWindow orderFrontRegardless];
    [_panel orderFrontRegardless];
    [_glowWindow displayIfNeeded];
    [_panel displayIfNeeded];

    return self;
}

- (void)updateWithSessionLabel:(NSString *)sessionLabel
                    statusLabel:(NSString *)statusLabel
                    panelColor:(TwarpComputerControlColor)panelColor
                     textColor:(TwarpComputerControlColor)textColor
                mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                     glowColor:(TwarpComputerControlColor)glowColor {
    if (_closed) {
        return;
    }

    NSColor *panelColorValue = TwarpColor(panelColor);
    [_panel setBackgroundColor:panelColorValue];
    [_panelContent layer].backgroundColor = [panelColorValue CGColor];
    [_sessionLabel setStringValue:sessionLabel ?: @""];
    [_statusLabel setStringValue:statusLabel ?: @"Latest: no actions yet"];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);
    [_sessionLabel setTextColor:muted];
    [_modeLabel setTextColor:text];
    [_statusLabel setTextColor:muted];
    if ([_stopButton respondsToSelector:@selector(setContentTintColor:)]) {
        [_stopButton setContentTintColor:text];
    }

    _glowLayer.borderColor = [TwarpColor(glowColor) CGColor];
}

- (void)stopPressed:(id)sender {
    [self closeWindows];
    if (_stopCallback) {
        _stopCallback(_stopContext);
    }
}

- (void)closeWindows {
    if (_closed) {
        return;
    }
    _closed = YES;

    if (_panel) {
        [_panel orderOut:nil];
        [_panel close];
        [_panel release];
        _panel = nil;
    }
    if (_glowWindow) {
        [_glowWindow orderOut:nil];
        [_glowWindow close];
        [_glowWindow release];
        _glowWindow = nil;
    }
}

- (void)dealloc {
    [self closeWindows];
    [super dealloc];
}

@end

static NSString *TwarpStringFromCString(const char *value) {
    if (!value) {
        return @"";
    }
    NSString *string = [NSString stringWithUTF8String:value];
    return string ?: @"";
}

TwarpComputerControlPermissionSnapshot twarp_computer_control_permissions_preflight(bool prompt_missing) {
    @autoreleasepool {
        bool screenPreflightGranted = false;
        bool screenGranted = false;
        bool screenSupported = false;

        if (@available(macOS 10.15, *)) {
            screenSupported = true;
            screenPreflightGranted = CGPreflightScreenCaptureAccess();
            screenGranted = screenPreflightGranted;
            if (!screenGranted && prompt_missing) {
                screenGranted = CGRequestScreenCaptureAccess();
            }
        }

        bool accessibilityPreflightGranted = AXIsProcessTrusted();
        bool accessibilityGranted = accessibilityPreflightGranted;
        if (!accessibilityGranted && prompt_missing) {
            NSDictionary *options = @{
                (__bridge id)kAXTrustedCheckOptionPrompt: @YES,
            };
            accessibilityGranted = AXIsProcessTrustedWithOptions((CFDictionaryRef)options);
        }

        TwarpComputerControlPermissionSnapshot snapshot = {
            .screen_recording_preflight_granted = screenPreflightGranted,
            .screen_recording_granted = screenGranted,
            .screen_recording_probe_supported = screenSupported,
            .accessibility_preflight_granted = accessibilityPreflightGranted,
            .accessibility_granted = accessibilityGranted,
            .accessibility_probe_supported = true,
        };
        return snapshot;
    }
}

void *twarp_computer_control_permissions_panel_create(
    const char *session_label,
    TwarpComputerControlPermissionState screen_recording_state,
    TwarpComputerControlPermissionState accessibility_state,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor accent_color,
    TwarpComputerControlPermissionCallback retry_callback,
    void *retry_context,
    TwarpComputerControlPermissionCallback dismiss_callback,
    void *dismiss_context) {
    @autoreleasepool {
        TwarpComputerControlPermissionsPanelHost *host =
            [[TwarpComputerControlPermissionsPanelHost alloc] initWithSessionLabel:TwarpStringFromCString(session_label)
                                                              screenRecordingState:screen_recording_state
                                                                 accessibilityState:accessibility_state
                                                                         panelColor:panel_color
                                                                          textColor:text_color
                                                                     mutedTextColor:muted_text_color
                                                                        accentColor:accent_color
                                                                      retryCallback:retry_callback
                                                                       retryContext:retry_context
                                                                    dismissCallback:dismiss_callback
                                                                     dismissContext:dismiss_context];
        return host;
    }
}

void twarp_computer_control_permissions_panel_update(
    void *host,
    const char *session_label,
    TwarpComputerControlPermissionState screen_recording_state,
    TwarpComputerControlPermissionState accessibility_state,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor accent_color) {
    if (!host) {
        return;
    }
    @autoreleasepool {
        TwarpComputerControlPermissionsPanelHost *panel = (TwarpComputerControlPermissionsPanelHost *)host;
        [panel updateWithSessionLabel:TwarpStringFromCString(session_label)
                  screenRecordingState:screen_recording_state
                     accessibilityState:accessibility_state
                             panelColor:panel_color
                              textColor:text_color
                         mutedTextColor:muted_text_color
                            accentColor:accent_color];
    }
}

void twarp_computer_control_permissions_panel_close(void *host) {
    if (!host) {
        return;
    }
    @autoreleasepool {
        TwarpComputerControlPermissionsPanelHost *panel = (TwarpComputerControlPermissionsPanelHost *)host;
        [panel closePanel];
        [panel release];
    }
}

void *twarp_computer_control_overlay_create(
    const char *session_label,
    const char *status_label,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color,
    TwarpComputerControlStopCallback stop_callback,
    void *stop_context) {
    @autoreleasepool {
        TwarpComputerControlOverlayHost *host =
            [[TwarpComputerControlOverlayHost alloc] initWithSessionLabel:TwarpStringFromCString(session_label)
                                                               statusLabel:TwarpStringFromCString(status_label)
                                                                panelColor:panel_color
                                                                 textColor:text_color
                                                            mutedTextColor:muted_text_color
                                                                 glowColor:glow_color
                                                               stopCallback:stop_callback
                                                                 stopContext:stop_context];
        return host;
    }
}

void twarp_computer_control_overlay_update(
    void *host,
    const char *session_label,
    const char *status_label,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color) {
    if (!host) {
        return;
    }
    @autoreleasepool {
        TwarpComputerControlOverlayHost *overlay = (TwarpComputerControlOverlayHost *)host;
        [overlay updateWithSessionLabel:TwarpStringFromCString(session_label)
                            statusLabel:TwarpStringFromCString(status_label)
                             panelColor:panel_color
                              textColor:text_color
                         mutedTextColor:muted_text_color
                              glowColor:glow_color];
    }
}

void twarp_computer_control_overlay_close(void *host) {
    if (!host) {
        return;
    }
    @autoreleasepool {
        TwarpComputerControlOverlayHost *overlay = (TwarpComputerControlOverlayHost *)host;
        [overlay closeWindows];
        [overlay release];
    }
}
