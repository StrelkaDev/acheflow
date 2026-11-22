use crate::audio_mixer::PcmBlock;
use acheflow_media_api::{
    MediaCapabilities,
    audio::AUDIO_SAMPLES,
    live::{OutputState, OutputStatus},
    workspace::{BitrateMode, Output, Resolution},
};
use image::RgbImage;
use std::{
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{broadcast, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

const PCM_BLOCK_BYTES: usize = AUDIO_SAMPLES * size_of::<i16>();
const MAX_AUDIO_GAP_BLOCKS: u64 = 8;
const VIDEO_CATCH_UP: Duration = Duration::from_millis(160);
const VIDEO_STARTUP_CATCH_UP: Duration = Duration::from_millis(300);
const RECONNECT_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
const STABLE_OUTPUT_SECONDS: u64 = 30;
static AUDIO_GAP_SILENCE: [u8; PCM_BLOCK_BYTES * MAX_AUDIO_GAP_BLOCKS as usize] =
    [0; PCM_BLOCK_BYTES * MAX_AUDIO_GAP_BLOCKS as usize];

#[derive(Clone)]
pub struct RenderedVideo {
    pub image: Arc<RgbImage>,
    pub rendered_at: std::time::Instant,
}

#[derive(Clone)]
pub struct VideoSpec {
    pub base: Resolution,
    pub fps: u32,
    #[cfg(test)]
    pub orientation: acheflow_media_api::workspace::Orientation,
}

pub async fn start(
    output: Output,
    spec: VideoSpec,
    caps: &MediaCapabilities,
    video: watch::Receiver<RenderedVideo>,
    audio: broadcast::Sender<Arc<PcmBlock>>,
    status: Arc<Mutex<OutputStatus>>,
    cancel: CancellationToken,
) -> Result<JoinHandle<()>, String> {
    let selected = caps.resolve_video_encoder(&output.codec, output.encoder.as_deref())?;
    let encoder = selected.name.clone();
    {
        let mut status = status.lock().unwrap();
        status.encoder = Some(encoder.clone());
        status.hardware = selected.hardware;
    }
    let mut attempt = prepare_attempt(
        &output,
        &spec,
        &encoder,
        video.clone(),
        audio.subscribe(),
        status.clone(),
        cancel.clone(),
    )
    .await?;
    Ok(tokio::spawn(async move {
        let mut retries = 0;
        loop {
            let Some(failure) = attempt.await else { break };
            if cancel.is_cancelled() {
                break;
            }
            {
                let mut output_status = status.lock().unwrap();
                if output_status.frames >= u64::from(spec.fps) * STABLE_OUTPUT_SECONDS {
                    retries = 0;
                }
                output_status.bitrate_kbps = 0.;
                if !failure.retryable || retries == RECONNECT_DELAYS.len() {
                    output_status.state = OutputState::Failed;
                    output_status.reconnect_attempt = None;
                    output_status.error = Some(if failure.retryable {
                        format!("Reconnect attempts exhausted: {}", failure.message)
                    } else {
                        failure.message
                    });
                    return;
                }
                output_status.state = OutputState::Starting;
                output_status.reconnect_attempt = Some(retries as u8 + 1);
                output_status.error = Some(failure.message);
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(RECONNECT_DELAYS[retries]) => {}
            }
            retries += 1;
            {
                let mut status = status.lock().unwrap();
                status.frames = 0;
                status.sent_bytes = 0;
                status.reconnects = status.reconnects.saturating_add(1);
            }
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                next = prepare_attempt(
                &output,
                &spec,
                &encoder,
                video.clone(),
                audio.subscribe(),
                status.clone(),
                cancel.clone(),
                ) => next,
            };
            match next {
                Ok(next) => attempt = next,
                Err(error) => {
                    let mut status = status.lock().unwrap();
                    status.state = OutputState::Failed;
                    status.reconnect_attempt = None;
                    status.error = Some(error);
                    return;
                }
            }
        }
        let mut status = status.lock().unwrap();
        status.state = OutputState::Stopped;
        status.reconnect_attempt = None;
        status.error = None;
        status.bitrate_kbps = 0.;
    }))
}

struct AttemptFailure {
    message: String,
    retryable: bool,
}

async fn prepare_attempt(
    output: &Output,
    spec: &VideoSpec,
    encoder: &str,
    video: watch::Receiver<RenderedVideo>,
    audio: broadcast::Receiver<Arc<PcmBlock>>,
    status: Arc<Mutex<OutputStatus>>,
    cancel: CancellationToken,
) -> Result<impl Future<Output = Option<AttemptFailure>> + use<>, String> {
    let fps = spec.fps;
    #[cfg(test)]
    let orientation = spec.orientation;
    let base = &spec.base;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot create audio pipe: {e}"))?;
    let audio_url = format!(
        "tcp://{}",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let destination = output.url.trim_end_matches('/').to_owned();
    let key = output.key.clone();
    let url = format!("{destination}/{key}");
    let mut command = tokio::process::Command::new(crate::ffmpeg::executable());
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-nostdin",
            "-thread_queue_size",
            "8",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-video_size",
        ])
        .arg(format!("{}x{}", base.width, base.height))
        .arg("-framerate")
        .arg(fps.to_string())
        .args([
            "-i",
            "pipe:0",
            "-thread_queue_size",
            "64",
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-i",
        ])
        .arg(audio_url)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-vf"])
        .arg(format!(
            "scale={}:{}",
            output.resolution.width, output.resolution.height
        ))
        .args(["-c:v", encoder, "-pix_fmt", "yuv420p"])
        .args(rate_control(
            encoder,
            output.bitrate_mode,
            output.bitrate_kbps,
        ))
        .args(keyframe_options(fps, output.keyframe_interval_secs));
    command.args(stream_encoder_options(
        encoder,
        &output.resolution,
        fps,
        output.bitrate_kbps,
    )?);
    command
        .args([
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-flvflags",
            "no_duration_filesize",
            "-progress",
            "pipe:1",
            "-stats_period",
            "0.5",
            "-rw_timeout",
            "3000000",
            "-f",
            "flv",
        ])
        .arg(&url);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Cannot start output encoder: {e}"))?;
    let stdin = child.stdin.take().ok_or("Missing encoder stdin")?;
    let stdout = child.stdout.take().ok_or("Missing encoder progress pipe")?;
    let stderr = child.stderr.take().ok_or("Missing encoder stderr")?;
    crate::diagnostics::record("info", "output", "Output encoder started");
    Ok(async move {
        let progress = report_progress(stdout, status.clone());
        let errors = read_encoder_errors(stderr, &url, &destination, &key);
        let (epoch_tx, epoch_rx) = oneshot::channel();
        tokio::pin!(errors);
        let mut completed_errors = None;
        let (reason, exited, local_failure) = tokio::select! {
            biased;
            _ = cancel.cancelled() => (None, false, false),
            result = child.wait() => (Some(format!("FFmpeg output exited: {result:?}")), true, false),
            diagnostics = &mut errors => {
                completed_errors = Some(diagnostics);
                (Some("FFmpeg output closed its diagnostics pipe".into()), false, false)
            }
            _ = progress => (Some("FFmpeg output closed its progress pipe".into()), false, false),
            error = write_video(stdin, video, fps, epoch_rx, &status, #[cfg(test)] orientation) => (Some(error), false, true),
            error = feed_audio(listener, audio, &cancel, epoch_tx, &status) => (error, false, true),
        };
        if !exited {
            let _ = child.kill().await;
        }
        let diagnostics = match completed_errors {
            Some(diagnostics) => Some(diagnostics),
            None => tokio::time::timeout(Duration::from_millis(250), errors)
                .await
                .ok(),
        };
        if cancel.is_cancelled() {
            return None;
        }
        let reason = reason?;
        let diagnostics = diagnostics.unwrap_or_default();
        let message = diagnostics
            .relevant_error
            .or(diagnostics.last_line.filter(|_| !local_failure))
            .unwrap_or(reason);
        crate::diagnostics::record("error", "output", &message);
        Some(AttemptFailure {
            message,
            retryable: diagnostics.retryable && !diagnostics.permanent,
        })
    })
}

async fn write_video(
    mut stdin: tokio::process::ChildStdin,
    video: watch::Receiver<RenderedVideo>,
    fps: u32,
    epoch: oneshot::Receiver<tokio::time::Instant>,
    status: &Mutex<OutputStatus>,
    #[cfg(test)] orientation: acheflow_media_api::workspace::Orientation,
) -> String {
    let Ok(started) = epoch.await else {
        return "Encoder audio clock did not start".into();
    };
    let period = Duration::from_secs_f64(1. / fps as f64);
    let mut ticker = tokio::time::interval_at(started, period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut next = 0;
    let mut previous = video.borrow().clone();
    #[cfg(test)]
    crate::av_timing_tests::trace_epoch(orientation, started.into_std());
    loop {
        ticker.tick().await;
        let elapsed = started.elapsed();
        let skipped =
            skipped_video_frames(next, elapsed, period, VIDEO_CATCH_UP).or_else(|error| {
                // FFmpeg probes the raw inputs before it reports progress. Allow
                // a bounded initial burst without raising the on-air budget.
                if status.lock().unwrap().frames == 0 {
                    skipped_video_frames(next, elapsed, period, VIDEO_STARTUP_CATCH_UP)
                } else {
                    Err(error)
                }
            });
        let skipped = match skipped {
            Ok(Some(skipped)) => skipped,
            Ok(None) => continue,
            Err(error) => return error,
        };
        let image = video.borrow().clone();
        let old_slots = skipped_before_render(
            next,
            skipped,
            period,
            image
                .rendered_at
                .saturating_duration_since(started.into_std()),
        );
        let write = async {
            for i in 0..=skipped {
                let frame = if i < old_slots { &previous } else { &image };
                stdin.write_all(frame.image.as_raw()).await?;
                #[cfg(test)]
                crate::av_timing_tests::trace_image(
                    "encoder",
                    orientation,
                    frame.rendered_at,
                    Some(((next + i) * period.as_nanos() * 48_000 / 1_000_000_000) as i64),
                    &frame.image,
                );
            }
            Ok::<(), std::io::Error>(())
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(3), write).await,
            Ok(Ok(()))
        ) {
            return "Video output stopped or stalled".into();
        }
        if skipped > 0 {
            let mut status = status.lock().unwrap();
            status.repeated_video_frames =
                status.repeated_video_frames.saturating_add(skipped as u64);
        }
        previous = image;
        next += skipped + 1;
    }
}

fn skipped_before_render(next: u128, skipped: u128, period: Duration, completed: Duration) -> u128 {
    completed
        .as_nanos()
        .div_ceil(period.as_nanos())
        .saturating_sub(next)
        .min(skipped)
}

#[test]
fn catch_up_uses_a_new_frame_only_after_its_render_deadline() {
    let period = Duration::from_millis(40);
    assert_eq!(
        skipped_before_render(2, 3, period, Duration::from_millis(109)),
        1
    );
    assert_eq!(
        skipped_before_render(2, 3, period, Duration::from_millis(80)),
        0
    );
    assert_eq!(
        skipped_before_render(2, 3, period, Duration::from_millis(205)),
        3
    );
}

fn skipped_video_frames(
    next: u128,
    elapsed: Duration,
    period: Duration,
    budget: Duration,
) -> Result<Option<u128>, String> {
    let slot = elapsed.as_nanos() / period.as_nanos();
    let Some(skipped) = slot.checked_sub(next) else {
        return Ok(None);
    };
    if skipped > budget.as_nanos().div_ceil(period.as_nanos()) {
        return Err(format!(
            "Video output fell behind: {skipped} frame slots missed"
        ));
    }
    Ok(Some(skipped))
}

#[cfg(test)]
#[test]
fn video_clock_preserves_missed_slots_within_its_catch_up_budget() {
    let frame = Duration::from_millis(40);
    assert_eq!(
        skipped_video_frames(1, frame, frame, VIDEO_CATCH_UP).unwrap(),
        Some(0)
    );
    assert_eq!(
        skipped_video_frames(1, Duration::from_millis(125), frame, VIDEO_CATCH_UP).unwrap(),
        Some(2)
    );
    assert_eq!(
        skipped_video_frames(4, Duration::from_millis(125), frame, VIDEO_CATCH_UP).unwrap(),
        None
    );
    assert!(skipped_video_frames(1, Duration::from_millis(260), frame, VIDEO_CATCH_UP).is_err());
    assert_eq!(
        skipped_video_frames(1, Duration::from_millis(260), frame, VIDEO_STARTUP_CATCH_UP).unwrap(),
        Some(5)
    );
}

async fn feed_audio(
    listener: TcpListener,
    mut audio: broadcast::Receiver<Arc<PcmBlock>>,
    cancel: &CancellationToken,
    epoch: oneshot::Sender<tokio::time::Instant>,
    status: &Mutex<OutputStatus>,
) -> Option<String> {
    // Wait for a mixer slot before feeding either encoder pipe. FFmpeg needs
    // video before it opens the PCM socket, so retain this first block here.
    let first = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match audio.recv().await {
                Ok(block) => return Some(block),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .await;
    let first = match first {
        Ok(Some(block)) => block,
        Ok(None) => return Some("Audio mixer channel closed".into()),
        Err(_) => return Some("Audio mixer clock did not start".into()),
    };
    if epoch.send(first.scheduled).is_err() {
        return None;
    }
    let Ok(Ok((socket, _))) =
        tokio::time::timeout(Duration::from_secs(10), listener.accept()).await
    else {
        return Some("Encoder audio connection failed".into());
    };
    write_audio(socket, audio, cancel, Some(first), Some(status))
        .await
        .err()
}

#[derive(Default)]
struct EncoderDiagnostics {
    last_line: Option<String>,
    relevant_error: Option<String>,
    retryable: bool,
    permanent: bool,
}

async fn read_encoder_errors(
    stderr: tokio::process::ChildStderr,
    url: &str,
    destination: &str,
    key: &str,
) -> EncoderDiagnostics {
    let mut lines = BufReader::new(stderr).lines();
    let mut diagnostics = EncoderDiagnostics::default();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let line = line
            .replace(url, "[RTMP destination]")
            .replace(destination, "[RTMP destination]")
            .replace(key, "[key]");
        crate::diagnostics::record("error", "output", &line);
        let message: String = line.chars().take(400).collect();
        let lower = line.to_ascii_lowercase();
        if [
            "access denied",
            "authentication",
            "unauthorized",
            "forbidden",
            "invalid stream key",
            "netstream.publish.badname",
            "certificate verify failed",
            "unrecognized option",
            "error while opening encoder",
        ]
        .iter()
        .any(|text| lower.contains(text))
        {
            diagnostics.permanent = true;
            diagnostics.relevant_error = Some(message.clone());
        } else if !diagnostics.permanent
            && [
                "connection refused",
                "connection reset by peer",
                "connection timed out",
                "operation timed out",
                "network is unreachable",
                "no route to host",
                "temporary failure in name resolution",
                "broken pipe",
            ]
            .iter()
            .any(|text| lower.contains(text))
        {
            diagnostics.retryable = true;
            diagnostics.relevant_error = Some(message.clone());
        }
        diagnostics.last_line = Some(message);
    }
    diagnostics
}

async fn write_audio(
    mut socket: impl AsyncWrite + Unpin,
    mut audio: broadcast::Receiver<Arc<PcmBlock>>,
    cancel: &CancellationToken,
    mut first: Option<Arc<PcmBlock>>,
    status: Option<&Mutex<OutputStatus>>,
) -> Result<(), String> {
    let mut lost_blocks = 0u64;
    let mut next_sequence = None;
    let mut initial_lag = 0u64;
    loop {
        let packet = if let Some(first) = first.take() {
            Ok(first)
        } else {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(()),
                packet = audio.recv() => packet,
            }
        };
        let packet = match packet {
            Ok(packet) => packet,
            Err(broadcast::error::RecvError::Closed) => {
                return Err("Audio mixer channel closed".into());
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                if next_sequence.is_none() {
                    initial_lag = initial_lag.saturating_add(skipped);
                }
                continue;
            }
        };
        let skipped = match next_sequence {
            Some(next) => packet
                .sequence
                .checked_sub(next)
                .ok_or("Audio mixer clock moved backwards")?,
            None => initial_lag,
        };
        next_sequence = Some(packet.sequence + 1);
        lost_blocks = lost_blocks.saturating_add(skipped);
        if lost_blocks > MAX_AUDIO_GAP_BLOCKS {
            return Err(format!(
                "Audio output fell behind: {lost_blocks} audio blocks lost before catching up"
            ));
        }
        // Raw PCM carries only sample count. Cover both mixer scheduling gaps
        // and dropped receiver packets exactly once.
        let write = async {
            if skipped > 0 {
                socket
                    .write_all(&AUDIO_GAP_SILENCE[..skipped as usize * PCM_BLOCK_BYTES])
                    .await?;
            }
            socket.write_all(&packet.bytes).await
        };
        let written = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            written = tokio::time::timeout(Duration::from_secs(3), write) => written,
        };
        match written {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(format!("Encoder audio socket write failed: {error}")),
            Err(_) => return Err("Encoder audio socket write timed out".into()),
        }
        if skipped > 0
            && let Some(status) = status
        {
            let mut status = status.lock().unwrap();
            status.inserted_audio_blocks = status.inserted_audio_blocks.saturating_add(skipped);
        }
        if lost_blocks != 0 && audio.is_empty() {
            crate::diagnostics::record(
                "warn",
                "output",
                &format!("Audio output recovered after replacing {lost_blocks} lost PCM blocks"),
            );
            lost_blocks = 0;
        }
    }
}

