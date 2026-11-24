#import "cursor_tracking.h"

#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <mach/mach_time.h>
#import <math.h>
#import <unistd.h>

static const NSUInteger kMaximumCursorSamples = 2000000;
static const uint64_t kCursorSampleInterval = NSEC_PER_SEC / 30;

@interface OtterCutCursorTracking ()
@property(nonatomic, strong) NSURL *outputURL;
@property(nonatomic, strong) SCContentFilter *filter;
@property(nonatomic) NSUInteger width;
@property(nonatomic) NSUInteger height;
@property(nonatomic) CGRect initialBounds;
@property(nonatomic) CGWindowID windowID;
@property(nonatomic) CGDirectDisplayID displayID;
@property(nonatomic) pid_t parentPID;
@property(nonatomic) mach_timebase_info_data_t timebase;
@property(nonatomic) uint64_t startedAt;
@property(nonatomic) BOOL running;
@property(nonatomic) BOOL finalized;
@property(nonatomic) BOOL leftWasPressed;
@property(nonatomic) BOOL rightWasPressed;
@property(nonatomic) BOOL reachedSampleLimit;
@property(nonatomic) dispatch_queue_t queue;
@property(nonatomic) dispatch_source_t timer;
@property(nonatomic, strong) NSMutableArray<NSDictionary<NSString *, id> *> *samples;
@property(nonatomic, strong, nullable) id scrollMonitor;
@end

@implementation OtterCutCursorTracking

- (instancetype)initWithOutputURL:(NSURL *)outputURL
                           filter:(SCContentFilter *)filter
                            width:(NSUInteger)width
                           height:(NSUInteger)height {
    self = [super init];
    if (self) {
        _outputURL = outputURL;
        _filter = filter;
        _width = MAX(width, 1);
        _height = MAX(height, 1);
        _initialBounds = filter.contentRect;
        _parentPID = getppid();
        _queue = dispatch_queue_create("io.github.muchenkezhan.ottercut.cursor-tracking",
                                       DISPATCH_QUEUE_SERIAL);
        _samples = [NSMutableArray array];
        mach_timebase_info(&_timebase);
        [self resolveCaptureSource];
    }
    return self;
}

- (void)resolveCaptureSource {
    if (self.filter.style == SCShareableContentStyleWindow) {
        if (@available(macOS 15.2, *)) {
            SCWindow *window = self.filter.includedWindows.firstObject;
            if (window) {
                self.windowID = window.windowID;
                return;
            }
        }

        // includedWindows was added in macOS 15.2. Match the selected frame on
        // earlier supported systems, then use its stable CGWindowID for motion.
        __weak typeof(self) weakSelf = self;
        [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                                   onScreenWindowsOnly:YES
                                                     completionHandler:^(SCShareableContent * _Nullable content,
                                                                         NSError * _Nullable error) {
            if (error || !content) {
                return;
            }
            OtterCutCursorTracking *strongSelf = weakSelf;
            if (!strongSelf) {
                return;
            }
            SCWindow *match = [strongSelf closestWindowToRect:strongSelf.initialBounds
                                                       windows:content.windows];
            if (match) {
                dispatch_async(strongSelf.queue, ^{
                    if (strongSelf.running && !strongSelf.finalized) {
                        strongSelf.windowID = match.windowID;
                    }
                });
            }
        }];
        return;
    }

    if (self.filter.style == SCShareableContentStyleDisplay) {
        if (@available(macOS 15.2, *)) {
            SCDisplay *display = self.filter.includedDisplays.firstObject;
            if (display) {
                self.displayID = display.displayID;
                return;
            }
        }
        self.displayID = [self displayMatchingRect:self.initialBounds];
    }
}

- (SCWindow *)closestWindowToRect:(CGRect)rect windows:(NSArray<SCWindow *> *)windows {
    SCWindow *bestMatch = nil;
    CGFloat bestScore = CGFLOAT_MAX;
    for (SCWindow *window in windows) {
        CGRect frame = window.frame;
        CGFloat score = fabs(frame.origin.x - rect.origin.x) +
                        fabs(frame.origin.y - rect.origin.y) +
                        fabs(frame.size.width - rect.size.width) +
                        fabs(frame.size.height - rect.size.height);
        if (score < bestScore) {
            bestScore = score;
            bestMatch = window;
        }
    }
    return bestScore <= 8.0 ? bestMatch : nil;
}

- (CGDirectDisplayID)displayMatchingRect:(CGRect)rect {
    uint32_t count = 0;
    if (CGGetActiveDisplayList(0, NULL, &count) != kCGErrorSuccess || count == 0) {
        return kCGNullDirectDisplay;
    }
    NSMutableData *storage = [NSMutableData dataWithLength:count * sizeof(CGDirectDisplayID)];
    CGDirectDisplayID *displays = storage.mutableBytes;
    if (CGGetActiveDisplayList(count, displays, &count) != kCGErrorSuccess) {
        return kCGNullDirectDisplay;
    }
    for (uint32_t index = 0; index < count; index++) {
        CGRect bounds = CGDisplayBounds(displays[index]);
        if ([self rect:bounds approximatelyMatches:rect]) {
            return displays[index];
        }
    }
    return kCGNullDirectDisplay;
}

