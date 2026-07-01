#import <AppKit/AppKit.h>
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
                          panelColor:(TwarpComputerControlColor)panelColor
                           textColor:(TwarpComputerControlColor)textColor
                      mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                           glowColor:(TwarpComputerControlColor)glowColor
                         stopCallback:(TwarpComputerControlStopCallback)stopCallback
                           stopContext:(void *)stopContext;
- (void)updateWithSessionLabel:(NSString *)sessionLabel
                    panelColor:(TwarpComputerControlColor)panelColor
                     textColor:(TwarpComputerControlColor)textColor
                mutedTextColor:(TwarpComputerControlColor)mutedTextColor
                     glowColor:(TwarpComputerControlColor)glowColor;
- (void)closeWindows;

@end

@implementation TwarpComputerControlOverlayHost

- (instancetype)initWithSessionLabel:(NSString *)sessionLabel
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
        @"Latest: no actions yet",
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

void *twarp_computer_control_overlay_create(
    const char *session_label,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color,
    TwarpComputerControlStopCallback stop_callback,
    void *stop_context) {
    @autoreleasepool {
        TwarpComputerControlOverlayHost *host =
            [[TwarpComputerControlOverlayHost alloc] initWithSessionLabel:TwarpStringFromCString(session_label)
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
