#import <AVFoundation/AVFoundation.h>
#import <CoreFoundation/CoreFoundation.h>
#import <CoreVideo/CoreVideo.h>
#include <stdint.h>

typedef void (*DeviceCallback)(void *, const char *, const char *);
typedef struct {
    uint32_t width, height, duration_value, duration_scale;
} CameraMode;
typedef void (*ModeCallback)(void *, CameraMode);
typedef void (*VideoCallback)(void *, const uint8_t *, size_t, size_t, size_t, size_t, int64_t);

static void keep_run_loop_alive(void *context) {}

void acheflow_device_run_main_loop(const void *context, bool (*finished)(const void *)) {
    if (![NSThread isMainThread]) return;
    CFRunLoopRef loop = CFRunLoopGetMain();
    CFRunLoopSourceContext sourceContext = {.perform = keep_run_loop_alive};
    CFRunLoopSourceRef source = CFRunLoopSourceCreate(NULL, 0, &sourceContext);
    CFRunLoopAddSource(loop, source, kCFRunLoopDefaultMode);
    while (!finished(context)) {
        @autoreleasepool {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false);
        }
    }
    CFRunLoopRemoveSource(loop, source, kCFRunLoopDefaultMode);
    CFRelease(source);
}

static NSArray<AVCaptureDevice *> *cameras(void) {
    NSMutableArray<AVCaptureDeviceType> *types = [NSMutableArray arrayWithObject:AVCaptureDeviceTypeBuiltInWideAngleCamera];
    if (@available(macOS 14.0, *)) {
        [types addObject:AVCaptureDeviceTypeExternal];
        [types addObject:AVCaptureDeviceTypeContinuityCamera];
    } else {
        [types addObject:AVCaptureDeviceTypeExternalUnknown];
    }
    if (@available(macOS 13.0, *)) {
        [types addObject:AVCaptureDeviceTypeDeskViewCamera];
    }
    NSArray<AVCaptureDevice *> *discovered = [AVCaptureDeviceDiscoverySession
        discoverySessionWithDeviceTypes:types
        mediaType:AVMediaTypeVideo position:AVCaptureDevicePositionUnspecified].devices;
    NSMutableDictionary<NSString *, AVCaptureDevice *> *connected = [NSMutableDictionary dictionary];
    // Ask for every video device as well, including devices outside the known types.
    for (AVCaptureDevice *device in [AVCaptureDevice devicesWithMediaType:AVMediaTypeVideo]) {
        if (device.connected) connected[device.uniqueID] = device;
    }
    for (AVCaptureDevice *device in discovered) {
        if (device.connected) connected[device.uniqueID] = device;
    }
    return connected.allValues;
}

static BOOL containsDuration(AVFrameRateRange *range, CMTime duration) {
    return CMTimeCompare(duration, range.minFrameDuration) >= 0
        && CMTimeCompare(duration, range.maxFrameDuration) <= 0;
}

static void publishMode(void *context, ModeCallback callback, CMVideoDimensions size, CMTime duration) {
    if (CMTIME_IS_NUMERIC(duration) && duration.value > 0
        && duration.value <= UINT32_MAX && duration.timescale > 0) {
        CameraMode mode = { (uint32_t)size.width, (uint32_t)size.height,
                            (uint32_t)duration.value, (uint32_t)duration.timescale };
        callback(context, mode);
    }
}