#[cfg(test)]
mod audio_tests;

async fn report_progress(
    stdout: impl tokio::io::AsyncRead + Unpin,
    status: Arc<Mutex<OutputStatus>>,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut traffic = crate::traffic::Traffic::default();
    let mut previous_bytes = 0;
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(bytes) = line
            .strip_prefix("total_size=")
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            traffic.record(bytes.saturating_sub(previous_bytes));
            previous_bytes = bytes;
            let (sent_bytes, bitrate_kbps) = traffic.snapshot();
            let mut status = status.lock().unwrap();
            status.sent_bytes = sent_bytes;
            status.bitrate_kbps = bitrate_kbps;
        }
        if let Some(frames) = line
            .strip_prefix("frame=")
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            let mut status = status.lock().unwrap();
            status.frames = frames;
            if frames > 0 && status.state != OutputState::Failed {
                status.state = OutputState::Streaming;
                status.reconnect_attempt = None;
                status.error = None;
            }
        }
    }
}

// A GOP size is only an encoder limit. Also request keyframes on the media
// timeline so scene decisions do not move the periodic keyframes.
// https://ffmpeg.org/ffmpeg.html#Advanced-Video-options
fn keyframe_options(fps: u32, seconds: u32) -> [String; 4] {
    [
        "-g".into(),
        (fps * seconds).to_string(),
        "-force_key_frames:v".into(),
        format!("expr:gte(t,n_forced*{seconds})"),
    ]
}

