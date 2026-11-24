#import <AVFoundation/AVFoundation.h>
#import <AppKit/AppKit.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import "cursor_tracking.h"
#import "camera_capture.h"
#include <dispatch/dispatch.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void emit_event(const char *event) {
    puts(event);
    fflush(stdout);
}

static BOOL parse_preview_shape(const char *name, OtterCutCameraPreviewShape *shape) {
    if (strcmp(name, "circle") == 0) {
        *shape = OtterCutCameraPreviewCircle;
        return YES;
    }
    if (strcmp(name, "square") == 0) {
        *shape = OtterCutCameraPreviewSquare;
        return YES;
    }
    return NO;
}

static BOOL parse_preview_shape_command(const char *line, OtterCutCameraPreviewShape *shape) {
    static const char prefix[] = "camera-preview-";
    const size_t prefixLength = sizeof(prefix) - 1;
    return strncmp(line, prefix, prefixLength) == 0 && parse_preview_shape(line + prefixLength, shape);
}

@interface OtterCutCapture : NSObject <
    SCContentSharingPickerObserver, SCRecordingOutputDelegate, SCStreamDelegate, NSWindowDelegate
>
- (instancetype)initWithOutputURL:(NSURL *)url microphone:(BOOL)microphone;
- (instancetype)initForSourceSelection:(NSString *)mode;
- (instancetype)initWithOutputURL:(NSURL *)url
              microphoneDeviceID:(nullable NSString *)microphoneDeviceID
                   sourceSelection:(NSDictionary<NSString *, id> *)sourceSelection
                       clickFollow:(BOOL)clickFollow
                       cameraDevice:(nullable NSString *)cameraDevice
                       cameraOutput:(nullable NSURL *)cameraOutput
                         cameraWidth:(NSInteger)cameraWidth
                        cameraHeight:(NSInteger)cameraHeight
                      cameraFPSMilli:(NSInteger)cameraFPSMilli;
- (void)presentPicker;
- (void)startRecordingFromSelection;
- (void)requestStop;
- (void)setCameraPreviewVisible:(BOOL)visible;
- (void)setCameraPreviewShape:(OtterCutCameraPreviewShape)shape;
- (void)cameraDidStart;
- (void)cameraDidFinish:(const char *)error;
@end

static void camera_started(void *context) {
    [(__bridge OtterCutCapture *)context cameraDidStart];
}

static void camera_finished(const char *error, void *context) {
    [(__bridge OtterCutCapture *)context cameraDidFinish:error];
}

typedef struct {
    OtterCutCameraPreview preview;
    int exitStatus;
} StandalonePreview;

static void standalone_preview_started(const char *error, void *context) {
    if (error) {
        StandalonePreview *preview = context;
        fprintf(stderr, "无法打开摄像头预览：%s\n", error);
        preview->exitStatus = EXIT_FAILURE;
        ottercut_camera_preview_close(preview->preview);
    }
}

@implementation OtterCutCapture {
    NSURL *_outputURL;
    SCStream *_stream;
    SCRecordingOutput *_recordingOutput;
    OtterCutCursorTracking *_cursorTracking;
    NSString *_microphoneDeviceID;
    NSDictionary<NSString *, id> *_sourceSelection;
    BOOL _selectionOnly;
    BOOL _clickFollow;
    SCContentSharingPickerMode _pickerMode;
    BOOL _selectedSource;
    BOOL _captureStarted;
    BOOL _recordingStarted;
    BOOL _stopRequested;
    BOOL _stopping;
    BOOL _screenOutputFinished;
    BOOL _cameraOutputFinished;
    NSString *_cameraDevice;
    NSURL *_cameraOutputURL;
    NSInteger _cameraWidth;
    NSInteger _cameraHeight;
    NSInteger _cameraFPSMilli;
    OtterCutCameraCapture _cameraCapture;
    BOOL _cameraStarted;
    BOOL _cameraPreviewVisible;
    BOOL _cameraPreviewExcluded;
    BOOL _cameraPreviewExclusionPending;
    BOOL _cameraPreviewUnavailable;
    BOOL _cameraPreviewLayerPending;
    BOOL _cameraPreviewAttached;
    OtterCutCameraPreviewShape _cameraPreviewShape;
    SCContentFilter *_captureFilter;
    NSArray<SCWindow *> *_excludedDisplayWindows;
    NSPanel *_cameraPreviewPanel;
}