void acheflow_cameras(void *context, DeviceCallback deviceCallback, ModeCallback modeCallback) {
    @autoreleasepool {
        for (AVCaptureDevice *device in cameras()) {
            const char *id = device.uniqueID.UTF8String;
            const char *name = device.localizedName.UTF8String;
            if (!id || !name) continue;
            deviceCallback(context, id, name);
            const int32_t rates[] = { 15, 24, 25, 30, 50, 60, 90, 120, 240 };
            const int32_t fractionalRates[] = { 24000, 30000, 60000, 120000 };
            for (AVCaptureDeviceFormat *format in device.formats) {
                CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(format.formatDescription);
                for (AVFrameRateRange *range in format.videoSupportedFrameRateRanges) {
                    publishMode(context, modeCallback, size, range.minFrameDuration);
                    publishMode(context, modeCallback, size, range.maxFrameDuration);
                    for (size_t i = 0; i < sizeof rates / sizeof rates[0]; i++) {
                        CMTime duration = CMTimeMake(1, rates[i]);
                        if (containsDuration(range, duration)) publishMode(context, modeCallback, size, duration);
                    }
                    for (size_t i = 0; i < sizeof fractionalRates / sizeof fractionalRates[0]; i++) {
                        CMTime duration = CMTimeMake(1001, fractionalRates[i]);
                        if (containsDuration(range, duration)) publishMode(context, modeCallback, size, duration);
                    }
                }
            }
        }
    }
}

static BOOL configureMode(AVCaptureDevice *device, const CameraMode *mode) {
    if (!mode->duration_value || !mode->duration_scale || mode->duration_scale > INT32_MAX) return NO;
    CMTime duration = CMTimeMake(mode->duration_value, (int32_t)mode->duration_scale);
    AVCaptureDeviceFormat *selected = nil;
    for (AVCaptureDeviceFormat *format in device.formats) {
        CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(format.formatDescription);
        if (size.width != (int64_t)mode->width || size.height != (int64_t)mode->height) continue;
        for (AVFrameRateRange *range in format.videoSupportedFrameRateRanges) {
            if (containsDuration(range, duration)) {
                selected = format;
                break;
            }
        }
        if (selected) break;
    }
    if (!selected || ![device lockForConfiguration:NULL]) return NO;
    @try {
        device.activeFormat = selected;
        if (CMTimeCompare(duration, device.activeVideoMaxFrameDuration) > 0) {
            device.activeVideoMaxFrameDuration = duration;
            device.activeVideoMinFrameDuration = duration;
        } else {
            device.activeVideoMinFrameDuration = duration;
            device.activeVideoMaxFrameDuration = duration;
        }
    } @catch (NSException *error) {
        [device unlockForConfiguration];
        return NO;
    }
    // Keep the explicit format locked while the macOS capture session is using it.
    return YES;
}

static BOOL modeIsActive(AVCaptureDevice *device, const CameraMode *mode) {
    CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(device.activeFormat.formatDescription);
    CMTime duration = CMTimeMake(mode->duration_value, (int32_t)mode->duration_scale);
    return size.width == (int64_t)mode->width && size.height == (int64_t)mode->height
        && CMTimeCompare(device.activeVideoMinFrameDuration, duration) == 0
        && CMTimeCompare(device.activeVideoMaxFrameDuration, duration) == 0;
}

@interface AcheflowCamera : NSObject <AVCaptureVideoDataOutputSampleBufferDelegate>
@property(nonatomic, strong) AVCaptureSession *session;
@property(nonatomic, strong) AVCaptureVideoDataOutput *output;
@property(nonatomic, strong) AVCaptureDevice *lockedDevice;
@property(nonatomic, strong) dispatch_queue_t queue;
@property void *context;
@property VideoCallback callback;
@end

@implementation AcheflowCamera
- (void)dealloc {
    [_lockedDevice unlockForConfiguration];
}

- (void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)sample
       fromConnection:(AVCaptureConnection *)connection {
    CVPixelBufferRef image = CMSampleBufferGetImageBuffer(sample);
    if (!image || CVPixelBufferGetPixelFormatType(image) != kCVPixelFormatType_32BGRA
        || CVPixelBufferLockBaseAddress(image, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) {
        return;
    }
    CMTime pts = CMTimeConvertScale(CMSampleBufferGetPresentationTimeStamp(sample), 48000,
                                  kCMTimeRoundingMethod_Default);
    const uint8_t *bytes = CVPixelBufferGetBaseAddress(image);
    if (bytes && CMTIME_IS_NUMERIC(pts)) {
        self.callback(self.context, bytes, CVPixelBufferGetDataSize(image), CVPixelBufferGetBytesPerRow(image),
                      CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image), pts.value);
    }
    CVPixelBufferUnlockBaseAddress(image, kCVPixelBufferLock_ReadOnly);
}
@end

