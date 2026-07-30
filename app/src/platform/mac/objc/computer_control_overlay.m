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

// Derives the animated border-gradient stops from the pane accent color:
// the accent itself, a brightened variant, and a hue-shifted companion.
static NSArray *TwarpGlowGradientColors(TwarpComputerControlColor color) {
    NSColor *base = [TwarpColor(color) colorUsingColorSpace:[NSColorSpace deviceRGBColorSpace]];
    CGFloat hue = 0.0, saturation = 0.0, brightness = 0.0, alpha = 1.0;
    [base getHue:&hue saturation:&saturation brightness:&brightness alpha:&alpha];
    NSColor *bright = [NSColor colorWithHue:hue
                                 saturation:MAX(saturation - 0.25, 0.0)
                                 brightness:MIN(brightness + 0.35, 1.0)
                                      alpha:alpha];
    NSColor *shifted = [NSColor colorWithHue:fmod(hue + 0.12, 1.0)
                                  saturation:saturation
                                  brightness:brightness
                                       alpha:alpha];
    NSColor *shiftedBack = [NSColor colorWithHue:fmod(hue + 1.0 - 0.10, 1.0)
                                      saturation:saturation
                                      brightness:brightness
                                           alpha:alpha];
    return @[
        (id)[base CGColor],
        (id)[shifted CGColor],
        (id)[bright CGColor],
        (id)[shiftedBack CGColor],
        (id)[base CGColor],
    ];
}

static NSString *TwarpPermissionSettingsURL(TwarpComputerControlPermissionState state, BOOL screenRecording) {
    if (state == TwarpComputerControlPermissionGranted ||
        state == TwarpComputerControlPermissionRestartRequired) {
        return nil;
    }

    return screenRecording
        ? @"x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        : @"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
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
        return @"Permissions were granted. Restart Twarp so this running process can use them.";
    }
    if (screenBlocked || accessibilityBlocked) {
        return @"Twarp Computer Use needs these permissions to use apps on your Mac. They are only used when you ask the agent to perform tasks.";
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
    NSTextField *_titleLabel;
    NSTextField *_sessionLabel;
    NSTextField *_summaryLabel;
    NSView *_accessibilityCard;
    NSView *_screenRecordingCard;
    NSTextField *_accessibilityTitle;
    NSTextField *_accessibilityDetail;
    NSTextField *_screenRecordingTitle;
    NSTextField *_screenRecordingDetail;
    NSTextField *_accessibilityStatus;
    NSTextField *_screenRecordingStatus;
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
- (NSView *)permissionCardWithFrame:(NSRect)frame
                         symbolName:(NSString *)symbolName
                              title:(NSString *)title
                             detail:(NSString *)detail
                         titleLabel:(NSTextField **)titleLabel
                        detailLabel:(NSTextField **)detailLabel
                        statusLabel:(NSTextField **)statusLabel
                        allowButton:(NSButton **)allowButton
                             action:(SEL)action
                          textColor:(NSColor *)textColor
                         mutedColor:(NSColor *)mutedColor;
- (void)applyState:(TwarpComputerControlPermissionState)state
       statusLabel:(NSTextField *)statusLabel
       allowButton:(NSButton *)allowButton
       accentColor:(NSColor *)accentColor;
- (void)closePanel;

@end

@implementation TwarpComputerControlPermissionsPanelHost

- (NSView *)permissionCardWithFrame:(NSRect)frame
                         symbolName:(NSString *)symbolName
                              title:(NSString *)title
                             detail:(NSString *)detail
                         titleLabel:(NSTextField **)titleLabel
                        detailLabel:(NSTextField **)detailLabel
                        statusLabel:(NSTextField **)statusLabel
                        allowButton:(NSButton **)allowButton
                             action:(SEL)action
                          textColor:(NSColor *)textColor
                         mutedColor:(NSColor *)mutedColor {
    NSView *card = [[[NSView alloc] initWithFrame:frame] autorelease];
    [card setWantsLayer:YES];
    [card layer].cornerRadius = 12.0;
    [card layer].masksToBounds = YES;
    [card layer].backgroundColor = [[textColor colorWithAlphaComponent:0.07] CGColor];

    if (@available(macOS 11.0, *)) {
        NSImage *symbol = [NSImage imageWithSystemSymbolName:symbolName
                                    accessibilityDescription:title];
        if (symbol) {
            NSImageView *symbolView =
                [[[NSImageView alloc] initWithFrame:NSMakeRect(18.0, 19.0, 30.0, 30.0)] autorelease];
            [symbolView setImage:symbol];
            [symbolView setContentTintColor:textColor];
            [symbolView setImageScaling:NSImageScaleProportionallyUpOrDown];
            [card addSubview:symbolView];
        }
    }

    *titleLabel = TwarpLabel(NSMakeRect(60.0, 34.0, 250.0, 20.0), title, 13.0, YES, textColor);
    [card addSubview:*titleLabel];

    *detailLabel = TwarpLabel(NSMakeRect(60.0, 14.0, 268.0, 16.0), detail, 11.0, NO, mutedColor);
    [card addSubview:*detailLabel];

    *allowButton = [NSButton buttonWithTitle:@"Allow" target:self action:action];
    [*allowButton setFrame:NSMakeRect(352.0, 19.0, 72.0, 30.0)];
    [*allowButton setBezelStyle:NSBezelStyleRounded];
    [*allowButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightSemibold]];
    [card addSubview:*allowButton];

    *statusLabel = TwarpLabel(NSMakeRect(300.0, 25.0, 124.0, 18.0), @"", 11.0, YES, mutedColor);
    [*statusLabel setAlignment:NSTextAlignmentRight];
    [*statusLabel setHidden:YES];
    [card addSubview:*statusLabel];

    return card;
}