- (instancetype)initWithOutputURL:(NSURL *)url microphone:(BOOL)microphone {
    self = [super init];
    if (self) {
        _outputURL = url;
        _microphoneDeviceID = microphone ? @"default" : nil;
    }
    return self;
}

- (instancetype)initForSourceSelection:(NSString *)mode {
    self = [super init];
    if (self) {
        _selectionOnly = YES;
        if ([mode isEqualToString:@"window"]) {
            _pickerMode = SCContentSharingPickerModeSingleWindow;
        } else if ([mode isEqualToString:@"display"]) {
            _pickerMode = SCContentSharingPickerModeSingleDisplay;
        }
    }
    return self;
}

- (instancetype)initWithOutputURL:(NSURL *)url
              microphoneDeviceID:(NSString *)microphoneDeviceID
                   sourceSelection:(NSDictionary<NSString *, id> *)sourceSelection
                       clickFollow:(BOOL)clickFollow
                       cameraDevice:(NSString *)cameraDevice
                       cameraOutput:(NSURL *)cameraOutput
                         cameraWidth:(NSInteger)cameraWidth
                        cameraHeight:(NSInteger)cameraHeight
                      cameraFPSMilli:(NSInteger)cameraFPSMilli {
    self = [super init];
    if (self) {
        _outputURL = url;
        _microphoneDeviceID = microphoneDeviceID.length ? microphoneDeviceID : nil;
        _sourceSelection = sourceSelection;
        _clickFollow = clickFollow;
        _cameraDevice = [cameraDevice copy];
        _cameraOutputURL = cameraOutput;
        _cameraWidth = cameraWidth;
        _cameraHeight = cameraHeight;
        _cameraFPSMilli = cameraFPSMilli;
    }
    return self;
}

- (void)finishWithStatus:(int)status {
    [self dismissCameraPreview];
    SCContentSharingPicker *picker = [SCContentSharingPicker sharedPicker];
    picker.active = NO;
    [picker removeObserver:self];
    exit(status);
}

- (void)fail:(NSString *)message error:(NSError *)error {
    if (error) {
        fprintf(stderr, "%s：%s\n", message.UTF8String,
                error.localizedDescription.UTF8String);
    } else {
        fprintf(stderr, "%s\n", message.UTF8String);
    }
    [self finishWithStatus:EXIT_FAILURE];
}

- (void)cancel {
    emit_event("cancelled");
    [self finishWithStatus:2];
}

- (void)presentPicker {
    if (_stopRequested) {
        return;
    }
    SCContentSharingPicker *picker = [SCContentSharingPicker sharedPicker];
    SCContentSharingPickerConfiguration *configuration =
        [SCContentSharingPickerConfiguration new];
    configuration.allowedPickerModes = _pickerMode ?: (SCContentSharingPickerModeSingleDisplay |
                                                         SCContentSharingPickerModeSingleWindow);
    configuration.allowsChangingSelectedContent = NO;
    picker.defaultConfiguration = configuration;
    picker.maximumStreamCount = @1;
    [picker addObserver:self];
    picker.active = YES;
    [picker present];
}

