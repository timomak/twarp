#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <QuartzCore/QuartzCore.h>

#import "computer_control_extras.h"

static const NSWindowSharingType TwarpExtrasWindowSharingNone = 0;
static const NSInteger TwarpExtrasCursorWindowLevel = 26;
static const NSInteger TwarpExtrasBadgeWindowLevel = 25;

static void TwarpExtrasRunOnMain(void (^block)(void)) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_async(dispatch_get_main_queue(), block);
    }
}

static void TwarpExtrasRunOnMainSync(void (^block)(void)) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

static NSColor *TwarpExtrasColor(TwarpComputerControlColor color) {
    return [NSColor colorWithCalibratedRed:(CGFloat)color.r / 255.0
                                     green:(CGFloat)color.g / 255.0
                                      blue:(CGFloat)color.b / 255.0
                                     alpha:(CGFloat)color.a / 255.0];
}

static NSString *TwarpExtrasString(const char *value) {
    if (!value) {
        return @"";
    }
    NSString *string = [NSString stringWithUTF8String:value];
    return string ?: @"";
}

// Physical-pixel top-left-origin screen coordinates -> Cocoa bottom-left points.
static NSPoint TwarpExtrasCocoaPoint(double x, double y) {
    NSScreen *screen = [NSScreen mainScreen];
    CGFloat scale = screen ? [screen backingScaleFactor] : 1.0;
    if (scale <= 0.0) {
        scale = 1.0;
    }
    CGFloat height = screen ? NSHeight([screen frame]) : 0.0;
    return NSMakePoint(x / scale, height - (y / scale));
}

// ---------------------------------------------------------------------------
// Fake cursor
// ---------------------------------------------------------------------------

static const CGFloat TwarpCursorWindowSize = 72.0;
// Where the arrow tip sits inside the cursor window.
static const CGFloat TwarpCursorTipX = 36.0;
static const CGFloat TwarpCursorTipY = 40.0;

static NSWindow *gCursorWindow = nil;

static NSBezierPath *TwarpCursorArrowPath(void) {
    // A classic pointer arrow, tip at (0, 0), pointing up-left, in a
    // y-flipped (top-left origin) view coordinate space.
    NSBezierPath *path = [NSBezierPath bezierPath];
    [path moveToPoint:NSMakePoint(0.0, 0.0)];
    [path lineToPoint:NSMakePoint(0.0, 17.5)];
    [path lineToPoint:NSMakePoint(4.1, 13.9)];
    [path lineToPoint:NSMakePoint(7.3, 20.6)];
    [path lineToPoint:NSMakePoint(10.4, 19.1)];
    [path lineToPoint:NSMakePoint(7.2, 12.5)];
    [path lineToPoint:NSMakePoint(12.6, 12.0)];
    [path closePath];
    return path;
}

static CGPathRef TwarpCursorArrowCGPath(void) {
    NSBezierPath *path = TwarpCursorArrowPath();
    CGMutablePathRef cgPath = CGPathCreateMutable();
    NSInteger elementCount = [path elementCount];
    NSPoint points[3];
    for (NSInteger index = 0; index < elementCount; index += 1) {
        switch ([path elementAtIndex:index associatedPoints:points]) {
            case NSBezierPathElementMoveTo:
                CGPathMoveToPoint(cgPath, NULL, points[0].x, points[0].y);
                break;
            case NSBezierPathElementLineTo:
                CGPathAddLineToPoint(cgPath, NULL, points[0].x, points[0].y);
                break;
            case NSBezierPathElementCurveTo:
                CGPathAddCurveToPoint(
                    cgPath, NULL,
                    points[0].x, points[0].y,
                    points[1].x, points[1].y,
                    points[2].x, points[2].y);
                break;
            case NSBezierPathElementClosePath:
                CGPathCloseSubpath(cgPath);
                break;
            default:
                break;
        }
    }
    return cgPath;
}

