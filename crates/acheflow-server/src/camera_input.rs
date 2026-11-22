use crate::ingest::Input;
use acheflow_media_api::live::CameraInventory;
use axum::{Json, http::header, response::IntoResponse};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub async fn list() -> impl IntoResponse {
    #[cfg(target_os = "macos")]
    let inventory = CameraInventory {
        supported: true,
        devices: tokio::task::spawn_blocking(macos::list)
            .await
            .unwrap_or_default(),
    };
    #[cfg(not(target_os = "macos"))]
    let inventory = CameraInventory {
        supported: false,
        devices: Vec::new(),
    };
    ([(header::CACHE_CONTROL, "no-store")], Json(inventory))
}

pub(crate) fn start(input: &Arc<Input>, shutdown: &CancellationToken) {
    #[cfg(target_os = "macos")]
    input.start_managed(shutdown, |cancel| {
        let input = input.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || macos::run(input, cancel, runtime))
    });
    #[cfg(not(target_os = "macos"))]
    {
        let _ = shutdown;
        *input.error.lock().unwrap() =
            Some("Local camera capture is unavailable on this server".into());
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::ingest::VideoFrame;
    use acheflow_camera::{Capture, Frame, OpenError};
    use acheflow_media_api::{
        live::CameraDevice,
        visual::{CameraMode, SourceKind},
    };
    use std::{
        sync::{Mutex, atomic::Ordering},
        time::{Duration, Instant},
    };

    pub(super) fn list() -> Vec<CameraDevice> {
        let mut devices = acheflow_camera::devices()
            .into_iter()
            .map(|device| CameraDevice {
                id: device.id,
                name: device.name,
                modes: sorted_modes(device.modes),
            })
            .collect::<Vec<_>>();
        devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        devices
    }

    fn sorted_modes(modes: Vec<acheflow_camera::Mode>) -> Vec<CameraMode> {
        let mut modes = modes
            .into_iter()
            .map(|mode| CameraMode {
                width: mode.width,
                height: mode.height,
                duration_value: mode.duration_value,
                duration_scale: mode.duration_scale,
            })
            .filter(|mode| mode.is_valid())
            .collect::<Vec<_>>();
        modes.sort_by(|left, right| {
            (right.width, right.height)
                .cmp(&(left.width, left.height))
                .then_with(|| right.fps().total_cmp(&left.fps()))
        });
        modes
    }

    pub(super) fn run(
        input: Arc<Input>,
        cancel: CancellationToken,
        runtime: tokio::runtime::Handle,
    ) {
        let SourceKind::Camera {
            device_id,
            mode,
            mirrored,
        } = &input.config.kind
        else {
            return;
        };
        let mirrored = *mirrored;
        let capture_mode = mode.map(|mode| acheflow_camera::Mode {
            width: mode.width,
            height: mode.height,
            duration_value: mode.duration_value,
            duration_scale: mode.duration_scale,
        });
        while !cancel.is_cancelled() {
            let generation = input.next_generation();
            let output = input.clone();
            let rate_sample = Mutex::new(None);
            let error = match Capture::open(device_id, capture_mode, move |frame| {
                publish(&output, generation, frame, mirrored, &rate_sample)
            }) {
                Err(OpenError::Disconnected) => "Camera is disconnected",
                Err(OpenError::PermissionPending) => "Waiting for macOS camera permission",
                Err(OpenError::AppBundleRequired) => {
                    "Open the Acheflow macOS app to grant camera permission"
                }
                Err(OpenError::PermissionDenied) => {
                    "Allow Acheflow camera access in macOS System Settings"
                }
                Err(OpenError::Unavailable) => {
                    "Cannot open the camera. Check whether another app is using it."
                }
                Err(OpenError::UnsupportedMode) => {
                    "Camera cannot apply this capture mode. Select another mode or Auto."
                }
                Ok(_capture) => {
                    let started = Instant::now();
                    runtime.block_on(async {
                        while !cancel.is_cancelled() {
                            let latest = input
                                .frame
                                .read()
                                .unwrap()
                                .as_ref()
                                .map(|frame| frame.received);
                            if latest.unwrap_or(started).elapsed() >= Duration::from_secs(4) {
                                break;
                            }
                            wait(&cancel, Duration::from_millis(250)).await;
                        }
                    });
                    "Camera stopped sending frames"
                }
            };
            input.connected.store(false, Ordering::Release);
            *input.frame.write().unwrap() = None;
            input.metadata.lock().unwrap().2 = None;
            if cancel.is_cancelled() {
                break;
            }
            *input.error.lock().unwrap() = Some(error.into());
            runtime.block_on(wait(&cancel, Duration::from_secs(1)));
        }
    }

    async fn wait(cancel: &CancellationToken, delay: Duration) {
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = tokio::time::sleep(delay) => {}
        }
    }

    fn pixels(frame: &Frame<'_>, mirrored: bool) -> Option<image::RgbImage> {
        let mut rgb = vec![0; frame.width * frame.height * 3];
        for (row, target) in frame
            .bgra
            .chunks_exact(frame.stride)
            .zip(rgb.chunks_exact_mut(frame.width * 3))
        {
            let source = row[..frame.width * 4].as_chunks::<4>().0;
            let target = target.as_chunks_mut::<3>().0;
            if mirrored {
                for (&[b, g, r, _], pixel) in source.iter().rev().zip(target) {
                    *pixel = [r, g, b];
                }
            } else {
                for (&[b, g, r, _], pixel) in source.iter().zip(target) {
                    *pixel = [r, g, b];
                }
            }
        }
        image::RgbImage::from_raw(frame.width as u32, frame.height as u32, rgb)
    }

    fn publish(
        input: &Input,
        generation: u64,
        frame: Frame<'_>,
        mirrored: bool,
        rate_sample: &Mutex<Option<(Instant, u64)>>,
    ) {
        let Some(image) = pixels(&frame, mirrored) else {
            return;
        };
        let received = Instant::now();
        *input.frame.write().unwrap() = Some(VideoFrame {
            pts: Some(frame.pts_48khz),
            generation,
            received,
            image: Arc::new(image),
        });
        let count = input.frames.fetch_add(1, Ordering::Relaxed) + 1;
        let mut sample = rate_sample.lock().unwrap();
        let (started, first) = *sample.get_or_insert((received, count));
        let elapsed = received.duration_since(started);
        if elapsed >= Duration::from_secs(1) {
            input.metadata.lock().unwrap().2 = Some((count - first) as f32 / elapsed.as_secs_f32());
            *sample = Some((received, count));
        }
        if !input.connected.swap(true, Ordering::AcqRel) {
            *input.error.lock().unwrap() = None;
        }
    }

    #[cfg(test)]
    #[test]
    fn camera_mirroring_preserves_portrait_dimensions_and_ignores_row_padding() {
        let mut bgra = Vec::new();
        for row in [[1, 2], [3, 4], [5, 6]] {
            for pixel in row {
                bgra.extend([0, 0, pixel, 255]);
            }
            bgra.extend([255; 4]);
        }
        let frame = Frame {
            bgra: &bgra,
            stride: 12,
            width: 2,
            height: 3,
            pts_48khz: 0,
        };
        for (mirrored, reds) in [(false, [1, 2, 3, 4, 5, 6]), (true, [2, 1, 4, 3, 6, 5])] {
            let image = pixels(&frame, mirrored).unwrap();
            assert_eq!(image.dimensions(), (2, 3));
            assert!(image.pixels().map(|pixel| pixel.0[0]).eq(reds));
        }
    }
}
