#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

@interface NSPasteboard (Warp)
- (NSArray *)getFilePaths;
@end

/// WarpHostView is the Content view of a Warp window.
// It is backed by a Metal CALayer.
@interface WarpHostView : NSView <CALayerDelegate, NSTextInputClient>
- (WarpHostView *)initWithFrame:(NSRect)frame
                    metalDevice:(id)metalDevice
             enableTitlebarDrag:(BOOL)enableTitlebarDrag
                       testMode:(BOOL)testMode;
- (void)setAsyncCallback:(BOOL)shouldAsync;
- (void)setNativeWebViewsOccluded:(BOOL)occluded;
- (void)prepareNativeWebViewsForFrame;
- (void)goBackNativeWebView:(NSUInteger)webViewId;
- (void)goForwardNativeWebView:(NSUInteger)webViewId;
- (void)reloadNativeWebView:(NSUInteger)webViewId;
- (void)stopLoadingNativeWebView:(NSUInteger)webViewId;
- (void)setPresentsWithTransaction:(BOOL)presentsWithTransaction;
- (BOOL)keyDownImpl:(NSEvent *)event;
@end