- (BOOL)rect:(CGRect)left approximatelyMatches:(CGRect)right {
    const CGFloat tolerance = 3.0;
    return fabs(left.origin.x - right.origin.x) <= tolerance &&
           fabs(left.origin.y - right.origin.y) <= tolerance &&
           fabs(left.size.width - right.size.width) <= tolerance &&
           fabs(left.size.height - right.size.height) <= tolerance;
}

- (void)start {
    if (self.running || self.finalized) {
        return;
    }
    self.running = YES;
    self.startedAt = mach_absolute_time();
    self.leftWasPressed = CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,
                                                   kCGMouseButtonLeft);
    self.rightWasPressed = CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,
                                                    kCGMouseButtonRight);

    __weak typeof(self) weakSelf = self;
    self.scrollMonitor = [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskScrollWheel
                                                                 handler:^(NSEvent *event) {
        OtterCutCursorTracking *strongSelf = weakSelf;
        if (!strongSelf) {
            return;
        }
        CGFloat horizontal = event.scrollingDeltaX;
        CGFloat vertical = event.scrollingDeltaY;
        dispatch_async(strongSelf.queue, ^{
            if (strongSelf.running && !strongSelf.finalized) {
                [strongSelf sampleAtTime:mach_absolute_time()
                               scrollX:horizontal
                               scrollY:vertical];
            }
        });
    }];

    self.timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, self.queue);
    dispatch_source_set_timer(self.timer,
                              dispatch_time(DISPATCH_TIME_NOW, 0),
                              kCursorSampleInterval,
                              NSEC_PER_MSEC);
    dispatch_source_set_event_handler(self.timer, ^{
        OtterCutCursorTracking *strongSelf = weakSelf;
        if (strongSelf && strongSelf.running && !strongSelf.finalized) {
            [strongSelf sampleAtTime:mach_absolute_time() scrollX:0.0 scrollY:0.0];
        }
    });
    dispatch_resume(self.timer);
}

- (void)sampleAtTime:(uint64_t)time scrollX:(CGFloat)scrollX scrollY:(CGFloat)scrollY {
    if (self.reachedSampleLimit) {
        return;
    }

    CGPoint location = CGPointZero;
    CGEventRef event = CGEventCreate(NULL);
    if (event) {
        location = CGEventGetLocation(event);
        CFRelease(event);
    } else {
        return;
    }

    BOOL leftPressed = CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,
                                                kCGMouseButtonLeft);
    BOOL rightPressed = CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,
                                                 kCGMouseButtonRight);
    BOOL clicked = (leftPressed && !self.leftWasPressed) ||
                   (rightPressed && !self.rightWasPressed);
    self.leftWasPressed = leftPressed;
    self.rightWasPressed = rightPressed;

    CGRect bounds = [self currentCaptureBounds];
    if (CGRectIsEmpty(bounds) || !CGRectContainsPoint(bounds, location)) {
        return;
    }

    NSDictionary<NSString *, id> *topmostWindow = [self topmostWindowAtPoint:location];
    NSNumber *ownerPID = topmostWindow[(id)kCGWindowOwnerPID];
    if (ownerPID && (ownerPID.intValue == getpid() || ownerPID.intValue == self.parentPID)) {
        return;
    }
    if (self.filter.style == SCShareableContentStyleWindow && self.windowID != kCGNullWindowID) {
        NSNumber *topmostWindowID = topmostWindow[(id)kCGWindowNumber];
        if (topmostWindowID.unsignedIntValue != self.windowID) {
            return;
        }
    }

    const CGFloat x = (location.x - CGRectGetMinX(bounds)) / CGRectGetWidth(bounds);
    const CGFloat y = (location.y - CGRectGetMinY(bounds)) / CGRectGetHeight(bounds);
    if (!isfinite(x) || !isfinite(y) || x < 0.0 || x > 1.0 || y < 0.0 || y > 1.0) {
        return;
    }

    const uint64_t elapsedTicks = time >= self.startedAt ? time - self.startedAt : 0;
    const __uint128_t nanoseconds = (__uint128_t)elapsedTicks * self.timebase.numer /
                                    self.timebase.denom;
    uint64_t elapsedMicroseconds = (uint64_t)(nanoseconds / 1000);
    NSNumber *lastTime = self.samples.lastObject[@"at_us"];
    if (lastTime && elapsedMicroseconds < lastTime.unsignedLongLongValue) {
        elapsedMicroseconds = lastTime.unsignedLongLongValue;
    }
    NSMutableDictionary<NSString *, id> *sample = [@{
        @"at_us": @(elapsedMicroseconds),
        @"x": @(x),
        @"y": @(y),
        @"pressed": @(clicked),
    } mutableCopy];
    if (scrollX != 0.0 || scrollY != 0.0) {
        sample[@"scroll_x"] = @(scrollX);
        sample[@"scroll_y"] = @(scrollY);
    }
    [self.samples addObject:sample];
    self.reachedSampleLimit = self.samples.count >= kMaximumCursorSamples;
}