static void TwarpCursorCreateWindowIfNeeded(void) {
    if (gCursorWindow) {
        return;
    }

    NSRect frame = NSMakeRect(0.0, 0.0, TwarpCursorWindowSize, TwarpCursorWindowSize);
    NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:NSWindowStyleMaskBorderless
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    if (!window) {
        return;
    }
    [window setOpaque:NO];
    [window setBackgroundColor:[NSColor clearColor]];
    [window setHasShadow:NO];
    [window setHidesOnDeactivate:NO];
    [window setCanHide:NO];
    [window setReleasedWhenClosed:NO];
    [window setIgnoresMouseEvents:YES];
    [window setCollectionBehavior:
        NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary |
        NSWindowCollectionBehaviorIgnoresCycle];
    [window setLevel:TwarpExtrasCursorWindowLevel];
    [window setSharingType:TwarpExtrasWindowSharingNone];
    [window setAnimationBehavior:NSWindowAnimationBehaviorNone];

    NSView *content = [[[NSView alloc] initWithFrame:frame] autorelease];
    [content setWantsLayer:YES];
    CALayer *root = [content layer];
    root.backgroundColor = [[NSColor clearColor] CGColor];

    // Soft radial glow behind the arrow.
    CAGradientLayer *glow = [CAGradientLayer layer];
    glow.frame = NSMakeRect(0.0, 0.0, TwarpCursorWindowSize, TwarpCursorWindowSize);
    glow.type = kCAGradientLayerRadial;
    glow.startPoint = CGPointMake(0.5, 0.5);
    glow.endPoint = CGPointMake(1.0, 1.0);
    glow.colors = @[
        (id)[[NSColor colorWithCalibratedWhite:0.55 alpha:0.55] CGColor],
        (id)[[NSColor colorWithCalibratedWhite:0.65 alpha:0.22] CGColor],
        (id)[[NSColor colorWithCalibratedWhite:0.75 alpha:0.0] CGColor],
    ];
    glow.cornerRadius = TwarpCursorWindowSize / 2.0;
    [root addSublayer:glow];

    // Black arrow with a white outline, tip anchored at the window centerish
    // point so the window origin maps cleanly to the event coordinate.
    CAShapeLayer *arrow = [CAShapeLayer layer];
    CGPathRef arrowPath = TwarpCursorArrowCGPath();
    arrow.path = arrowPath;
    CGPathRelease(arrowPath);
    arrow.fillColor = [[NSColor blackColor] CGColor];
    arrow.strokeColor = [[NSColor whiteColor] CGColor];
    arrow.lineWidth = 1.5;
    arrow.lineJoin = kCALineJoinRound;
    // The path is authored tip-at-origin growing downward; flip vertically to
    // render in the layer's bottom-left-origin space.
    arrow.frame = NSMakeRect(TwarpCursorTipX, TwarpCursorTipY - 21.0, 21.0, 21.0);
    arrow.affineTransform = CGAffineTransformMake(1.0, 0.0, 0.0, -1.0, 0.0, 21.0);
    [root addSublayer:arrow];

    [window setContentView:content];
    gCursorWindow = window;
}

void twarp_computer_control_cursor_show(void) {
    TwarpExtrasRunOnMain(^{
        TwarpCursorCreateWindowIfNeeded();
        if (!gCursorWindow) {
            return;
        }
        NSPoint mouse = [NSEvent mouseLocation];
        [gCursorWindow setFrameOrigin:NSMakePoint(
            mouse.x - TwarpCursorTipX,
            mouse.y - TwarpCursorTipY)];
        [gCursorWindow orderFrontRegardless];
    });
}

void twarp_computer_control_cursor_move(double x, double y, bool animate) {
    TwarpExtrasRunOnMain(^{
        TwarpCursorCreateWindowIfNeeded();
        if (!gCursorWindow) {
            return;
        }
        NSPoint tip = TwarpExtrasCocoaPoint(x, y);
        NSRect target = [gCursorWindow frame];
        target.origin = NSMakePoint(tip.x - TwarpCursorTipX, tip.y - TwarpCursorTipY);
        [gCursorWindow orderFrontRegardless];
        if (animate) {
            [NSAnimationContext runAnimationGroup:^(NSAnimationContext *context) {
                context.duration = 0.28;
                context.timingFunction =
                    [CAMediaTimingFunction functionWithName:kCAMediaTimingFunctionEaseInEaseOut];
                [[gCursorWindow animator] setFrame:target display:YES];
            }];
        } else {
            [gCursorWindow setFrame:target display:YES];
        }
    });
}

void twarp_computer_control_cursor_hide(void) {
    TwarpExtrasRunOnMain(^{
        if (gCursorWindow) {
            [gCursorWindow orderOut:nil];
        }
    });
}

// ---------------------------------------------------------------------------
// Menu-bar status item
// ---------------------------------------------------------------------------

@interface TwarpComputerControlStatusItemHost : NSObject {
@public
    NSStatusItem *_item;
    TwarpComputerControlExtrasCallback _callback;
    void *_context;
}
- (void)stopPressed:(id)sender;
@end