- (void)applyState:(TwarpComputerControlPermissionState)state
       statusLabel:(NSTextField *)statusLabel
       allowButton:(NSButton *)allowButton
       accentColor:(NSColor *)accentColor {
    switch (state) {
        case TwarpComputerControlPermissionGranted:
            [allowButton setHidden:YES];
            [statusLabel setStringValue:@"Granted ✓"];
            [statusLabel setTextColor:[NSColor systemGreenColor]];
            [statusLabel setHidden:NO];
            break;
        case TwarpComputerControlPermissionRestartRequired:
            [allowButton setHidden:YES];
            [statusLabel setStringValue:@"Restart Twarp"];
            [statusLabel setTextColor:[NSColor systemOrangeColor]];
            [statusLabel setHidden:NO];
            break;
        case TwarpComputerControlPermissionMissing:
        case TwarpComputerControlPermissionDeniedOrUnknown:
        default:
            [statusLabel setHidden:YES];
            [allowButton setHidden:NO];
            if ([allowButton respondsToSelector:@selector(setBezelColor:)]) {
                [allowButton setBezelColor:accentColor];
            }
            break;
    }
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
    NSSize panelSize = NSMakeSize(480.0, 440.0);
    NSRect panelRect = NSMakeRect(
        NSMidX(visibleFrame) - panelSize.width / 2.0,
        NSMidY(visibleFrame) - panelSize.height / 2.0,
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
    [_panel setMovable:YES];
    [_panel setMovableByWindowBackground:YES];
    [_panel setBecomesKeyOnlyIfNeeded:YES];
    [_panel setCollectionBehavior:
        NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary |
        NSWindowCollectionBehaviorIgnoresCycle];
    [_panel setLevel:TwarpStatusWindowLevel];
    [_panel setSharingType:TwarpWindowSharingNone];

    _content = [[[NSView alloc] initWithFrame:NSMakeRect(0.0, 0.0, panelSize.width, panelSize.height)] autorelease];
    [_content setWantsLayer:YES];
    [_content layer].cornerRadius = 14.0;
    [_content layer].masksToBounds = YES;
    [_panel setContentView:_content];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);

    NSImageView *iconView =
        [[[NSImageView alloc] initWithFrame:NSMakeRect(208.0, 352.0, 64.0, 64.0)] autorelease];
    [iconView setImage:[NSApp applicationIconImage]];
    [iconView setImageScaling:NSImageScaleProportionallyUpOrDown];
    [_content addSubview:iconView];

    _titleLabel = TwarpLabel(
        NSMakeRect(20.0, 314.0, 440.0, 28.0),
        @"Enable Twarp Computer Use",
        20.0,
        YES,
        text);
    [_titleLabel setAlignment:NSTextAlignmentCenter];
    [_content addSubview:_titleLabel];

    _sessionLabel = TwarpLabel(
        NSMakeRect(20.0, 294.0, 440.0, 16.0),
        sessionLabel,
        11.0,
        NO,
        muted);
    [_sessionLabel setAlignment:NSTextAlignmentCenter];
    [_content addSubview:_sessionLabel];

    _summaryLabel = TwarpLabel(
        NSMakeRect(48.0, 250.0, 384.0, 38.0),
        @"",
        12.0,
        NO,
        muted);
    [_summaryLabel setAlignment:NSTextAlignmentCenter];
    [_summaryLabel setLineBreakMode:NSLineBreakByWordWrapping];
    [_summaryLabel setUsesSingleLineMode:NO];
    [_content addSubview:_summaryLabel];

    _accessibilityCard = [self permissionCardWithFrame:NSMakeRect(20.0, 166.0, 440.0, 68.0)
                                            symbolName:@"accessibility"
                                                 title:@"Accessibility"
                                                detail:@"Allows Twarp to send clicks and keystrokes"
                                            titleLabel:&_accessibilityTitle
                                           detailLabel:&_accessibilityDetail
                                           statusLabel:&_accessibilityStatus
                                           allowButton:&_accessibilityButton
                                                action:@selector(openAccessibility:)
                                             textColor:text
                                            mutedColor:muted];
    [_content addSubview:_accessibilityCard];

    _screenRecordingCard = [self permissionCardWithFrame:NSMakeRect(20.0, 88.0, 440.0, 68.0)
                                              symbolName:@"camera.viewfinder"
                                                   title:@"Screenshots"
                                                  detail:@"Twarp uses screenshots to know where to click"
                                              titleLabel:&_screenRecordingTitle
                                             detailLabel:&_screenRecordingDetail
                                             statusLabel:&_screenRecordingStatus
                                             allowButton:&_screenRecordingButton
                                                  action:@selector(openScreenRecording:)
                                               textColor:text
                                              mutedColor:muted];
    [_content addSubview:_screenRecordingCard];

    _retryButton = [NSButton buttonWithTitle:@"Retry" target:self action:@selector(retryPressed:)];
    [_retryButton setFrame:NSMakeRect(282.0, 22.0, 80.0, 30.0)];
    [_retryButton setBezelStyle:NSBezelStyleRounded];
    [_retryButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightSemibold]];
    [_content addSubview:_retryButton];

    _dismissButton = [NSButton buttonWithTitle:@"Not now" target:self action:@selector(dismissPressed:)];
    [_dismissButton setFrame:NSMakeRect(370.0, 22.0, 90.0, 30.0)];
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
    [_content layer].borderColor = [[TwarpColor(textColor) colorWithAlphaComponent:0.12] CGColor];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);
    NSColor *accent = TwarpColor(accentColor);
    [_titleLabel setTextColor:text];
    [_sessionLabel setStringValue:sessionLabel ?: @""];
    [_sessionLabel setTextColor:muted];
    [_summaryLabel setTextColor:muted];
    [_accessibilityTitle setTextColor:text];
    [_screenRecordingTitle setTextColor:text];
    [_accessibilityDetail setTextColor:muted];
    [_screenRecordingDetail setTextColor:muted];
    [_accessibilityCard layer].backgroundColor = [[text colorWithAlphaComponent:0.07] CGColor];
    [_screenRecordingCard layer].backgroundColor = [[text colorWithAlphaComponent:0.07] CGColor];

    [_summaryLabel setStringValue:TwarpPermissionSummary(screenRecordingState, accessibilityState)];

    [self applyState:accessibilityState
         statusLabel:_accessibilityStatus
         allowButton:_accessibilityButton
         accentColor:accent];
    [self applyState:screenRecordingState
         statusLabel:_screenRecordingStatus
         allowButton:_screenRecordingButton
         accentColor:accent];
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
    CAGradientLayer *_glowLayer;
    NSTextField *_titleLabel;
    NSTextField *_sessionLabel;
    NSTextField *_modeLabel;
    NSTextField *_statusLabel;
    NSButton *_stopButton;
    NSButton *_approveButton;
    NSButton *_rejectButton;
    NSButton *_logButton;
    NSScrollView *_logScrollView;
    NSTextView *_logTextView;
    TwarpComputerControlStopCallback _stopCallback;
    void *_stopContext;
    TwarpComputerControlStopCallback _approveCallback;
    void *_approveContext;
    TwarpComputerControlStopCallback _rejectCallback;
    void *_rejectContext;
    BOOL _confirmationPending;
    BOOL _logVisible;
    BOOL _closed;
}

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
                          statusLabel:(NSString *)statusLabel
                            actionLog:(NSString *)actionLog
                  confirmationPending:(BOOL)confirmationPending
                          panelColor:(TwarpComputerControlColor)panelColor
                           textColor:(TwarpComputerControlColor)textColor
                      mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                           glowColor:(TwarpComputerControlColor)glowColor
                         stopCallback:(TwarpComputerControlStopCallback)stopCallback
                           stopContext:(void *)stopContext
                       approveCallback:(TwarpComputerControlStopCallback)approveCallback
                        approveContext:(void *)approveContext
                        rejectCallback:(TwarpComputerControlStopCallback)rejectCallback
                         rejectContext:(void *)rejectContext;
