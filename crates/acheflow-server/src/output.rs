use acheflow_media_api::{
    MediaCapabilities,
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
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{broadcast, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct VideoSpec {
    pub base: Resolution,
    pub fps: u32,
}

pub async fn start(
    output: Output,
    spec: VideoSpec,
    caps: &MediaCapabilities,
    video: watch::Receiver<Arc<RgbImage>>,
    mut audio: broadcast::Receiver<Arc<Vec<u8>>>,
    status: Arc<Mutex<OutputStatus>>,
    cancel: CancellationToken,
) -> Result<JoinHandle<()>, String> {
    let VideoSpec { base, fps } = spec;
    let selected = caps.resolve_video_encoder(&output.codec, output.encoder.as_deref())?;
    let encoder = selected.name.clone();
    {
        let mut status = status.lock().unwrap();
        status.encoder = Some(encoder.clone());
        status.hardware = selected.hardware;
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot create audio pipe: {e}"))?;
    let audio_url = format!(
        "tcp://{}",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let url = format!("{}/{}", output.url.trim_end_matches('/'), output.key);
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
        .args(["-c:v", &encoder, "-pix_fmt", "yuv420p"])
        .args(rate_control(
            &encoder,
            output.bitrate_mode,
            output.bitrate_kbps,
        ))
        .args(keyframe_options(fps, output.keyframe_interval_secs));
    command.args(stream_encoder_options(
        &encoder,
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
    let mut stdin = child.stdin.take().ok_or("Missing encoder stdin")?;
    let stdout = child.stdout.take().ok_or("Missing encoder progress pipe")?;
    let stderr = child.stderr.take().ok_or("Missing encoder stderr")?;
    crate::diagnostics::record("info", "output", "Output encoder started");
    Ok(tokio::spawn(async move {
        let video_cancel = cancel.clone();
        let video_status = status.clone();
        let writer = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs_f64(1. / fps as f64));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {_=video_cancel.cancelled()=>break,_=ticker.tick()=>{}}
                let image = video.borrow().clone();
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(3), stdin.write_all(image.as_raw()))
                        .await,
                    Ok(Ok(()))
                ) {
                    fail(&video_status, "Video output stopped or stalled".into());
                    video_cancel.cancel();
                    break;
                }
            }
        });
        let audio_cancel = cancel.clone();
        let audio_status = status.clone();
        let audio_writer = tokio::spawn(async move {
            let accepted = tokio::select! {_=audio_cancel.cancelled()=>return,result=tokio::time::timeout(Duration::from_secs(10),listener.accept())=>result};
            let Ok(Ok((mut socket, _))) = accepted else {
                fail(&audio_status, "Encoder audio connection failed".into());
                audio_cancel.cancel();
                return;
            };
            loop {
                let packet =
                    tokio::select! {_=audio_cancel.cancelled()=>break,packet=audio.recv()=>packet};
                let Ok(packet) = packet else {
                    fail(&audio_status, "Audio output fell behind or stopped".into());
                    audio_cancel.cancel();
                    break;
                };
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(3), socket.write_all(&packet)).await,
                    Ok(Ok(()))
                ) {
                    fail(&audio_status, "Audio output stopped or stalled".into());
                    audio_cancel.cancel();
                    break;
                }
            }
        });
        let progress = tokio::spawn(report_progress(stdout, status.clone()));
        let error_status = status.clone();
        let errors = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    let line = line
                        .replace(&url, "[RTMP destination]")
                        .replace(&output.key, "[key]");
                    crate::diagnostics::record("error", "output", &line);
                    error_status.lock().unwrap().error = Some(line.chars().take(400).collect());
                }
            }
        });
        tokio::select! {
            _=cancel.cancelled()=>{let _=child.kill().await;let _=child.wait().await;let mut state=status.lock().unwrap();if state.state!=OutputState::Failed{state.state=OutputState::Stopped;}}
            result=child.wait()=>{crate::diagnostics::record("error","output",&format!("Output encoder exited: {result:?}"));let mut state=status.lock().unwrap();state.state=OutputState::Failed;if state.error.is_none(){state.error=Some(format!("FFmpeg output exited: {result:?}"));}}
        }
        cancel.cancel();
        writer.abort();
        audio_writer.abort();
        progress.abort();
        errors.abort();
        let _ = writer.await;
        let _ = audio_writer.await;
        let _ = progress.await;
        let _ = errors.await;
        let mut status = status.lock().unwrap();
        status.bitrate_kbps = 0.;
        if status.state != OutputState::Failed {
            status.state = OutputState::Stopped;
            crate::diagnostics::record("info", "output", "Output encoder stopped");
        }
    }))
}

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
fn fail(status: &Mutex<OutputStatus>, error: String) {
    crate::diagnostics::record("error", "output", &error);
    let mut status = status.lock().unwrap();
    status.state = OutputState::Failed;
    status.error = Some(error);
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
