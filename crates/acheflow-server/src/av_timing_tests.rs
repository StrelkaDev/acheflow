//! Opt-in, loopback-only marker capture. The analysis is separate so a capture
//! can be measured again without repeating the broadcast.
use crate::{live, pipeline_tests};
use acheflow_media_api::{
    live::{OutputState, RestartOutputRequest, StartRequest},
    workspace::{BitrateMode, Orientation, Output, Resolution, Workspace, new_token},
};
use axum::{Json, extract::State};
use serde_json::json;
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    process::{Child, Command},
    time::Instant,
};

const FPS: u32 = 30;
const PTS_JUMP_AT: u64 = 142;
const PTS_JUMP_SECONDS: f64 = 1.25;

type WallTime = std::time::Instant;
type TraceKey = (&'static str, &'static str);

struct MarkerTrace {
    start: WallTime,
    file: std::io::BufWriter<std::fs::File>,
    active: std::collections::HashMap<TraceKey, bool>,
}

static MARKERS: std::sync::OnceLock<std::sync::Mutex<Option<MarkerTrace>>> =
    std::sync::OnceLock::new();

fn trace_event(
    stage: &'static str,
    side: &'static str,
    frame_at: WallTime,
    pts: Option<i64>,
    lit: bool,
) {
    use std::io::Write;
    let Some(trace) = MARKERS.get() else {
        return;
    };
    let mut trace = trace.lock().unwrap();
    let Some(trace) = trace.as_mut() else {
        return;
    };
    let old = trace.active.insert((stage, side), lit).unwrap_or(false);
    if (lit && !old) || stage == "epoch" {
        let now = trace.start.elapsed().as_secs_f64();
        let image = frame_at
            .saturating_duration_since(trace.start)
            .as_secs_f64();
        let pts = pts.map(|pts| pts.to_string()).unwrap_or_default();
        writeln!(trace.file, "{stage},{side},{now:.6},{image:.6},{pts}").unwrap();
    }
}

pub(crate) fn trace_epoch(orientation: Orientation, at: WallTime) {
    if let Some(trace) = MARKERS.get()
        && let Some(trace) = trace.lock().unwrap().as_mut()
    {
        trace.active.remove(&("encoder", orientation.label()));
    }
    trace_event("epoch", orientation.label(), at, Some(0), true);
}

pub(crate) fn trace_image(
    stage: &'static str,
    orientation: Orientation,
    at: WallTime,
    pts: Option<i64>,
    image: &image::RgbImage,
) {
    if MARKERS.get().is_some() {
        let side = if stage == "decode" {
            "Source"
        } else {
            orientation.label()
        };
        trace_event(
            stage,
            side,
            at,
            pts,
            image.get_pixel(image.width() / 2, image.height() / 2).0[0] > 180,
        );
    }
}

pub(crate) fn trace_pcm(
    stage: &'static str,
    side: &'static str,
    at: WallTime,
    pts: Option<i64>,
    samples: impl Iterator<Item = i16>,
) {
    if MARKERS.get().is_some() {
        let offset = samples
            .step_by(2)
            .position(|sample| sample.unsigned_abs() > 1200);
        trace_event(
            stage,
            side,
            at,
            pts.map(|pts| pts + offset.unwrap_or(0) as i64),
            offset.is_some(),
        );
    }
}

fn ffmpeg() -> Command {
    let mut command = Command::new(crate::ffmpeg::executable());
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .kill_on_drop(true);
    command
}