void *acheflow_camera_open(const char *id, const CameraMode *mode, void *context, VideoCallback callback, int *status) {
    @autoreleasepool {
        *status = 1;
        NSString *deviceID = [NSString stringWithUTF8String:id];
        AVCaptureDevice *selected = nil;
        for (AVCaptureDevice *device in cameras()) {
            if ([device.uniqueID isEqualToString:deviceID]) {
                selected = device;
                break;
            }
        }
        if (!selected) return NULL;

        AVAuthorizationStatus permission = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
        if (permission == AVAuthorizationStatusNotDetermined) {
            *status = 2;
            if (![[NSBundle mainBundle] objectForInfoDictionaryKey:@"NSCameraUsageDescription"]) {
                *status = 3;
                return NULL;
            }
            static dispatch_once_t request;
            dispatch_once(&request, ^{
                [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) {}];
            });
            return NULL;
        }
        if (permission != AVAuthorizationStatusAuthorized) {
            *status = 4;
            return NULL;
        }

        *status = 5;
        AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:selected error:NULL];
        if (!input) return NULL;
        AcheflowCamera *capture = [[AcheflowCamera alloc] init];
        capture.session = [[AVCaptureSession alloc] init];
        capture.output = [[AVCaptureVideoDataOutput alloc] init];
        capture.queue = dispatch_queue_create("com.acheflow.camera.frames", DISPATCH_QUEUE_SERIAL);
        capture.context = context;
        capture.callback = callback;
        capture.output.alwaysDiscardsLateVideoFrames = YES;
        capture.output.videoSettings = @{ (__bridge NSString *)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA) };
        [capture.session beginConfiguration];
        if (![capture.session canAddInput:input] || ![capture.session canAddOutput:capture.output]) {
            [capture.session commitConfiguration];
            return NULL;
        }
        [capture.session addInput:input];
        [capture.session addOutput:capture.output];
        if (mode) {
            if (!configureMode(selected, mode)) {
                *status = 6;
                [capture.session commitConfiguration];
                return NULL;
            }
            capture.lockedDevice = selected;
        }
        [capture.session commitConfiguration];
        [capture.session startRunning];
        if (!capture.session.running) return NULL;
        if (mode && !modeIsActive(selected, mode)) {
            [capture.session stopRunning];
            *status = 6;
            return NULL;
        }
        [capture.output setSampleBufferDelegate:capture queue:capture.queue];
        return (__bridge_retained void *)capture;
    }
}

void acheflow_camera_close(void *handle) {
    @autoreleasepool {
        AcheflowCamera *capture = (__bridge_transfer AcheflowCamera *)handle;
        [capture.session stopRunning];
        [capture.output setSampleBufferDelegate:nil queue:NULL];
        dispatch_sync(capture.queue, ^{});
    }
}

typedef void (*AudioCallback)(void *, const int16_t *, size_t, size_t, int64_t);

static NSArray<AVCaptureDevice *> *microphones(void) {
    return [[AVCaptureDevice devicesWithMediaType:AVMediaTypeAudio]
        filteredArrayUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(AVCaptureDevice *device, NSDictionary *bindings) {
            return device.connected;
        }]];
}

typedef struct {
    uint32_t sample_rate, channels, bit_depth;
} MicrophoneMode;
typedef void (*MicrophoneModeCallback)(void *, MicrophoneMode);

static const AudioStreamBasicDescription *microphone_format(AVCaptureDeviceFormat *format) {
    CMFormatDescriptionRef description = format.formatDescription;
    return description ? CMAudioFormatDescriptionGetStreamBasicDescription(description) : NULL;
}