- (void)startWithFilter:(SCContentFilter *)filter {
    _captureFilter = filter;
    SCStreamConfiguration *configuration = [SCStreamConfiguration new];
    const CGRect rect = filter.contentRect;
    const CGFloat scale = filter.pointPixelScale;
    // H.264 needs even dimensions. Keep the selected source's native aspect ratio.
    configuration.width = MAX(2, (NSInteger)ceil(CGRectGetWidth(rect) * scale / 2) * 2);
    configuration.height = MAX(2, (NSInteger)ceil(CGRectGetHeight(rect) * scale / 2) * 2);
    configuration.minimumFrameInterval = CMTimeMake(1, 30);
    configuration.showsCursor = !_clickFollow;
    configuration.captureMicrophone = _microphoneDeviceID != nil;
    if (_microphoneDeviceID && ![_microphoneDeviceID isEqualToString:@"default"]) {
        configuration.microphoneCaptureDeviceID = _microphoneDeviceID;
    }

    SCRecordingOutputConfiguration *outputConfiguration =
        [SCRecordingOutputConfiguration new];
    outputConfiguration.outputURL = _outputURL;
    outputConfiguration.outputFileType = AVFileTypeMPEG4;
    outputConfiguration.videoCodecType = AVVideoCodecTypeH264;
    _recordingOutput = [[SCRecordingOutput alloc] initWithConfiguration:outputConfiguration
                                                             delegate:self];
    _stream = [[SCStream alloc] initWithFilter:filter
                               configuration:configuration
                                    delegate:self];
    NSError *error = nil;
    if (![_stream addRecordingOutput:_recordingOutput error:&error]) {
        [self fail:@"无法配置 MP4 录制" error:error];
        return;
    }
    if (_clickFollow) {
        _cursorTracking = [[OtterCutCursorTracking alloc]
            initWithOutputURL:_outputURL
                       filter:filter
                        width:configuration.width
                       height:configuration.height];
    }
    if (_cameraDevice.length && _cameraOutputURL) {
        char *cameraError = NULL;
        _cameraCapture = ottercut_camera_capture_create(
            _cameraDevice.UTF8String,
            _cameraOutputURL.fileSystemRepresentation,
            (int)_cameraWidth,
            (int)_cameraHeight,
            (int)_cameraFPSMilli,
            camera_started,
            camera_finished,
            (__bridge void *)self,
            &cameraError);
        if (!_cameraCapture) {
            NSString *description = cameraError ? [NSString stringWithUTF8String:cameraError] : nil;
            ottercut_camera_free_string(cameraError);
            fprintf(stderr, "摄像头录像未启动：%s\n", description.UTF8String ?: "未知错误");
            _cameraOutputFinished = YES;
            emit_event("camera-unavailable");
        }
    } else {
        _cameraOutputFinished = YES;
    }
    [_stream startCaptureWithCompletionHandler:^(NSError *startError) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (startError) {
                if ([startError.domain isEqualToString:SCStreamErrorDomain] &&
                    startError.code == SCStreamErrorUserStopped) {
                    [self cancel];
                } else {
                    [self fail:@"无法开始屏幕采集" error:startError];
                }
                return;
            }
            self->_captureStarted = YES;
            if (self->_stopRequested) {
                [self requestStop];
            }
        });
    }];
}

- (void)waitForRecordingOutput {
    _stopping = YES;
    [self dismissCameraPreview];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC),
                   dispatch_get_main_queue(), ^{
        [self fail:@"等待录制文件写入超时" error:nil];
    });
}

- (void)requestStop {
    _stopRequested = YES;
    [self dismissCameraPreview];
    if (_cameraCapture) {
        ottercut_camera_capture_request_stop(_cameraCapture);
    }
    if (!_stream) {
        [self cancel];
        return;
    }
    if (!_captureStarted || _stopping) {
        return;
    }
    [self waitForRecordingOutput];
    [_stream stopCaptureWithCompletionHandler:^(NSError *error) {
        if (error) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (![error.domain isEqualToString:SCStreamErrorDomain] ||
                    (error.code != SCStreamErrorUserStopped &&
                     error.code != SCStreamErrorAttemptToStopStreamState)) {
                    [self fail:@"停止屏幕采集失败" error:error];
                }
            });
        }
    }];
}

- (void)contentSharingPicker:(SCContentSharingPicker *)picker
             didCancelForStream:(SCStream *)stream {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (!self->_selectedSource) {
            [self cancel];
        }
    });
}

- (void)contentSharingPicker:(SCContentSharingPicker *)picker
          didUpdateWithFilter:(SCContentFilter *)filter
                    forStream:(SCStream *)stream {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self->_selectedSource) {
            return;
        }
        self->_selectedSource = YES;
        if (self->_selectionOnly) {
            [self emitSelection:filter];
            return;
        }
        if (!self->_sourceSelection) {
            [self startWithFilter:filter];
        }
    });
}

- (void)contentSharingPickerStartDidFailWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{
        [self fail:@"无法打开系统录制来源选择器" error:error];
    });
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{
        if ([error.domain isEqualToString:SCStreamErrorDomain] &&
            error.code == SCStreamErrorUserStopped) {
            if (!self->_recordingStarted) {
                [self cancel];
            } else if (!self->_stopping) {
                [self waitForRecordingOutput];
            }
            return;
        }
        [self fail:@"屏幕采集已中断" error:error];
    });
}