fn marker_source(url: &str, file: &Path, jump: bool) -> Child {
    let video =
        "color=black:s=320x180:r=30,drawbox=c=white:t=fill:enable='between(mod(t,2),0.5,0.7)'";
    let audio = r"aevalsrc=0.35*sin(2*PI*880*t)*between(mod(t\,2)\,0.5\,0.7):s=48000";
    let mut command = ffmpeg();
    command.args([
        "-re", "-f", "lavfi", "-i", video, "-f", "lavfi", "-i", audio,
    ]);
    if jump {
        let pts = format!("PTS+if(gte(T,{PTS_JUMP_AT}),{PTS_JUMP_SECONDS}/TB,0)");
        // Escape filter argument separators, rather than relying on shell quoting.
        let pts = pts.replace(',', "\\,");
        command.args([
            "-vf",
            &format!("setpts={pts}"),
            "-af",
            &format!("asetpts={pts}"),
        ]);
    }
    command
        .args([
            "-fps_mode",
            "passthrough",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-bf",
            "0",
            "-g",
            "30",
            "-c:a",
            "aac",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-f",
            "tee",
        ])
        .arg(format!(
            "[f=flv:flvflags=no_duration_filesize]{url}|[f=flv:flush_packets=1]{}",
            file.display()
        ))
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

async fn receiver_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!("rtmp://{}/live", listener.local_addr().unwrap())
}