- (void)updateWithSessionLabel:(NSString *)sessionLabel
                    statusLabel:(NSString *)statusLabel
                      actionLog:(NSString *)actionLog
            confirmationPending:(BOOL)confirmationPending
                    panelColor:(TwarpComputerControlColor)panelColor
                     textColor:(TwarpComputerControlColor)textColor
                mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                     glowColor:(TwarpComputerControlColor)glowColor;
- (void)closeWindows;

@end

@implementation TwarpComputerControlOverlayHost

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
                          statusLabel:(NSString *)statusLabel
                            actionLog:(NSString *)actionLog
                  confirmationPending:(BOOL)confirmationPending
                          panelColor:(TwarpComputerControlColor)panelColor
                           textColor:(TwarpComputerControlColor)textColor
                      mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                           glowColor:(TwarpComputerControlColor)glowColor
                         stopCallback:(TwarpComputerControlStopCallback)stopCallback
                           stopContext:(void *)stopContext
                       approveCallback:(TwarpComputerControlStopCallback)approveCallback
                        approveContext:(void *)approveContext
                        rejectCallback:(TwarpComputerControlStopCallback)rejectCallback
                         rejectContext:(void *)rejectContext {
    self = [super init];
    if (!self) {
        return nil;
    }

    _stopCallback = stopCallback;
    _stopContext = stopContext;
    _approveCallback = approveCallback;
    _approveContext = approveContext;
    _rejectCallback = rejectCallback;
    _rejectContext = rejectContext;
    _confirmationPending = confirmationPending;
    _logVisible = NO;

    NSScreen *screen = [NSScreen mainScreen];
    if (!screen) {
        [self release];
        return nil;
    }

    NSRect screenFrame = [screen frame];
    NSRect visibleFrame = [screen visibleFrame];
    NSSize panelSize = NSMakeSize(342.0, 174.0);
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

    _titleLabel = TwarpLabel(
        NSMakeRect(16.0, 136.0, 214.0, 20.0),
        @"Claude control live",
        13.0,
        YES,
        text);
    [_panelContent addSubview:_titleLabel];

    _sessionLabel = TwarpLabel(
        NSMakeRect(16.0, 115.0, 310.0, 18.0),
        sessionLabel,
        11.0,
        NO,
        muted);
    [_panelContent addSubview:_sessionLabel];

    _modeLabel = TwarpLabel(
        NSMakeRect(16.0, 91.0, 310.0, 18.0),
        @"Mode: confirm before act",
        11.0,
        NO,
        text);
    [_panelContent addSubview:_modeLabel];

    _statusLabel = TwarpLabel(
        NSMakeRect(16.0, 70.0, 310.0, 18.0),
        statusLabel ?: @"Latest: no actions yet",
        11.0,
        NO,
        muted);
    [_panelContent addSubview:_statusLabel];

    _stopButton = [NSButton buttonWithTitle:@"Stop" target:self action:@selector(stopPressed:)];
    [_stopButton setFrame:NSMakeRect(262.0, 131.0, 64.0, 28.0)];
    [_stopButton setBezelStyle:NSBezelStyleRounded];
    [_stopButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightSemibold]];
    if ([_stopButton respondsToSelector:@selector(setContentTintColor:)]) {
        [_stopButton setContentTintColor:text];
    }
    [_panelContent addSubview:_stopButton];

    _approveButton = [NSButton buttonWithTitle:@"Approve" target:self action:@selector(approvePressed:)];
    [_approveButton setFrame:NSMakeRect(16.0, 34.0, 92.0, 28.0)];
    [_approveButton setBezelStyle:NSBezelStyleRounded];
    [_approveButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightSemibold]];
    [_approveButton setHidden:!_confirmationPending];
    [_panelContent addSubview:_approveButton];

    _rejectButton = [NSButton buttonWithTitle:@"Reject" target:self action:@selector(rejectPressed:)];
    [_rejectButton setFrame:NSMakeRect(116.0, 34.0, 82.0, 28.0)];
    [_rejectButton setBezelStyle:NSBezelStyleRounded];
    [_rejectButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightRegular]];
    [_rejectButton setHidden:!_confirmationPending];
    [_panelContent addSubview:_rejectButton];

    _logButton = [NSButton buttonWithTitle:@"Log" target:self action:@selector(toggleLog:)];
    [_logButton setFrame:NSMakeRect(258.0, 34.0, 68.0, 28.0)];
    [_logButton setBezelStyle:NSBezelStyleRounded];
    [_logButton setFont:[NSFont systemFontOfSize:12.0 weight:NSFontWeightRegular]];
    [_panelContent addSubview:_logButton];

    _logTextView = [[[NSTextView alloc] initWithFrame:NSMakeRect(0.0, 0.0, 294.0, 96.0)] autorelease];
    [_logTextView setEditable:NO];
    [_logTextView setSelectable:YES];
    [_logTextView setDrawsBackground:NO];
    [_logTextView setTextColor:muted];
    [_logTextView setFont:[NSFont userFixedPitchFontOfSize:11.0]];
    [_logTextView setString:actionLog ?: @"No computer-control actions yet."];

    _logScrollView = [[[NSScrollView alloc] initWithFrame:NSMakeRect(16.0, 34.0, 310.0, 96.0)] autorelease];
    [_logScrollView setDocumentView:_logTextView];
    [_logScrollView setHasVerticalScroller:YES];
    [_logScrollView setDrawsBackground:NO];
    [_logScrollView setBorderType:NSNoBorder];
    [_logScrollView setHidden:YES];
    [_panelContent addSubview:_logScrollView];

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
    [glowContent layer].backgroundColor = [[NSColor clearColor] CGColor];

    // A slowly-shimmering gradient ring instead of a flat border: a
    // full-screen gradient layer masked down to a 7pt edge ring.
    _glowLayer = [CAGradientLayer layer];
    _glowLayer.frame = NSMakeRect(0.0, 0.0, screenFrame.size.width, screenFrame.size.height);
    _glowLayer.startPoint = CGPointMake(0.0, 0.0);
    _glowLayer.endPoint = CGPointMake(1.0, 1.0);
    _glowLayer.colors = TwarpGlowGradientColors(glowColor);

    CAShapeLayer *ringMask = [CAShapeLayer layer];
    CGMutablePathRef ringPath = CGPathCreateMutable();
    CGPathAddRect(ringPath, NULL, NSRectToCGRect([glowContent bounds]));
    CGPathAddRect(ringPath, NULL, CGRectInset(NSRectToCGRect([glowContent bounds]), 7.0, 7.0));
    ringMask.path = ringPath;
    CGPathRelease(ringPath);
    ringMask.fillRule = kCAFillRuleEvenOdd;
    _glowLayer.mask = ringMask;

    CABasicAnimation *sweep = [CABasicAnimation animationWithKeyPath:@"startPoint"];
    sweep.fromValue = [NSValue valueWithPoint:NSMakePoint(0.0, 0.0)];
    sweep.toValue = [NSValue valueWithPoint:NSMakePoint(0.0, 1.0)];
    sweep.duration = 3.5;
    sweep.autoreverses = YES;
    sweep.repeatCount = HUGE_VALF;
    sweep.timingFunction =
        [CAMediaTimingFunction functionWithName:kCAMediaTimingFunctionEaseInEaseOut];
    [_glowLayer addAnimation:sweep forKey:@"twarp_glow_sweep"];
    CABasicAnimation *sweepEnd = [CABasicAnimation animationWithKeyPath:@"endPoint"];
    sweepEnd.fromValue = [NSValue valueWithPoint:NSMakePoint(1.0, 1.0)];
    sweepEnd.toValue = [NSValue valueWithPoint:NSMakePoint(1.0, 0.0)];
    sweepEnd.duration = 3.5;
    sweepEnd.autoreverses = YES;
    sweepEnd.repeatCount = HUGE_VALF;
    sweepEnd.timingFunction =
        [CAMediaTimingFunction functionWithName:kCAMediaTimingFunctionEaseInEaseOut];
    [_glowLayer addAnimation:sweepEnd forKey:@"twarp_glow_sweep_end"];

    [[glowContent layer] addSublayer:_glowLayer];
    [_glowWindow setContentView:glowContent];

    [_glowWindow orderFrontRegardless];
    [_panel orderFrontRegardless];
    [_glowWindow displayIfNeeded];
    [_panel displayIfNeeded];

    return self;
}