@implementation TwarpComputerControlStatusItemHost

- (void)stopPressed:(id)sender {
    if (_callback) {
        _callback(_context);
    }
}

- (void)dealloc {
    if (_item) {
        [[NSStatusBar systemStatusBar] removeStatusItem:_item];
        [_item release];
        _item = nil;
    }
    [super dealloc];
}

@end

static TwarpComputerControlStatusItemHost *gStatusItemHost = nil;

void twarp_computer_control_status_item_show(
    const char *stop_title,
    TwarpComputerControlExtrasCallback callback,
    void *context) {
    NSString *title = TwarpExtrasString(stop_title);
    TwarpExtrasRunOnMain(^{
        if (gStatusItemHost) {
            twarp_computer_control_status_item_hide();
        }
        TwarpComputerControlStatusItemHost *host = [[TwarpComputerControlStatusItemHost alloc] init];
        host->_callback = callback;
        host->_context = context;

        NSStatusItem *item =
            [[[NSStatusBar systemStatusBar] statusItemWithLength:NSSquareStatusItemLength] retain];
        host->_item = item;

        NSImage *image = nil;
        if (@available(macOS 11.0, *)) {
            image = [NSImage imageWithSystemSymbolName:@"cursorarrow"
                              accessibilityDescription:@"Computer control active"];
        }
        if (image) {
            [image setTemplate:YES];
            [[item button] setImage:image];
        } else {
            [[item button] setTitle:@"⌖"];
        }

        NSMenu *menu = [[[NSMenu alloc] init] autorelease];
        NSMenuItem *stopEntry = [[[NSMenuItem alloc] initWithTitle:title
                                                            action:@selector(stopPressed:)
                                                     keyEquivalent:@""] autorelease];
        [stopEntry setTarget:host];
        if (@available(macOS 11.0, *)) {
            NSImage *stopImage = [NSImage imageWithSystemSymbolName:@"stop.circle"
                                           accessibilityDescription:@"Stop"];
            [stopEntry setImage:stopImage];
        }
        [menu addItem:stopEntry];
        [item setMenu:menu];

        gStatusItemHost = host;
    });
}

void twarp_computer_control_status_item_set_title(const char *stop_title) {
    NSString *title = TwarpExtrasString(stop_title);
    TwarpExtrasRunOnMain(^{
        if (!gStatusItemHost || !gStatusItemHost->_item) {
            return;
        }
        NSMenu *menu = [gStatusItemHost->_item menu];
        if ([menu numberOfItems] > 0) {
            [[menu itemAtIndex:0] setTitle:title];
        }
    });
}

void twarp_computer_control_status_item_hide(void) {
    TwarpExtrasRunOnMain(^{
        if (gStatusItemHost) {
            [gStatusItemHost release];
            gStatusItemHost = nil;
        }
    });
}

// ---------------------------------------------------------------------------
// App targeting
// ---------------------------------------------------------------------------

static void TwarpExtrasCopyString(char *destination, size_t capacity, NSString *value) {
    if (!destination || capacity == 0) {
        return;
    }
    destination[0] = '\0';
    if (!value) {
        return;
    }
    const char *utf8 = [value UTF8String];
    if (!utf8) {
        return;
    }
    strlcpy(destination, utf8, capacity);
}

bool twarp_computer_control_resolve_app(const char *query, TwarpComputerControlAppInfo *out) {
    if (!query || !out) {
        return false;
    }
    NSString *needle = TwarpExtrasString(query);
    if ([needle length] == 0) {
        return false;
    }

    __block NSRunningApplication *match = nil;
    TwarpExtrasRunOnMainSync(^{
        NSArray<NSRunningApplication *> *apps =
            [[NSWorkspace sharedWorkspace] runningApplications];
        NSRunningApplication *exact = nil;
        NSRunningApplication *partial = nil;
        for (NSRunningApplication *app in apps) {
            if ([app activationPolicy] != NSApplicationActivationPolicyRegular) {
                continue;
            }
            if ([app processIdentifier] == [[NSRunningApplication currentApplication] processIdentifier]) {
                continue;
            }
            NSString *bundleId = [app bundleIdentifier];
            NSString *name = [app localizedName];
            if (bundleId && [bundleId caseInsensitiveCompare:needle] == NSOrderedSame) {
                exact = app;
                break;
            }
            if (name && [name caseInsensitiveCompare:needle] == NSOrderedSame) {
                exact = app;
                break;
            }
            if (!partial && name &&
                [name rangeOfString:needle options:NSCaseInsensitiveSearch].location != NSNotFound) {
                partial = app;
            }
        }
        match = [(exact ?: partial) retain];
    });

    if (!match) {
        return false;
    }
    out->pid = [match processIdentifier];
    TwarpExtrasCopyString(out->name, sizeof(out->name), [match localizedName]);
    TwarpExtrasCopyString(out->bundle_id, sizeof(out->bundle_id), [match bundleIdentifier]);
    [match release];
    return true;
}

