//! Opt-in mixed RTMP/SRT render and preview probe. All traffic stays on loopback.
use super::*;
use acheflow_media_api::{
    live::{FormatStatus, LiveStatus},
    workspace::OutputProtocol,
};
use axum::{body::to_bytes, extract::Query, http::StatusCode};
use std::collections::HashMap;

const WINDOW: Duration = Duration::from_secs(8);
const SNAPSHOT_PERIOD: Duration = Duration::from_millis(100);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measures headless GPU render and FFmpeg RTMP/SRT using loopback sockets"]
async fn mixed_output_preview_load_at_30_and_60_fps() {
    let caps = crate::ffmpeg::discover().await;
    assert!(
        caps.srt_output,
        "Use FFmpeg with SRT output and MPEG-TS support"
    );
    let encoder =
        std::env::var("ACHEFLOW_PREVIEW_PROBE_ENCODER").unwrap_or_else(|_| "libx264".into());
    caps.resolve_video_encoder("h264", Some(&encoder)).unwrap();
    let root = capture_directory();
    eprintln!("Preview load captures: {}", root.display());
    for fps in [30, 60] {
        let dir = root.join(fps.to_string());
        std::fs::create_dir(&dir).unwrap();
        tokio::time::timeout(
            Duration::from_secs(110),
            probe(&dir, fps, &encoder, caps.clone()),
        )
        .await
        .unwrap_or_else(|_| panic!("{fps} FPS probe timed out"));
    }
}

async fn probe(dir: &Path, fps: u32, encoder: &str, caps: acheflow_media_api::MediaCapabilities) {
    let mut workspace = Workspace::default();
    workspace.add_source();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let srt_address = udp.local_addr().unwrap();
    drop(udp);
    for (orientation, width, height) in [
        (Orientation::Horizontal, 1280, 720),
        (Orientation::Vertical, 720, 1280),
    ] {
        let resolution = Resolution { width, height };
        let scene = workspace.add_scene(orientation);
        workspace.resize_base(orientation, resolution.clone());
        workspace.add_layer(scene, 1);
        let stream = workspace.stream_mut(orientation);
        stream.fps = fps;
        let mut destination = match orientation {
            Orientation::Horizontal => {
                output("RTMP receiver", receiver_address().await, resolution)
            }
            Orientation::Vertical => {
                let mut destination =
                    output("SRT receiver", format!("srt://{srt_address}"), resolution);
                destination.protocol = OutputProtocol::Srt;
                destination.key.clear();
                destination
            }
        };
        destination.encoder = Some(encoder.into());
        stream.outputs.push(destination);
    }
    let (runtime, listener, ingest) =
        pipeline_tests::server(workspace, Arc::new(tokio::sync::Mutex::new(caps))).await;
    let _shutdown = runtime.shutdown.clone().drop_guard();
    let workspace = runtime.saved_state().1.unwrap();
    let rtmp = &workspace.horizontal.outputs[0];
    let rtmp_url = format!("{}/{}", rtmp.url, rtmp.key);
    let rtmp_file = dir.join("horizontal.flv");
    let rtmp_reader = capture(&rtmp_url, &rtmp_file);
    let srt_file = dir.join("vertical.ts");
    let srt_reader = ffmpeg()
        .args(["-copyts", "-i"])
        .arg(format!("srt://{srt_address}?mode=listener&timeout=4000000"))
        .args(["-map", "0:v", "-map", "0:a", "-c", "copy", "-f", "mpegts"])
        .arg(&srt_file)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut publisher = marker_source(
        &format!("{ingest}/{}", workspace.sources[0].key),
        &dir.join("source.flv"),
        false,
    );
    let source = runtime.inputs.lock().unwrap()[&1].clone();
    pipeline_tests::until(|| source.status().width == Some(320) && source.status().audio).await;
    let _ = live::start(
        State(runtime.clone()),
        Json(StartRequest {
            workspace: workspace.clone(),
            formats: Orientation::ALL.to_vec(),
            preview_only: false,
            output: None,
        }),
    )
    .await
    .unwrap();
    for orientation in Orientation::ALL {
        streaming(&runtime, orientation).await;
    }
    let ready = Instant::now();
    let mut measurements = Vec::new();
    measurements.push(measure(&runtime, fps, "outputs", &[]).await);
    measurements.push(measure(&runtime, fps, "snapshots", &["jpeg", "png"]).await);
    measurements.push(measure(&runtime, fps, "encoded", &["encoded"]).await);

    let before_reconnect = Instant::now();
    publisher.kill().await.unwrap();
    pipeline_tests::until(|| !source.status().connected).await;
    let lost = Instant::now();
    let mut publisher = marker_source(
        &format!("{ingest}/{}", workspace.sources[0].key),
        &dir.join("source-rejoined.flv"),
        false,
    );
    pipeline_tests::until(|| {
        source.status().connected && source.status().width == Some(320) && source.status().audio
    })
    .await;
    let rejoined = Instant::now();
    let status = runtime.status().await;
    let before_srt = format_status(&status, Orientation::Vertical).outputs[0].frames;

    let before_restart = Instant::now();
    let restart = live::restart_output(
        State(runtime.clone()),
        Json(RestartOutputRequest {
            workspace_id: workspace.id.clone(),
            orientation: Orientation::Horizontal,
            index: 0,
        }),
    );
    let (restarted, ()) = tokio::join!(restart, finish_capture(rtmp_reader, &rtmp_file));
    let _ = restarted.unwrap();
    let restarted_file = dir.join("horizontal-restarted.flv");
    let restarted_capture = capture(&rtmp_url, &restarted_file);
    streaming(&runtime, Orientation::Horizontal).await;
    let resumed = Instant::now();
    let status = runtime.status().await;
    let other = &format_status(&status, Orientation::Vertical).outputs[0];
    assert_eq!(other.state, OutputState::Streaming);
    assert!(
        other.frames > before_srt && other.reconnects == 0,
        "{other:?}"
    );
    tokio::time::sleep(Duration::from_secs(5)).await;

    let _ = publisher.kill().await;
    runtime.close().await;
    finish_capture(restarted_capture, &restarted_file).await;
    finish_capture(srt_reader, &srt_file).await;
    listener.await.unwrap();

    std::fs::write(
        dir.join("preview-load.json"),
        serde_json::to_vec_pretty(&json!({
            "fps": fps, "encoder": encoder, "phases": measurements,
            "source_loss_ms": (lost - before_reconnect).as_secs_f64() * 1000.,
            "source_rejoin_ms": (rejoined - lost).as_secs_f64() * 1000.,
            "rtmp_restart_ms": (resumed - before_restart).as_secs_f64() * 1000.,
            "srt_frames_during_restart": other.frames - before_srt,
            "seconds_before_restart": (before_restart - ready).as_secs_f64()
        }))
        .unwrap(),
    )
    .unwrap();
}

