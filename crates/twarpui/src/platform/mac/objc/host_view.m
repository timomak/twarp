#import "host_view.h"

#import <LocalAuthentication/LocalAuthentication.h>
#import <Metal/Metal.h>
#import <WebKit/WebKit.h>
#import <stdint.h>

void warp_view_did_change_backing_properties(WarpHostView *, BOOL);
void warp_view_set_frame_size(WarpHostView *, NSSize, BOOL);
void warp_update_layer(WarpHostView *);
BOOL warp_handle_view_event(WarpHostView *, NSEvent *, BOOL);
BOOL warp_handle_first_mouse_event(WarpHostView *, NSEvent *);
void warp_handle_insert_text(WarpHostView *, id);
void warp_update_ime_state(WarpHostView *, BOOL);
void warp_handle_drag_and_drop(WarpHostView *, NSArray *, NSPoint);
void warp_handle_file_drag(WarpHostView *, NSPoint);
void warp_handle_file_drag_exit(WarpHostView *);
NSRect warp_ime_position(WarpHostView *, NSRect *);
id warp_get_accessibility_contents(WarpHostView *);
void warp_marked_text_updated(WarpHostView *, NSString *, NSRange);
void warp_marked_text_cleared(WarpHostView *);

typedef void (*WarpBrowserStringCallback)(void *, const char *, const char *);
typedef void (*WarpBrowserBytesCallback)(void *, const uint8_t *, uintptr_t, const char *);

// Rust handler for WebAuthn (passkey) requests bridged out of a browser
// webview. `userVerified` reports whether the local-auth (Touch ID) ceremony
// succeeded; the handler answers the page via evaluateJavaScript.
void warp_browser_webauthn_request(WarpHostView *, NSUInteger webViewId, const char *requestJSON,
                                   BOOL userVerified);

@class WarpNativeWebViewEntry;

@interface WarpAutomationScriptMessageHandler : NSObject <WKScriptMessageHandler>
@property(nonatomic, assign) WarpNativeWebViewEntry *entry;
- (instancetype)initWithEntry:(WarpNativeWebViewEntry *)entry;
@end

@class WarpNativeWebViewEntry;

/// twarp 14l: transparent view layered over a webview while an agent holds
/// the input lease. Swallows all mouse/scroll/key input so the user can't
/// interact with the page mid-automation (the pane chrome stays interactive —
/// this only covers the webview rect). In annotation-capture mode the
/// swallowed click's page coordinates are recorded instead of discarded.
@interface WarpWebViewInputShield : NSView
@property(nonatomic) BOOL captureClicks;
// The owning entry outlives the shield (it removes the shield before dying).
@property(nonatomic, assign) WarpNativeWebViewEntry *entry;
@end

@interface WarpNativeWebViewEntry : NSObject <WKNavigationDelegate, WKUIDelegate>
@property(nonatomic, retain) NSView *containerView;
@property(nonatomic, retain) WKWebView *webView;
@property(nonatomic, retain) WKUserContentController *userContentController;
@property(nonatomic, retain) WarpAutomationScriptMessageHandler *messageHandler;
@property(nonatomic, retain) NSMutableArray<NSString *> *consoleMessages;
@property(nonatomic, retain) NSMutableArray<NSString *> *networkMessages;
@property(nonatomic, retain) WarpWebViewInputShield *inputShieldView;
@property(nonatomic) BOOL hiddenRequested;
// twarp 14l: shield ownership flags — the shield exists while either is set.
@property(nonatomic) BOOL inputBlockRequested;
@property(nonatomic) BOOL annotationCaptureRequested;
// twarp 14l-2: the last annotation click (webview top-left coords), if any.
@property(nonatomic, retain) NSValue *pendingAnnotationClick;
// WebAuthn bridge backrefs — the host view owns the entry, so assign is safe.
@property(nonatomic, assign) WarpHostView *hostView;
@property(nonatomic) NSUInteger webViewId;
- (instancetype)initWithContainerView:(NSView *)containerView
                              webView:(WKWebView *)webView
                userContentController:(WKUserContentController *)userContentController;
- (void)appendAutomationMessage:(id)message;
- (void)clearAutomationMessages;
- (void)recordAnnotationClickAt:(NSPoint)webViewPoint;
- (NSString *)consoleMessagesJSON;
- (NSString *)networkMessagesJSON;
- (void)downloadRequest:(NSURLRequest *)request response:(NSURLResponse *)response;
@end

/// A `window.open()` popup spawned by a browser webview (OAuth/sign-in flows).
/// The child webview MUST be built from the configuration WebKit hands to
/// `createWebViewWithConfiguration:` — that is what wires `window.opener`,
/// `postMessage` back to the opener, and `window.close()`; returning nil there
/// makes sites report "your browser is blocking popups".
@interface WarpBrowserPopup : NSObject <WKNavigationDelegate, WKUIDelegate, NSWindowDelegate>
@property(nonatomic, retain) NSWindow *window;
@property(nonatomic, retain) WKWebView *webView;
+ (WKWebView *)openPopupFromWebView:(WKWebView *)parentWebView
                      configuration:(WKWebViewConfiguration *)configuration
                forNavigationAction:(WKNavigationAction *)navigationAction
                     windowFeatures:(WKWindowFeatures *)windowFeatures;
@end

/// Shield implementation lives after the entry interface so it can call back
/// into the entry when capturing annotation clicks.
@implementation WarpWebViewInputShield
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    return YES;
}
- (void)mouseDown:(NSEvent *)event {
    if (self.captureClicks && self.entry) {
        // WKWebView is flipped, so converting into it yields top-left-origin
        // coordinates matching CSS's elementFromPoint space.
        NSPoint point = [self.entry.webView convertPoint:event.locationInWindow fromView:nil];
        [self.entry recordAnnotationClickAt:point];
    }
}
- (void)mouseUp:(NSEvent *)event {
}
- (void)mouseDragged:(NSEvent *)event {
}
- (void)mouseMoved:(NSEvent *)event {
}
- (void)rightMouseDown:(NSEvent *)event {
}
- (void)rightMouseUp:(NSEvent *)event {
}
- (void)otherMouseDown:(NSEvent *)event {
}
- (void)otherMouseUp:(NSEvent *)event {
}
- (void)scrollWheel:(NSEvent *)event {
}
- (void)keyDown:(NSEvent *)event {
}
- (void)keyUp:(NSEvent *)event {
}
@end

static const NSUInteger WarpAutomationMessageLimit = 200;

// JS dialog panels shared by pane webviews and their popup children.
static void WarpRunJSAlert(WKWebView *webView, NSString *message, void (^completionHandler)(void)) {
    NSAlert *alert = [[[NSAlert alloc] init] autorelease];
    alert.messageText = webView.title.length > 0 ? webView.title : @"Browser";
    alert.informativeText = message ?: @"";
    [alert addButtonWithTitle:@"OK"];
    [alert runModal];
    completionHandler();
}

static void WarpRunJSConfirm(WKWebView *webView, NSString *message, void (^completionHandler)(BOOL)) {
    NSAlert *alert = [[[NSAlert alloc] init] autorelease];
    alert.messageText = webView.title.length > 0 ? webView.title : @"Browser";
    alert.informativeText = message ?: @"";
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    completionHandler([alert runModal] == NSAlertFirstButtonReturn);
}

static void WarpRunJSPrompt(WKWebView *webView,
                            NSString *prompt,
                            NSString *defaultText,
                            void (^completionHandler)(NSString *)) {
    NSAlert *alert = [[[NSAlert alloc] init] autorelease];
    alert.messageText = webView.title.length > 0 ? webView.title : @"Browser";
    alert.informativeText = prompt ?: @"";
    NSTextField *input = [[[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 280, 24)] autorelease];
    input.stringValue = defaultText ?: @"";
    alert.accessoryView = input;
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    completionHandler([alert runModal] == NSAlertFirstButtonReturn ? input.stringValue : nil);
}