- (void)recordingOutputDidStartRecording:(SCRecordingOutput *)recordingOutput {
    dispatch_async(dispatch_get_main_queue(), ^{
        self->_recordingStarted = YES;
        [self->_cursorTracking start];
        if (self->_cameraCapture && !ottercut_camera_capture_start(self->_cameraCapture)) {
            fprintf(stderr, "摄像头录像未启动：无法提交录制请求。\n");
            ottercut_camera_capture_destroy(self->_cameraCapture);
            self->_cameraCapture = NULL;
            self->_cameraOutputFinished = YES;
            emit_event("camera-unavailable");
        }
        emit_event("recording");
    });
}

- (void)recordingOutputDidFinishRecording:(SCRecordingOutput *)recordingOutput {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self->_recordingStarted) {
            if (self->_cursorTracking && ![self->_cursorTracking finish]) {
                fprintf(stderr, "无法保存鼠标轨迹文件。\n");
            }
            self->_screenOutputFinished = YES;
            if (self->_cameraCapture && !self->_cameraOutputFinished) {
                ottercut_camera_capture_request_stop(self->_cameraCapture);
                return;
            }
            [self finishWithStatus:EXIT_SUCCESS];
        } else {
            [self cancel];
        }
    });
}

- (void)cameraDidStart {
    _cameraStarted = YES;
    [self updateCameraPreview];
}

- (void)cameraDidFinish:(const char *)error {
    NSString *message = error ? [NSString stringWithUTF8String:error] : nil;
    dispatch_async(dispatch_get_main_queue(), ^{
        self->_cameraOutputFinished = YES;
        self->_cameraStarted = NO;
        [self dismissCameraPreview];
        if (message.length) {
            fprintf(stderr, "摄像头录像失败：%s\n", message.UTF8String);
            emit_event("camera-unavailable");
        } else {
            emit_event("camera-recorded");
        }
        if (self->_cameraCapture) {
            ottercut_camera_capture_destroy(self->_cameraCapture);
            self->_cameraCapture = NULL;
        }
        if (self->_screenOutputFinished) {
            [self finishWithStatus:EXIT_SUCCESS];
        }
    });
}

- (void)dismissCameraPreview {
    _cameraPreviewVisible = NO;
    _cameraPreviewPanel.delegate = nil;
    [_cameraPreviewPanel close];
    _cameraPreviewPanel = nil;
    _cameraPreviewAttached = NO;
}

- (void)setCameraPreviewVisible:(BOOL)visible {
    if (_stopping || _stopRequested || _cameraOutputFinished || _cameraPreviewUnavailable) {
        return;
    }
    _cameraPreviewVisible = visible;
    [self updateCameraPreview];
    if (!visible) {
        emit_event("camera-preview-hidden");
    }
}

- (void)setCameraPreviewShape:(OtterCutCameraPreviewShape)shape {
    _cameraPreviewShape = shape;
    if (_cameraPreviewPanel) {
        ottercut_camera_preview_panel_set_shape(_cameraPreviewPanel, shape);
    }
}

- (void)showCameraPreviewPanel {
    [_cameraPreviewPanel orderFrontRegardless];
    emit_event("camera-preview-shown");
}

- (BOOL)windowShouldClose:(NSWindow *)window {
    if (window == _cameraPreviewPanel) {
        _cameraPreviewVisible = NO;
        [self updateCameraPreview];
        emit_event("camera-preview-closed");
        return NO;
    }
    return YES;
}

- (void)cameraPreviewUnavailable:(NSString *)message error:(NSError *)error {
    _cameraPreviewUnavailable = YES;
    [self dismissCameraPreview];
    if (error) {
        fprintf(stderr, "%s：%s\n", message.UTF8String, error.localizedDescription.UTF8String);
    } else {
        fprintf(stderr, "%s\n", message.UTF8String);
    }
    emit_event("camera-preview-unavailable");
}

