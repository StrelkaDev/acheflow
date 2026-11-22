#![cfg(target_os = "macos")]
//! Synchronous device discovery and RAII capture over a small AVFoundation bridge.

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
};

unsafe extern "C" {
    fn acheflow_device_run_main_loop(
        context: *const c_void,
        finished: extern "C" fn(*const c_void) -> bool,
    );
}

/// Deliver system capture-device notifications while a headless server runs.
pub fn run_main_loop_until<F: Fn() -> bool>(finished: F) {
    extern "C" fn check<F: Fn() -> bool>(context: *const c_void) -> bool {
        // SAFETY: The synchronous native loop borrows the live callback on this thread.
        let finished = unsafe { &*context.cast::<F>() };
        catch_unwind(AssertUnwindSafe(finished)).unwrap_or(true)
    }

    // SAFETY: The native loop does not retain the pointer after it returns.
    unsafe { acheflow_device_run_main_loop(std::ptr::from_ref(&finished).cast(), check::<F>) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub modes: Vec<Mode>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub duration_value: u32,
    pub duration_scale: u32,
}

pub struct Frame<'a> {
    pub bgra: &'a [u8],
    pub stride: usize,
    pub width: usize,
    pub height: usize,
    pub pts_48khz: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    Disconnected,
    PermissionPending,
    AppBundleRequired,
    PermissionDenied,
    Unavailable,
    UnsupportedMode,
}

type DeviceCallback = extern "C" fn(*mut c_void, *const c_char, *const c_char);
type ModeCallback = extern "C" fn(*mut c_void, Mode);
type VideoCallback = extern "C" fn(*mut c_void, *const u8, usize, usize, usize, usize, i64);

unsafe extern "C" {
    fn acheflow_cameras(context: *mut c_void, device: DeviceCallback, mode: ModeCallback);
    fn acheflow_camera_open(
        id: *const c_char,
        mode: *const Mode,
        context: *mut c_void,
        callback: VideoCallback,
        status: *mut c_int,
    ) -> *mut c_void;
    fn acheflow_camera_close(handle: *mut c_void);
}

pub fn devices() -> Vec<Device> {
    let mut devices = Vec::new();
    // SAFETY: Native discovery invokes the callback synchronously and never retains the pointer.
    unsafe {
        acheflow_cameras(
            std::ptr::from_mut(&mut devices).cast(),
            add_device,
            add_mode,
        )
    };
    devices
}

extern "C" fn add_device(context: *mut c_void, id: *const c_char, name: *const c_char) {
    if id.is_null() || name.is_null() {
        return;
    }
    // SAFETY: The caller owns and serially borrows the vector for discovery.
    // AVFoundation supplies NUL-terminated strings valid through this call.
    let (devices, id, name) = unsafe {
        (
            &mut *context.cast::<Vec<Device>>(),
            CStr::from_ptr(id),
            CStr::from_ptr(name),
        )
    };
    devices.push(Device {
        id: id.to_string_lossy().into_owned(),
        name: name.to_string_lossy().into_owned(),
        modes: Vec::new(),
    });
}

