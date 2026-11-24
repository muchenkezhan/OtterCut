#import "camera_capture.h"

#import <AVFoundation/AVFoundation.h>
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <dispatch/dispatch.h>
#import <math.h>
#import <pthread.h>
#import <stdint.h>
#import <stdlib.h>
#import <string.h>

static NSString *const OtterCutCameraErrorDomain = @"io.github.muchenkezhan.ottercut.camera";

static NSError *camera_error(NSInteger code, NSString *description) {
    return [NSError errorWithDomain:OtterCutCameraErrorDomain
                               code:code
                           userInfo:@{NSLocalizedDescriptionKey : description}];
}

static void set_error_message(char **destination, NSError *error) {
    if (destination) {
        *destination = strdup(error.localizedDescription.UTF8String ?: "Camera operation failed");
    }
}

static NSError *error_from_exception(NSException *exception) {
    return camera_error(13, exception.reason ?: @"An AVFoundation operation failed");
}

static NSArray<AVCaptureDevice *> *camera_devices(void) {
    AVCaptureDeviceDiscoverySession *discovery =
        [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:@[
            AVCaptureDeviceTypeBuiltInWideAngleCamera,
            AVCaptureDeviceTypeExternal,
            AVCaptureDeviceTypeContinuityCamera,
            AVCaptureDeviceTypeDeskViewCamera,
        ]
                                                            mediaType:AVMediaTypeVideo
                                                             position:AVCaptureDevicePositionUnspecified];
    return discovery.devices;
}

static AVCaptureDevice *camera_device(NSString *uniqueID) {
    for (AVCaptureDevice *device in camera_devices()) {
        if ([device.uniqueID isEqualToString:uniqueID]) {
            return device;
        }
    }
    return nil;
}

static AVCaptureDeviceFormat *camera_format(AVCaptureDevice *device,
                                             NSInteger width,
                                             NSInteger height,
                                             NSInteger framesPerSecondMilli) {
    for (AVCaptureDeviceFormat *format in device.formats) {
        CMVideoDimensions dimensions = CMVideoFormatDescriptionGetDimensions(format.formatDescription);
        if (dimensions.width != width || dimensions.height != height) {
            continue;
        }
        double framesPerSecond = framesPerSecondMilli / 1000.0;
        for (AVFrameRateRange *range in format.videoSupportedFrameRateRanges) {
            if (framesPerSecond >= range.minFrameRate && framesPerSecond <= range.maxFrameRate) {
                return format;
            }
        }
    }
    return nil;
}

static BOOL camera_mode_exists(const OtterCutCameraMode *modes,
                               size_t count,
                               int width,
                               int height,
                               int fpsMilli) {
    for (size_t index = 0; index < count; ++index) {
        if (modes[index].width == width && modes[index].height == height &&
            modes[index].fps_milli == fpsMilli) {
            return YES;
        }
    }
    return NO;
}

static BOOL append_camera_mode(OtterCutCameraMode **modes,
                               size_t *count,
                               size_t *capacity,
                               int width,
                               int height,
                               double framesPerSecond) {
    if (width <= 0 || height <= 0 || !isfinite(framesPerSecond) ||
        framesPerSecond < 0.001 || framesPerSecond > 1000.0) {
        return YES;
    }
    int fpsMilli = (int)llround(framesPerSecond * 1000.0);
    if (camera_mode_exists(*modes, *count, width, height, fpsMilli)) {
        return YES;
    }
    if (*count == *capacity) {
        size_t nextCapacity = *capacity == 0 ? 16 : *capacity * 2;
        OtterCutCameraMode *resized = realloc(*modes, nextCapacity * sizeof(**modes));
        if (!resized) {
            return NO;
        }
        *modes = resized;
        *capacity = nextCapacity;
    }
    (*modes)[*count] = (OtterCutCameraMode){
        .width = width,
        .height = height,
        .fps_milli = fpsMilli,
    };
    ++*count;
    return YES;
}

static int compare_camera_modes(const void *left, const void *right) {
    const OtterCutCameraMode *first = left;
    const OtterCutCameraMode *second = right;
    uint64_t firstArea = (uint64_t)first->width * (uint64_t)first->height;
    uint64_t secondArea = (uint64_t)second->width * (uint64_t)second->height;
    if (firstArea != secondArea) {
        return firstArea > secondArea ? -1 : 1;
    }
    if (first->width != second->width) {
        return first->width > second->width ? -1 : 1;
    }
    if (first->fps_milli != second->fps_milli) {
        return first->fps_milli > second->fps_milli ? -1 : 1;
    }
    return 0;
}