// Live popups — the set is the strong owner; each popup removes itself when
// its window closes.
static NSMutableSet<WarpBrowserPopup *> *warpBrowserPopups(void) {
    static NSMutableSet *popups = nil;
    if (!popups) {
        popups = [[NSMutableSet alloc] init];
    }
    return popups;
}

@implementation WarpBrowserPopup

+ (WKWebView *)openPopupFromWebView:(WKWebView *)parentWebView
                      configuration:(WKWebViewConfiguration *)configuration
                forNavigationAction:(WKNavigationAction *)navigationAction
                     windowFeatures:(WKWindowFeatures *)windowFeatures {
    (void)navigationAction;
    CGFloat width = windowFeatures.width ? windowFeatures.width.doubleValue : 560.0;
    CGFloat height = windowFeatures.height ? windowFeatures.height.doubleValue : 680.0;
    width = MIN(MAX(width, 320.0), 1200.0);
    height = MIN(MAX(height, 240.0), 1000.0);

    // WebKit requires the child to be created with exactly this configuration.
    WKWebView *webView =
        [[[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, width, height)
                            configuration:configuration] autorelease];

    NSWindow *window = [[[NSWindow alloc]
        initWithContentRect:NSMakeRect(0, 0, width, height)
                  styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                            NSWindowStyleMaskResizable
                    backing:NSBackingStoreBuffered
                      defer:NO] autorelease];
    window.releasedWhenClosed = NO;
    window.title = @"Loading…";
    webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [window.contentView addSubview:webView];

    NSWindow *parentWindow = parentWebView.window;
    if (parentWindow) {
        NSRect parentFrame = parentWindow.frame;
        [window setFrameOrigin:NSMakePoint(NSMidX(parentFrame) - width / 2,
                                           NSMidY(parentFrame) - height / 2)];
    } else {
        [window center];
    }

    WarpBrowserPopup *popup = [[[WarpBrowserPopup alloc] init] autorelease];
    popup.window = window;
    popup.webView = webView;
    window.delegate = popup;
    webView.UIDelegate = popup;
    webView.navigationDelegate = popup;
    [webView addObserver:popup forKeyPath:@"title" options:0 context:NULL];
    [warpBrowserPopups() addObject:popup];

    // Attach as a child of the twarp window rather than opening an independent
    // top-level window. A standalone window makes AppKit's application/document
    // termination machinery treat it as an app window: when it opens/closes
    // while twarp is active, a quit is dispatched through
    // NSDocumentController and the app terminates (looks like a crash on
    // popups). A child window floats above its parent, moves/closes with it,
    // and stays out of that machinery.
    if (parentWindow) {
        [parentWindow addChildWindow:window ordered:NSWindowAbove];
    }
    [window makeKeyAndOrderFront:nil];
    return webView;
}

- (void)observeValueForKeyPath:(NSString *)keyPath
                      ofObject:(id)object
                        change:(NSDictionary *)change
                       context:(void *)context {
    if ([keyPath isEqualToString:@"title"] && object == self.webView) {
        self.window.title = self.webView.title.length > 0 ? self.webView.title : @"Browser";
    }
}

// The page called window.close() — OAuth popups do this after handing the
// result back to the opener.
- (void)webViewDidClose:(WKWebView *)webView {
    [self.window close];
}

- (void)windowWillClose:(NSNotification *)notification {
    [self.webView removeObserver:self forKeyPath:@"title"];
    self.webView.UIDelegate = nil;
    self.webView.navigationDelegate = nil;
    self.window.delegate = nil;
    [[self retain] autorelease];
    [warpBrowserPopups() removeObject:self];
}

- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)navigationAction
                    windowFeatures:(WKWindowFeatures *)windowFeatures {
    return [WarpBrowserPopup openPopupFromWebView:webView
                                    configuration:configuration
                              forNavigationAction:navigationAction
                                   windowFeatures:windowFeatures];
}

- (void)webView:(WKWebView *)webView
    runJavaScriptAlertPanelWithMessage:(NSString *)message
                      initiatedByFrame:(WKFrameInfo *)frame
                     completionHandler:(void (^)(void))completionHandler {
    WarpRunJSAlert(webView, message, completionHandler);
}

- (void)webView:(WKWebView *)webView
    runJavaScriptConfirmPanelWithMessage:(NSString *)message
                        initiatedByFrame:(WKFrameInfo *)frame
                       completionHandler:(void (^)(BOOL result))completionHandler {
    WarpRunJSConfirm(webView, message, completionHandler);
}

- (void)webView:(WKWebView *)webView
    runJavaScriptTextInputPanelWithPrompt:(NSString *)prompt
                              defaultText:(NSString *)defaultText
                         initiatedByFrame:(WKFrameInfo *)frame
                        completionHandler:(void (^)(NSString *result))completionHandler {
    WarpRunJSPrompt(webView, prompt, defaultText, completionHandler);
}

- (void)dealloc {
    [_webView release];
    [_window release];
    [super dealloc];
}

@end

@implementation WarpNativeWebViewEntry

- (instancetype)initWithContainerView:(NSView *)containerView
                              webView:(WKWebView *)webView
                userContentController:(WKUserContentController *)userContentController {
    self = [super init];
    if (self) {
        _containerView = [containerView retain];
        _webView = [webView retain];
        _userContentController = [userContentController retain];
        _consoleMessages = [[NSMutableArray alloc] init];
        _networkMessages = [[NSMutableArray alloc] init];
        _hiddenRequested = YES;
    }
    return self;
}

- (void)recordAnnotationClickAt:(NSPoint)webViewPoint {
    self.pendingAnnotationClick = [NSValue valueWithPoint:webViewPoint];
}

- (void)dealloc {
    [_userContentController removeScriptMessageHandlerForName:@"twarpAutomation"];
    _webView.navigationDelegate = nil;
    _webView.UIDelegate = nil;
    _inputShieldView.entry = nil;
    [_pendingAnnotationClick release];
    [_inputShieldView release];
    [_messageHandler release];
    [_networkMessages release];
    [_consoleMessages release];
    [_userContentController release];
    [_webView release];
    [_containerView release];
    [super dealloc];
}

- (void)appendJSONString:(NSString *)jsonString toMessages:(NSMutableArray<NSString *> *)messages {
    if (!jsonString) return;
    [messages addObject:jsonString];
    while (messages.count > WarpAutomationMessageLimit) {
        [messages removeObjectAtIndex:0];
    }
}

- (void)appendAutomationMessage:(id)message {
    if (!message || ![NSJSONSerialization isValidJSONObject:message]) return;

    NSError *error = nil;
    NSData *jsonData = [NSJSONSerialization dataWithJSONObject:message options:0 error:&error];
    if (!jsonData || error) return;

    NSString *jsonString = [[[NSString alloc] initWithData:jsonData encoding:NSUTF8StringEncoding] autorelease];
    NSString *type = [message isKindOfClass:[NSDictionary class]] ? [(NSDictionary *)message objectForKey:@"type"] : nil;
    if ([type isEqualToString:@"network"]) {
        [self appendJSONString:jsonString toMessages:_networkMessages];
    } else if ([type isEqualToString:@"console"]) {
        [self appendJSONString:jsonString toMessages:_consoleMessages];
    } else if ([type isEqualToString:@"webauthn"]) {
        [self handleWebAuthnMessage:(NSDictionary *)message json:jsonString];
    }
}