static BOOL microphone_mode(AVCaptureDeviceFormat *format, MicrophoneMode *mode) {
    const AudioStreamBasicDescription *pcm = microphone_format(format);
    if (!pcm || pcm->mFormatID != kAudioFormatLinearPCM ||
        !(pcm->mSampleRate >= 1000 && pcm->mSampleRate <= 768000) ||
        pcm->mSampleRate != (uint32_t)pcm->mSampleRate ||
        !pcm->mChannelsPerFrame || pcm->mChannelsPerFrame > 32 ||
        !pcm->mBitsPerChannel || pcm->mBitsPerChannel > 64) return NO;
    *mode = (MicrophoneMode){(uint32_t)pcm->mSampleRate, pcm->mChannelsPerFrame, pcm->mBitsPerChannel};
    return YES;
}

static BOOL matches_microphone_mode(AVCaptureDeviceFormat *format, const MicrophoneMode *wanted) {
    MicrophoneMode mode;
    return microphone_mode(format, &mode) && mode.sample_rate == wanted->sample_rate &&
           mode.channels == wanted->channels && mode.bit_depth == wanted->bit_depth;
}

static BOOL set_microphone_format(AVCaptureDevice *device, AVCaptureDeviceFormat *format) {
    if (![device lockForConfiguration:NULL]) return NO;
    @try {
        device.activeFormat = format;
    } @catch (NSException *exception) {
        return NO;
    } @finally {
        [device unlockForConfiguration];
    }
    return YES;
}

void acheflow_microphones(void *context, DeviceCallback callback, MicrophoneModeCallback modeCallback) {
    @autoreleasepool {
        for (AVCaptureDevice *device in microphones()) {
            const char *id = device.uniqueID.UTF8String;
            const char *name = device.localizedName.UTF8String;
            if (!id || !name) continue;
            callback(context, id, name);
            for (AVCaptureDeviceFormat *format in device.formats) {
                MicrophoneMode mode;
                if (microphone_mode(format, &mode)) modeCallback(context, mode);
            }
        }
    }
}

@interface AcheflowMicrophone : NSObject <AVCaptureAudioDataOutputSampleBufferDelegate>
@property(nonatomic, strong) AVCaptureSession *session;
@property(nonatomic, strong) AVCaptureAudioDataOutput *output;
@property(nonatomic, strong) AVCaptureDevice *device;
@property(nonatomic, strong) AVCaptureDeviceFormat *previousFormat;
@property(nonatomic, strong) AVCaptureDeviceFormat *selectedFormat;
@property(nonatomic, strong) dispatch_queue_t queue;
@property void *context;
@property AudioCallback callback;
@end

@implementation AcheflowMicrophone
- (void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)sample
       fromConnection:(AVCaptureConnection *)connection {
    CMFormatDescriptionRef description = CMSampleBufferGetFormatDescription(sample);
    if (!description) return;
    const AudioStreamBasicDescription *format = CMAudioFormatDescriptionGetStreamBasicDescription(description);
    if (!format || format->mFormatID != kAudioFormatLinearPCM || format->mSampleRate != 48000 ||
        format->mBitsPerChannel != 16 || !(format->mFormatFlags & kAudioFormatFlagIsSignedInteger) ||
        (format->mFormatFlags & (kAudioFormatFlagIsNonInterleaved | kAudioFormatFlagIsBigEndian)) ||
        format->mChannelsPerFrame < 1 || format->mChannelsPerFrame > 2 ||
        format->mBytesPerFrame != format->mChannelsPerFrame * sizeof(int16_t)) return;

    CMTime time = CMSampleBufferGetPresentationTimeStamp(sample);
    if (!CMTIME_IS_NUMERIC(time)) return;
    CMItemCount frames = CMSampleBufferGetNumSamples(sample);
    int16_t samples[8192];
    if (frames <= 0 || frames > sizeof(samples) / format->mBytesPerFrame) return;
    CMBlockBufferRef data = CMSampleBufferGetDataBuffer(sample);
    size_t size = frames * format->mBytesPerFrame;
    if (!data || CMBlockBufferCopyDataBytes(data, 0, size, samples) != kCMBlockBufferNoErr) return;

    int64_t pts = CMTimeConvertScale(time, 48000, kCMTimeRoundingMethod_Default).value;
    self.callback(self.context, samples, frames, format->mChannelsPerFrame, pts);
}
@end