static void on_main_thread(dispatch_block_t block) {
    if (pthread_main_np()) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

int ottercut_camera_list_devices(OtterCutCameraDeviceList *result, char **error_message) {
    if (result) {
        result->count = 0;
        result->devices = NULL;
    }
    if (error_message) {
        *error_message = NULL;
    }
    if (!result) {
        set_error_message(error_message, camera_error(1, @"Device list output is required"));
        return 0;
    }

    NSArray<AVCaptureDevice *> *devices = camera_devices();
    OtterCutCameraDevice *items = calloc(devices.count, sizeof(*items));
    if (devices.count && !items) {
        set_error_message(error_message, camera_error(2, @"Unable to allocate camera device list"));
        return 0;
    }

    for (NSUInteger index = 0; index < devices.count; ++index) {
        AVCaptureDevice *device = devices[index];
        items[index].unique_id = strdup(device.uniqueID.UTF8String ?: "");
        items[index].name = strdup(device.localizedName.UTF8String ?: "");
        if (!items[index].unique_id || !items[index].name) {
            result->count = devices.count;
            result->devices = items;
            ottercut_camera_free_device_list(result);
            set_error_message(error_message, camera_error(2, @"Unable to allocate camera device names"));
            return 0;
        }
    }

    result->count = devices.count;
    result->devices = items;
    return 1;
}

void ottercut_camera_free_device_list(OtterCutCameraDeviceList *list) {
    if (!list) {
        return;
    }
    for (size_t index = 0; index < list->count; ++index) {
        free(list->devices[index].unique_id);
        free(list->devices[index].name);
    }
    free(list->devices);
    list->devices = NULL;
    list->count = 0;
}

int ottercut_camera_list_modes(const char *device_unique_id,
                               OtterCutCameraModeList *result,
                               char **error_message) {
    if (result) {
        result->count = 0;
        result->modes = NULL;
    }
    if (error_message) {
        *error_message = NULL;
    }
    if (!result || !device_unique_id) {
        set_error_message(error_message, camera_error(1, @"Camera ID and mode list output are required"));
        return 0;
    }
    NSString *uniqueID = [NSString stringWithUTF8String:device_unique_id];
    if (!uniqueID.length) {
        set_error_message(error_message, camera_error(1, @"Camera ID must be valid UTF-8"));
        return 0;
    }
    AVCaptureDevice *device = camera_device(uniqueID);
    if (!device) {
        set_error_message(error_message, camera_error(6, @"The selected camera is no longer available"));
        return 0;
    }

    OtterCutCameraMode *modes = NULL;
    size_t count = 0;
    size_t capacity = 0;
    static const int commonFrameRatesMilli[] = {
        12000, 15000, 23976, 24000, 25000, 29970, 30000,
        48000, 50000, 59940, 60000, 90000, 120000, 240000,
    };
    for (AVCaptureDeviceFormat *format in device.formats) {
        CMVideoDimensions dimensions = CMVideoFormatDescriptionGetDimensions(format.formatDescription);
        for (AVFrameRateRange *range in format.videoSupportedFrameRateRanges) {
            if (!append_camera_mode(&modes, &count, &capacity,
                                    dimensions.width, dimensions.height, range.minFrameRate) ||
                !append_camera_mode(&modes, &count, &capacity,
                                    dimensions.width, dimensions.height, range.maxFrameRate)) {
                free(modes);
                set_error_message(error_message, camera_error(2, @"Unable to allocate camera modes"));
                return 0;
            }
            for (size_t index = 0;
                 index < sizeof(commonFrameRatesMilli) / sizeof(commonFrameRatesMilli[0]);
                 ++index) {
                double framesPerSecond = commonFrameRatesMilli[index] / 1000.0;
                if (framesPerSecond >= range.minFrameRate && framesPerSecond <= range.maxFrameRate &&
                    !append_camera_mode(&modes, &count, &capacity, dimensions.width,
                                        dimensions.height, framesPerSecond)) {
                    free(modes);
                    set_error_message(error_message, camera_error(2, @"Unable to allocate camera modes"));
                    return 0;
                }
            }
        }
    }
    qsort(modes, count, sizeof(*modes), compare_camera_modes);
    result->count = count;
    result->modes = modes;
    return 1;
}

void ottercut_camera_free_mode_list(OtterCutCameraModeList *list) {
    if (!list) {
        return;
    }
    free(list->modes);
    list->modes = NULL;
    list->count = 0;
}

void ottercut_camera_free_string(char *string) {
    free(string);
}

typedef NS_ENUM(NSInteger, OtterCutCaptureState) {
    OtterCutCaptureStateIdle,
    OtterCutCaptureStateWaitingForPermission,
    OtterCutCaptureStateStarting,
    OtterCutCaptureStateRecording,
    OtterCutCaptureStateStopping,
    OtterCutCaptureStateFinished,
};

@interface OtterCutCameraCaptureController : NSObject <AVCaptureFileOutputRecordingDelegate>
- (instancetype)initWithDeviceID:(NSString *)deviceID
                       outputURL:(NSURL *)outputURL
                           width:(NSInteger)width
                          height:(NSInteger)height
               framesPerSecondMilli:(NSInteger)framesPerSecondMilli
                       onStarted:(OtterCutCameraStartedCallback)onStarted
                      onFinished:(OtterCutCameraFinishedCallback)onFinished
                         context:(void *)context;
- (BOOL)acceptStart;
- (void)requestStop;
- (void)previewLayerWithCompletion:(void (^)(AVCaptureVideoPreviewLayer *layer))completion;
@end

@implementation OtterCutCameraCaptureController {
    dispatch_queue_t _queue;
    NSString *_deviceID;
    NSURL *_outputURL;
    NSInteger _width;
    NSInteger _height;
    NSInteger _framesPerSecondMilli;
    OtterCutCameraStartedCallback _onStarted;
    OtterCutCameraFinishedCallback _onFinished;
    void *_context;
    OtterCutCaptureState _state;
    BOOL _stopRequested;
    AVCaptureSession *_session;
    AVCaptureMovieFileOutput *_output;
    AVCaptureDevice *_device;
    AVCaptureDeviceFormat *_originalFormat;
    CMTime _originalMinFrameDuration;
    CMTime _originalMaxFrameDuration;
    NSURL *_temporaryMovieURL;
    NSURL *_temporaryMP4URL;
    AVAssetExportSession *_exportSession;
}

- (instancetype)initWithDeviceID:(NSString *)deviceID
                       outputURL:(NSURL *)outputURL
                           width:(NSInteger)width
                          height:(NSInteger)height
               framesPerSecondMilli:(NSInteger)framesPerSecondMilli
                       onStarted:(OtterCutCameraStartedCallback)onStarted
                      onFinished:(OtterCutCameraFinishedCallback)onFinished
                         context:(void *)context {
    self = [super init];
    if (self) {
        _queue = dispatch_queue_create("io.github.ottercut.camera-capture", DISPATCH_QUEUE_SERIAL);
        _deviceID = [deviceID copy];
        _outputURL = [outputURL copy];
        _width = width;
        _height = height;
        _framesPerSecondMilli = framesPerSecondMilli;
        _onStarted = onStarted;
        _onFinished = onFinished;
        _context = context;
        _state = OtterCutCaptureStateIdle;
    }
    return self;
}

- (BOOL)acceptStart {
    @synchronized(self) {
        if (_state != OtterCutCaptureStateIdle) {
            return NO;
        }
        _state = OtterCutCaptureStateWaitingForPermission;
    }
    dispatch_async(_queue, ^{
        AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
        if (status == AVAuthorizationStatusAuthorized) {
            [self configureAndRecord];
        } else if (status == AVAuthorizationStatusNotDetermined) {
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) {
                dispatch_async(self->_queue, ^{
                    BOOL finished;
                    BOOL stopRequested;
                    @synchronized(self) {
                        finished = self->_state == OtterCutCaptureStateFinished;
                        stopRequested = self->_stopRequested;
                    }
                    if (finished) {
                        return;
                    }
                    if (stopRequested) {
                        [self finishWithError:camera_error(4, @"Camera recording was cancelled before it started")];
                        return;
                    }
                    if (!granted) {
                        [self finishWithError:camera_error(3, @"Camera access was not granted")];
                        return;
                    }
                    [self configureAndRecord];
                });
            }];
        } else {
            [self finishWithError:camera_error(3, @"Camera access is denied. Allow camera access in System Settings and retry.")];
        }
    });
    return YES;
}

