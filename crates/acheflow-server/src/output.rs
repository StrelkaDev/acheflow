use acheflow_media_api::{
    MediaCapabilities,
    live::{OutputState, OutputStatus},
    workspace::{Output, Resolution},
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
    let encoder = output
        .encoder
        .clone()
        .or_else(|| {
            let preferred = match output.codec.as_str() {
                "h264" => "libx264",
                "hevc" => "libx265",
                "av1" => "libsvtav1",
                "vp9" => "libvpx-vp9",
                _ => "",
            };
            caps.video_encoders(&output.codec)
                .find(|e| e.name == preferred)
                .or_else(|| caps.video_encoders(&output.codec).next())
                .map(|e| e.name.clone())
        })
        .ok_or("No matching FFmpeg encoder")?;
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
        .args(["-c:v", &encoder, "-pix_fmt", "yuv420p", "-b:v"])
        .arg(format!("{}k", output.bitrate_kbps))
        .arg("-maxrate")
        .arg(format!("{}k", output.bitrate_kbps))
        .arg("-bufsize")
        .arg(format!("{}k", output.bitrate_kbps * 2))
        .arg("-g")
        .arg((fps * 2).to_string());
    match encoder.as_str() {
        "libx264" | "libx264rgb" => {
            command.args(["-preset", "veryfast", "-tune", "zerolatency"]);
        }
        "libx265" => {
            command.args(["-preset", "ultrafast", "-x265-params", "log-level=error"]);
        }
        "libsvtav1" => {
            command.args(["-preset", "12"]);
        }
        "libvpx-vp9" => {
            command.args(["-deadline", "realtime", "-cpu-used", "8"]);
        }
        name if name.ends_with("_videotoolbox") => {
            command.args(["-realtime", "1"]);
        }
        _ => {}
    }
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
        let progress_status = status.clone();
        let progress = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(frames) = line
                    .strip_prefix("frame=")
                    .and_then(|s| s.trim().parse::<u64>().ok())
                {
                    let mut status = progress_status.lock().unwrap();
                    status.frames = frames;
                    if frames > 0 && status.state != OutputState::Failed {
                        status.state = OutputState::Streaming;
                        status.error = None;
                    }
                }
            }
        });
        let error_status = status.clone();
        let errors = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    let line = line
                        .replace(&url, "[RTMP destination]")
                        .replace(&output.key, "[key]");
                    error_status.lock().unwrap().error = Some(line.chars().take(400).collect());
                }
            }
        });
        tokio::select! {
            _=cancel.cancelled()=>{let _=child.kill().await;let _=child.wait().await;let mut state=status.lock().unwrap();if state.state!=OutputState::Failed{state.state=OutputState::Stopped;}}
            result=child.wait()=>{let mut state=status.lock().unwrap();state.state=OutputState::Failed;if state.error.is_none(){state.error=Some(format!("FFmpeg output exited: {result:?}"));}}
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
    }))
}
fn fail(status: &Mutex<OutputStatus>, error: String) {
    let mut status = status.lock().unwrap();
    status.state = OutputState::Failed;
    status.error = Some(error);
}
