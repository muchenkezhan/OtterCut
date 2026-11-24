#import <Foundation/Foundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

NS_ASSUME_NONNULL_BEGIN

@interface OtterCutCursorTracking : NSObject
- (instancetype)initWithOutputURL:(NSURL *)outputURL
                           filter:(SCContentFilter *)filter
                            width:(NSUInteger)width
                           height:(NSUInteger)height;
- (void)start;
- (BOOL)finish;
@end

NS_ASSUME_NONNULL_END