- (void)configureAndRecord {
    @synchronized(self) {
        if (_state == OtterCutCaptureStateFinished || _stopRequested) {
            [self finishWithError:camera_error(4, @"Camera recording was cancelled before it started")];
            return;
        }
    }
    if (![self->_outputURL.pathExtension.lowercaseString isEqualToString:@"mp4"]) {
        [self finishWithError:camera_error(5, @"Camera output path must end in .mp4")];
        return;
    }
    if ([[NSFileManager defaultManager] fileExistsAtPath:self->_outputURL.path]) {
        [self finishWithError:camera_error(5, @"The camera output file already exists")];
        return;
    }

    AVCaptureDevice *device = camera_device(self->_deviceID);
    if (!device) {
        [self finishWithError:camera_error(6, @"The selected camera is no longer available")];
        return;
    }
    AVCaptureDeviceFormat *format = camera_format(device, self->_width, self->_height,
                                                   self->_framesPerSecondMilli);
    if (!format) {
        [self finishWithError:camera_error(7, @"The selected camera does not support the requested size and frame rate")];
        return;
    }

    NSError *error = nil;
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
    if (!input) {
        [self finishWithError:error ?: camera_error(9, @"Unable to open the selected camera")];
        return;
    }

    AVCaptureSession *session = [AVCaptureSession new];
    [session beginConfiguration];
    if (![session canAddInput:input]) {
        [session commitConfiguration];
        [self finishWithError:camera_error(10, @"Unable to add the selected camera to the capture session")];
        return;
    }
    [session addInput:input];

    if (![device lockForConfiguration:&error]) {
        [session commitConfiguration];
        [self finishWithError:error ?: camera_error(8, @"Unable to configure the selected camera")];
        return;
    }
    self->_device = device;
    self->_originalFormat = device.activeFormat;
    self->_originalMinFrameDuration = device.activeVideoMinFrameDuration;
    self->_originalMaxFrameDuration = device.activeVideoMaxFrameDuration;
    @try {
        device.activeFormat = format;
        CMTime frameDuration = CMTimeMake(1000, (int32_t)self->_framesPerSecondMilli);
        device.activeVideoMinFrameDuration = frameDuration;
        device.activeVideoMaxFrameDuration = frameDuration;
    } @catch (NSException *exception) {
        [device unlockForConfiguration];
        [session commitConfiguration];
        [self finishWithError:error_from_exception(exception)];
        return;
    }
    [device unlockForConfiguration];

    AVCaptureMovieFileOutput *output = [AVCaptureMovieFileOutput new];
    if (![session canAddOutput:output]) {
        [session commitConfiguration];
        [self finishWithError:camera_error(11, @"Unable to add movie recording to the capture session")];
        return;
    }
    [session addOutput:output];
    AVCaptureConnection *videoConnection = [output connectionWithMediaType:AVMediaTypeVideo];
    if (videoConnection.supportsVideoMinFrameDuration) {
        videoConnection.videoMinFrameDuration = CMTimeMake(1000, (int32_t)self->_framesPerSecondMilli);
    }
    [session commitConfiguration];

    self->_session = session;
    self->_output = output;
    NSString *temporaryName = [NSString stringWithFormat:@".ottercut-camera-%@.mov", NSUUID.UUID.UUIDString];
    self->_temporaryMovieURL = [self->_outputURL.URLByDeletingLastPathComponent URLByAppendingPathComponent:temporaryName];
    NSString *temporaryMP4Name = [NSString stringWithFormat:@".ottercut-camera-%@.mp4", NSUUID.UUID.UUIDString];
    self->_temporaryMP4URL = [self->_outputURL.URLByDeletingLastPathComponent URLByAppendingPathComponent:temporaryMP4Name];
    @synchronized(self) {
        self->_state = OtterCutCaptureStateStarting;
    }
    [session startRunning];
    if (!session.isRunning) {
        [self finishWithError:camera_error(14, @"AVFoundation could not start the camera session")];
        return;
    }
    [output startRecordingToOutputFileURL:self->_temporaryMovieURL recordingDelegate:self];
    BOOL stopRequested;
    @synchronized(self) {
        stopRequested = self->_stopRequested;
    }
    if (stopRequested) {
        [self stopRecordingIfReady];
    }
}