- (void)excludePreviewFromDisplayCapture {
    if (_cameraPreviewExclusionPending) {
        return;
    }
    _cameraPreviewExclusionPending = YES;
    // Register a WindowServer ID while the panel is still transparent and has no camera layer.
    _cameraPreviewPanel.alphaValue = 0;
    [_cameraPreviewPanel orderFrontRegardless];
    const NSInteger previewWindowID = _cameraPreviewPanel.windowNumber;
    SCContentFilter *originalFilter = _captureFilter;
    [SCShareableContent getCurrentProcessShareableContentWithCompletionHandler:^(
        SCShareableContent *content, NSError *error
    ) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self->_stopping || self->_stopRequested || self->_cameraOutputFinished) {
                return;
            }
            SCDisplay *display = nil;
            if (@available(macOS 15.2, *)) {
                display = originalFilter.includedDisplays.firstObject;
            }
            if (!display) {
                for (SCDisplay *candidate in content.displays) {
                    if (CGRectEqualToRect(candidate.frame, originalFilter.contentRect)) {
                        display = candidate;
                        break;
                    }
                }
            }
            SCWindow *previewWindow = nil;
            for (SCWindow *window in content.windows) {
                if (window.windowID == previewWindowID) {
                    previewWindow = window;
                    break;
                }
            }
            NSMutableArray<SCWindow *> *excludedWindows =
                [self->_excludedDisplayWindows mutableCopy] ?: [NSMutableArray array];
            SCContentFilter *filter = nil;
            if (display && previewWindow) {
                [excludedWindows addObject:previewWindow];
                filter = [[SCContentFilter alloc] initWithDisplay:display
                                               excludingWindows:excludedWindows];
                filter.includeMenuBar = originalFilter.includeMenuBar;
            }
            if (!filter) {
                self->_cameraPreviewExclusionPending = NO;
                [self cameraPreviewUnavailable:@"无法从屏幕录像中排除摄像头预览" error:error];
                return;
            }
            [self->_stream updateContentFilter:filter completionHandler:^(NSError *updateError) {
                dispatch_async(dispatch_get_main_queue(), ^{
                    self->_cameraPreviewExclusionPending = NO;
                    if (self->_stopping || self->_stopRequested || self->_cameraOutputFinished) {
                        return;
                    }
                    if (updateError) {
                        [self cameraPreviewUnavailable:@"无法从屏幕录像中排除摄像头预览" error:updateError];
                        return;
                    }
                    self->_cameraPreviewExcluded = YES;
                    [self updateCameraPreview];
                });
            }];
        });
    }];
}

- (void)updateCameraPreview {
    if (!_cameraPreviewVisible) {
        if (!_cameraPreviewExclusionPending) {
            [_cameraPreviewPanel orderOut:nil];
        }
        return;
    }
    if (!_cameraCapture || !_cameraStarted || _stopping || _stopRequested || _cameraPreviewUnavailable) {
        return;
    }
    if (!_cameraPreviewPanel) {
        _cameraPreviewPanel = ottercut_camera_preview_panel(@"OtterCut 摄像头预览");
        _cameraPreviewPanel.delegate = self;
        ottercut_camera_preview_panel_set_shape(_cameraPreviewPanel, _cameraPreviewShape);
    }
    if (_captureFilter.style == SCShareableContentStyleDisplay && !_cameraPreviewExcluded) {
        [self excludePreviewFromDisplayCapture];
        return;
    }
    if (_cameraPreviewAttached) {
        [self showCameraPreviewPanel];
        return;
    }
    if (_cameraPreviewLayerPending) {
        return;
    }
    _cameraPreviewLayerPending = YES;
    ottercut_camera_capture_preview_layer(_cameraCapture, ^(AVCaptureVideoPreviewLayer *layer) {
        self->_cameraPreviewLayerPending = NO;
        if (!self->_cameraPreviewVisible || !self->_cameraStarted || self->_stopping || self->_stopRequested) {
            return;
        }
        if (!layer) {
            [self cameraPreviewUnavailable:@"无法显示当前摄像头的画面" error:nil];
            return;
        }
        ottercut_camera_preview_attach_layer(self->_cameraPreviewPanel, layer);
        self->_cameraPreviewAttached = YES;
        self->_cameraPreviewPanel.alphaValue = 1;
        [self showCameraPreviewPanel];
    });
}

