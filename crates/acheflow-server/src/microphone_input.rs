use crate::ingest::Input;
use acheflow_media_api::live::MicrophoneInventory;
use axum::{Json, http::header, response::IntoResponse};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub async fn list() -> impl IntoResponse {
    #[cfg(target_os = "macos")]
    let inventory = MicrophoneInventory {
        supported: true,
        devices: tokio::task::spawn_blocking(macos::list)
            .await
            .unwrap_or_default(),
    };
    #[cfg(not(target_os = "macos"))]
    let inventory = MicrophoneInventory {
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
            Some("Microphone capture is unavailable on this server".into());
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::audio_input::AudioTrack;
    use acheflow_camera::{MicrophoneCapture, OpenError};
    use acheflow_media_api::{
        audio::AUDIO_SAMPLES,
        live::MicrophoneDevice,
        visual::{MicrophoneMode, SourceKind},
    };
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };

    pub(super) fn list() -> Vec<MicrophoneDevice> {
        let mut devices = acheflow_camera::microphones()
            .into_iter()
            .map(|device| {
                let mut modes = device
                    .modes
                    .into_iter()
                    .map(|mode| MicrophoneMode {
                        sample_rate: mode.sample_rate,
                        channels: mode.channels,
                        bit_depth: mode.bit_depth,
                    })
                    .filter(|mode| mode.is_valid())
                    .collect::<Vec<_>>();
                modes.sort_unstable_by_key(|mode| std::cmp::Reverse(*mode));
                modes.dedup();
                MicrophoneDevice {
                    id: device.id,
                    name: device.name,
                    modes,
                }
            })
            .collect::<Vec<_>>();
        devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        devices
    }

    pub(super) fn run(
        input: Arc<Input>,
        cancel: CancellationToken,
        runtime: tokio::runtime::Handle,
    ) {
        let SourceKind::Microphone { device_id, mode } = &input.config.kind else {
            return;
        };
        let capture_mode = mode.map(|mode| acheflow_camera::MicrophoneMode {
            sample_rate: mode.sample_rate,
            channels: mode.channels,
            bit_depth: mode.bit_depth,
        });
        while !cancel.is_cancelled() {
            input.next_generation();
            let track = Arc::new(AudioTrack::new(0, "PCM"));
            input.audio_tracks.lock().unwrap().insert(0, track.clone());
            let output = input.clone();
            let captured_track = track.clone();
            let callback_cancel = cancel.clone();
            let mut pcm = PcmBlocks::default();
            let error = match MicrophoneCapture::open(
                device_id,
                capture_mode,
                move |samples, channels, pts| {
                    if callback_cancel.is_cancelled() {
                        return;
                    }
                    pcm.push(samples, channels, pts, |samples, pts| {
                        captured_track.publish(&output, pts, Arc::new(samples));
                        if !output.connected.swap(true, Ordering::AcqRel) {
                            *output.error.lock().unwrap() = None;
                        }
                    });
                },
            ) {
                Err(OpenError::Disconnected) => "Microphone is disconnected",
                Err(OpenError::PermissionPending) => "Waiting for macOS microphone permission",
                Err(OpenError::AppBundleRequired) => {
                    "Open the Acheflow macOS app to grant microphone permission"
                }
                Err(OpenError::PermissionDenied) => {
                    "Allow Acheflow microphone access in macOS System Settings"
                }
                Err(OpenError::UnsupportedMode) => {
                    "Microphone cannot apply this capture quality. Select another mode or Auto."
                }
                Err(OpenError::Unavailable) => "Cannot open the microphone",
                Ok(_capture) => {
                    let started = Instant::now();
                    runtime.block_on(async {
                        while !cancel.is_cancelled()
                            && track.latest_sample().unwrap_or(started).elapsed()
                                < Duration::from_secs(4)
                        {
                            wait(&cancel, Duration::from_millis(250)).await;
                        }
                    });
                    "Microphone stopped sending compatible PCM"
                }
            };
            input.connected.store(false, Ordering::Release);
            input.has_audio.store(false, Ordering::Relaxed);
            input.audio_tracks.lock().unwrap().clear();
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

    #[derive(Default)]
    struct PcmBlocks {
        samples: Vec<i16>,
        pts: i64,
        next_pts: Option<i64>,
    }

    impl PcmBlocks {
        fn push(
            &mut self,
            samples: &[i16],
            channels: usize,
            pts: i64,
            mut publish: impl FnMut(Vec<i16>, i64),
        ) {
            // Adjacent buffers can round their timestamps a sample apart.
            let start = match self.next_pts {
                Some(next) if next.abs_diff(pts) <= 1 => next,
                _ => {
                    self.samples.clear();
                    self.pts = pts;
                    pts
                }
            };
            self.next_pts = Some(start + (samples.len() / channels) as i64);
            for frame in samples.chunks_exact(channels) {
                self.samples.extend([frame[0], frame[channels - 1]]);
                if self.samples.len() == AUDIO_SAMPLES {
                    publish(
                        std::mem::replace(&mut self.samples, Vec::with_capacity(AUDIO_SAMPLES)),
                        self.pts,
                    );
                    self.pts += (AUDIO_SAMPLES / 2) as i64;
                }
            }
        }
    }

    #[cfg(test)]
    #[test]
    fn microphone_blocks_duplicate_mono_keep_stereo_and_reset_on_gaps() {
        let mut blocks = PcmBlocks::default();
        let mut output = Vec::new();
        let mut publish = |samples, pts| output.push((samples, pts));
        blocks.push(&[17; 480], 1, 8000, &mut publish);
        blocks.push(&[-2, 5].repeat(480), 2, 8481, &mut publish);
        blocks.push(&[99; 100], 1, 8960, &mut publish);
        blocks.push(&[7; 960], 1, 12000, publish);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].1, 8000);
        assert!(output[0].0[..960].iter().all(|&sample| sample == 17));
        assert_eq!(output[0].0[960..], [-2, 5].repeat(480));
        assert_eq!(output[1], (vec![7; AUDIO_SAMPLES], 12000));
    }
}