void *acheflow_microphone_open(const char *id, const MicrophoneMode *mode, void *context, AudioCallback callback, int *status) {
    @autoreleasepool {
        *status = 1;
        NSString *deviceID = [NSString stringWithUTF8String:id];
        AVCaptureDevice *selected = nil;
        for (AVCaptureDevice *device in microphones()) {
            if ([device.uniqueID isEqualToString:deviceID]) {
                selected = device;
                break;
            }
        }
        if (!selected) return NULL;

        AVAuthorizationStatus permission = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
        if (permission == AVAuthorizationStatusNotDetermined) {
            *status = 2;
            if (![[NSBundle mainBundle] objectForInfoDictionaryKey:@"NSMicrophoneUsageDescription"]) {
                *status = 3;
                return NULL;
            }
            static dispatch_once_t request;
            dispatch_once(&request, ^{
                [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {}];
            });
            return NULL;
        }
        if (permission != AVAuthorizationStatusAuthorized) {
            *status = 4;
            return NULL;
        }

        *status = 5;
        AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:selected error:NULL];
        if (!input) return NULL;
        AVCaptureDeviceFormat *selectedFormat = nil;
        if (mode) {
            for (AVCaptureDeviceFormat *format in selected.formats) {
                if (matches_microphone_mode(format, mode)) {
                    selectedFormat = format;
                    break;
                }
            }
            if (!selectedFormat) {
                *status = 6;
                return NULL;
            }
        }

        AcheflowMicrophone *capture = [[AcheflowMicrophone alloc] init];
        capture.session = [[AVCaptureSession alloc] init];
        capture.output = [[AVCaptureAudioDataOutput alloc] init];
        capture.queue = dispatch_queue_create("com.acheflow.microphone.samples", DISPATCH_QUEUE_SERIAL);
        capture.context = context;
        capture.callback = callback;
        [capture.session beginConfiguration];
        if (![capture.session canAddInput:input] || ![capture.session canAddOutput:capture.output]) {
            [capture.session commitConfiguration];
            return NULL;
        }
        [capture.session addInput:input];
        [capture.session addOutput:capture.output];
        if (selectedFormat) {
            capture.device = selected;
            capture.previousFormat = selected.activeFormat;
            capture.selectedFormat = selectedFormat;
            if (!set_microphone_format(selected, selectedFormat)) {
                *status = 6;
                [capture.session commitConfiguration];
                return NULL;
            }
        }
        const AudioStreamBasicDescription *format = microphone_format(selected.activeFormat);
        unsigned int channels = format && format->mChannelsPerFrame == 1 ? 1 : 2;
        capture.output.audioSettings = @{
            AVFormatIDKey: @(kAudioFormatLinearPCM),
            AVSampleRateKey: @48000,
            AVNumberOfChannelsKey: @(channels),
            AVLinearPCMBitDepthKey: @16,
            AVLinearPCMIsFloatKey: @NO,
            AVLinearPCMIsBigEndianKey: @NO,
            AVLinearPCMIsNonInterleaved: @NO,
        };
        [capture.session commitConfiguration];
        [capture.session startRunning];
        if (mode && !matches_microphone_mode(selected.activeFormat, mode)) {
            [capture.session stopRunning];
            *status = 6;
            return NULL;
        }
        [capture.output setSampleBufferDelegate:capture queue:capture.queue];
        return (__bridge_retained void *)capture;
    }
}

void acheflow_microphone_close(void *handle) {
    @autoreleasepool {
        AcheflowMicrophone *capture = (__bridge_transfer AcheflowMicrophone *)handle;
        [capture.session stopRunning];
        [capture.output setSampleBufferDelegate:nil queue:NULL];
        dispatch_sync(capture.queue, ^{});
        if (capture.previousFormat && [capture.device.activeFormat isEqual:capture.selectedFormat]) {
            set_microphone_format(capture.device, capture.previousFormat);
        }
    }
}