// Encoder-specific switches select actual rate control rather than only a ceiling.
fn rate_control(encoder: &str, mode: BitrateMode, kbps: u32) -> Vec<String> {
    if mode == BitrateMode::Cbr && encoder.ends_with("_videotoolbox") {
        // FFmpeg maps maxrate to DataRateLimits after ConstantBitRate. Setting
        // both overrides strict CBR on Apple Silicon, even for static video.
        return vec![
            "-b:v".into(),
            format!("{kbps}k"),
            "-constant_bit_rate".into(),
            "1".into(),
        ];
    }
    let mut args = vec![
        "-b:v".into(),
        format!("{kbps}k"),
        "-maxrate".into(),
        format!("{kbps}k"),
        "-bufsize".into(),
        format!("{}k", kbps * 2),
    ];
    if mode == BitrateMode::Cbr {
        args.extend(["-minrate".into(), format!("{kbps}k")]);
    }
    let cbr = mode == BitrateMode::Cbr;
    match encoder {
        "libx264" | "libx264rgb" if cbr => {
            args.extend(["-x264-params".into(), "nal-hrd=cbr:force-cfr=1".into()])
        }
        "libx265" => args.extend([
            "-x265-params".into(),
            if cbr {
                "log-level=error:strict-cbr=1:hrd=1"
            } else {
                "log-level=error"
            }
            .into(),
        ]),
        name if name.ends_with("_videotoolbox") => {
            args.extend(["-constant_bit_rate".into(), u8::from(cbr).to_string()])
        }
        name if name.ends_with("_nvenc") => {
            args.extend(["-rc".into(), if cbr { "cbr" } else { "vbr" }.into()])
        }
        name if name.ends_with("_amf") => {
            args.extend(["-rc".into(), if cbr { "cbr" } else { "vbr_peak" }.into()])
        }
        _ => {}
    }
    args
}