/// WebAuthn (passkey) request from the injected page script. User
/// verification happens here — Touch ID with password fallback — so the Rust
/// authenticator core only has to do crypto and storage. The request is
/// forwarded either way; Rust rejects unverified requests with
/// NotAllowedError so the page gets a well-formed WebAuthn failure.
- (void)handleWebAuthnMessage:(NSDictionary *)message json:(NSString *)jsonString {
    NSString *origin = [[message objectForKey:@"origin"] isKindOfClass:[NSString class]]
        ? [message objectForKey:@"origin"]
        : @"this site";
    NSString *kind = [[message objectForKey:@"kind"] isKindOfClass:[NSString class]]
        ? [message objectForKey:@"kind"]
        : @"get";
    NSString *reason = [kind isEqualToString:@"create"]
        ? [NSString stringWithFormat:@"create a passkey for %@", origin]
        : [NSString stringWithFormat:@"sign in to %@ with a passkey", origin];

    WarpHostView *hostView = self.hostView;
    NSUInteger webViewId = self.webViewId;
    LAContext *context = [[[LAContext alloc] init] autorelease];
    [context evaluatePolicy:LAPolicyDeviceOwnerAuthentication
            localizedReason:reason
                      reply:^(BOOL success, NSError *authError) {
                          (void)authError;
                          dispatch_async(dispatch_get_main_queue(), ^{
                              warp_browser_webauthn_request(hostView, webViewId,
                                                            jsonString.UTF8String, success);
                          });
                      }];
}

- (void)clearAutomationMessages {
    [_consoleMessages removeAllObjects];
    [_networkMessages removeAllObjects];
}

- (NSString *)jsonArrayForMessages:(NSArray<NSString *> *)messages {
    if (messages.count == 0) return @"[]";
    return [NSString stringWithFormat:@"[%@]", [messages componentsJoinedByString:@","]];
}

- (NSString *)consoleMessagesJSON {
    return [self jsonArrayForMessages:_consoleMessages];
}

- (NSString *)networkMessagesJSON {
    return [self jsonArrayForMessages:_networkMessages];
}

- (void)webView:(WKWebView *)webView didStartProvisionalNavigation:(WKNavigation *)navigation {
    [self clearAutomationMessages];
}

- (BOOL)responseShouldDownload:(WKNavigationResponse *)navigationResponse {
    if (!navigationResponse.canShowMIMEType) return YES;

    NSURLResponse *response = navigationResponse.response;
    if (![response isKindOfClass:[NSHTTPURLResponse class]]) return NO;

    NSDictionary *headers = [(NSHTTPURLResponse *)response allHeaderFields];
    for (id key in headers) {
        if ([[key description] caseInsensitiveCompare:@"Content-Disposition"] == NSOrderedSame) {
            NSString *value = [[headers objectForKey:key] description];
            return [value rangeOfString:@"attachment" options:NSCaseInsensitiveSearch].location != NSNotFound;
        }
    }
    return NO;
}

- (NSURL *)uniqueDownloadURLForFilename:(NSString *)filename {
    NSURL *downloadsDirectory =
        [[[NSFileManager defaultManager] URLsForDirectory:NSDownloadsDirectory
                                                inDomains:NSUserDomainMask] firstObject];
    if (!downloadsDirectory) {
        downloadsDirectory = [NSURL fileURLWithPath:NSTemporaryDirectory() isDirectory:YES];
    }

    NSString *safeFilename = filename.lastPathComponent.length > 0 ? filename.lastPathComponent : @"download";
    NSString *baseName = [safeFilename stringByDeletingPathExtension];
    NSString *extension = [safeFilename pathExtension];
    NSURL *candidate = [downloadsDirectory URLByAppendingPathComponent:safeFilename];
    NSUInteger suffix = 2;
    while ([[NSFileManager defaultManager] fileExistsAtPath:candidate.path]) {
        NSString *name = extension.length > 0
            ? [NSString stringWithFormat:@"%@-%lu.%@", baseName, (unsigned long)suffix, extension]
            : [NSString stringWithFormat:@"%@-%lu", baseName, (unsigned long)suffix];
        candidate = [downloadsDirectory URLByAppendingPathComponent:name];
        suffix++;
    }
    return candidate;
}

- (void)downloadRequest:(NSURLRequest *)request response:(NSURLResponse *)response {
    if (!request.URL) return;

    NSString *filename = response.suggestedFilename;
    if (filename.length == 0) {
        filename = request.URL.lastPathComponent.length > 0 ? request.URL.lastPathComponent : @"download";
    }
    NSURL *destinationURL = [self uniqueDownloadURLForFilename:filename];

    NSURLSessionDownloadTask *task =
        [[NSURLSession sharedSession] downloadTaskWithRequest:request
                                            completionHandler:^(NSURL *location,
                                                                NSURLResponse *downloadResponse,
                                                                NSError *error) {
                                                (void)downloadResponse;
                                                if (error || !location) return;

                                                NSFileManager *fileManager = [NSFileManager defaultManager];
                                                NSURL *finalURL = [self uniqueDownloadURLForFilename:destinationURL.lastPathComponent];
                                                [fileManager moveItemAtURL:location toURL:finalURL error:nil];
                                            }];
    [task resume];
}

- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationAction:(WKNavigationAction *)navigationAction
                    decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    if (!navigationAction.targetFrame && navigationAction.request.URL) {
        [webView loadRequest:navigationAction.request];
        decisionHandler(WKNavigationActionPolicyCancel);
        return;
    }
    decisionHandler(WKNavigationActionPolicyAllow);
}

- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationResponse:(WKNavigationResponse *)navigationResponse
                      decisionHandler:(void (^)(WKNavigationResponsePolicy))decisionHandler {
    if ([self responseShouldDownload:navigationResponse]) {
        [self downloadRequest:navigationResponse.response.URL
            ? [NSURLRequest requestWithURL:navigationResponse.response.URL]
            : webView.URL ? [NSURLRequest requestWithURL:webView.URL] : nil
                     response:navigationResponse.response];
        decisionHandler(WKNavigationResponsePolicyCancel);
        return;
    }
    decisionHandler(WKNavigationResponsePolicyAllow);
}

- (void)webView:(WKWebView *)webView
    runJavaScriptAlertPanelWithMessage:(NSString *)message
                      initiatedByFrame:(WKFrameInfo *)frame
                     completionHandler:(void (^)(void))completionHandler {
    WarpRunJSAlert(webView, message, completionHandler);
}

- (void)webView:(WKWebView *)webView
    runJavaScriptConfirmPanelWithMessage:(NSString *)message
                        initiatedByFrame:(WKFrameInfo *)frame
                       completionHandler:(void (^)(BOOL result))completionHandler {
    WarpRunJSConfirm(webView, message, completionHandler);
}

- (void)webView:(WKWebView *)webView
    runJavaScriptTextInputPanelWithPrompt:(NSString *)prompt
                              defaultText:(NSString *)defaultText
                         initiatedByFrame:(WKFrameInfo *)frame
                        completionHandler:(void (^)(NSString *result))completionHandler {
    WarpRunJSPrompt(webView, prompt, defaultText, completionHandler);
}

- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)navigationAction
                    windowFeatures:(WKWindowFeatures *)windowFeatures {
    return [WarpBrowserPopup openPopupFromWebView:webView
                                    configuration:configuration
                              forNavigationAction:navigationAction
                                   windowFeatures:windowFeatures];
}

@end

@implementation WarpAutomationScriptMessageHandler

- (instancetype)initWithEntry:(WarpNativeWebViewEntry *)entry {
    self = [super init];
    if (self) {
        _entry = entry;
    }
    return self;
}

- (void)userContentController:(WKUserContentController *)userContentController
      didReceiveScriptMessage:(WKScriptMessage *)message {
    [_entry appendAutomationMessage:message.body];
}

- (void)dealloc {
    [super dealloc];
}

@end

@implementation NSPasteboard (Warp)

- (NSArray *)getFilePaths {
    NSMutableArray *paths = [NSMutableArray array];
    NSArray<NSURL *> *urls = [self readObjectsForClasses:@[ [NSURL class] ] options:0];
    for (NSURL *url in urls) {
        NSString *path = url.path;
        if (path) {
            [paths addObject:path];
        }
    }
    return paths;
}