- (void)requestStop {
    dispatch_async(_queue, ^{
        BOOL shouldFinishWithoutStarting = NO;
        @synchronized(self) {
            if (self->_state == OtterCutCaptureStateFinished || self->_state == OtterCutCaptureStateStopping) {
                return;
            }
            self->_stopRequested = YES;
            if (self->_state == OtterCutCaptureStateIdle ||
                self->_state == OtterCutCaptureStateWaitingForPermission) {
                self->_state = OtterCutCaptureStateStopping;
                shouldFinishWithoutStarting = YES;
            }
        }
        if (shouldFinishWithoutStarting) {
            [self finishWithError:camera_error(4, @"Camera recording was cancelled before it started")];
        } else if (self->_state == OtterCutCaptureStateRecording ||
                   self->_state == OtterCutCaptureStateStarting) {
            [self stopRecordingIfReady];
        }
    });
}

- (void)stopRecordingIfReady {
    if (self->_state == OtterCutCaptureStateStarting && self->_output.isRecording) {
        @synchronized(self) { self->_state = OtterCutCaptureStateStopping; }
        [self->_output stopRecording];
    } else if (self->_state == OtterCutCaptureStateRecording && self->_output.isRecording) {
        @synchronized(self) { self->_state = OtterCutCaptureStateStopping; }
        [self->_output stopRecording];
    }
}

- (void)previewLayerWithCompletion:(void (^)(AVCaptureVideoPreviewLayer *layer))completion {
    dispatch_async(_queue, ^{
        AVCaptureSession *session = self->_state == OtterCutCaptureStateRecording
            ? self->_session
            : nil;
        // Connecting the preview layer also changes the camera's capture graph.
        AVCaptureVideoPreviewLayer *layer = session
            ? [AVCaptureVideoPreviewLayer layerWithSession:session]
            : nil;
        dispatch_async(dispatch_get_main_queue(), ^{
            completion(layer);
        });
    });
}

- (void)captureOutput:(AVCaptureFileOutput *)output
didStartRecordingToOutputFileAtURL:(NSURL *)fileURL
       fromConnections:(NSArray<AVCaptureConnection *> *)connections {
    dispatch_async(_queue, ^{
        @synchronized(self) {
            if (self->_state == OtterCutCaptureStateFinished) {
                return;
            }
            self->_state = OtterCutCaptureStateRecording;
        }
        OtterCutCameraStartedCallback callback = self->_onStarted;
        void *context = self->_context;
        if (callback) {
            dispatch_async(dispatch_get_main_queue(), ^{
                callback(context);
            });
        }
        BOOL stopRequested;
        @synchronized(self) { stopRequested = self->_stopRequested; }
        if (stopRequested) {
            [self stopRecordingIfReady];
        }
    });
}