fn capture(url: &str, file: &Path) -> Child {
    ffmpeg()
        .args(["-listen", "1", "-copyts", "-i"])
        .arg(url)
        .args(["-map", "0:v", "-map", "0:a", "-c", "copy", "-f", "flv"])
        .arg(file)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

async fn finish_capture(child: Child, file: &Path) {
    let result = tokio::time::timeout(Duration::from_secs(12), child.wait_with_output())
        .await
        .expect("FFmpeg capture did not close")
        .unwrap();
    assert!(
        std::fs::metadata(file).is_ok_and(|metadata| metadata.len() > 1024),
        "empty capture: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn output(name: &str, url: String, resolution: Resolution) -> Output {
    Output {
        name: name.into(),
        url,
        key: new_token(),
        resolution,
        codec: "h264".into(),
        quality: "Custom".into(),
        encoder: Some("libx264".into()),
        bitrate_kbps: 1600,
        bitrate_mode: BitrateMode::Vbr,
        keyframe_interval_secs: 1,
        enabled: true,
        audio: Default::default(),
    }
}

async fn streaming(runtime: &live::RuntimeState, orientation: Orientation) {
    let status = runtime
        .formats
        .lock()
        .await
        .iter()
        .find(|format| format.orientation == orientation)
        .unwrap()
        .outputs[0]
        .clone();
    pipeline_tests::until(|| {
        let status = status.lock().unwrap();
        assert_ne!(status.state, OutputState::Failed, "{:?}", status.error);
        status.state == OutputState::Streaming && status.frames > 0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "runs a sustained loopback RTMP broadcast with FFmpeg and headless GPU rendering"]
async fn sustained_rtmp_av_markers() {
    let dir = capture_directory();
    eprintln!("A/V timing captures: {}", dir.display());
    use std::io::Write;
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(dir.join("marker-trace.csv")).unwrap());
    writeln!(file, "stage,format,observed_s,frame_or_block_s,pts_48khz").unwrap();
    *MARKERS.get_or_init(Default::default).lock().unwrap() = Some(MarkerTrace {
        start: WallTime::now(),
        file,
        active: Default::default(),
    });
    sustained_capture(&dir).await;
    MARKERS
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .file
        .flush()
        .unwrap();
}

fn capture_directory() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("acheflow-av-{}", new_token()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&dir).unwrap();
    dir
}

async fn sustained_capture(dir: &Path) {
    let vertical_fps = match std::env::var("ACHEFLOW_AV_VERTICAL_FPS") {
        Ok(value) => value.parse::<u32>().expect("invalid vertical probe FPS"),
        Err(std::env::VarError::NotPresent) => FPS,
        Err(error) => panic!("cannot read vertical probe FPS: {error}"),
    };
    assert!((1..=120).contains(&vertical_fps));
    let mut workspace = Workspace::default();
    workspace.add_source();
    for (orientation, size) in [
        (
            Orientation::Horizontal,
            Resolution {
                width: 320,
                height: 180,
            },
        ),
        (
            Orientation::Vertical,
            Resolution {
                width: 1080,
                height: 1920,
            },
        ),
    ] {
        let scene = workspace.add_scene(orientation);
        workspace.add_layer(scene, 1);
        let layer = &mut workspace.scenes.last_mut().unwrap().layers[0];
        layer.width = size.width;
        layer.height = size.height;
        let stream = match orientation {
            Orientation::Horizontal => &mut workspace.horizontal,
            Orientation::Vertical => &mut workspace.vertical,
        };
        stream.base = size.clone();
        stream.fps = match orientation {
            Orientation::Horizontal => FPS,
            Orientation::Vertical => vertical_fps,
        };
        stream
            .outputs
            .push(output(orientation.label(), receiver_address().await, size));
    }
    let caps = crate::ffmpeg::discover().await;
    let (runtime, server, ingest) =
        pipeline_tests::server(workspace, Arc::new(tokio::sync::Mutex::new(caps.clone()))).await;
    let _shutdown = runtime.shutdown.clone().drop_guard();
    let mut workspace = runtime.saved_state().1.unwrap();
    workspace.edit_revision = runtime.saved_state().0;
    let ingest = format!("{ingest}/{}", workspace.sources[0].key);
    let horizontal = &workspace.horizontal.outputs[0];
    let horizontal = format!("{}/{}", horizontal.url, horizontal.key);
    let vertical = &workspace.vertical.outputs[0];
    let vertical = format!("{}/{}", vertical.url, vertical.key);

    let primary_file = dir.join("primary.flv");
    let primary = capture(&horizontal, &primary_file);
    let started = Instant::now();
    let _ = live::start(
        State(runtime.clone()),
        Json(StartRequest {
            workspace: workspace.clone(),
            formats: vec![Orientation::Horizontal],
            preview_only: false,
            output: None,
        }),
    )
    .await
    .unwrap();
    streaming(&runtime, Orientation::Horizontal).await;
    let source_start = started.elapsed().as_secs_f64();
    let mut publisher = marker_source(&ingest, &dir.join("source.flv"), true);
    let baseline_end = Instant::now() + Duration::from_secs(120);
    tokio::time::sleep_until(baseline_end).await;

    let loaded_start = started.elapsed().as_secs_f64();
    let load_file = dir.join("loaded.flv");
    let load = capture(&vertical, &load_file);
    let format = crate::compositor::start(runtime.clone(), Orientation::Vertical, &caps, false)
        .await
        .unwrap();
    runtime.formats.lock().await.push(format);
    streaming(&runtime, Orientation::Vertical).await;
    tokio::time::sleep_until(baseline_end + Duration::from_secs(41)).await;

    let source_lost = started.elapsed().as_secs_f64();
    publisher.kill().await.unwrap();
    let input = runtime.inputs.lock().unwrap()[&1].clone();
    pipeline_tests::until(|| !input.connected.load(std::sync::atomic::Ordering::Acquire)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let source_rejoin = started.elapsed().as_secs_f64();
    let rejoin_file = dir.join("source-rejoin.flv");
    let mut publisher = marker_source(&ingest, &rejoin_file, false);
    pipeline_tests::until(|| {
        input.frames.load(std::sync::atomic::Ordering::Acquire) > 4
            && input.has_audio.load(std::sync::atomic::Ordering::Acquire)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(18)).await;

    let output_restart = started.elapsed().as_secs_f64();
    let restart = live::restart_output(
        State(runtime.clone()),
        Json(RestartOutputRequest {
            workspace_id: workspace.id,
            orientation: Orientation::Vertical,
            index: 0,
        }),
    );
    let (result, ()) = tokio::join!(restart, finish_capture(load, &load_file));
    let _ = result.unwrap();
    let restarted_file = dir.join("restarted.flv");
    let restarted = capture(&vertical, &restarted_file);
    streaming(&runtime, Orientation::Vertical).await;
    let restart_streaming = started.elapsed().as_secs_f64();
    tokio::time::sleep(Duration::from_secs(22)).await;
    let end = started.elapsed().as_secs_f64();

    let _ = publisher.kill().await;
    runtime.close().await;
    assert!(std::fs::metadata(&rejoin_file).is_ok_and(|file| file.len() > 1024));
    finish_capture(primary, &primary_file).await;
    finish_capture(restarted, &restarted_file).await;
    server.await.unwrap();
    let phases = json!({
        "fps": FPS, "vertical_fps": vertical_fps,
        "source_start": source_start, "loaded_start": loaded_start,
        "source_jump": source_start + PTS_JUMP_AT as f64, "source_lost": source_lost,
        "source_rejoin": source_rejoin, "output_restart": output_restart,
        "restart_streaming": restart_streaming, "end": end,
    });
    std::fs::write(dir.join("phases.json"), phases.to_string()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "runs manual A/V trim through real loopback RTMP and GPU rendering"]
async fn live_manual_av_trim() {
    use acheflow_media_api::audio::{AudioStrip, SyncMode};

    let dir = capture_directory();
    eprintln!("Manual A/V captures: {}", dir.display());
    let mut workspace = Workspace::default();
    workspace.add_source();
    let scene = workspace.add_scene(Orientation::Horizontal);
    workspace.add_layer(scene, 1);
    let size = Resolution {
        width: 320,
        height: 180,
    };
    workspace.scenes[0].layers[0].width = size.width;
    workspace.scenes[0].layers[0].height = size.height;
    workspace.scenes[0].audio.strips.push(AudioStrip {
        source_id: 1,
        sends: [(1, 100)].into(),
        sync_mode: SyncMode::Manual,
        sync_offset_ms: 200,
        ..Default::default()
    });
    workspace.horizontal.base = size.clone();
    workspace.horizontal.fps = FPS;
    workspace
        .horizontal
        .outputs
        .push(output("Manual trim", receiver_address().await, size));
    let caps = Arc::new(tokio::sync::Mutex::new(crate::ffmpeg::discover().await));
    let (runtime, server, ingest) = pipeline_tests::server(workspace, caps).await;
    let _shutdown = runtime.shutdown.clone().drop_guard();
    let workspace = runtime.saved_state().1.unwrap();
    let output = &workspace.horizontal.outputs[0];
    let output_url = format!("{}/{}", output.url, output.key);
    let ingest = format!("{ingest}/{}", workspace.sources[0].key);
    let file = dir.join("manual.flv");
    let receiver = capture(&output_url, &file);
    let _ = live::start(
        State(runtime.clone()),
        Json(StartRequest {
            workspace,
            formats: vec![Orientation::Horizontal],
            preview_only: false,
            output: None,
        }),
    )
    .await
    .unwrap();
    streaming(&runtime, Orientation::Horizontal).await;
    let mut publisher = marker_source(&ingest, &dir.join("source.flv"), false);
    tokio::time::sleep(Duration::from_secs(12)).await;
    let status = runtime.status().await;
    let before = &status.formats[0].audio_sync[0];
    assert_eq!((before.audio_delay_ms, before.video_delay_ms), (200, 0));
    assert!(!status.server_load.is_empty());
    assert!(status.formats[0].render_load.observed_fps > 0.);
    let mut edited = runtime.saved_state().1.unwrap();
    edited.scenes[0].audio.strips[0].sync_offset_ms = -200;
    let _ = pipeline_tests::configure_current(State(runtime.clone()), Json(edited))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(12)).await;
    let status = runtime.status().await;
    let after = &status.formats[0].audio_sync[0];
    assert_eq!((after.audio_delay_ms, after.video_delay_ms), (0, 200));
    assert!(!after.video_delay_limited);
    let _ = publisher.kill().await;
    runtime.close().await;
    finish_capture(receiver, &file).await;
    server.await.unwrap();
}