- (void)updateWithSessionLabel:(NSString *)sessionLabel
                    statusLabel:(NSString *)statusLabel
                      actionLog:(NSString *)actionLog
            confirmationPending:(BOOL)confirmationPending
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
    [_logTextView setString:actionLog ?: @"No computer-control actions yet."];
    _confirmationPending = confirmationPending;
    [_approveButton setHidden:!_confirmationPending || _logVisible];
    [_rejectButton setHidden:!_confirmationPending || _logVisible];

    NSColor *text = TwarpColor(textColor);
    NSColor *muted = TwarpColor(mutedTextColor);
    [_sessionLabel setTextColor:muted];
    [_modeLabel setTextColor:text];
    [_statusLabel setTextColor:muted];
    [_logTextView setTextColor:muted];
    if ([_stopButton respondsToSelector:@selector(setContentTintColor:)]) {
        [_stopButton setContentTintColor:text];
    }

    _glowLayer.colors = TwarpGlowGradientColors(glowColor);
}

- (void)stopPressed:(id)sender {
    [self closeWindows];
    if (_stopCallback) {
        _stopCallback(_stopContext);
    }
}

- (void)approvePressed:(id)sender {
    [_approveButton setHidden:YES];
    [_rejectButton setHidden:YES];
    if (_approveCallback) {
        _approveCallback(_approveContext);
    }
}