// Focused (or first) AX window frame in global top-left-origin points.
static bool TwarpExtrasAXWindowFrame(int32_t pid, NSRect *outFrame) {
    AXUIElementRef app = AXUIElementCreateApplication(pid);
    if (!app) {
        return false;
    }

    CFTypeRef window = NULL;
    AXError error = AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute, &window);
    if (error != kAXErrorSuccess || !window) {
        CFArrayRef windows = NULL;
        error = AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, (CFTypeRef *)&windows);
        if (error == kAXErrorSuccess && windows && CFArrayGetCount(windows) > 0) {
            window = CFRetain(CFArrayGetValueAtIndex(windows, 0));
        }
        if (windows) {
            CFRelease(windows);
        }
    }
    CFRelease(app);
    if (!window) {
        return false;
    }

    bool ok = false;
    CFTypeRef positionValue = NULL;
    CFTypeRef sizeValue = NULL;
    CGPoint position = CGPointZero;
    CGSize size = CGSizeZero;
    if (AXUIElementCopyAttributeValue((AXUIElementRef)window, kAXPositionAttribute, &positionValue) ==
            kAXErrorSuccess &&
        AXUIElementCopyAttributeValue((AXUIElementRef)window, kAXSizeAttribute, &sizeValue) ==
            kAXErrorSuccess &&
        positionValue && sizeValue &&
        AXValueGetValue((AXValueRef)positionValue, kAXValueTypeCGPoint, &position) &&
        AXValueGetValue((AXValueRef)sizeValue, kAXValueTypeCGSize, &size)) {
        *outFrame = NSMakeRect(position.x, position.y, size.width, size.height);
        ok = true;
    }
    if (positionValue) {
        CFRelease(positionValue);
    }
    if (sizeValue) {
        CFRelease(sizeValue);
    }
    CFRelease(window);
    return ok;
}

bool twarp_computer_control_app_window_bounds(
    int32_t pid,
    double *out_x,
    double *out_y,
    double *out_width,
    double *out_height) {
    NSRect frame = NSZeroRect;
    if (!TwarpExtrasAXWindowFrame(pid, &frame)) {
        return false;
    }

    __block CGFloat scale = 1.0;
    TwarpExtrasRunOnMainSync(^{
        NSScreen *screen = [NSScreen mainScreen];
        scale = screen ? [screen backingScaleFactor] : 1.0;
    });
    if (scale <= 0.0) {
        scale = 1.0;
    }

    if (out_x) {
        *out_x = frame.origin.x * scale;
    }
    if (out_y) {
        *out_y = frame.origin.y * scale;
    }
    if (out_width) {
        *out_width = frame.size.width * scale;
    }
    if (out_height) {
        *out_height = frame.size.height * scale;
    }
    return true;
}