@end

@implementation WarpHostView {
    // The windowState is managed on the Rust side.
    // Note Rust expects this name even though we are not a window.
    void *windowState;

    // Whether we start a window drag on an unhandled mouseDown event inside the title bar
    BOOL titlebarDragEnabled;

    // Whether we are in test mode, which suppresses drawing.
    BOOL testMode;

    // The metal device for our layer.
    id metalDevice;

    NSMutableAttributedString *markedText;
    NSMutableString *textToInsert;

    // Whether to have resize event callback called asynchronously.
    BOOL asyncCallback;

    // Whether we're in the middle of a call to interpretKeyEvents.
    BOOL interpretingKeyEvents;

    // Whether the IME modified marked text (via setMarkedText: or unmarkText)
    // during the current interpretKeyEvents: pass. Used to avoid wiping a
    // freshly-set marked text in the split-commit scenario where an IME
    // calls insertText: (committing some text) and then setMarkedText: (with
    // new in-progress text) in the same keystroke. Without this, the trailing
    // unmarkText in keyDownImpl would clobber that new marked text.
    BOOL imeTouchedMarkedTextDuringInterpret;

    NSMutableDictionary<NSNumber *, WarpNativeWebViewEntry *> *nativeWebViews;
    NSUInteger nextNativeWebViewId;
    BOOL nativeWebViewsOccluded;
}

- (BOOL)acceptsFirstResponder {
    return YES;
}

- (BOOL)mouseDownCanMoveWindow {
    return !titlebarDragEnabled;
}

- (BOOL)readyForWarp {
    return windowState != NULL;
}

/// Returns the height of the titlebar.
- (CGFloat)titlebarHeight {
    NSButton *closeButton = [self.window standardWindowButton:NSWindowCloseButton];
    NSView *titlebar = [closeButton superview];
    return titlebar.frame.size.height;
}

- (BOOL)mouseInTitleBar:(NSEvent *)event {
    NSPoint windowLoc = [self convertPoint:event.locationInWindow fromView:nil];
    // windowLoc.y is the distance from the bottom of the window to the cursor
    // NSHeight(window.frame) will be the height of the whole window, so
    // NSHeight - titlebarHeight will be the bottom border of the titlebar
    return NSHeight(self.window.frame) - [self titlebarHeight] <= windowLoc.y;
}

// See if the user double clicked in the titlebar. If so, do whatever
// action is given by preferences.
// \return true if handled, false otherwise.
- (BOOL)handleTitleBarDoubleClick:(NSEvent *)event {
    NSWindow *window = self.window;
    NSWindowStyleMask styleMask = window.styleMask;
    // Was this a double click in a full-sized content view, not in full screen?
    if (event.clickCount != 2) return NO;
    if (!(styleMask & NSWindowStyleMaskFullSizeContentView)) return NO;
    if (styleMask & NSWindowStyleMaskFullScreen) return NO;

    // See if our point is in the titlebar of the window.
    if (![self mouseInTitleBar:event]) return NO;

    // Ok, do the action.
    NSString *action =
        [[NSUserDefaults standardUserDefaults] objectForKey:@"AppleActionOnDoubleClick"];

    // When user has not explicitly ticked or unticked the `Double-click the window's
    // title bar to` option in system preferences, the NSUserDefaults will not have the key
    // "AppleActionOnDoubleClick", despite in system preferences the default is to "Zoom".
    // To make the behavior consistent, when the key is nil, we set performZoom as the
    // default behavior here.
    if ([action isEqualToString:@"Minimize"]) {
        [window performMiniaturize:nil];
        return YES;
    } else if (action == nil || [action isEqualToString:@"Maximize"]) {
        [window performZoom:nil];
        return YES;
    }
    return NO;
}

- (void)viewDidChangeBackingProperties {
    if (self.readyForWarp) warp_view_did_change_backing_properties(self, asyncCallback);
    [super viewDidChangeBackingProperties];
}

- (void)setFrameSize:(NSSize)size {
    BOOL changed = !NSEqualSizes(size, self.frame.size);
    // We could receive invalid frame sizes when the window is moved offscreen.
    // Validate the size against the minimum drawable size of the window before
    // passing to the rust side.
    if (size.height >= self.window.minSize.height && size.width >= self.window.minSize.width) {
        [super setFrameSize:size];
        // It's an important optimization to only invoke this if the size changed.
        if (self.readyForWarp && changed) {
            warp_view_set_frame_size(self, size, asyncCallback);
        }
    }
}

- (void)displayLayer:(CALayer *)layer {
    if (!testMode && self.readyForWarp) {
        warp_update_layer(self);
    }
}

- (void)setAsyncCallback:(BOOL)shouldAsync {
    asyncCallback = shouldAsync;
}
- (void)setPresentsWithTransaction:(BOOL)presentsWithTransaction {
    CAMetalLayer *layer = (CAMetalLayer *)self.layer;
    layer.presentsWithTransaction = presentsWithTransaction;
}

- (WarpNativeWebViewEntry *)nativeWebViewEntry:(NSUInteger)webViewId {
    return [nativeWebViews objectForKey:@(webViewId)];
}

- (void)applyHiddenStateToNativeWebViewEntry:(WarpNativeWebViewEntry *)entry {
    BOOL hidden = entry.hiddenRequested || nativeWebViewsOccluded;
    [entry.containerView setHidden:hidden];
    [entry.webView setHidden:hidden];
    if (!hidden) {
        [entry.containerView setNeedsLayout:YES];
        [entry.webView setNeedsLayout:YES];
        [entry.containerView setNeedsDisplay:YES];
        [entry.webView setNeedsDisplay:YES];
    }
}

- (void)setNativeWebViewsOccluded:(BOOL)occluded {
    nativeWebViewsOccluded = occluded;
    for (WarpNativeWebViewEntry *entry in [nativeWebViews allValues]) {
        [self applyHiddenStateToNativeWebViewEntry:entry];
    }
    if (occluded) {
        [self.window makeFirstResponder:self];
    }
}

- (void)prepareNativeWebViewsForFrame {
    for (WarpNativeWebViewEntry *entry in [nativeWebViews allValues]) {
        entry.hiddenRequested = YES;
        [self applyHiddenStateToNativeWebViewEntry:entry];
    }
}

- (NSUInteger)createNativeWebViewWithPersistentDataStore:(BOOL)persistentDataStore {
    WKWebViewConfiguration *configuration = [[[WKWebViewConfiguration alloc] init] autorelease];
    WKUserContentController *userContentController =
        [[[WKUserContentController alloc] init] autorelease];
    configuration.userContentController = userContentController;
    configuration.websiteDataStore = persistentDataStore
        ? [WKWebsiteDataStore defaultDataStore]
        : [WKWebsiteDataStore nonPersistentDataStore];
    // Let sign-in flows open popups even when the click's user-activation
    // doesn't propagate to the window.open() call (some OAuth SDKs open the
    // window from an async callback). createWebViewWithConfiguration: still
    // hosts them in a real popup window.
    configuration.preferences.javaScriptCanOpenWindowsAutomatically = YES;
    WKWebView *webView = [[[WKWebView alloc] initWithFrame:NSZeroRect
                                             configuration:configuration] autorelease];
    webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    webView.wantsLayer = YES;
    webView.layer.opaque = YES;
    webView.layer.zPosition = 1.0;

    NSView *containerView = [[[NSView alloc] initWithFrame:NSZeroRect] autorelease];
    containerView.autoresizingMask = NSViewMinYMargin;
    containerView.wantsLayer = YES;
    containerView.canDrawSubviewsIntoLayer = NO;
    containerView.layer.masksToBounds = YES;
    containerView.layer.opaque = NO;
    containerView.layer.zPosition = 1.0;
    [containerView addSubview:webView];

    WarpNativeWebViewEntry *entry = [[[WarpNativeWebViewEntry alloc]
        initWithContainerView:containerView
                      webView:webView
        userContentController:userContentController] autorelease];
    WarpAutomationScriptMessageHandler *messageHandler =
        [[[WarpAutomationScriptMessageHandler alloc] initWithEntry:entry] autorelease];
    entry.messageHandler = messageHandler;
    [userContentController addScriptMessageHandler:messageHandler name:@"twarpAutomation"];
    webView.navigationDelegate = entry;
    webView.UIDelegate = entry;

    NSUInteger webViewId = nextNativeWebViewId++;
    entry.hostView = self;
    entry.webViewId = webViewId;
    [nativeWebViews setObject:entry forKey:@(webViewId)];
    [self addSubview:containerView positioned:NSWindowAbove relativeTo:nil];
    [self applyHiddenStateToNativeWebViewEntry:entry];
    return webViewId;
}