extern "C" fn add_mode(context: *mut c_void, mut mode: Mode) {
    // SAFETY: Discovery delivers each device followed by its modes on this thread.
    let devices = unsafe { &mut *context.cast::<Vec<Device>>() };
    let (mut a, mut b) = (mode.duration_value, mode.duration_scale);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    if a != 0
        && let Some(device) = devices.last_mut()
    {
        mode.duration_value /= a;
        mode.duration_scale /= a;
        if !device.modes.contains(&mode) {
            device.modes.push(mode);
        }
    }
}
type Callback = Box<dyn Fn(Frame<'_>) + Send + Sync>;

/// One running device. The handle is confined to the thread that opened it.
pub struct Capture {
    handle: NonNull<c_void>,
    _callback: Box<Callback>,
}

impl Capture {
    pub fn open(
        id: &str,
        mode: Option<Mode>,
        callback: impl Fn(Frame<'_>) + Send + Sync + 'static,
    ) -> Result<Self, OpenError> {
        let id = CString::new(id).map_err(|_| OpenError::Disconnected)?;
        let mut callback: Box<Callback> = Box::new(Box::new(callback));
        let mut status = 0;
        // SAFETY: The callback allocation stays at a stable address until the
        // serial dispatch queue has drained in Drop. Failed open schedules no callbacks.
        let handle = unsafe {
            acheflow_camera_open(
                id.as_ptr(),
                mode.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
                std::ptr::from_mut(callback.as_mut()).cast(),
                deliver_frame,
                &mut status,
            )
        };
        let handle = NonNull::new(handle).ok_or(match status {
            1 => OpenError::Disconnected,
            2 => OpenError::PermissionPending,
            3 => OpenError::AppBundleRequired,
            4 => OpenError::PermissionDenied,
            6 => OpenError::UnsupportedMode,
            _ => OpenError::Unavailable,
        })?;
        Ok(Self {
            handle,
            _callback: callback,
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: Capture uniquely owns the retained handle. Close stops the
        // session and drains the dispatch queue before the callback is dropped.
        unsafe { acheflow_camera_close(self.handle.as_ptr()) };
    }
}

extern "C" fn deliver_frame(
    context: *mut c_void,
    data: *const u8,
    length: usize,
    stride: usize,
    width: usize,
    height: usize,
    pts_48khz: i64,
) {
    let Some(size) = stride.checked_mul(height) else {
        return;
    };
    if width == 0
        || height == 0
        || width.saturating_mul(height) > 8_294_400
        || stride < width * 4
        || size > length
        || size > 64 * 1024 * 1024
        || data.is_null()
    {
        return;
    }
    // SAFETY: The native callback owns a lock on the pixel buffer for the
    // duration of this call. The callback allocation outlives this dispatch.
    let (callback, bgra) = unsafe {
        (
            &*context.cast::<Callback>(),
            std::slice::from_raw_parts(data, size),
        )
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        callback(Frame {
            bgra,
            stride,
            width,
            height,
            pts_48khz,
        })
    }));
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    pub modes: Vec<MicrophoneMode>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicrophoneMode {
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_depth: u32,
}

type MicrophoneModeCallback = extern "C" fn(*mut c_void, MicrophoneMode);
type AudioCallback = extern "C" fn(*mut c_void, *const i16, usize, usize, i64);
type MicrophoneCallback = Box<dyn FnMut(&[i16], usize, i64) + Send>;

unsafe extern "C" {
    fn acheflow_microphones(
        context: *mut c_void,
        device: DeviceCallback,
        mode: MicrophoneModeCallback,
    );
    fn acheflow_microphone_open(
        id: *const c_char,
        mode: *const MicrophoneMode,
        context: *mut c_void,
        callback: AudioCallback,
        status: *mut c_int,
    ) -> *mut c_void;
    fn acheflow_microphone_close(handle: *mut c_void);
}

pub fn microphones() -> Vec<MicrophoneDevice> {
    let mut devices = Vec::new();
    // SAFETY: Discovery delivers each device and its modes synchronously.
    unsafe {
        acheflow_microphones(
            std::ptr::from_mut(&mut devices).cast(),
            add_microphone,
            add_microphone_mode,
        )
    };
    devices
}

extern "C" fn add_microphone(context: *mut c_void, id: *const c_char, name: *const c_char) {
    if id.is_null() || name.is_null() {
        return;
    }
    // SAFETY: The discovery call owns the vector. The native strings last
    // throughout this callback and are copied into Rust-owned storage.
    let (devices, id, name) = unsafe {
        (
            &mut *context.cast::<Vec<MicrophoneDevice>>(),
            CStr::from_ptr(id),
            CStr::from_ptr(name),
        )
    };
    devices.push(MicrophoneDevice {
        id: id.to_string_lossy().into_owned(),
        name: name.to_string_lossy().into_owned(),
        modes: Vec::new(),
    });
}

extern "C" fn add_microphone_mode(context: *mut c_void, mode: MicrophoneMode) {
    // SAFETY: Discovery invokes this callback serially with the same vector.
    let devices = unsafe { &mut *context.cast::<Vec<MicrophoneDevice>>() };
    if let Some(device) = devices.last_mut() {
        device.modes.push(mode);
    }
}

pub struct MicrophoneCapture {
    handle: NonNull<c_void>,
    _callback: Box<MicrophoneCallback>,
}

impl MicrophoneCapture {
    pub fn open(
        id: &str,
        mode: Option<MicrophoneMode>,
        callback: impl FnMut(&[i16], usize, i64) + Send + 'static,
    ) -> Result<Self, OpenError> {
        let id = CString::new(id).map_err(|_| OpenError::Disconnected)?;
        let mut callback: Box<MicrophoneCallback> = Box::new(Box::new(callback));
        let mut status = 0;
        // SAFETY: The callback allocation stays in place until close drains
        // the serial queue. Failed open registers no delegate.
        let handle = unsafe {
            acheflow_microphone_open(
                id.as_ptr(),
                mode.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
                std::ptr::from_mut(callback.as_mut()).cast(),
                deliver_audio,
                &mut status,
            )
        };
        let handle = NonNull::new(handle).ok_or(match status {
            1 => OpenError::Disconnected,
            2 => OpenError::PermissionPending,
            3 => OpenError::AppBundleRequired,
            4 => OpenError::PermissionDenied,
            6 => OpenError::UnsupportedMode,
            _ => OpenError::Unavailable,
        })?;
        Ok(Self {
            handle,
            _callback: callback,
        })
    }
}

impl Drop for MicrophoneCapture {
    fn drop(&mut self) {
        // SAFETY: This is the only owner of the retained handle. Close drains
        // the native dispatch queue while the Rust callback is still alive.
        unsafe { acheflow_microphone_close(self.handle.as_ptr()) };
    }
}

extern "C" fn deliver_audio(
    context: *mut c_void,
    samples: *const i16,
    frames: usize,
    channels: usize,
    pts: i64,
) {
    if samples.is_null() || !(1..=2).contains(&channels) || !(1..=8192).contains(&frames) {
        return;
    }
    // SAFETY: The serial native callback owns the initialized sample array
    // for this call and has exclusive access to the Rust callback allocation.
    let (callback, samples) = unsafe {
        (
            &mut *context.cast::<MicrophoneCallback>(),
            std::slice::from_raw_parts(samples, frames * channels),
        )
    };
    let _ = catch_unwind(AssertUnwindSafe(|| callback(samples, channels, pts)));
}