bool twarp_computer_control_activate_app(int32_t pid) {
    __block bool activated = false;
    TwarpExtrasRunOnMainSync(^{
        NSRunningApplication *app =
            [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
        if (app) {
            activated = [app activateWithOptions:NSApplicationActivateIgnoringOtherApps];
        }
    });
    return activated;
}

// ---------------------------------------------------------------------------
// Controlled-app badge
// ---------------------------------------------------------------------------

@interface TwarpComputerControlBadgeHost : NSObject {
@public
    NSPanel *_panel;
    NSTimer *_trackTimer;
    int32_t _pid;
    TwarpComputerControlExtrasCallback _callback;
    void *_context;
}
- (void)track:(NSTimer *)timer;
- (void)badgePressed:(id)sender;
- (void)closeBadge;
@end

@implementation TwarpComputerControlBadgeHost

- (void)track:(NSTimer *)timer {
    NSRect frame = NSZeroRect;
    if (!TwarpExtrasAXWindowFrame(_pid, &frame)) {
        [_panel orderOut:nil];
        return;
    }
    // AX frames are top-left-origin points; pin the badge over the target
    // window's top-left corner (the traffic-light area).
    NSScreen *screen = [NSScreen mainScreen];
    CGFloat screenHeight = screen ? NSHeight([screen frame]) : 0.0;
    NSPoint topLeft = NSMakePoint(frame.origin.x + 8.0, screenHeight - frame.origin.y - 6.0);
    NSRect badgeFrame = [_panel frame];
    badgeFrame.origin = NSMakePoint(topLeft.x, topLeft.y - NSHeight(badgeFrame));
    [_panel setFrame:badgeFrame display:YES];
    [_panel orderFrontRegardless];
}

- (void)badgePressed:(id)sender {
    if (_callback) {
        _callback(_context);
    }
}

- (void)closeBadge {
    if (_trackTimer) {
        [_trackTimer invalidate];
        _trackTimer = nil;
    }
    if (_panel) {
        [_panel orderOut:nil];
        [_panel close];
        [_panel release];
        _panel = nil;
    }
}

- (void)dealloc {
    [self closeBadge];
    [super dealloc];
}

@end

static TwarpComputerControlBadgeHost *gBadgeHost = nil;

void twarp_computer_control_badge_show(
    int32_t pid,
    const char *label,
    TwarpComputerControlColor accent_color,
    TwarpComputerControlExtrasCallback callback,
    void *context) {
    NSString *labelValue = TwarpExtrasString(label);
    TwarpExtrasRunOnMain(^{
        twarp_computer_control_badge_hide();

        TwarpComputerControlBadgeHost *host = [[TwarpComputerControlBadgeHost alloc] init];
        host->_pid = pid;
        host->_callback = callback;
        host->_context = context;

        NSRect frame = NSMakeRect(0.0, 0.0, 132.0, 24.0);
        NSPanel *panel = [[NSPanel alloc] initWithContentRect:frame
                                                    styleMask:NSWindowStyleMaskBorderless |
                                                              NSWindowStyleMaskNonactivatingPanel
                                                      backing:NSBackingStoreBuffered
                                                        defer:NO];
        [panel setOpaque:NO];
        [panel setBackgroundColor:[NSColor clearColor]];
        [panel setHasShadow:YES];
        [panel setHidesOnDeactivate:NO];
        [panel setCanHide:NO];
        [panel setReleasedWhenClosed:NO];
        [panel setMovable:NO];
        [panel setCollectionBehavior:
            NSWindowCollectionBehaviorCanJoinAllSpaces |
            NSWindowCollectionBehaviorFullScreenAuxiliary |
            NSWindowCollectionBehaviorIgnoresCycle];
        [panel setLevel:TwarpExtrasBadgeWindowLevel];
        [panel setSharingType:TwarpExtrasWindowSharingNone];
        host->_panel = panel;

        NSView *content = [[[NSView alloc] initWithFrame:frame] autorelease];
        [content setWantsLayer:YES];
        [content layer].cornerRadius = 12.0;
        [content layer].masksToBounds = YES;
        [content layer].backgroundColor = [TwarpExtrasColor(accent_color) CGColor];
        [panel setContentView:content];

        NSButton *button = [NSButton buttonWithTitle:labelValue
                                              target:host
                                              action:@selector(badgePressed:)];
        [button setFrame:NSMakeRect(0.0, 0.0, frame.size.width, frame.size.height)];
        [button setBordered:NO];
        [button setFont:[NSFont systemFontOfSize:11.0 weight:NSFontWeightSemibold]];
        if ([button respondsToSelector:@selector(setContentTintColor:)]) {
            [button setContentTintColor:[NSColor whiteColor]];
        }
        if (@available(macOS 11.0, *)) {
            NSImage *image = [NSImage imageWithSystemSymbolName:@"rectangle.inset.filled.and.person.filled"
                                       accessibilityDescription:@"Controlled window"];
            if (image) {
                [button setImage:image];
                [button setImagePosition:NSImageLeft];
            }
        }
        [content addSubview:button];

        host->_trackTimer = [NSTimer scheduledTimerWithTimeInterval:0.4
                                                             target:host
                                                           selector:@selector(track:)
                                                           userInfo:nil
                                                            repeats:YES];
        [host track:nil];

        gBadgeHost = host;
    });
}

void twarp_computer_control_badge_hide(void) {
    TwarpExtrasRunOnMain(^{
        if (gBadgeHost) {
            [gBadgeHost closeBadge];
            [gBadgeHost release];
            gBadgeHost = nil;
        }
    });
}