- (void)clearNativeBrowserWebsiteData {
    NSSet *dataTypes = [WKWebsiteDataStore allWebsiteDataTypes];
    [[WKWebsiteDataStore defaultDataStore] removeDataOfTypes:dataTypes
                                               modifiedSince:[NSDate distantPast]
                                           completionHandler:^{}];
    for (WarpNativeWebViewEntry *entry in [nativeWebViews allValues]) {
        [entry clearAutomationMessages];
    }
}

- (void)installNativeWebViewAutomationScript:(NSUInteger)webViewId source:(NSString *)source {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry || !source) return;

    WKUserScript *userScript =
        [[[WKUserScript alloc] initWithSource:source
                                injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                             forMainFrameOnly:NO] autorelease];
    [entry.userContentController addUserScript:userScript];
}

- (void)setNativeWebViewFrame:(NSUInteger)webViewId frame:(NSRect)frame {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;

    [entry.containerView setFrame:frame];
    [entry.webView setFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
    [entry.containerView setNeedsLayout:YES];
    [entry.webView setNeedsLayout:YES];
    [entry.containerView setNeedsDisplay:YES];
    [entry.webView setNeedsDisplay:YES];
}

/// twarp 14l: block or unblock direct user input to the webview while an
/// agent holds the input lease. The shield only covers the page rect; pane
/// chrome (tabs/omnibar) stays interactive.
- (void)setNativeWebViewInputBlocked:(NSUInteger)webViewId blocked:(BOOL)blocked {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;
    entry.inputBlockRequested = blocked;
    [self applyShieldStateToNativeWebViewEntry:entry];
}

/// twarp 14l-2: arm/disarm annotation-click capture. While armed the shield
/// records the next page click's coordinates instead of delivering it.
- (void)setNativeWebViewAnnotationCapture:(NSUInteger)webViewId enabled:(BOOL)enabled {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;
    entry.annotationCaptureRequested = enabled;
    if (!enabled) {
        entry.pendingAnnotationClick = nil;
    }
    [self applyShieldStateToNativeWebViewEntry:entry];
}

/// twarp 14l-2: pops the recorded annotation click, if any.
- (BOOL)takeNativeWebViewAnnotationClick:(NSUInteger)webViewId x:(double *)x y:(double *)y {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry || !entry.pendingAnnotationClick) return NO;
    NSPoint point = [entry.pendingAnnotationClick pointValue];
    entry.pendingAnnotationClick = nil;
    if (x) *x = point.x;
    if (y) *y = point.y;
    return YES;
}

- (void)applyShieldStateToNativeWebViewEntry:(WarpNativeWebViewEntry *)entry {
    BOOL wantShield = entry.inputBlockRequested || entry.annotationCaptureRequested;
    if (wantShield) {
        if (!entry.inputShieldView) {
            WarpWebViewInputShield *shield =
                [[[WarpWebViewInputShield alloc] initWithFrame:entry.containerView.bounds] autorelease];
            shield.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
            [entry.containerView addSubview:shield positioned:NSWindowAbove relativeTo:entry.webView];
            entry.inputShieldView = shield;
        }
        entry.inputShieldView.entry = entry;
        entry.inputShieldView.captureClicks = entry.annotationCaptureRequested;
        // If the user was mid-interaction with the page, take the keyboard
        // back so their keystrokes don't keep flowing into it.
        if ([self responder:self.window.firstResponder isInWebView:entry.webView]) {
            [self.window makeFirstResponder:self];
        }
    } else if (entry.inputShieldView) {
        entry.inputShieldView.entry = nil;
        [entry.inputShieldView removeFromSuperview];
        entry.inputShieldView = nil;
    }
}

- (void)setNativeWebViewHidden:(NSUInteger)webViewId hidden:(BOOL)hidden {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;

    entry.hiddenRequested = hidden;
    [self applyHiddenStateToNativeWebViewEntry:entry];
}

- (void)loadNativeWebView:(NSUInteger)webViewId urlString:(NSString *)urlString {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;

    NSURL *url = [NSURL URLWithString:urlString];
    if (!url) return;

    NSURLRequest *request = [NSURLRequest requestWithURL:url];
    [entry.webView loadRequest:request];
}

- (void)goBackNativeWebView:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry || !entry.webView.canGoBack) return;

    [entry.webView goBack];
}

- (void)goForwardNativeWebView:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry || !entry.webView.canGoForward) return;

    [entry.webView goForward];
}

- (void)reloadNativeWebView:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;

    [entry.webView reload];
}

- (void)stopLoadingNativeWebView:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;

    [entry.webView stopLoading];
}

- (BOOL)nativeWebViewCanGoBack:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    return entry && entry.webView.canGoBack;
}

- (BOOL)nativeWebViewCanGoForward:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    return entry && entry.webView.canGoForward;
}

- (BOOL)nativeWebViewIsLoading:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    return entry && entry.webView.isLoading;
}

- (BOOL)copyNativeWebViewString:(NSString *)string buffer:(char *)buffer bufferLength:(NSUInteger)bufferLength {
    if (!buffer || bufferLength == 0 || !string) return NO;

    return [string getCString:buffer maxLength:bufferLength encoding:NSUTF8StringEncoding];
}

- (BOOL)copyNativeWebViewURL:(NSUInteger)webViewId buffer:(char *)buffer bufferLength:(NSUInteger)bufferLength {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry || !entry.webView.URL) return NO;

    return [self copyNativeWebViewString:entry.webView.URL.absoluteString
                                  buffer:buffer
                            bufferLength:bufferLength];
}

- (BOOL)copyNativeWebViewTitle:(NSUInteger)webViewId buffer:(char *)buffer bufferLength:(NSUInteger)bufferLength {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return NO;

    return [self copyNativeWebViewString:entry.webView.title buffer:buffer bufferLength:bufferLength];
}

- (BOOL)copyNativeWebViewConsoleJSON:(NSUInteger)webViewId buffer:(char *)buffer bufferLength:(NSUInteger)bufferLength {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return NO;

    return [self copyNativeWebViewString:[entry consoleMessagesJSON] buffer:buffer bufferLength:bufferLength];
}

- (BOOL)copyNativeWebViewNetworkJSON:(NSUInteger)webViewId buffer:(char *)buffer bufferLength:(NSUInteger)bufferLength {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return NO;

    return [self copyNativeWebViewString:[entry networkMessagesJSON] buffer:buffer bufferLength:bufferLength];
}

/// Whether the responder is a view inside the given webview.
- (BOOL)responder:(NSResponder *)responder isInWebView:(WKWebView *)webView {
    if (![responder isKindOfClass:[NSView class]]) return NO;
    return [(NSView *)responder isDescendantOf:webView];
}