- (void)emitSelection:(SCContentFilter *)filter {
    SCShareableContentInfo *info = [SCShareableContent infoForFilter:filter];
    NSMutableDictionary<NSString *, id> *selection = [@{
        @"style": @(info.style),
        @"x": @(CGRectGetMinX(info.contentRect)),
        @"y": @(CGRectGetMinY(info.contentRect)),
        @"width": @(CGRectGetWidth(info.contentRect)),
        @"height": @(CGRectGetHeight(info.contentRect)),
    } mutableCopy];
    if (info.style == SCShareableContentStyleWindow) {
        if (@available(macOS 15.2, *)) {
            SCWindow *window = filter.includedWindows.firstObject;
            if (window) {
                selection[@"window_id"] = @(window.windowID);
            }
        }
    }
    if (info.style == SCShareableContentStyleDisplay) {
        if (@available(macOS 15.2, *)) {
            SCDisplay *display = filter.includedDisplays.firstObject;
            if (display) {
                selection[@"display_id"] = @(display.displayID);
            }
        }
    }
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:selection options:0 error:&error];
    if (!data) {
        [self fail:@"无法保存录制来源" error:error];
        return;
    }
    fwrite(data.bytes, 1, data.length, stdout);
    fputc('\n', stdout);
    fflush(stdout);
    [self finishWithStatus:EXIT_SUCCESS];
}

- (SCContentFilter *)filterFromSelection:(NSDictionary<NSString *, id> *)selection
                                content:(SCShareableContent *)content {
    const NSInteger style = [selection[@"style"] integerValue];
    CGRect target = CGRectMake([selection[@"x"] doubleValue],
                               [selection[@"y"] doubleValue],
                               [selection[@"width"] doubleValue],
                               [selection[@"height"] doubleValue]);
    if (style == SCShareableContentStyleWindow) {
        NSNumber *windowID = selection[@"window_id"];
        SCWindow *match = nil;
        for (SCWindow *window in content.windows) {
            if (windowID && window.windowID == windowID.unsignedIntValue) {
                match = window;
                break;
            }
            CGRect frame = window.frame;
            if (fabs(frame.origin.x - target.origin.x) <= 8.0 &&
                fabs(frame.origin.y - target.origin.y) <= 8.0 &&
                fabs(frame.size.width - target.size.width) <= 8.0 &&
                fabs(frame.size.height - target.size.height) <= 8.0) {
                match = window;
            }
        }
        return match ? [[SCContentFilter alloc] initWithDesktopIndependentWindow:match] : nil;
    }
    if (style == SCShareableContentStyleDisplay) {
        NSNumber *displayID = selection[@"display_id"];
        SCDisplay *match = nil;
        for (SCDisplay *display in content.displays) {
            if (displayID && display.displayID == displayID.unsignedIntValue) {
                match = display;
                break;
            }
            CGRect frame = display.frame;
            if (fabs(frame.origin.x - target.origin.x) <= 8.0 &&
                fabs(frame.origin.y - target.origin.y) <= 8.0 &&
                fabs(frame.size.width - target.size.width) <= 8.0 &&
                fabs(frame.size.height - target.size.height) <= 8.0) {
                match = display;
            }
        }
        if (!match) {
            return nil;
        }
        NSMutableArray<SCWindow *> *excluded = [NSMutableArray array];
        const pid_t parentPID = getppid();
        for (SCWindow *window in content.windows) {
            const pid_t ownerPID = window.owningApplication.processID;
            if (ownerPID == getpid() || ownerPID == parentPID) {
                [excluded addObject:window];
            }
        }
        _excludedDisplayWindows = [excluded copy];
        return [[SCContentFilter alloc] initWithDisplay:match excludingWindows:excluded];
    }
    return nil;
}

- (void)startRecordingFromSelection {
    [SCShareableContent getShareableContentWithCompletionHandler:^(SCShareableContent *content,
                                                                   NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (error || !content) {
                [self fail:@"无法读取录制来源" error:error];
                return;
            }
            SCContentFilter *filter = [self filterFromSelection:self->_sourceSelection
                                                       content:content];
            if (!filter) {
                [self fail:@"所选屏幕或窗口已不可用，请重新选择来源" error:nil];
                return;
            }
            [self startWithFilter:filter];
        });
    }];
}

