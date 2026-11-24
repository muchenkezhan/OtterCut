#ifndef OTTERCUT_MACOS_CAMERA_CAPTURE_H
#define OTTERCUT_MACOS_CAMERA_CAPTURE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    char *unique_id;
    char *name;
} OtterCutCameraDevice;

typedef struct {
    size_t count;
    OtterCutCameraDevice *devices;
} OtterCutCameraDeviceList;

typedef struct {
    int width;
    int height;
    int fps_milli;
} OtterCutCameraMode;

typedef struct {
    size_t count;
    OtterCutCameraMode *modes;
} OtterCutCameraModeList;

typedef void (*OtterCutCameraStartedCallback)(void *context);
/* error_message is null when the recording completed successfully. */
typedef void (*OtterCutCameraFinishedCallback)(const char *error_message, void *context);
/* error_message is null after preview has started successfully. */
typedef void (*OtterCutCameraPreviewCallback)(const char *error_message, void *context);

typedef void *OtterCutCameraCapture;
typedef void *OtterCutCameraPreview;

typedef enum {
    OtterCutCameraPreviewCircle,
    OtterCutCameraPreviewSquare,
} OtterCutCameraPreviewShape;

/* Returned strings and device arrays are owned by the caller. */
int ottercut_camera_list_devices(OtterCutCameraDeviceList *result, char **error_message);
void ottercut_camera_free_device_list(OtterCutCameraDeviceList *list);
/* Lists unique sizes with common frame rates supported by each camera format. */
int ottercut_camera_list_modes(const char *device_unique_id,
                               OtterCutCameraModeList *result,
                               char **error_message);
void ottercut_camera_free_mode_list(OtterCutCameraModeList *list);
void ottercut_camera_free_string(char *string);

/* Create one capture. All values are copied before the function returns. */
OtterCutCameraCapture ottercut_camera_capture_create(
    const char *device_unique_id,
    const char *output_path,
    int width,
    int height,
    int frames_per_second_milli,
    OtterCutCameraStartedCallback on_started,
    OtterCutCameraFinishedCallback on_finished,
    void *context,
    char **error_message);

/* Start returns 1 only when this capture accepted its first start request. */
int ottercut_camera_capture_start(OtterCutCameraCapture capture);
/* Stop is asynchronous, thread-safe, and idempotent per capture. */
void ottercut_camera_capture_request_stop(OtterCutCameraCapture capture);
/* Destroy only after on_finished has run, or before start was requested. */
void ottercut_camera_capture_destroy(OtterCutCameraCapture capture);

/* Preview windows are created and controlled on the main thread internally. */
OtterCutCameraPreview ottercut_camera_preview_create(
    const char *device_unique_id,
    const char *window_title,
    char **error_message);
/* Returns 1 only when the preview accepted its first start request. */
int ottercut_camera_preview_start(
    OtterCutCameraPreview preview,
    OtterCutCameraPreviewCallback on_started,
    void *context);
/* Run the Cocoa event loop on the main thread until the preview window closes. */
int ottercut_camera_preview_run(OtterCutCameraPreview preview);
void ottercut_camera_preview_set_shape(OtterCutCameraPreview preview,
                                       OtterCutCameraPreviewShape shape);
void ottercut_camera_preview_stop(OtterCutCameraPreview preview);
void ottercut_camera_preview_close(OtterCutCameraPreview preview);
void ottercut_camera_preview_destroy(OtterCutCameraPreview preview);

#ifdef __cplusplus
}
#endif

#ifdef __OBJC__
@class AVCaptureVideoPreviewLayer, NSPanel, NSString;

NSPanel *ottercut_camera_preview_panel(NSString *title);
void ottercut_camera_preview_panel_set_shape(NSPanel *panel,
                                             OtterCutCameraPreviewShape shape);
void ottercut_camera_preview_attach_layer(NSPanel *panel, AVCaptureVideoPreviewLayer *layer);
/* Obtain the active camera's preview layer. The callback runs on the main queue. */
void ottercut_camera_capture_preview_layer(
    OtterCutCameraCapture capture,
    void (^completion)(AVCaptureVideoPreviewLayer *layer));
#endif

#endif