- (void)captureOutput:(AVCaptureFileOutput *)output
didFinishRecordingToOutputFileAtURL:(NSURL *)outputFileURL
       fromConnections:(NSArray<AVCaptureConnection *> *)connections
                 error:(NSError *)error {
    dispatch_async(_queue, ^{
        @synchronized(self) {
            if (self->_state == OtterCutCaptureStateFinished) {
                return;
            }
        }
        BOOL completed = !error || [error.userInfo[AVErrorRecordingSuccessfullyFinishedKey] boolValue];
        if (completed) {
            [self exportTemporaryMovie];
        } else {
            [self finishWithError:error];
        }
    });
}

- (void)exportTemporaryMovie {
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:self->_temporaryMovieURL options:nil];
    AVAssetExportSession *exporter = [[AVAssetExportSession alloc]
        initWithAsset:asset
           presetName:AVAssetExportPresetPassthrough];
    if (!exporter || ![exporter.supportedFileTypes containsObject:AVFileTypeMPEG4]) {
        exporter = [[AVAssetExportSession alloc]
            initWithAsset:asset
               presetName:AVAssetExportPresetHighestQuality];
    }
    if (!exporter || ![exporter.supportedFileTypes containsObject:AVFileTypeMPEG4]) {
        [self finishWithError:camera_error(15, @"AVFoundation cannot export the camera recording as MP4")];
        return;
    }
    self->_exportSession = exporter;
    exporter.outputURL = self->_temporaryMP4URL;
    exporter.outputFileType = AVFileTypeMPEG4;
    exporter.shouldOptimizeForNetworkUse = YES;
    [exporter exportAsynchronouslyWithCompletionHandler:^{
        dispatch_async(self->_queue, ^{
            NSError *error = nil;
            if (exporter.status == AVAssetExportSessionStatusCompleted) {
                if (![[NSFileManager defaultManager] moveItemAtURL:self->_temporaryMP4URL
                                                             toURL:self->_outputURL
                                                             error:&error]) {
                    error = error ?: camera_error(16, @"Unable to publish the camera MP4 file");
                }
            } else {
                error = exporter.error ?: camera_error(16, @"Unable to finalize the camera MP4 file");
            }
            [[NSFileManager defaultManager] removeItemAtURL:self->_temporaryMovieURL error:nil];
            [[NSFileManager defaultManager] removeItemAtURL:self->_temporaryMP4URL error:nil];
            self->_exportSession = nil;
            [self finishWithError:error];
        });
    }];
}

- (void)finishWithError:(NSError *)error {
    @synchronized(self) {
        if (self->_state == OtterCutCaptureStateFinished) {
            return;
        }
        self->_state = OtterCutCaptureStateFinished;
    }
    if (self->_session.isRunning) {
        [self->_session stopRunning];
    }
    if (self->_device && self->_originalFormat && [self->_device lockForConfiguration:nil]) {
        @try {
            self->_device.activeFormat = self->_originalFormat;
            self->_device.activeVideoMinFrameDuration = self->_originalMinFrameDuration;
            self->_device.activeVideoMaxFrameDuration = self->_originalMaxFrameDuration;
        } @catch (__unused NSException *exception) {
        }
        [self->_device unlockForConfiguration];
    }
    if (error && self->_temporaryMovieURL) {
        [[NSFileManager defaultManager] removeItemAtURL:self->_temporaryMovieURL error:nil];
    }
    if (error && self->_temporaryMP4URL) {
        [[NSFileManager defaultManager] removeItemAtURL:self->_temporaryMP4URL error:nil];
    }
    self->_session = nil;
    self->_output = nil;
    self->_device = nil;
    self->_originalFormat = nil;

    OtterCutCameraFinishedCallback callback = self->_onFinished;
    void *context = self->_context;
    NSString *message = [error.localizedDescription copy];
    if (callback) {
        dispatch_async(dispatch_get_main_queue(), ^{
            callback(message.UTF8String, context);
        });
    }
}

@end

OtterCutCameraCapture ottercut_camera_capture_create(
    const char *device_unique_id,
    const char *output_path,
    int width,
    int height,
    int frames_per_second_milli,
    OtterCutCameraStartedCallback on_started,
    OtterCutCameraFinishedCallback on_finished,
    void *context,
    char **error_message) {
    if (error_message) {
        *error_message = NULL;
    }
    if (!device_unique_id || !output_path || width <= 0 || height <= 0 || frames_per_second_milli <= 0) {
        set_error_message(error_message, camera_error(1, @"Camera ID, output path, size, and frame rate are required"));
        return NULL;
    }
    NSString *deviceID = [NSString stringWithUTF8String:device_unique_id];
    NSString *path = [NSString stringWithUTF8String:output_path];
    if (!deviceID.length || !path.length) {
        set_error_message(error_message, camera_error(1, @"Camera ID and output path must be valid UTF-8"));
        return NULL;
    }
    OtterCutCameraCaptureController *capture = [[OtterCutCameraCaptureController alloc]
        initWithDeviceID:deviceID
               outputURL:[NSURL fileURLWithPath:path]
                   width:width
                  height:height
        framesPerSecondMilli:frames_per_second_milli
               onStarted:on_started
              onFinished:on_finished
                 context:context];
    return (OtterCutCameraCapture)CFBridgingRetain(capture);
}

