#import <AVFoundation/AVFoundation.h>
#import <AppKit/AppKit.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <dispatch/dispatch.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void emit_event(const char *event) {
    puts(event);
    fflush(stdout);
}

@interface OtterCutCapture : NSObject <
    SCContentSharingPickerObserver, SCRecordingOutputDelegate, SCStreamDelegate
>
- (instancetype)initWithOutputURL:(NSURL *)url microphone:(BOOL)microphone;
- (void)presentPicker;
- (void)requestStop;
@end

@implementation OtterCutCapture {
    NSURL *_outputURL;
    SCStream *_stream;
    SCRecordingOutput *_recordingOutput;
    BOOL _microphone;
    BOOL _selectedSource;
    BOOL _captureStarted;
    BOOL _recordingStarted;
    BOOL _stopRequested;
    BOOL _stopping;
}

- (instancetype)initWithOutputURL:(NSURL *)url microphone:(BOOL)microphone {
    self = [super init];
    if (self) {
        _outputURL = url;
        _microphone = microphone;
    }
    return self;
}

- (void)finishWithStatus:(int)status {
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
    configuration.allowedPickerModes =
        SCContentSharingPickerModeSingleDisplay | SCContentSharingPickerModeSingleWindow;
    configuration.allowsChangingSelectedContent = NO;
    picker.defaultConfiguration = configuration;
    picker.maximumStreamCount = @1;
    [picker addObserver:self];
    picker.active = YES;
    [picker present];
}

- (void)startWithFilter:(SCContentFilter *)filter {
    SCStreamConfiguration *configuration = [SCStreamConfiguration new];
    const CGRect rect = filter.contentRect;
    const CGFloat scale = filter.pointPixelScale;
    // H.264 needs even dimensions. Keep the selected source's native aspect ratio.
    configuration.width = MAX(2, (NSInteger)ceil(CGRectGetWidth(rect) * scale / 2) * 2);
    configuration.height = MAX(2, (NSInteger)ceil(CGRectGetHeight(rect) * scale / 2) * 2);
    configuration.minimumFrameInterval = CMTimeMake(1, 30);
    configuration.showsCursor = YES;
    configuration.captureMicrophone = _microphone;

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
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC),
                   dispatch_get_main_queue(), ^{
        [self fail:@"等待录制文件写入超时" error:nil];
    });
}

- (void)requestStop {
    _stopRequested = YES;
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
        if (!self->_microphone) {
            [self startWithFilter:filter];
            return;
        }
        [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio
                                 completionHandler:^(BOOL granted) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (granted) {
                    [self startWithFilter:filter];
                } else {
                    [self fail:@"麦克风权限未授予，请在系统设置中授权或关闭麦克风后重试"
                         error:nil];
                }
            });
        }];
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
        emit_event("recording");
    });
}

- (void)recordingOutputDidFinishRecording:(SCRecordingOutput *)recordingOutput {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self->_recordingStarted) {
            [self finishWithStatus:EXIT_SUCCESS];
        } else {
            [self cancel];
        }
    });
}

- (void)recordingOutput:(SCRecordingOutput *)recordingOutput didFailWithError:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{
        [self fail:@"写入录像失败" error:error];
    });
}
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 3 || (strcmp(argv[2], "mic") != 0 && strcmp(argv[2], "silent") != 0)) {
            fprintf(stderr, "用法：ottercut-macos-capture <output.mp4> <mic|silent>\n");
            return EXIT_FAILURE;
        }
        NSString *fileName = [[NSFileManager defaultManager]
            stringWithFileSystemRepresentation:argv[1]
                                        length:strlen(argv[1])];
        if (!fileName.length || [[NSFileManager defaultManager] fileExistsAtPath:fileName]) {
            fprintf(stderr, "录制输出路径无效或文件已存在。\n");
            return EXIT_FAILURE;
        }
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        OtterCutCapture *capture = [[OtterCutCapture alloc]
            initWithOutputURL:[NSURL fileURLWithPath:fileName]
                   microphone:strcmp(argv[2], "mic") == 0];
        dispatch_async(dispatch_get_main_queue(), ^{
            [app activate];
            [capture presentPicker];
        });
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            char line[64];
            while (fgets(line, sizeof(line), stdin)) {
                if (strcmp(line, "stop\n") == 0 || strcmp(line, "stop\r\n") == 0) {
                    break;
                }
            }
            dispatch_async(dispatch_get_main_queue(), ^{
                [capture requestStop];
            });
        });
        [app run];
    }
    return EXIT_FAILURE;
}