- (void)evaluateNativeWebView:(NSUInteger)webViewId
                       script:(NSString *)script
                     callback:(WarpBrowserStringCallback)callback
                      context:(void *)context {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) {
        callback(context, NULL, "browser webview not found");
        return;
    }

    // Automation scripts call element.focus(), which can pull the window's
    // first responder into the webview — hijacking the keyboard from whatever
    // the user was typing in (e.g. a Claude composer). Automation must be
    // able to drive an unfocused webview, so if the responder moves into the
    // webview during evaluation, put it back.
    NSResponder *responderBefore = self.window.firstResponder;
    BOOL webViewHadFocus = [self responder:responderBefore isInWebView:entry.webView];
    WKWebView *webView = entry.webView;

    void (^completion)(id, NSError *) = ^(id result, NSError *error) {
        if (!webViewHadFocus && [self responder:self.window.firstResponder isInWebView:webView]) {
            [self.window makeFirstResponder:responderBefore ?: self];
        }
        if (error) {
            callback(context, NULL, error.localizedDescription.UTF8String);
            return;
        }

        NSString *resultString = nil;
        if (!result || result == [NSNull null]) {
            resultString = @"";
        } else if ([result isKindOfClass:[NSString class]]) {
            resultString = (NSString *)result;
        } else {
            resultString = [result description];
        }
        callback(context, resultString.UTF8String, NULL);
    };

    if (@available(macOS 11.0, *)) {
        // The automation scripts evaluate to a Promise; the legacy
        // evaluateJavaScript: API cannot serialize one and fails with
        // WKErrorJavaScriptResultTypeIsUnsupported. callAsyncJavaScript
        // awaits the Promise, but treats the source as a function *body*,
        // so the expression must be returned explicitly.
        NSString *body = [NSString stringWithFormat:@"return (%@);", script];
        [entry.webView callAsyncJavaScript:body
                                 arguments:@{}
                                   inFrame:nil
                            inContentWorld:WKContentWorld.pageWorld
                         completionHandler:completion];
    } else {
        [entry.webView evaluateJavaScript:script completionHandler:completion];
    }
}

- (void)takeNativeWebViewSnapshot:(NSUInteger)webViewId
                         callback:(WarpBrowserBytesCallback)callback
                          context:(void *)context {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) {
        callback(context, NULL, 0, "browser webview not found");
        return;
    }

    if (@available(macOS 10.13, *)) {
        [entry.webView takeSnapshotWithConfiguration:nil
                                   completionHandler:^(NSImage *snapshotImage, NSError *error) {
                                       if (error) {
                                           callback(context, NULL, 0, error.localizedDescription.UTF8String);
                                           return;
                                       }
                                       NSData *tiffData = [snapshotImage TIFFRepresentation];
                                       NSBitmapImageRep *bitmap =
                                           [NSBitmapImageRep imageRepWithData:tiffData];
                                       NSData *pngData =
                                           [bitmap representationUsingType:NSBitmapImageFileTypePNG
                                                                properties:@{}];
                                       if (!pngData) {
                                           callback(context, NULL, 0, "failed to encode browser snapshot as PNG");
                                           return;
                                       }
                                       callback(context,
                                                (const uint8_t *)pngData.bytes,
                                                (uintptr_t)pngData.length,
                                                NULL);
                                   }];
    } else {
        callback(context, NULL, 0, "browser snapshots require macOS 10.13 or newer");
    }
}

- (void)focusNativeWebView:(NSUInteger)webViewId {
    WarpNativeWebViewEntry *entry = [self nativeWebViewEntry:webViewId];
    if (!entry) return;

    [self.window makeFirstResponder:entry.webView];
}

- (void)destroyNativeWebView:(NSUInteger)webViewId {
    NSNumber *key = @(webViewId);
    WarpNativeWebViewEntry *entry = [nativeWebViews objectForKey:key];
    if (!entry) return;

    if (self.window.firstResponder == entry.webView) {
        [self.window makeFirstResponder:self];
    }
    [entry.webView stopLoading];
    [entry.webView removeFromSuperview];
    [entry.containerView removeFromSuperview];
    [nativeWebViews removeObjectForKey:key];
}

- (void)keyDown:(NSEvent *)event {
    [self keyDownImpl:event];
}

- (BOOL)keyDownImpl:(NSEvent *)event {
    BOOL wasComposing = [self hasMarkedText];
    [textToInsert setString:@""];
    imeTouchedMarkedTextDuringInterpret = NO;

    // Interpret the key events here so we could check whether user is composing
    // text within the IME and pass the state down to the KeyDown events.
    interpretingKeyEvents = YES;
    [self interpretKeyEvents:[NSArray arrayWithObject:event]];
    interpretingKeyEvents = NO;

    BOOL handled = NO;
    if (self.readyForWarp) {
        handled = warp_handle_view_event(self, event, wasComposing || [self hasMarkedText]);
    }

    // It's possible to have keybinding conflicts between terminal apps which use the meta key and
    // MacOS "dead keys". Dead keys are used to add diacritical marks to other characters, and they
    // start composing marked text. To detect if a keybinding was triggered in the app, `handled`
    // will be true. If that is the case, we don't want MacOS to also start composing because we
    // already handled that keydown elsewhere. So, if `justStartedComposing` is also true, clear
    // out the marked text.
    // https://support.apple.com/guide/mac-help/enter-characters-with-accent-marks-on-mac-mh27474/mac#mchl45cdda7f
    BOOL justStartedComposing = !wasComposing && [self hasMarkedText];
    if (handled && justStartedComposing) {
        NSTextInputContext *inputContext = [self inputContext];
        [inputContext discardMarkedText];
        [self unmarkText];
    }

    // Dispatch TypedCharacter event after KeyDown has been dispatched.
    if ([textToInsert length] > 0 && !handled) {
        warp_handle_insert_text(self, (NSString *)textToInsert);
        // Only clear marked text if the IME did not touch it during this
        // interpretKeyEvents pass. Otherwise we'd either fire a redundant
        // ClearMarkedText (if IME already cleared) or, worse, wipe the new
        // marked text the IME just set in a split-commit (e.g. Japanese IME
        // committing a phrase and queuing the next character as marked text).
        if (!imeTouchedMarkedTextDuringInterpret) {
            [self unmarkText];
        }
    }

    return handled;
}

- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    // We want to receive mouseDown events even if the window is not key
    // and we explicity fire the event here so that Warp can handle it.
    if (self.readyForWarp) warp_handle_first_mouse_event(self, event);

    // We return NO though so that the event is not fired twice (returning YES
    // would result in the event being passed to the mouseDown handler).
    return NO;
}

- (void)mouseDown:(NSEvent *)event {
    if (self.readyForWarp) {
        BOOL eventHandled = warp_handle_view_event(self, event, NO);
        if (self->titlebarDragEnabled && !eventHandled && [self mouseInTitleBar:event]) {
            // If Warp doesn't do anything with the event, indicated by returning `false`, and
            // if the drag starts in the titlebar, begin dragging the window
            [self.window performWindowDragWithEvent:event];
        }
    }
}

- (void)mouseUp:(NSEvent *)event {
    // Our content view is full-size so we don't get the default behavior
    // on titlebar clicks. Implement it manually.
    BOOL warp_handled = NO;
    if (self.readyForWarp) {
        warp_handled = warp_handle_view_event(self, event, NO);
    }
    if (!warp_handled) {
        [self handleTitleBarDoubleClick:event];
    }
}

- (void)otherMouseDown:(NSEvent *)event {
    if (self.readyForWarp) warp_handle_view_event(self, event, NO);
}

- (void)rightMouseDown:(NSEvent *)event {
    if (self.readyForWarp) warp_handle_view_event(self, event, NO);
}

- (void)mouseDragged:(NSEvent *)event {
    if (self.readyForWarp) warp_handle_view_event(self, event, NO);
}

- (void)scrollWheel:(NSEvent *)event {
    if (self.readyForWarp) warp_handle_view_event(self, event, NO);
}