- (void)rejectPressed:(id)sender {
    [_approveButton setHidden:YES];
    [_rejectButton setHidden:YES];
    if (_rejectCallback) {
        _rejectCallback(_rejectContext);
    }
}

- (void)toggleLog:(id)sender {
    _logVisible = !_logVisible;
    [_logButton setTitle:_logVisible ? @"Hide" : @"Log"];
    [_logScrollView setHidden:!_logVisible];
    [_approveButton setHidden:!_confirmationPending || _logVisible];
    [_rejectButton setHidden:!_confirmationPending || _logVisible];

    CGFloat targetHeight = _logVisible ? 318.0 : 174.0;
    NSRect frame = [_panel frame];
    CGFloat top = NSMaxY(frame);
    frame.origin.y = top - targetHeight;
    frame.size.height = targetHeight;
    [_panel setFrame:frame display:YES animate:NO];
    [_panelContent setFrame:NSMakeRect(0.0, 0.0, frame.size.width, targetHeight)];

    CGFloat offset = targetHeight - 174.0;
    [_titleLabel setFrameOrigin:NSMakePoint(16.0, 136.0 + offset)];
    [_stopButton setFrameOrigin:NSMakePoint(262.0, 131.0 + offset)];
    [_sessionLabel setFrameOrigin:NSMakePoint(16.0, 115.0 + offset)];
    [_modeLabel setFrameOrigin:NSMakePoint(16.0, 91.0 + offset)];
    [_statusLabel setFrameOrigin:NSMakePoint(16.0, 70.0 + offset)];
    [_approveButton setFrameOrigin:NSMakePoint(16.0, 34.0 + offset)];
    [_rejectButton setFrameOrigin:NSMakePoint(116.0, 34.0 + offset)];
    [_logButton setFrameOrigin:NSMakePoint(258.0, 34.0 + offset)];
    [_logScrollView setFrame:NSMakeRect(16.0, 34.0, 310.0, 112.0)];
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
    const char *action_log,
    bool confirmation_pending,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color,
    TwarpComputerControlStopCallback stop_callback,
    void *stop_context,
    TwarpComputerControlStopCallback approve_callback,
    void *approve_context,
    TwarpComputerControlStopCallback reject_callback,
    void *reject_context) {
    @autoreleasepool {
        TwarpComputerControlOverlayHost *host =
            [[TwarpComputerControlOverlayHost alloc] initWithSessionLabel:TwarpStringFromCString(session_label)
                                                               statusLabel:TwarpStringFromCString(status_label)
                                                                 actionLog:TwarpStringFromCString(action_log)
                                                       confirmationPending:confirmation_pending
                                                                panelColor:panel_color
                                                                 textColor:text_color
                                                            mutedTextColor:muted_text_color
                                                                 glowColor:glow_color
                                                               stopCallback:stop_callback
                                                                 stopContext:stop_context
                                                             approveCallback:approve_callback
                                                              approveContext:approve_context
                                                              rejectCallback:reject_callback
                                                               rejectContext:reject_context];
        return host;
    }
}

void twarp_computer_control_overlay_update(
    void *host,
    const char *session_label,
    const char *status_label,
    const char *action_log,
    bool confirmation_pending,
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
                              actionLog:TwarpStringFromCString(action_log)
                    confirmationPending:confirmation_pending
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