- (void)recordingOutput:(SCRecordingOutput *)recordingOutput didFailWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{
        [self fail:@"写入录像失败" error:error];
    });
}
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc == 2 && strcmp(argv[1], "devices") == 0) {
            AVCaptureDeviceDiscoverySession *microphoneDiscovery =
                [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:
                    @[ AVCaptureDeviceTypeMicrophone ]
                    mediaType:AVMediaTypeAudio
                    position:AVCaptureDevicePositionUnspecified];
            AVCaptureDeviceDiscoverySession *cameraDiscovery =
                [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:
                    @[ AVCaptureDeviceTypeBuiltInWideAngleCamera,
                       AVCaptureDeviceTypeExternal,
                       AVCaptureDeviceTypeContinuityCamera ]
                    mediaType:AVMediaTypeVideo
                    position:AVCaptureDevicePositionUnspecified];
            NSMutableArray *microphones = [NSMutableArray array];
            for (AVCaptureDevice *device in microphoneDiscovery.devices) {
                [microphones addObject:@{
                    @"id": device.uniqueID,
                    @"name": device.localizedName,
                }];
            }
            NSMutableArray *cameras = [NSMutableArray array];
            for (AVCaptureDevice *device in cameraDiscovery.devices) {
                OtterCutCameraModeList modeList = {0};
                char *modeError = NULL;
                NSMutableArray *modes = [NSMutableArray array];
                if (ottercut_camera_list_modes(device.uniqueID.UTF8String, &modeList, &modeError)) {
                    for (size_t index = 0; index < modeList.count; ++index) {
                        OtterCutCameraMode mode = modeList.modes[index];
                        [modes addObject:@{
                            @"width": @(mode.width),
                            @"height": @(mode.height),
                            @"fps_milli": @(mode.fps_milli),
                        }];
                    }
                }
                ottercut_camera_free_mode_list(&modeList);
                ottercut_camera_free_string(modeError);
                [cameras addObject:@{
                    @"id": device.uniqueID,
                    @"name": device.localizedName,
                    @"modes": modes,
                }];
            }
            NSData *data = [NSJSONSerialization dataWithJSONObject:@{
                @"microphones": microphones,
                @"cameras": cameras,
            } options:0 error:nil];
            if (!data) {
                fprintf(stderr, "无法序列化设备列表。\n");
                return EXIT_FAILURE;
            }
            fwrite(data.bytes, 1, data.length, stdout);
            fputc('\n', stdout);
            return EXIT_SUCCESS;
        }

        if ((argc == 3 || argc == 4) && strcmp(argv[1], "preview-camera") == 0) {
            OtterCutCameraPreviewShape shape = OtterCutCameraPreviewCircle;
            if (argc == 4 && !parse_preview_shape(argv[3], &shape)) {
                fprintf(stderr, "摄像头预览形状无效，请使用 circle 或 square。\n");
                return EXIT_FAILURE;
            }
            NSString *deviceID = [NSString stringWithUTF8String:argv[2]];
            char *error = NULL;
            OtterCutCameraPreview preview = ottercut_camera_preview_create(
                deviceID.UTF8String,
                @"OtterCut 摄像头预览".UTF8String,
                &error);
            if (!preview) {
                fprintf(stderr, "无法打开摄像头预览：%s\n", error ?: "未知错误");
                ottercut_camera_free_string(error);
                return EXIT_FAILURE;
            }
            ottercut_camera_free_string(error);
            ottercut_camera_preview_set_shape(preview, shape);
            StandalonePreview state = { preview, EXIT_SUCCESS };
            if (!ottercut_camera_preview_start(preview, standalone_preview_started, &state)) {
                ottercut_camera_preview_close(preview);
                ottercut_camera_preview_destroy(preview);
                return EXIT_FAILURE;
            }
            __block OtterCutCameraPreview activePreview = preview;
            dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
                char line[64];
                while (fgets(line, sizeof(line), stdin)) {
                    line[strcspn(line, "\r\n")] = '\0';
                    OtterCutCameraPreviewShape requestedShape;
                    if (parse_preview_shape_command(line, &requestedShape)) {
                        dispatch_async(dispatch_get_main_queue(), ^{
                            if (activePreview) {
                                ottercut_camera_preview_set_shape(activePreview, requestedShape);
                            }
                        });
                    }
                }
            });
            const int ran = ottercut_camera_preview_run(preview);
            activePreview = NULL;
            ottercut_camera_preview_destroy(preview);
            return ran ? state.exitStatus : EXIT_FAILURE;
        }

        const BOOL selectMode = argc == 3 && strcmp(argv[1], "select") == 0;
        const BOOL recordMode = argc == 11 && strcmp(argv[1], "record") == 0;
        if (!selectMode && !recordMode) {
            fprintf(stderr,
                    "用法：ottercut-macos-capture select <display|window> | "
                    "preview-camera <id> [circle|square] | "
                    "record <output.mp4> <microphone-id|silent> <source-json|picker> "
                    "<click-follow|no-click-follow> <camera-id> <camera-output> "
                    "<width> <height> <fps-milli>\n");
            return EXIT_FAILURE;
        }

        NSURL *outputURL = nil;
        NSString *microphoneDeviceID = nil;
        NSDictionary<NSString *, id> *sourceSelection = nil;
        BOOL clickFollow = NO;
        NSString *cameraDevice = nil;
        NSURL *cameraOutput = nil;
        NSInteger cameraWidth = 0;
        NSInteger cameraHeight = 0;
        NSInteger cameraFPSMilli = 0;
        if (recordMode) {
            NSString *fileName = [[NSFileManager defaultManager]
                stringWithFileSystemRepresentation:argv[2]
                                            length:strlen(argv[2])];
            if (!fileName.length || [[NSFileManager defaultManager] fileExistsAtPath:fileName]) {
                fprintf(stderr, "录制输出路径无效或文件已存在。\n");
                return EXIT_FAILURE;
            }
            outputURL = [NSURL fileURLWithPath:fileName];
            if (strcmp(argv[3], "silent") != 0) {
                microphoneDeviceID = [NSString stringWithUTF8String:argv[3]];
            }
            if (strcmp(argv[4], "picker") != 0) {
                NSData *selectionData = [[NSString stringWithUTF8String:argv[4]]
                    dataUsingEncoding:NSUTF8StringEncoding];
                NSError *error = nil;
                id object = [NSJSONSerialization JSONObjectWithData:selectionData
                                                            options:0
                                                              error:&error];
                if (![object isKindOfClass:NSDictionary.class]) {
                    fprintf(stderr, "录制来源信息无效：%s\n",
                            error.localizedDescription.UTF8String ?: "未知错误");
                    return EXIT_FAILURE;
                }
                sourceSelection = object;
            }
            clickFollow = strcmp(argv[5], "click-follow") == 0;
            if (argv[6][0] != '\0') {
                cameraDevice = [NSString stringWithUTF8String:argv[6]];
                NSString *cameraPath = [[NSFileManager defaultManager]
                    stringWithFileSystemRepresentation:argv[7] length:strlen(argv[7])];
                cameraOutput = [NSURL fileURLWithPath:cameraPath];
                cameraWidth = strtoll(argv[8], NULL, 10);
                cameraHeight = strtoll(argv[9], NULL, 10);
                cameraFPSMilli = strtoll(argv[10], NULL, 10);
            }
        }

        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        OtterCutCapture *capture = selectMode
            ? [[OtterCutCapture alloc] initForSourceSelection:[NSString stringWithUTF8String:argv[2]]]
            : [[OtterCutCapture alloc] initWithOutputURL:outputURL
                                     microphoneDeviceID:microphoneDeviceID
                                          sourceSelection:sourceSelection
                                              clickFollow:clickFollow
                                              cameraDevice:cameraDevice
                                              cameraOutput:cameraOutput
                                                cameraWidth:cameraWidth
                                               cameraHeight:cameraHeight
                                             cameraFPSMilli:cameraFPSMilli];
        dispatch_async(dispatch_get_main_queue(), ^{
            [app activate];
            if (selectMode || !sourceSelection) {
                [capture presentPicker];
            } else {
                [capture startRecordingFromSelection];
            }
        });
        if (recordMode) {
            dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
                char line[64];
                while (fgets(line, sizeof(line), stdin)) {
                    line[strcspn(line, "\r\n")] = '\0';
                    if (strcmp(line, "stop") == 0) {
                        break;
                    }
                    OtterCutCameraPreviewShape requestedShape;
                    if (parse_preview_shape_command(line, &requestedShape)) {
                        dispatch_async(dispatch_get_main_queue(), ^{
                            [capture setCameraPreviewShape:requestedShape];
                        });
                    } else if (strcmp(line, "show-camera-preview") == 0) {
                        dispatch_async(dispatch_get_main_queue(), ^{
                            [capture setCameraPreviewVisible:YES];
                        });
                    } else if (strcmp(line, "hide-camera-preview") == 0) {
                        dispatch_async(dispatch_get_main_queue(), ^{
                            [capture setCameraPreviewVisible:NO];
                        });
                    }
                }
                dispatch_async(dispatch_get_main_queue(), ^{
                    [capture requestStop];
                });
            });
        }
        [app run];
    }
    return EXIT_FAILURE;
}