- (void)mouseMoved:(NSEvent *)event {
    if (self.readyForWarp) warp_handle_view_event(self, event, NO);
}

- (void)flagsChanged:(NSEvent *)event {
    if (self.readyForWarp) warp_handle_view_event(self, event, NO);
}

- (void)dealloc {
    NSArray<NSNumber *> *keys = [[nativeWebViews allKeys] copy];
    for (NSNumber *key in keys) {
        [self destroyNativeWebView:[key unsignedIntegerValue]];
    }
    [keys release];
    [nativeWebViews release];
    [markedText release];
    [textToInsert release];
    [metalDevice release];
    [super dealloc];
}

- (CALayer *)makeBackingLayer {
    CAMetalLayer *layer = [CAMetalLayer layer];
    layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    layer.device = metalDevice;
    layer.allowsNextDrawableTimeout = NO;
    layer.autoresizingMask = kCALayerWidthSizable | kCALayerHeightSizable;
    layer.needsDisplayOnBoundsChange = YES;
    layer.presentsWithTransaction = NO;
    layer.delegate = self;
    layer.opaque = NO;
    return layer;
}

- (WarpHostView *)initWithFrame:(NSRect)frame
                    metalDevice:(id)device
             enableTitlebarDrag:(BOOL)enableTitlebarDrag
                       testMode:(BOOL)testModeFlag {
    NSAssert(testModeFlag || device, @"Nil metal device not in test mode");
    [super initWithFrame:frame];

    // Register here so we could receive drag and drop events.
    [self registerForDraggedTypes:@[
        NSPasteboardTypeFileURL,
    ]];
    self->testMode = testModeFlag;
    self->titlebarDragEnabled = enableTitlebarDrag;
    self->metalDevice = [device retain];
    self->markedText = [[NSMutableAttributedString alloc] init];
    self->textToInsert = [[NSMutableString alloc] init];
    self->nativeWebViews = [[NSMutableDictionary alloc] init];
    self->nextNativeWebViewId = 1;
    self->nativeWebViewsOccluded = NO;
    self->asyncCallback = YES;
    self.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    self.wantsLayer = YES;
    self.canDrawSubviewsIntoLayer = NO;
    self.layerContentsRedrawPolicy = NSViewLayerContentsRedrawDuringViewResize;
    return self;
}

// Entry point for drag & drop. Check whether the source is an acceptable type and if so
// pass it down to performDragOperaion.
- (NSDragOperation)draggingEntered:(id<NSDraggingInfo>)sender {
    NSDragOperation sourceMask = [sender draggingSourceOperationMask];

    BOOL pasteOK =
        !![[sender draggingPasteboard] availableTypeFromArray:@[ NSPasteboardTypeFileURL ]];
    if (pasteOK && (sourceMask & NSDragOperationCopy)) {
        return NSDragOperationCopy;
    }
    return NSDragOperationNone;
}

// Called continuously while the drag operation is occurring within the view
- (NSDragOperation)draggingUpdated:(id<NSDraggingInfo>)sender {
    NSPoint dragPoint = [sender draggingLocation];
    NSPoint localPoint = [self convertPoint:dragPoint fromView:nil];

    NSPasteboard *pasteboard = [sender draggingPasteboard];
    if (self.readyForWarp) {
        NSArray *types = [pasteboard types];
        if ([types containsObject:NSPasteboardTypeFileURL]) {
            warp_handle_file_drag(self, localPoint);
            return YES;
        }
    }
    return NSDragOperationNone;
}

- (void)draggingExited:(id<NSDraggingInfo>)sender {
    if (self.readyForWarp) {
        warp_handle_file_drag_exit(self);
    }
}

- (BOOL)performDragOperation:(id<NSDraggingInfo>)sender {
    NSPasteboard *pasteboard = [sender draggingPasteboard];
    NSDragOperation dragOperation = [sender draggingSourceOperationMask];

    NSPoint dragPoint = [sender draggingLocation];
    NSPoint localPoint = [self convertPoint:dragPoint fromView:nil];

    if (self.readyForWarp && (dragOperation & NSDragOperationCopy)) {
        NSArray *types = [pasteboard types];
        if ([types containsObject:NSPasteboardTypeFileURL]) {
            warp_handle_drag_and_drop(self, [pasteboard getFilePaths], localPoint);
            return YES;
        }
    }
    return NO;
}

- (void)closeIMEAsync {
    dispatch_async(dispatch_get_main_queue(), ^{
      NSTextInputContext *inputContext = [self inputContext];
      [inputContext discardMarkedText];

      [self unmarkText];
    });
}

#pragma mark - Accessibility
- (BOOL)isAccessibilityElement {
    return YES;
}

- (NSAccessibilityRole)accessibilityRole {
    return NSAccessibilityTextAreaRole;
}

- (NSString *)accessibilityRoleDescription {
    return NSAccessibilityRoleDescriptionForUIElement(self);
}

- (BOOL)isAccessibilityFocused {
    return YES;
}

- (id)accessibilityValue {
    return warp_get_accessibility_contents(self);
}

- (NSInteger)accessibilityNumberOfCharacters {
    return 0;
}

- (NSInteger)accessibilityInsertionPointLineNumber {
    return 0;
}

- (NSString *)accessibilityDocument {
    return nil;
}

////////////////////////////////////////////////////////////////////////////////
// NSTextInputClient protocol implementation
////////////////////////////////////////////////////////////////////////////////

- (nullable NSAttributedString *)attributedSubstringForProposedRange:(NSRange)range
                                                         actualRange:
                                                             (nullable NSRangePointer)actualRange {
    return nil;
}

- (NSUInteger)characterIndexForPoint:(NSPoint)thePoint {
    return (NSUInteger)0;
}

// This is a no-op as we will be handling control characters in KeyDown events.
- (void)doCommandBySelector:(SEL)selector {
}

- (NSRect)firstRectForCharacterRange:(NSRange)range
                         actualRange:(nullable NSRangePointer)actualRange {
    NSWindow *window = self.window;
    if (self.readyForWarp) {
        NSRect contentRect = [window contentRectForFrameRect:[window frame]];
        NSRect rect = warp_ime_position(self, &contentRect);
        return rect;
    } else {
        return NSZeroRect;
    }
}

- (BOOL)hasMarkedText {
    return [markedText length] > 0;
}

// Referenced glfw for this implementation.
// https://github.com/glfw/glfw/blob/7ef34eb06de54dd9186d3d21a401b2ef819b59e7/src/cocoa_window.m#L814
- (void)insertText:(id)string replacementRange:(NSRange)replacementRange {
    if (self.readyForWarp) {
        NSMutableString *characters = [[NSMutableString alloc] init];

        if ([string isKindOfClass:[NSAttributedString class]]) {
            // We are appending rather than replacing here because sometimes insertText
            // could be fired multiple times in a row. For example, when user types
            // Option-E followed by g, insertText will fire ´ first and then g.
            [characters appendString:[string string]];
        } else {
            [characters appendString:(NSString *)string];
        }

        // If we're in the middle of a call to interpretKeyEvents, batch up all
        // inserted text, as we may handle the event during `keyDown`.  If this
        // call to `insertText` is not in a call stack underneath `keyDown`
        // (e.g.: when inserting an emoji from the emoji composer), just insert
        // the text directly.
        if (interpretingKeyEvents) {
            [textToInsert appendString:characters];
        } else {
            warp_handle_insert_text(self, (NSString *)characters);
        }

        [characters release];
    }
    // When handling the key down Enter, we might need to rely on the IME being open
    // to accept the marked text as-is and so can't call unmarkText.
    if (!interpretingKeyEvents) {
        [self unmarkText];
    }
}

- (NSRange)markedRange {
    if ([markedText length] > 0)
        return NSMakeRange(0, [markedText length]);
    else
        return NSMakeRange(NSNotFound, 0);
}