int ottercut_camera_capture_start(OtterCutCameraCapture capture) {
    if (!capture) {
        return 0;
    }
    return [(__bridge OtterCutCameraCaptureController *)capture acceptStart] ? 1 : 0;
}

void ottercut_camera_capture_request_stop(OtterCutCameraCapture capture) {
    if (capture) {
        [(__bridge OtterCutCameraCaptureController *)capture requestStop];
    }
}

void ottercut_camera_capture_destroy(OtterCutCameraCapture capture) {
    if (capture) {
        CFBridgingRelease(capture);
    }
}

void ottercut_camera_capture_preview_layer(
    OtterCutCameraCapture capture,
    void (^completion)(AVCaptureVideoPreviewLayer *layer)) {
    [(__bridge OtterCutCameraCaptureController *)capture previewLayerWithCompletion:completion];
}

@interface OtterCutCameraPanel : NSPanel
@end

@implementation OtterCutCameraPanel

- (BOOL)canBecomeKeyWindow {
    return YES;
}

- (void)performClose:(id)sender {
    id<NSWindowDelegate> delegate = self.delegate;
    if (![delegate respondsToSelector:@selector(windowShouldClose:)] ||
        [delegate windowShouldClose:self]) {
        [self close];
    }
}

- (void)cancelOperation:(id)sender {
    [self performClose:sender];
}

- (BOOL)performKeyEquivalent:(NSEvent *)event {
    NSEventModifierFlags modifiers = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
    if (modifiers == NSEventModifierFlagCommand &&
        [event.charactersIgnoringModifiers isEqualToString:@"w"]) {
        [self performClose:nil];
        return YES;
    }
    return [super performKeyEquivalent:event];
}

@end

@interface OtterCutCameraPreviewView : NSView
@end

@implementation OtterCutCameraPreviewView

- (BOOL)mouseDownCanMoveWindow {
    return YES;
}

@end

NSPanel *ottercut_camera_preview_panel(NSString *title) {
    NSRect frame = NSMakeRect(0, 0, 220, 220);
    NSPanel *panel = [[OtterCutCameraPanel alloc]
        initWithContentRect:frame
                  styleMask:NSWindowStyleMaskBorderless |
                            NSWindowStyleMaskNonactivatingPanel
                    backing:NSBackingStoreBuffered
                      defer:NO];
    panel.title = title;
    panel.releasedWhenClosed = NO;
    panel.floatingPanel = YES;
    panel.opaque = NO;
    panel.backgroundColor = NSColor.clearColor;
    panel.hasShadow = NO;
    panel.movableByWindowBackground = YES;
    panel.level = NSFloatingWindowLevel;
    panel.hidesOnDeactivate = NO;
    panel.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                               NSWindowCollectionBehaviorFullScreenAuxiliary;

    NSView *content = [[OtterCutCameraPreviewView alloc] initWithFrame:frame];
    content.wantsLayer = YES;
    content.layer.backgroundColor = NSColor.blackColor.CGColor;
    panel.contentView = content;
    ottercut_camera_preview_panel_set_shape(panel, OtterCutCameraPreviewCircle);

    NSScreen *screen = NSScreen.mainScreen;
    if (screen) {
        NSRect visible = screen.visibleFrame;
        [panel setFrameTopLeftPoint:NSMakePoint(NSMaxX(visible) - NSWidth(panel.frame) - 20,
                                               NSMaxY(visible) - 20)];
    } else {
        [panel center];
    }
    return panel;
}

void ottercut_camera_preview_panel_set_shape(NSPanel *panel,
                                             OtterCutCameraPreviewShape shape) {
    NSView *content = panel.contentView;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    content.layer.masksToBounds = YES;
    content.layer.cornerRadius = shape == OtterCutCameraPreviewCircle
        ? NSWidth(content.bounds) / 2
        : 22;
    [CATransaction commit];
}

void ottercut_camera_preview_attach_layer(NSPanel *panel, AVCaptureVideoPreviewLayer *layer) {
    NSView *content = panel.contentView;
    layer.videoGravity = AVLayerVideoGravityResizeAspectFill;
    layer.frame = content.bounds;
    layer.autoresizingMask = kCALayerWidthSizable | kCALayerHeightSizable;
    [content.layer addSublayer:layer];
}

typedef NS_ENUM(NSInteger, OtterCutPreviewState) {
    OtterCutPreviewStateIdle,
    OtterCutPreviewStateWaitingForPermission,
    OtterCutPreviewStateStarting,
    OtterCutPreviewStateRunning,
    OtterCutPreviewStateClosed,
};

