#import "host_view.h"

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

@class WarpNativeWebViewEntry;

@interface WarpAutomationScriptMessageHandler : NSObject <WKScriptMessageHandler>
@property(nonatomic, assign) WarpNativeWebViewEntry *entry;
- (instancetype)initWithEntry:(WarpNativeWebViewEntry *)entry;
@end

@interface WarpNativeWebViewEntry : NSObject <WKNavigationDelegate, WKUIDelegate>
@property(nonatomic, retain) NSView *containerView;
@property(nonatomic, retain) WKWebView *webView;
@property(nonatomic, retain) WKUserContentController *userContentController;
@property(nonatomic, retain) WarpAutomationScriptMessageHandler *messageHandler;
@property(nonatomic, retain) NSMutableArray<NSString *> *consoleMessages;
@property(nonatomic, retain) NSMutableArray<NSString *> *networkMessages;
@property(nonatomic) BOOL hiddenRequested;
- (instancetype)initWithContainerView:(NSView *)containerView
                              webView:(WKWebView *)webView
                userContentController:(WKUserContentController *)userContentController;
- (void)appendAutomationMessage:(id)message;
- (void)clearAutomationMessages;
- (NSString *)consoleMessagesJSON;
- (NSString *)networkMessagesJSON;
- (void)downloadRequest:(NSURLRequest *)request response:(NSURLResponse *)response;
@end

static const NSUInteger WarpAutomationMessageLimit = 200;

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

- (void)dealloc {
    [_userContentController removeScriptMessageHandlerForName:@"twarpAutomation"];
    _webView.navigationDelegate = nil;
    _webView.UIDelegate = nil;
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
    }
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
    NSAlert *alert = [[[NSAlert alloc] init] autorelease];
    alert.messageText = webView.title.length > 0 ? webView.title : @"Browser";
    alert.informativeText = message ?: @"";
    [alert addButtonWithTitle:@"OK"];
    [alert runModal];
    completionHandler();
}

- (void)webView:(WKWebView *)webView
    runJavaScriptConfirmPanelWithMessage:(NSString *)message
                        initiatedByFrame:(WKFrameInfo *)frame
                       completionHandler:(void (^)(BOOL result))completionHandler {
    NSAlert *alert = [[[NSAlert alloc] init] autorelease];
    alert.messageText = webView.title.length > 0 ? webView.title : @"Browser";
    alert.informativeText = message ?: @"";
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    completionHandler([alert runModal] == NSAlertFirstButtonReturn);
}

- (void)webView:(WKWebView *)webView
    runJavaScriptTextInputPanelWithPrompt:(NSString *)prompt
                              defaultText:(NSString *)defaultText
                         initiatedByFrame:(WKFrameInfo *)frame
                        completionHandler:(void (^)(NSString *result))completionHandler {
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

- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)navigationAction
                    windowFeatures:(WKWindowFeatures *)windowFeatures {
    if (!navigationAction.targetFrame && navigationAction.request.URL) {
        [webView loadRequest:navigationAction.request];
    }
    return nil;
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

uintptr_t warp_host_create_webview(WarpHostView *host, BOOL persistentDataStore) {
    return [host createNativeWebViewWithPersistentDataStore:persistentDataStore];
}

void warp_host_install_automation_script(WarpHostView *host, uintptr_t webViewId, NSString *source) {
    [host installNativeWebViewAutomationScript:(NSUInteger)webViewId source:source];
}

void warp_host_set_webview_frame(WarpHostView *host, uintptr_t webViewId, NSRect frame) {
    [host setNativeWebViewFrame:(NSUInteger)webViewId frame:frame];
}

void warp_host_set_webview_hidden(WarpHostView *host, uintptr_t webViewId, BOOL hidden) {
    [host setNativeWebViewHidden:(NSUInteger)webViewId hidden:hidden];
}

void warp_host_load_url(WarpHostView *host, uintptr_t webViewId, NSString *urlString) {
    [host loadNativeWebView:(NSUInteger)webViewId urlString:urlString];
}

void warp_host_go_back(WarpHostView *host, uintptr_t webViewId) {
    [host goBackNativeWebView:(NSUInteger)webViewId];
}

void warp_host_go_forward(WarpHostView *host, uintptr_t webViewId) {
    [host goForwardNativeWebView:(NSUInteger)webViewId];
}

void warp_host_reload(WarpHostView *host, uintptr_t webViewId) {
    [host reloadNativeWebView:(NSUInteger)webViewId];
}

void warp_host_stop_loading(WarpHostView *host, uintptr_t webViewId) {
    [host stopLoadingNativeWebView:(NSUInteger)webViewId];
}

BOOL warp_host_can_go_back(WarpHostView *host, uintptr_t webViewId) {
    return [host nativeWebViewCanGoBack:(NSUInteger)webViewId];
}

BOOL warp_host_can_go_forward(WarpHostView *host, uintptr_t webViewId) {
    return [host nativeWebViewCanGoForward:(NSUInteger)webViewId];
}

BOOL warp_host_is_loading(WarpHostView *host, uintptr_t webViewId) {
    return [host nativeWebViewIsLoading:(NSUInteger)webViewId];
}

BOOL warp_host_copy_url(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    return [host copyNativeWebViewURL:(NSUInteger)webViewId
                               buffer:buffer
                         bufferLength:(NSUInteger)bufferLength];
}

BOOL warp_host_copy_title(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    return [host copyNativeWebViewTitle:(NSUInteger)webViewId
                                 buffer:buffer
                           bufferLength:(NSUInteger)bufferLength];
}

BOOL warp_host_copy_console_json(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    return [host copyNativeWebViewConsoleJSON:(NSUInteger)webViewId
                                       buffer:buffer
                                 bufferLength:(NSUInteger)bufferLength];
}

BOOL warp_host_copy_network_json(WarpHostView *host, uintptr_t webViewId, char *buffer, uintptr_t bufferLength) {
    return [host copyNativeWebViewNetworkJSON:(NSUInteger)webViewId
                                       buffer:buffer
                                 bufferLength:(NSUInteger)bufferLength];
}

void warp_host_evaluate_javascript(WarpHostView *host,
                                   uintptr_t webViewId,
                                   NSString *script,
                                   WarpBrowserStringCallback callback,
                                   void *context) {
    [host evaluateNativeWebView:(NSUInteger)webViewId script:script callback:callback context:context];
}

void warp_host_take_snapshot(WarpHostView *host,
                             uintptr_t webViewId,
                             WarpBrowserBytesCallback callback,
                             void *context) {
    [host takeNativeWebViewSnapshot:(NSUInteger)webViewId callback:callback context:context];
}

void warp_host_focus_webview(WarpHostView *host, uintptr_t webViewId) {
    [host focusNativeWebView:(NSUInteger)webViewId];
}

void warp_host_destroy_webview(WarpHostView *host, uintptr_t webViewId) {
    [host destroyNativeWebView:(NSUInteger)webViewId];
}

void warp_host_clear_browser_website_data(WarpHostView *host) {
    [host clearNativeBrowserWebsiteData];
}

void warp_host_prepare_webviews_for_frame(WarpHostView *host) {
    [host prepareNativeWebViewsForFrame];
}