- (NSRange)selectedRange {
    return NSMakeRange(0, 0);
}

- (void)setMarkedText:(id)string
        selectedRange:(NSRange)selectedRange
     replacementRange:(NSRange)replacementRange {
    if (interpretingKeyEvents) {
        imeTouchedMarkedTextDuringInterpret = YES;
    }

    [markedText release];
    if ([string isKindOfClass:[NSAttributedString class]])
        markedText = [[NSMutableAttributedString alloc] initWithAttributedString:string];
    else
        markedText = [[NSMutableAttributedString alloc] initWithString:string];

    if (self.readyForWarp) {
        warp_marked_text_updated(self, markedText.string, selectedRange);
        if ([markedText length] > 0) {
            warp_update_ime_state(self, YES);
        } else {
            warp_update_ime_state(self, NO);
        }
    }
}

- (void)unmarkText {
    if (interpretingKeyEvents) {
        imeTouchedMarkedTextDuringInterpret = YES;
    }
    [[markedText mutableString] setString:@""];
    if (self.readyForWarp) {
        warp_update_ime_state(self, NO);
        warp_marked_text_cleared(self);
    }
}

- (NSArray<NSString *> *)validAttributesForMarkedText {
    return [NSArray array];
}

@end

// twarp 14m: WKWebView (and the entry bookkeeping around it) is
// main-thread-only, but the browser MCP invokes these shims from a tokio
// worker thread. [WKWebView goBack] hard-asserts off-main (EXC_BREAKPOINT in
// WebPageProxy::goBack — live crash 2026-07-09); the others are silent UB.
// Every shim therefore hops to the main queue: fire-and-forget for commands,
// synchronous for reads (cheap main-thread work only — no re-entrant waits,
// so no deadlock pairing exists with the MCP thread).
static void warp_host_on_main_async(dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_async(dispatch_get_main_queue(), block);
    }
}

static void warp_host_on_main_sync(NS_NOESCAPE dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

uintptr_t warp_host_create_webview(WarpHostView *host, BOOL persistentDataStore) {
    __block uintptr_t webViewId = 0;
    warp_host_on_main_sync(^{
      webViewId = [host createNativeWebViewWithPersistentDataStore:persistentDataStore];
    });
    return webViewId;
}

void warp_host_install_automation_script(WarpHostView *host, uintptr_t webViewId, NSString *source) {
    warp_host_on_main_async(^{
      [host installNativeWebViewAutomationScript:(NSUInteger)webViewId source:source];
    });
}

void warp_host_set_webview_frame(WarpHostView *host, uintptr_t webViewId, NSRect frame) {
    warp_host_on_main_async(^{
      [host setNativeWebViewFrame:(NSUInteger)webViewId frame:frame];
    });
}

void warp_host_set_webview_hidden(WarpHostView *host, uintptr_t webViewId, BOOL hidden) {
    warp_host_on_main_async(^{
      [host setNativeWebViewHidden:(NSUInteger)webViewId hidden:hidden];
    });
}

void warp_host_set_webview_input_blocked(WarpHostView *host, uintptr_t webViewId, BOOL blocked) {
    warp_host_on_main_async(^{
      [host setNativeWebViewInputBlocked:(NSUInteger)webViewId blocked:blocked];
    });
}

void warp_host_set_webview_annotation_capture(WarpHostView *host, uintptr_t webViewId, BOOL enabled) {
    warp_host_on_main_async(^{
      [host setNativeWebViewAnnotationCapture:(NSUInteger)webViewId enabled:enabled];
    });
}

BOOL warp_host_take_webview_annotation_click(WarpHostView *host,
                                             uintptr_t webViewId,
                                             double *x,
                                             double *y) {
    __block BOOL taken = NO;
    warp_host_on_main_sync(^{
      taken = [host takeNativeWebViewAnnotationClick:(NSUInteger)webViewId x:x y:y];
    });
    return taken;
}

void warp_host_load_url(WarpHostView *host, uintptr_t webViewId, NSString *urlString) {
    warp_host_on_main_async(^{
      [host loadNativeWebView:(NSUInteger)webViewId urlString:urlString];
    });
}

void warp_host_go_back(WarpHostView *host, uintptr_t webViewId) {
    warp_host_on_main_async(^{
      [host goBackNativeWebView:(NSUInteger)webViewId];
    });
}

void warp_host_go_forward(WarpHostView *host, uintptr_t webViewId) {
    warp_host_on_main_async(^{
      [host goForwardNativeWebView:(NSUInteger)webViewId];
    });
}

void warp_host_reload(WarpHostView *host, uintptr_t webViewId) {
    warp_host_on_main_async(^{
      [host reloadNativeWebView:(NSUInteger)webViewId];
    });
}

void warp_host_stop_loading(WarpHostView *host, uintptr_t webViewId) {
    warp_host_on_main_async(^{
      [host stopLoadingNativeWebView:(NSUInteger)webViewId];
    });
}

BOOL warp_host_can_go_back(WarpHostView *host, uintptr_t webViewId) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host nativeWebViewCanGoBack:(NSUInteger)webViewId];
    });
    return result;
}

BOOL warp_host_can_go_forward(WarpHostView *host, uintptr_t webViewId) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host nativeWebViewCanGoForward:(NSUInteger)webViewId];
    });
    return result;
}

BOOL warp_host_is_loading(WarpHostView *host, uintptr_t webViewId) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host nativeWebViewIsLoading:(NSUInteger)webViewId];
    });
    return result;
}

BOOL warp_host_copy_url(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host copyNativeWebViewURL:(NSUInteger)webViewId
                                   buffer:buffer
                             bufferLength:(NSUInteger)bufferLength];
    });
    return result;
}

BOOL warp_host_copy_title(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host copyNativeWebViewTitle:(NSUInteger)webViewId
                                     buffer:buffer
                               bufferLength:(NSUInteger)bufferLength];
    });
    return result;
}

BOOL warp_host_copy_console_json(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host copyNativeWebViewConsoleJSON:(NSUInteger)webViewId
                                           buffer:buffer
                                     bufferLength:(NSUInteger)bufferLength];
    });
    return result;
}

BOOL warp_host_copy_network_json(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    __block BOOL result = NO;
    warp_host_on_main_sync(^{
      result = [host copyNativeWebViewNetworkJSON:(NSUInteger)webViewId
                                           buffer:buffer
                                     bufferLength:(NSUInteger)bufferLength];
    });
    return result;
}

void warp_host_evaluate_javascript(WarpHostView *host,
                                   uintptr_t webViewId,
                                   NSString *script,
                                   WarpBrowserStringCallback callback,
                                   void *context) {
    warp_host_on_main_async(^{
      [host evaluateNativeWebView:(NSUInteger)webViewId
                           script:script
                         callback:callback
                          context:context];
    });
}

void warp_host_take_snapshot(WarpHostView *host,
                             uintptr_t webViewId,
                             WarpBrowserBytesCallback callback,
                             void *context) {
    warp_host_on_main_async(^{
      [host takeNativeWebViewSnapshot:(NSUInteger)webViewId callback:callback context:context];
    });
}

void warp_host_focus_webview(WarpHostView *host, uintptr_t webViewId) {
    warp_host_on_main_async(^{
      [host focusNativeWebView:(NSUInteger)webViewId];
    });
}

void warp_host_destroy_webview(WarpHostView *host, uintptr_t webViewId) {
    warp_host_on_main_async(^{
      [host destroyNativeWebView:(NSUInteger)webViewId];
    });
}

void warp_host_clear_browser_website_data(WarpHostView *host) {
    warp_host_on_main_async(^{
      [host clearNativeBrowserWebsiteData];
    });
}

void warp_host_prepare_webviews_for_frame(WarpHostView *host) {
    warp_host_on_main_async(^{
      [host prepareNativeWebViewsForFrame];
    });
}