fn format_status(status: &LiveStatus, orientation: Orientation) -> &FormatStatus {
    status
        .formats
        .iter()
        .find(|format| format.orientation == orientation)
        .unwrap()
}

async fn measure(
    runtime: &live::RuntimeState,
    fps: u32,
    phase: &str,
    modes: &[&str],
) -> serde_json::Value {
    let before = runtime.status().await;
    let start = Instant::now();
    let deadline = start + WINDOW;
    let requests = Orientation::ALL.into_iter().flat_map(|orientation| {
        modes.iter().flat_map(move |mode| {
            // Two concurrent viewers request each format/mode and share the server cache.
            (1..=2).map(move |viewer| snapshots(runtime, orientation, mode, viewer, deadline))
        })
    });
    let (snapshots, ()) = tokio::join!(
        futures_util::future::join_all(requests),
        tokio::time::sleep_until(deadline),
    );
    let seconds = start.elapsed().as_secs_f64();
    let after = runtime.status().await;
    let formats = Orientation::ALL.map(|orientation| {
        let before = format_status(&before, orientation);
        let after = format_status(&after, orientation);
        let render_fps = after.frames.saturating_sub(before.frames) as f64 / seconds;
        let output = &after.outputs[0];
        assert_eq!(output.state, OutputState::Streaming, "{output:?}");
        let repeated = output.repeated_video_frames.saturating_sub(before.outputs[0].repeated_video_frames);
        eprintln!("{fps} {phase} {}: {render_fps:.1} render FPS, {:.2} ms render p95, {repeated} repeated output frames",
            orientation.label(), after.render_load.p95_ms);
        json!({
            "format": orientation.label(), "renderer": after.renderer,
            "output_hardware": output.hardware, "render_fps": render_fps,
            "render_p95_ms_last_2s": after.render_load.p95_ms,
            "render_deadline_miss_pct_last_2s": after.render_load.deadline_miss_percent,
            "repeated_output_frames": repeated,
        })
    });
    json!({"phase":phase, "seconds":seconds, "formats":formats, "viewers":snapshots})
}

async fn snapshots(
    runtime: &live::RuntimeState,
    orientation: Orientation,
    mode: &str,
    viewer: usize,
    deadline: Instant,
) -> serde_json::Value {
    let mut latencies = Vec::new();
    let mut size = 0;
    let mut warmups = 0;
    let mut failed = 0;
    while Instant::now() < deadline {
        let start = Instant::now();
        let query = match mode {
            "png" => HashMap::from([("mode".into(), "lossless".into())]),
            "encoded" => HashMap::from([
                ("mode".into(), "encoded".into()),
                ("output".into(), "0".into()),
            ]),
            _ => HashMap::new(),
        };
        let response = live::preview(
            State(runtime.clone()),
            axum::extract::Path(format!("{}.jpg", orientation.label().to_lowercase())),
            Query(query),
        )
        .await;
        match response.status() {
            StatusCode::OK => {
                let body = to_bytes(response.into_body(), 32 * 1024 * 1024)
                    .await
                    .unwrap();
                size += body.len();
                latencies.push(start.elapsed().as_secs_f64() * 1000.);
            }
            StatusCode::ACCEPTED => warmups += 1,
            _ => failed += 1,
        }
        tokio::time::sleep_until((start + SNAPSHOT_PERIOD).min(deadline)).await;
    }
    latencies.sort_unstable_by(f64::total_cmp);
    let count = latencies.len();
    let p95 = count
        .checked_sub(1)
        .map(|_| latencies[(count * 95).div_ceil(100) - 1]);
    assert!(
        count > 0,
        "{} {mode} received no snapshots ({failed} errors)",
        orientation.label()
    );
    json!({
        "format":orientation.label(),"mode":mode,"viewer":viewer,
        "snapshots":count,"bytes":size,"warming_up":warmups,"errors":failed,"response_p95_ms":p95,
    })
}
