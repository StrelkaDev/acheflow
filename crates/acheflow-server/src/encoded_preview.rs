//! Local video encode/decode for comparing destination profiles.
use crate::output::{self, VideoSpec};
use acheflow_media_api::{
    MediaCapabilities,
    live::ENCODED_PREVIEW_SNAPSHOT_FPS,
    workspace::{Output, Resolution},
};
use axum::{
    body::Bytes,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use image::{ImageEncoder, RgbImage};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Weak},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, watch},
    time::{Duration, Instant, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;

const MAX_PROFILES: usize = 2;
const IDLE: Duration = Duration::from_secs(4);
const RETRY_DELAY: Duration = Duration::from_secs(5);

pub(crate) struct PreviewSource {
    pub spec: VideoSpec,
    pub frames: watch::Receiver<Arc<RgbImage>>,
    pub cancel: CancellationToken,
}

#[derive(Default)]
pub(crate) struct EncodedPreviews {
    views: HashMap<usize, Arc<EncodedView>>,
}

pub(crate) struct EncodedView {
    cancel: CancellationToken,
    last_request: watch::Sender<Instant>,
    state: watch::Receiver<PreviewState>,
    png: Mutex<Option<(Weak<RgbImage>, Bytes)>>,
}

enum PreviewState {
    Starting,
    Frame(Arc<RgbImage>),
    Failed(Instant),
}

impl EncodedPreviews {
    pub(crate) fn invalidate(&mut self, index: usize) {
        if let Some(view) = self.views.remove(&index) {
            view.cancel.cancel();
        }
    }

    pub(crate) fn view(
        &mut self,
        index: usize,
        output: Output,
        mut source: PreviewSource,
        capabilities: &MediaCapabilities,
    ) -> Result<Arc<EncodedView>, StatusCode> {
        self.views.retain(|_, view| {
            if let PreviewState::Failed(at) = *view.state.borrow() {
                at.elapsed() < RETRY_DELAY
            } else {
                view.last_request.receiver_count() > 0
            }
        });
        if let Some(view) = self.views.get(&index) {
            view.last_request.send_replace(Instant::now());
            return Ok(view.clone());
        }
        if self
            .views
            .values()
            .filter(|view| view.last_request.receiver_count() > 0)
            .count()
            >= MAX_PROFILES
        {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        if !output.has_valid_video_profile()
            || u64::from(output.resolution.width) * u64::from(output.resolution.height)
                > 3840 * 2160
            || !capabilities
                .output_codecs(output.protocol)
                .contains(&output.codec)
        {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let encoder = capabilities
            .resolve_video_encoder(&output.codec, output.encoder.as_deref())
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?
            .name
            .clone();
        source.cancel = source.cancel.child_token();
        let (last_request, lease) = watch::channel(Instant::now());
        let (state, receiver) = watch::channel(PreviewState::Starting);
        let view = Arc::new(EncodedView {
            cancel: source.cancel.clone(),
            last_request,
            state: receiver,
            png: Mutex::new(None),
        });
        tokio::spawn(async move {
            let result = run(output, source, &encoder, &state, lease).await;
            if let Err(error) = result {
                crate::diagnostics::record("warn", "preview", &error);
                state.send_replace(PreviewState::Failed(Instant::now()));
            }
        });
        self.views.insert(index, view.clone());
        Ok(view)
    }
}

impl EncodedView {
    pub(crate) async fn response(&self) -> Response {
        let mut png = self.png.lock().await;
        let image = match &*self.state.borrow() {
            PreviewState::Starting => return StatusCode::ACCEPTED.into_response(),
            PreviewState::Failed(_) => return StatusCode::BAD_GATEWAY.into_response(),
            PreviewState::Frame(image) => image.clone(),
        };
        let (width, height) = image.dimensions();
        let identity = Arc::downgrade(&image);
        let bytes =
            if let Some((_, bytes)) = png.as_ref().filter(|(frame, _)| frame.ptr_eq(&identity)) {
                bytes.clone()
            } else {
                let Ok(Ok(bytes)) = tokio::task::spawn_blocking(move || encode_png(&image)).await
                else {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                };
                png.replace((identity, bytes.clone()));
                bytes
            };
        let mut response = (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response();
        response.headers_mut().insert("x-frame-width", width.into());
        response
            .headers_mut()
            .insert("x-frame-height", height.into());
        response
    }
}

pub(crate) fn encode_png(image: &RgbImage) -> image::ImageResult<Bytes> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(bytes.into())
}

async fn run(
    output: Output,
    source: PreviewSource,
    encoder_name: &str,
    state: &watch::Sender<PreviewState>,
    mut lease: watch::Receiver<Instant>,
) -> Result<(), String> {
    let PreviewSource {
        spec,
        frames,
        cancel,
    } = source;
    let mut command = output::video_input(&spec);
    command.args(["-map", "0:v:0", "-an"]);
    output::configure_video_output(&mut command, &output, encoder_name, spec.fps)?;
    command.args(["-flush_packets", "1"]);
    output::configure_muxer(&mut command, output.protocol);
    let mut encoder = command
        .arg("pipe:1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdin = encoder.stdin.take().ok_or("Missing preview input pipe")?;
    let connection: Stdio = encoder
        .stdout
        .take()
        .ok_or("Missing preview stream pipe")?
        .try_into()
        .map_err(|error: std::io::Error| error.to_string())?;
    let mut decoder = tokio::process::Command::new(crate::ffmpeg::executable())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-threads",
            "1",
            "-probesize",
            "32768",
            "-analyzeduration",
            "100000",
            "-f",
            output.protocol.muxer(),
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-an",
            "-vf",
        ])
        .arg(format!(
            "fps={}",
            spec.fps.min(ENCODED_PREVIEW_SNAPSHOT_FPS)
        ))
        .args([
            "-pix_fmt",
            "rgb24",
            "-fps_mode",
            "passthrough",
            "-threads:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(connection)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = decoder
        .stdout
        .take()
        .ok_or("Missing decoded preview pipe")?;
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Ok(()),
        _ = wait_for_idle(&mut lease) => Ok(()),
        result = feed_frames(stdin, frames, spec.fps) => result,
        result = read_frames(stdout, output.resolution, spec.fps, state) => result,
    };
    let _ = tokio::join!(encoder.kill(), decoder.kill());
    result
}

async fn wait_for_idle(lease: &mut watch::Receiver<Instant>) {
    loop {
        let deadline = *lease.borrow_and_update() + IDLE;
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return,
            changed = lease.changed() => if changed.is_err() { return; },
        }
    }
}

async fn feed_frames(
    mut pipe: tokio::process::ChildStdin,
    frames: watch::Receiver<Arc<RgbImage>>,
    fps: u32,
) -> Result<(), String> {
    let mut ticks = tokio::time::interval(Duration::from_secs_f64(1. / f64::from(fps)));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        let image = frames.borrow().clone();
        tokio::time::timeout(Duration::from_secs(3), pipe.write_all(image.as_raw()))
            .await
            .map_err(|_| "Preview encoder stalled")?
            .map_err(|error| error.to_string())?;
    }
}

async fn read_frames(
    mut pipe: tokio::process::ChildStdout,
    size: Resolution,
    fps: u32,
    state: &watch::Sender<PreviewState>,
) -> Result<(), String> {
    let bytes = size.width as usize * size.height as usize * 3;
    let timeout = Duration::from_secs(10) + Duration::from_secs_f64(30. / f64::from(fps));
    loop {
        let mut pixels = vec![0; bytes];
        tokio::time::timeout(timeout, pipe.read_exact(&mut pixels))
            .await
            .map_err(|_| "Preview decoder stalled")?
            .map_err(|error| error.to_string())?;
        let image =
            RgbImage::from_raw(size.width, size.height, pixels).expect("matching RGB frame size");
        state.send_replace(PreviewState::Frame(Arc::new(image)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::workspace::{BitrateMode, Orientation};

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn local_profile_preview_uses_the_configured_bitrate_and_expires_when_idle() {
        let capabilities = crate::ffmpeg::discover().await;
        let spec = VideoSpec {
            base: Resolution {
                width: 320,
                height: 180,
            },
            fps: 30,
            orientation: Orientation::Horizontal,
        };
        let original = RgbImage::from_fn(320, 180, |x, y| {
            let noise = ((x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263))
                .rotate_left(13)
                .wrapping_mul(1274126177)
                >> 24) as u8;
            image::Rgb([noise; 3])
        });
        let (_tx, frames) = watch::channel(Arc::new(original.clone()));
        let cancel = CancellationToken::new();
        let mut preview = EncodedPreviews::default();
        let mut views = Vec::new();
        for (index, bitrate_kbps) in [100, 4000].into_iter().enumerate() {
            let output = Output {
                protocol: Default::default(),
                name: format!("Profile {index}"),
                url: String::new(),
                key: String::new(),
                resolution: spec.base.clone(),
                quality: "Custom".into(),
                codec: "h264".into(),
                encoder: Some("libx264".into()),
                bitrate_kbps,
                bitrate_mode: BitrateMode::Cbr,
                keyframe_interval_secs: 1,
                enabled: false,
                audio: Default::default(),
            };
            views.push(
                preview
                    .view(
                        index,
                        output,
                        PreviewSource {
                            spec: spec.clone(),
                            frames: frames.clone(),
                            cancel: cancel.clone(),
                        },
                        &capabilities,
                    )
                    .unwrap(),
            );
        }
        let score = |view: Arc<EncodedView>| {
            let original = &original;
            async move {
                let response = tokio::time::timeout(Duration::from_secs(12), async {
                    loop {
                        view.last_request.send_replace(Instant::now());
                        let response = view.response().await;
                        if response.status() != StatusCode::ACCEPTED {
                            break response;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
                assert_eq!(decoded.dimensions(), original.dimensions());
                decoded
                    .as_raw()
                    .iter()
                    .zip(original.as_raw())
                    .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                    .sum::<f64>()
                    / original.as_raw().len() as f64
            }
        };
        let (low, high) = tokio::join!(score(views[0].clone()), score(views[1].clone()));
        assert!(
            high < low * 0.7,
            "low bitrate MSE {low}, high bitrate MSE {high}"
        );
        tokio::time::timeout(IDLE + Duration::from_secs(2), async {
            while views
                .iter()
                .any(|view| view.last_request.receiver_count() > 0)
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("preview workers did not expire");
    }
}