fn encoder_options(encoder: &str) -> &'static [&'static str] {
    match encoder {
        "libx264" | "libx264rgb" => &["-preset", "veryfast", "-tune", "zerolatency"],
        "libx265" => &["-preset", "ultrafast"],
        "libsvtav1" => &["-preset", "12"],
        "libvpx-vp9" => &["-deadline", "realtime", "-cpu-used", "8"],
        // FFmpeg VideoToolbox can use a software implementation. Require hardware.
        // https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavcodec/videotoolboxenc.c
        name if name.ends_with("_videotoolbox") => {
            &["-realtime", "1", "-allow_sw", "0", "-require_sw", "0"]
        }
        // https://ffmpeg.org/doxygen/9.0/mfenc_8c.html
        name if name.ends_with("_mf") => &["-hw_encoding", "1"],
        _ => &[],
    }
}
fn stream_encoder_options(
    encoder: &str,
    size: &Resolution,
    fps: u32,
    kbps: u32,
) -> Result<Vec<String>, String> {
    let mut options: Vec<String> = encoder_options(encoder)
        .iter()
        .map(|s| (*s).into())
        .collect();
    if encoder == "h264_videotoolbox" {
        // VideoToolbox AutoLevel can declare 4.0 even for 1080p60. Select a
        // supported High-profile level using H.264 frame, rate and bitrate limits.
        // https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavcodec/h264_levels.c
        let width = u64::from(size.width).div_ceil(16);
        let height = u64::from(size.height).div_ceil(16);
        let blocks = width * height;
        let level = [
            ("4.0", 8192, 245_760, 25_000),
            ("4.1", 8192, 245_760, 62_500),
            ("4.2", 8704, 522_240, 62_500),
            ("5.1", 36_864, 983_040, 300_000),
            ("5.2", 36_864, 2_073_600, 300_000),
        ]
        .into_iter()
        .find(|(_, max_frame, max_rate, max_kbps)| {
            blocks <= *max_frame
                && width * width <= max_frame * 8
                && height * height <= max_frame * 8
                && blocks * u64::from(fps) <= *max_rate
                && kbps <= *max_kbps
        })
        .map(|(level, ..)| level)
        .ok_or("Output exceeds supported VideoToolbox H.264 levels; reduce resolution or FPS")?;
        options.extend([
            "-profile:v".into(),
            "high".into(),
            "-level:v".into(),
            level.into(),
        ]);
    }
    Ok(options)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires FFmpeg, ffprobe and actual VideoToolbox hardware; run explicitly"]
    async fn configured_keyframe_intervals_produce_periodic_keyframes() {
        for encoder in ["libx264", "h264_videotoolbox", "hevc_videotoolbox"] {
            for (fps, seconds) in [(24, 1), (30, 2), (60, 3)] {
                let encoded = tokio::process::Command::new(crate::ffmpeg::executable())
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-nostdin",
                        "-f",
                        "lavfi",
                        "-i",
                    ])
                    .arg(format!("testsrc2=s=320x180:r={fps}"))
                    .args([
                        "-t",
                        &(seconds * 3).to_string(),
                        "-c:v",
                        encoder,
                        "-pix_fmt",
                        "yuv420p",
                    ])
                    .args(rate_control(encoder, BitrateMode::Cbr, 500))
                    .args(
                        stream_encoder_options(
                            encoder,
                            &Resolution {
                                width: 320,
                                height: 180,
                            },
                            fps,
                            500,
                        )
                        .unwrap(),
                    )
                    .args(keyframe_options(fps, seconds))
                    .args([
                        "-movflags",
                        "frag_keyframe+empty_moov",
                        "-f",
                        "mp4",
                        "pipe:1",
                    ])
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                assert!(
                    encoded.status.success(),
                    "{encoder}: {}",
                    String::from_utf8_lossy(&encoded.stderr)
                );
                let mut probe = tokio::process::Command::new("ffprobe")
                    .args([
                        "-v",
                        "error",
                        "-select_streams",
                        "v:0",
                        "-skip_frame",
                        "nokey",
                        "-show_frames",
                        "-show_entries",
                        "frame=best_effort_timestamp_time",
                        "-of",
                        "json",
                        "pipe:0",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap();
                probe
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(&encoded.stdout)
                    .await
                    .unwrap();
                let decoded = probe.wait_with_output().await.unwrap();
                assert!(
                    decoded.status.success(),
                    "{}",
                    String::from_utf8_lossy(&decoded.stderr)
                );
                let json: serde_json::Value = serde_json::from_slice(&decoded.stdout).unwrap();
                let times: Vec<f64> = json["frames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|frame| {
                        frame["best_effort_timestamp_time"]
                            .as_str()
                            .unwrap()
                            .parse()
                            .unwrap()
                    })
                    .collect();
                let first = times[0];
                for n in 0..3 {
                    let expected = first + f64::from(n * seconds);
                    assert!(
                        times
                            .iter()
                            .any(|t| (t - expected).abs() < 1. / f64::from(fps)),
                        "{encoder} {fps} FPS / {seconds}s: missing keyframe at {expected}; {times:?}"
                    );
                }
                assert!(
                    times
                        .windows(2)
                        .all(|pair| pair[1] - pair[0] <= f64::from(seconds) + 1. / f64::from(fps))
                );
                println!("{encoder} {fps} FPS / {seconds}s: keyframes {times:?}");
            }
        }
    }

    #[test]
    fn videotoolbox_cbr_does_not_override_constant_rate_with_data_rate_limits() {
        for encoder in ["h264_videotoolbox", "hevc_videotoolbox"] {
            let cbr = rate_control(encoder, BitrateMode::Cbr, 9000);
            assert!(cbr.windows(2).any(|p| p == ["-constant_bit_rate", "1"]));
            assert!(cbr.windows(2).any(|p| p == ["-b:v", "9000k"]));
            assert!(
                !cbr.iter()
                    .any(|s| ["-maxrate", "-minrate", "-bufsize"].contains(&s.as_str()))
            );
            let vbr = rate_control(encoder, BitrateMode::Vbr, 9000);
            assert!(vbr.windows(2).any(|p| p == ["-maxrate", "9000k"]));
            assert!(vbr.windows(2).any(|p| p == ["-constant_bit_rate", "0"]));
        }
    }
    #[tokio::test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires actual VideoToolbox hardware; run explicitly"]
    async fn videotoolbox_cbr_keeps_the_requested_rate_on_static_video() {
        for (width, height, fps, kbps) in [(1920, 1080, 60, 9000), (1080, 1920, 30, 8000)] {
            for pattern in ["color=c=red:s", "testsrc2=s"] {
                let result = tokio::process::Command::new(crate::ffmpeg::executable())
                    .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
                    .arg(format!("{pattern}={width}x{height}:r={fps}"))
                    .args(["-t", "8", "-c:v", "h264_videotoolbox"])
                    .args(rate_control("h264_videotoolbox", BitrateMode::Cbr, kbps))
                    .args(
                        stream_encoder_options(
                            "h264_videotoolbox",
                            &Resolution { width, height },
                            fps,
                            kbps,
                        )
                        .unwrap(),
                    )
                    .args(["-f", "h264", "pipe:1"])
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let actual_kbps = result.stdout.len() as f64 * 8. / 8. / 1000.;
                assert!(
                    (0.85..=1.15).contains(&(actual_kbps / f64::from(kbps))),
                    "{pattern} {width}x{height}@{fps}: requested {kbps} kbps, measured {actual_kbps:.0} kbps"
                );
                println!(
                    "{pattern} {width}x{height}@{fps}: requested {kbps} kbps, measured {actual_kbps:.0} kbps"
                );
            }
        }
    }
    #[tokio::test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires actual VideoToolbox hardware; run explicitly"]
    async fn videotoolbox_hd60_declares_a_compatible_h264_level() {
        let size = Resolution {
            width: 1920,
            height: 1080,
        };
        let result = tokio::process::Command::new(crate::ffmpeg::executable())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=1920x1080:r=60",
                "-frames:v",
                "6",
                "-c:v",
                "h264_videotoolbox",
            ])
            .args(rate_control("h264_videotoolbox", BitrateMode::Cbr, 9000))
            .args(stream_encoder_options("h264_videotoolbox", &size, 60, 9000).unwrap())
            .args(["-f", "h264", "pipe:1"])
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let sps = result
            .stdout
            .windows(7)
            .find(|b| b[..3] == [0, 0, 1] && b[3] & 31 == 7)
            .expect("encoded video must include an SPS");
        assert!(
            sps[6] >= 42,
            "1080p60 needs H.264 level 4.2 or higher, got {}",
            sps[6]
        );
    }
    #[test]
    fn videotoolbox_levels_cover_frame_rate_orientation_and_bitrate() {
        for (width, height, fps, kbps, level) in [
            (1920, 1080, 30, 9000, "4.0"),
            (1080, 1920, 30, 8000, "4.0"),
            (1920, 1080, 60, 9000, "4.2"),
            (1080, 1920, 60, 9000, "4.2"),
            (1920, 1080, 30, 30_000, "4.1"),
            (3840, 2160, 30, 30_000, "5.1"),
            (3840, 2160, 60, 40_000, "5.2"),
        ] {
            let options = stream_encoder_options(
                "h264_videotoolbox",
                &Resolution { width, height },
                fps,
                kbps,
            )
            .unwrap();
            assert!(options.windows(2).any(|p| p == ["-level:v", level]));
        }
        let size = Resolution {
            width: 7680,
            height: 4320,
        };
        assert!(stream_encoder_options("h264_videotoolbox", &size, 60, 50_000).is_err());
        assert_eq!(
            stream_encoder_options("libx264", &size, 60, 50_000).unwrap(),
            encoder_options("libx264")
        );
        assert_eq!(
            stream_encoder_options("hevc_videotoolbox", &size, 60, 50_000).unwrap(),
            encoder_options("hevc_videotoolbox")
        );
    }
    #[test]
    fn videotoolbox_is_required_to_use_hardware() {
        for encoder in ["h264_videotoolbox", "hevc_videotoolbox"] {
            let options = encoder_options(encoder);
            assert!(options.windows(2).any(|p| p == ["-allow_sw", "0"]));
            assert!(options.windows(2).any(|p| p == ["-require_sw", "0"]));
        }
        assert!(!encoder_options("libx264").contains(&"-allow_sw"));
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    #[ignore = "requires FFmpeg and actual VideoToolbox hardware; run explicitly"]
    async fn installed_hardware_and_software_encoders_produce_real_frames() {
        let caps = crate::ffmpeg::discover().await;
        for name in ["libx264", "h264_videotoolbox", "hevc_videotoolbox"] {
            let codec = if name.starts_with("hevc") {
                "hevc"
            } else {
                "h264"
            };
            let encoder = caps.resolve_video_encoder(codec, Some(name)).unwrap();
            assert_eq!(encoder.hardware, name.ends_with("_videotoolbox"));
            let mut command = tokio::process::Command::new(crate::ffmpeg::executable());
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=red:s=320x180:r=30",
                    "-frames:v",
                    "6",
                    "-c:v",
                    name,
                    "-pix_fmt",
                    "yuv420p",
                    "-b:v",
                    "500k",
                ])
                .args(encoder_options(name))
                .args(["-f", "framecrc", "-"])
                .kill_on_drop(true);
            let result = tokio::time::timeout(Duration::from_secs(20), command.output())
                .await
                .unwrap()
                .unwrap();
            assert!(
                result.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let frames = String::from_utf8_lossy(&result.stdout)
                .lines()
                .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                .count();
            assert_eq!(frames, 6, "{name} must encode every frame");
            println!(
                "{name}: {} · {frames} encoded frames",
                if encoder.hardware {
                    "Hardware"
                } else {
                    "Software"
                }
            );
        }
    }
}