@interface OtterCutCameraPreviewController : NSObject <NSWindowDelegate>
- (instancetype)initWithDeviceID:(NSString *)deviceID title:(NSString *)title;
- (BOOL)startWithCallback:(OtterCutCameraPreviewCallback)callback context:(void *)context;
- (void)setShape:(OtterCutCameraPreviewShape)shape;
- (void)stop;
- (void)close;
- (int)run;
@end

@implementation OtterCutCameraPreviewController {
    dispatch_queue_t _queue;
    NSString *_deviceID;
    NSPanel *_window;
    AVCaptureVideoPreviewLayer *_previewLayer;
    AVCaptureSession *_session;
    OtterCutPreviewState _state;
    OtterCutCameraPreviewCallback _startCallback;
    void *_startContext;
}

- (instancetype)initWithDeviceID:(NSString *)deviceID title:(NSString *)title {
    self = [super init];
    if (!self) {
        return nil;
    }
    _queue = dispatch_queue_create("io.github.ottercut.camera-preview", DISPATCH_QUEUE_SERIAL);
    _deviceID = [deviceID copy];
    _state = OtterCutPreviewStateIdle;

    NSApplication *application = [NSApplication sharedApplication];
    [application setActivationPolicy:NSApplicationActivationPolicyAccessory];
    [application finishLaunching];
    _window = ottercut_camera_preview_panel(title.length ? title : @"OtterCut Camera Preview");
    _window.delegate = self;
    return self;
}

- (BOOL)startWithCallback:(OtterCutCameraPreviewCallback)callback context:(void *)context {
    @synchronized(self) {
        if (_state != OtterCutPreviewStateIdle) {
            return NO;
        }
        _state = OtterCutPreviewStateWaitingForPermission;
    }
    dispatch_async(_queue, ^{
        self->_startCallback = callback;
        self->_startContext = context;
        AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
        if (status == AVAuthorizationStatusAuthorized) {
            [self configurePreview];
        } else if (status == AVAuthorizationStatusNotDetermined) {
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) {
                dispatch_async(self->_queue, ^{
                    BOOL cancelled;
                    @synchronized(self) {
                        cancelled = self->_state != OtterCutPreviewStateWaitingForPermission;
                    }
                    if (cancelled) {
                        return;
                    }
                    if (granted) {
                        [self configurePreview];
                    } else {
                        [self completeStartWithError:camera_error(3, @"Camera access was not granted")];
                    }
                });
            }];
        } else {
            [self completeStartWithError:camera_error(3, @"Camera access is denied. Allow camera access in System Settings and retry.")];
        }
    });
    return YES;
}

- (void)setShape:(OtterCutCameraPreviewShape)shape {
    on_main_thread(^{
        ottercut_camera_preview_panel_set_shape(self->_window, shape);
    });
}

- (void)configurePreview {
    @synchronized(self) {
        if (self->_state == OtterCutPreviewStateClosed) {
            return;
        }
    }
    AVCaptureDevice *device = camera_device(self->_deviceID);
    if (!device) {
        [self completeStartWithError:camera_error(6, @"The selected camera is no longer available")];
        return;
    }
    NSError *error = nil;
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
    if (!input) {
        [self completeStartWithError:error ?: camera_error(9, @"Unable to open the selected camera")];
        return;
    }
    AVCaptureSession *session = [AVCaptureSession new];
    [session beginConfiguration];
    if ([session canSetSessionPreset:AVCaptureSessionPresetHigh]) {
        session.sessionPreset = AVCaptureSessionPresetHigh;
    }
    if (![session canAddInput:input]) {
        [session commitConfiguration];
        [self completeStartWithError:camera_error(10, @"Unable to add the selected camera to the preview session")];
        return;
    }
    [session addInput:input];
    [session commitConfiguration];
    @synchronized(self) {
        if (self->_state != OtterCutPreviewStateWaitingForPermission) {
            return;
        }
        self->_state = OtterCutPreviewStateStarting;
    }
    [session startRunning];
    if (!session.isRunning) {
        [self completeStartWithError:camera_error(14, @"AVFoundation could not start the camera session")];
        return;
    }
    self->_session = session;

    dispatch_async(dispatch_get_main_queue(), ^{
        BOOL canShowPreview;
        @synchronized(self) { canShowPreview = self->_state == OtterCutPreviewStateStarting; }
        if (!canShowPreview) {
            dispatch_async(self->_queue, ^{
                [self stopSession];
            });
            return;
        }
        AVCaptureVideoPreviewLayer *previewLayer = [AVCaptureVideoPreviewLayer layerWithSession:session];
        self->_previewLayer = previewLayer;
        ottercut_camera_preview_attach_layer(self->_window, previewLayer);
        [self->_window orderFrontRegardless];
        @synchronized(self) { self->_state = OtterCutPreviewStateRunning; }
        [self completeStartWithError:nil];
    });
}