- (CGRect)currentCaptureBounds {
    if (self.filter.style == SCShareableContentStyleWindow && self.windowID != kCGNullWindowID) {
        NSDictionary<NSString *, id> *window = [self windowInfoWithID:self.windowID];
        CGRect bounds = [self boundsFromWindowInfo:window];
        if (!CGRectIsEmpty(bounds)) {
            return bounds;
        }
    }
    if (self.filter.style == SCShareableContentStyleDisplay && self.displayID != kCGNullDirectDisplay) {
        CGRect bounds = CGDisplayBounds(self.displayID);
        if (!CGRectIsEmpty(bounds)) {
            return bounds;
        }
    }
    return self.initialBounds;
}

- (NSDictionary<NSString *, id> *)windowInfoWithID:(CGWindowID)windowID {
    CFArrayRef windowList = CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow |
                                                       kCGWindowListExcludeDesktopElements,
                                                       windowID);
    if (!windowList) {
        return @{};
    }
    NSArray *windows = CFBridgingRelease(windowList);
    for (id value in windows) {
        if (![value isKindOfClass:NSDictionary.class]) {
            continue;
        }
        NSDictionary *window = value;
        if ([window[(id)kCGWindowNumber] unsignedIntValue] == windowID) {
            return window;
        }
    }
    return @{};
}

- (CGRect)boundsFromWindowInfo:(NSDictionary<NSString *, id> *)window {
    NSDictionary *bounds = window[(id)kCGWindowBounds];
    CGRect rect = CGRectZero;
    if (bounds && CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)bounds, &rect)) {
        return rect;
    }
    return CGRectZero;
}

- (NSDictionary<NSString *, id> *)topmostWindowAtPoint:(CGPoint)point {
    CFArrayRef windowList = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly |
                                                       kCGWindowListExcludeDesktopElements,
                                                       kCGNullWindowID);
    if (!windowList) {
        return @{};
    }
    NSArray *windows = CFBridgingRelease(windowList);
    for (id value in windows) {
        if (![value isKindOfClass:NSDictionary.class]) {
            continue;
        }
        NSDictionary *window = value;
        NSNumber *alpha = window[(id)kCGWindowAlpha];
        if (alpha && alpha.doubleValue <= 0.001) {
            continue;
        }
        CGRect bounds = [self boundsFromWindowInfo:window];
        if (CGRectContainsPoint(bounds, point)) {
            return window;
        }
    }
    return @{};
}

- (BOOL)finish {
    if (!self.running || self.finalized) {
        return NO;
    }
    self.finalized = YES;
    self.running = NO;
    if (self.scrollMonitor) {
        [NSEvent removeMonitor:self.scrollMonitor];
        self.scrollMonitor = nil;
    }

    __block BOOL saved = NO;
    dispatch_sync(self.queue, ^{
        if (self.timer) {
            dispatch_source_cancel(self.timer);
            self.timer = nil;
        }
        if (self.samples.count == 0) {
            return;
        }
        NSURL *sidecarURL = [NSURL fileURLWithPath:[self.outputURL.path stringByAppendingString:@".cursor.json"]];
        if ([[NSFileManager defaultManager] fileExistsAtPath:sidecarURL.path]) {
            NSLog(@"鼠标轨迹文件已存在，未覆盖：%@", sidecarURL.path);
            return;
        }
        NSDictionary<NSString *, id> *telemetry = @{
            @"version": @1,
            @"width": @(self.width),
            @"height": @(self.height),
            @"samples": self.samples,
        };
        NSError *error = nil;
        NSData *data = [NSJSONSerialization dataWithJSONObject:telemetry
                                                       options:NSJSONWritingPrettyPrinted
                                                         error:&error];
        if (!data || ![data writeToURL:sidecarURL options:NSDataWritingAtomic error:&error]) {
            NSLog(@"无法保存鼠标轨迹文件（%@）", error.localizedDescription ?: @"未知错误");
            return;
        }
        saved = YES;
    });
    return saved;
}

- (void)dealloc {
    if (self.scrollMonitor) {
        [NSEvent removeMonitor:self.scrollMonitor];
    }
    if (self.timer) {
        dispatch_source_cancel(self.timer);
    }
}

@end