- (void)completeStartWithError:(NSError *)error {
    if (error) {
        @synchronized(self) {
            if (self->_state != OtterCutPreviewStateClosed) {
                self->_state = OtterCutPreviewStateIdle;
            }
        }
    }
    OtterCutCameraPreviewCallback callback = self->_startCallback;
    void *context = self->_startContext;
    self->_startCallback = NULL;
    self->_startContext = NULL;
    if (callback) {
        NSString *message = [error.localizedDescription copy];
        dispatch_async(dispatch_get_main_queue(), ^{
            callback(message.UTF8String, context);
        });
    }
}

- (void)stop {
    dispatch_async(_queue, ^{
        @synchronized(self) {
            if (self->_state == OtterCutPreviewStateClosed ||
                self->_state == OtterCutPreviewStateIdle) {
                return;
            }
            self->_state = OtterCutPreviewStateIdle;
        }
        AVCaptureSession *session = self->_session;
        self->_session = nil;
        if (session.isRunning) {
            [session stopRunning];
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            if (self->_previewLayer.session == session) {
                self->_previewLayer.session = nil;
            }
        });
    });
}

- (void)stopSession {
    AVCaptureSession *session = self->_session;
    self->_session = nil;
    if (session.isRunning) {
        [session stopRunning];
    }
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self->_previewLayer.session == session) {
            self->_previewLayer.session = nil;
        }
    });
}

- (void)close {
    on_main_thread(^{
        @synchronized(self) {
            if (self->_state == OtterCutPreviewStateClosed) {
                return;
            }
            self->_state = OtterCutPreviewStateClosed;
        }
        [self->_window close];
        dispatch_async(self->_queue, ^{
            [self stopSession];
        });
    });
}

- (int)run {
    if (!pthread_main_np()) {
        return 0;
    }
    BOOL closed;
    @synchronized(self) { closed = self->_state == OtterCutPreviewStateClosed; }
    if (closed || !self->_window) {
        return 0;
    }
    [[NSApplication sharedApplication] run];
    return 1;
}

- (BOOL)windowShouldClose:(NSWindow *)sender {
    return YES;
}

- (void)windowWillClose:(NSNotification *)notification {
    dispatch_async(_queue, ^{
        @synchronized(self) { self->_state = OtterCutPreviewStateClosed; }
        [self stopSession];
    });
    [NSApp stop:nil];
    // NSApplication checks its run-loop stop flag after dispatching an event.
    NSEvent *wake = [NSEvent otherEventWithType:NSEventTypeApplicationDefined
                                      location:NSZeroPoint modifierFlags:0 timestamp:0
                                  windowNumber:0 context:nil subtype:0 data1:0 data2:0];
    [NSApp postEvent:wake atStart:YES];
}

@end

OtterCutCameraPreview ottercut_camera_preview_create(
    const char *device_unique_id,
    const char *window_title,
    char **error_message) {
    if (error_message) {
        *error_message = NULL;
    }
    if (!device_unique_id) {
        set_error_message(error_message, camera_error(1, @"Camera ID is required"));
        return NULL;
    }
    NSString *deviceID = [NSString stringWithUTF8String:device_unique_id];
    NSString *title = window_title ? [NSString stringWithUTF8String:window_title] : @"OtterCut Camera Preview";
    if (!deviceID.length || !title) {
        set_error_message(error_message, camera_error(1, @"Camera ID and window title must be valid UTF-8"));
        return NULL;
    }
    __block OtterCutCameraPreviewController *preview = nil;
    on_main_thread(^{
        preview = [[OtterCutCameraPreviewController alloc] initWithDeviceID:deviceID title:title];
    });
    if (!preview) {
        set_error_message(error_message, camera_error(12, @"Unable to create the camera preview window"));
        return NULL;
    }
    return (OtterCutCameraPreview)CFBridgingRetain(preview);
}

int ottercut_camera_preview_start(
    OtterCutCameraPreview preview,
    OtterCutCameraPreviewCallback on_started,
    void *context) {
    if (!preview) {
        return 0;
    }
    return [(__bridge OtterCutCameraPreviewController *)preview startWithCallback:on_started context:context] ? 1 : 0;
}

int ottercut_camera_preview_run(OtterCutCameraPreview preview) {
    if (!preview) {
        return 0;
    }
    __block int result = 0;
    on_main_thread(^{
        result = [(__bridge OtterCutCameraPreviewController *)preview run];
    });
    return result;
}

void ottercut_camera_preview_set_shape(OtterCutCameraPreview preview,
                                       OtterCutCameraPreviewShape shape) {
    [(__bridge OtterCutCameraPreviewController *)preview setShape:shape];
}

void ottercut_camera_preview_stop(OtterCutCameraPreview preview) {
    if (preview) {
        [(__bridge OtterCutCameraPreviewController *)preview stop];
    }
}

void ottercut_camera_preview_close(OtterCutCameraPreview preview) {
    if (preview) {
        [(__bridge OtterCutCameraPreviewController *)preview close];
    }
}

void ottercut_camera_preview_destroy(OtterCutCameraPreview preview) {
    if (!preview) {
        return;
    }
    OtterCutCameraPreviewController *instance = (__bridge OtterCutCameraPreviewController *)preview;
    [instance close];
    CFBridgingRelease(preview);
}
