//! Real RTMP -> decode -> scene -> encode -> RTMP integration, loopback only.
use crate::{
    live::{self, Runtime, RuntimeState},
    rtmp,
};
use acheflow_media_api::{
    live::StartRequest,
    workspace::{Layer, Orientation, Output, Resolution, Workspace},
};
use axum::{Json, extract::State};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    process::{Child, Command},
};

// This fixture deliberately edits the current studio repeatedly. Acknowledge
// each durable revision as a real synchronized operator would before Apply.
pub(super) async fn configure_current(
    state: State<RuntimeState>,
    mut request: Json<Workspace>,
) -> live::ApiResult {
    request.0.edit_revision = state.0.saved_state().0;
    crate::live::configure(state, request).await
}

pub(super) async fn server(
    mut workspace: Workspace,
    caps: crate::ffmpeg::CapabilityState,
) -> (RuntimeState, tokio::task::JoinHandle<()>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("rtmp://{address}/live");
    for source in &mut workspace.sources {
        source.url = url.clone();
    }
    let runtime = Runtime::new(caps, address.to_string());
    let _ = configure_current(State(runtime.clone()), Json(workspace))
        .await
        .unwrap();
    let server_runtime = runtime.clone();
    let task = tokio::spawn(async move {
        rtmp::serve(listener, server_runtime, None).await.unwrap();
    });
    (runtime, task, url)
}

fn publisher(url: &str, token: &str, color: &str, tone: u32) -> Child {
    sized_publisher(url, token, color, tone, "160x90")
}

fn sized_publisher(url: &str, token: &str, color: &str, tone: u32, size: &str) -> Child {
    Command::new(crate::ffmpeg::executable())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-re",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("color=c={color}:s={size}:r=10"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency={tone}:sample_rate=48000"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-g",
            "10",
            "-bf",
            "0",
            "-c:a",
            "aac",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-f",
            "flv",
        ])
        .arg(format!("{url}/{token}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

pub(super) async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("media condition timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires FFmpeg with libx264 and local TCP; run explicitly"]
async fn incoming_frames_keep_native_size_across_reconnects() {
    let mut workspace = Workspace::default();
    workspace.add_source();
    workspace.sources[0].resolution = Resolution {
        width: 64,
        height: 64,
    };
    let token = workspace.sources[0].key.clone();
    let caps = Arc::new(tokio::sync::Mutex::new(Default::default()));
    let (runtime, task, url) = server(workspace, caps.clone()).await;
    let input = runtime.inputs.lock().unwrap()[&1].clone();
    for (size, expected) in [("160x90", (160, 90)), ("90x160", (90, 160))] {
        let mut publisher = sized_publisher(&url, &token, "red", 440, size);
        until(|| {
            input.connected.load(Ordering::Acquire)
                && input.frames.load(Ordering::Relaxed) > 2
                && input.frame.read().unwrap().is_some()
        })
        .await;
        let image = input.frame.read().unwrap().as_ref().unwrap().image.clone();
        let dimensions = image.dimensions();
        let status = input.status();
        publisher.kill().await.unwrap();
        let _ = publisher.wait().await;
        until(|| !input.connected.load(Ordering::Acquire)).await;
        assert_eq!(
            dimensions, expected,
            "ingest must not resize or pad the source"
        );
        assert_eq!(
            (status.width, status.height),
            (Some(expected.0), Some(expected.1))
        );
        assert!(
            image
                .pixels()
                .all(|p| p.0[0] > 160 && p.0[1] < 50 && p.0[2] < 50),
            "ingest added borders"
        );
    }
    runtime.close().await;
    task.await.unwrap();
}

fn colors(runtime: &RuntimeState, left: &str, right: &str) -> bool {
    let inputs = runtime.inputs.lock().unwrap();
    let Some(input) = inputs.get(&1) else {
        return false;
    };
    let frame = input.frame.read().unwrap();
    let Some(frame) = frame.as_ref() else {
        return false;
    };
    let test = |rgb: [u8; 3], color: &str| match color {
        "red" => rgb[0] > 160 && rgb[1] < 50 && rgb[2] < 50,
        "blue" => rgb[2] > 160 && rgb[0] < 50 && rgb[1] < 50,
        "black" => rgb.iter().all(|c| *c < 30),
        _ => false,
    };
    test(frame.image.get_pixel(80, 90).0, left) && test(frame.image.get_pixel(240, 90).0, right)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires FFmpeg with libx264 and local TCP; run explicitly"]
async fn real_rtmp_tokens_scene_audio_reposition_reconnect_and_independent_outputs() {
    output_pipeline("libx264").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg(target_os = "macos")]
#[ignore = "requires FFmpeg, VideoToolbox hardware, Metal and local TCP; run explicitly"]
async fn real_rtmp_videotoolbox_output_reports_hardware_and_delivers_media() {
    output_pipeline("h264_videotoolbox").await;
}

async fn restart_failed_output(
    runtime: &RuntimeState,
    workspace: &Workspace,
    caps: crate::ffmpeg::CapabilityState,
    refused_port: u16,
) -> (RuntimeState, tokio::task::JoinHandle<()>) {
    let status = runtime.status().await;
    let before = runtime.formats.lock().await[0].frames.clone();
    assert_eq!(status.formats[0].outputs[1].index, 2);
    let restart = |id: String, index| {
        live::restart_output(
            State(runtime.clone()),
            Json(acheflow_media_api::live::RestartOutputRequest {
                workspace_id: id,
                orientation: Orientation::Horizontal,
                index,
            }),
        )
    };
    assert_eq!(
        restart("wrong-owner".into(), 2).await.unwrap_err().0,
        axum::http::StatusCode::CONFLICT
    );
    assert!(restart(workspace.id.clone(), 1).await.is_err()); // disabled destination
    assert!(restart(workspace.id.clone(), 100).await.is_err());
    let (recovered, recovered_task) = bind_recovery_sink(workspace, caps, refused_port).await;
    let working_before = status.formats[0].outputs[0].frames;
    let restarted = restart(workspace.id.clone(), 2).await.unwrap().0;
    assert_eq!(
        restarted.formats[0].outputs[0].state,
        acheflow_media_api::live::OutputState::Streaming
    );
    until(|| colors(&recovered, "blue", "red")).await;
    let output_status = runtime.formats.lock().await[0].outputs[1].clone();
    until(|| {
        output_status.lock().unwrap().state == acheflow_media_api::live::OutputState::Streaming
    })
    .await;
    let recovered_status = runtime.status().await;
    assert_eq!(
        recovered_status.formats[0].outputs[1].state,
        acheflow_media_api::live::OutputState::Streaming
    );
    assert!(recovered_status.formats[0].outputs[0].frames > working_before);
    assert!(Arc::ptr_eq(
        &before,
        &runtime.formats.lock().await[0].frames
    ));
    (recovered, recovered_task)
}

async fn bind_recovery_sink(
    workspace: &Workspace,
    caps: crate::ffmpeg::CapabilityState,
    port: u16,
) -> (RuntimeState, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let mut config = Workspace::default();
    config.add_source();
    config.sources[0].url = workspace.horizontal.outputs[2].url.clone();
    config.sources[0].key = workspace.horizontal.outputs[2].key.clone();
    let sink = Runtime::new(caps, listener.local_addr().unwrap().to_string());
    let _ = configure_current(State(sink.clone()), Json(config))
        .await
        .unwrap();
    let runtime = sink.clone();
    let task = tokio::spawn(async move { rtmp::serve(listener, runtime, None).await.unwrap() });
    (sink, task)
}

async fn output_pipeline(encoder: &str) {
    let caps = Arc::new(tokio::sync::Mutex::new(crate::ffmpeg::discover().await));
    let hardware = caps
        .lock()
        .await
        .resolve_video_encoder("h264", Some(encoder))
        .unwrap()
        .hardware;
    assert!(
        caps.lock()
            .await
            .video_encoders("h264")
            .any(|e| e.name == "libx264")
    );
    let mut sink_workspace = Workspace::default();
    sink_workspace.add_source();
    sink_workspace.sources[0].resolution = Resolution {
        width: 320,
        height: 180,
    };
    let sink_token = sink_workspace.sources[0].key.clone();
    let (sink, sink_task, sink_url) = server(sink_workspace, caps.clone()).await;
    let mut sink_audio = sink
        .inputs
        .lock()
        .unwrap()
        .get(&1)
        .unwrap()
        .audio
        .subscribe();

    let mut workspace = Workspace::default();
    workspace.horizontal.base = Resolution {
        width: 320,
        height: 180,
    };
    workspace.horizontal.fps = 10;
    workspace.add_source();
    workspace.add_source();
    for source in &mut workspace.sources {
        source.resolution = Resolution {
            // Offline patterns must not resize the live 160x90 inputs.
            width: 64,
            height: 64,
        };
    }
    let scene = workspace.add_scene(Orientation::Horizontal);
    workspace.scenes[0].layers = vec![
        Layer {
            source_id: 1,
            x: 0,
            y: 0,
            width: 160,
            height: 180,
            ..Layer::default()
        },
        Layer {
            source_id: 2,
            x: 160,
            y: 0,
            width: 160,
            height: 180,
            ..Layer::default()
        },
    ];
    workspace.vertical.base = Resolution {
        width: 180,
        height: 320,
    };
    workspace.vertical.fps = 15;
    workspace.add_scene(Orientation::Vertical);
    workspace.scenes[1].layers = vec![
        Layer {
            source_id: 1,
            x: 0,
            y: 0,
            width: 180,
            height: 160,
            ..Layer::default()
        },
        Layer {
            source_id: 2,
            x: 0,
            y: 160,
            width: 180,
            height: 160,
            ..Layer::default()
        },
    ];
    workspace.horizontal.outputs.push(Output {
        name: "Working sink".into(),
        url: sink_url.clone(),
        key: sink_token,
        resolution: workspace.horizontal.base.clone(),
        quality: "Custom".into(),
        codec: "h264".into(),
        encoder: Some(encoder.into()),
        bitrate_kbps: 500,
        bitrate_mode: acheflow_media_api::workspace::BitrateMode::Cbr,
        keyframe_interval_secs: 2,
        enabled: true,
        audio: Default::default(),
    });
    let refused = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let refused_port = refused.local_addr().unwrap().port();
    drop(refused);
    let mut failing = workspace.horizontal.outputs[0].clone();
    failing.name = "Refused sink".into();
    failing.bitrate_mode = acheflow_media_api::workspace::BitrateMode::Vbr;
    failing.url = format!("rtmp://127.0.0.1:{refused_port}/live");
    let mut disabled = failing.clone();
    disabled.enabled = false;
    workspace.horizontal.outputs.push(disabled);
    workspace.horizontal.outputs.push(failing);
    let (runtime, task, url) = server(workspace, caps.clone()).await;
    let _shutdown = runtime.shutdown.clone().drop_guard();
    let mut workspace = runtime.workspace.read().unwrap().clone().unwrap();
    assert!(
        runtime
            .authorize("live", "not-a-valid-token", &runtime.shutdown)
            .is_err()
    );
    let mut wrong = publisher(&url, "not-a-valid-token", "red", 440);
    let exit = tokio::time::timeout(Duration::from_secs(8), wrong.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!exit.success());
    assert!(
        runtime
            .inputs
            .lock()
            .unwrap()
            .values()
            .all(|i| !i.connected.load(Ordering::Relaxed))
    );
    let mut red = publisher(&url, &workspace.sources[0].key, "red", 440);
    let mut blue = publisher(&url, &workspace.sources[1].key, "blue", 880);
    until(|| {
        runtime
            .inputs
            .lock()
            .unwrap()
            .values()
            .all(|i| i.frames.load(Ordering::Relaxed) > 2 && i.has_audio.load(Ordering::Relaxed))
    })
    .await;
    assert!(
        runtime
            .authorize("live", &workspace.sources[0].key, &runtime.shutdown)
            .is_err()
    );
    workspace.edit_revision = runtime.saved_state().0;
    let started = live::start(
        State(runtime.clone()),
        Json(StartRequest {
            workspace: workspace.clone(),
            formats: vec![Orientation::Horizontal, Orientation::Vertical],
            preview_only: false,
            output: Some(acheflow_media_api::live::OutputTarget {
                orientation: Orientation::Horizontal,
                index: 0,
            }),
        }),
    )
    .await
    .unwrap()
    .0;
    use acheflow_media_api::live::{ManageOutputsRequest, OutputAction, OutputState, OutputTarget};
    assert_eq!(started.formats[0].outputs.len(), 2);
    assert_eq!(started.formats[0].outputs[1].index, 2);
    assert_eq!(started.formats[0].outputs[1].state, OutputState::Stopped);
    if tokio::time::timeout(Duration::from_secs(20), async {
        while !colors(&sink, "red", "blue") {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_err()
    {
        panic!(
            "Initial output did not deliver the expected scene. Publisher: {:?}. Receiver: {:?}",
            runtime.status().await.formats[0].outputs[0],
            sink.status().await.sources[0],
        );
    }
    let healthy = runtime.formats.lock().await[0].outputs[0].clone();
    until(|| healthy.lock().unwrap().state == OutputState::Streaming).await;
    let sent_before = healthy.lock().unwrap().sent_bytes;
    let decoded = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let response = live::preview(
                State(runtime.clone()),
                axum::extract::Path("horizontal.jpg".into()),
                axum::extract::Query(std::collections::HashMap::from([
                    ("workspace".into(), workspace.id.clone()),
                    ("output".into(), "1".into()),
                    ("mode".into(), "encoded".into()),
                ])),
            )
            .await;
            if response.status() != axum::http::StatusCode::ACCEPTED {
                break response;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(decoded.status(), axum::http::StatusCode::OK);
    assert_eq!(decoded.headers()["content-type"], "image/png");
    assert_eq!(decoded.headers()["x-frame-width"], "320");
    until(|| {
        let output = healthy.lock().unwrap();
        output.state == OutputState::Streaming && output.sent_bytes > sent_before
    })
    .await;
    for (workspace_id, output) in [
        ("other-workspace".to_owned(), None),
        (
            workspace.id.clone(),
            Some(OutputTarget {
                orientation: Orientation::Horizontal,
                index: 1,
            }),
        ),
    ] {
        assert!(
            live::manage_outputs(
                State(runtime.clone()),
                Json(ManageOutputsRequest {
                    workspace_id,
                    action: OutputAction::Start,
                    output
                }),
            )
            .await
            .is_err()
        );
    }
    let all = live::manage_outputs(
        State(runtime.clone()),
        Json(ManageOutputsRequest {
            workspace_id: workspace.id.clone(),
            action: OutputAction::Start,
            output: None,
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(all.formats[0].outputs[0].sent_bytes >= sent_before);
    assert!(matches!(
        all.formats[0].outputs[1].state,
        OutputState::Starting | OutputState::Streaming
    ));
    spatial_chat_roundtrip(&runtime, &sink, scene).await;
    let portrait = runtime.formats.lock().await[1].frame.clone();
    until(|| {
        let image = portrait.borrow();
        image.width() == 180
            && image.height() == 320
            && image.get_pixel(90, 80).0[0] > 160
            && image.get_pixel(90, 240).0[2] > 160
    })
    .await;
    assert!(
        live::stop(
            State(runtime.clone()),
            Json(acheflow_media_api::live::ControlRequest {
                workspace_id: "stale-client".into()
            })
        )
        .await
        .is_err()
    );

    let heard = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match sink_audio.recv().await {
                Ok(samples)
                    if samples.iter().map(|s| i32::from(*s).abs()).sum::<i32>()
                        / samples.len() as i32
                        > 100 =>
                {
                    break;
                }
                true_or_error => {
                    let _ = true_or_error;
                }
            }
        }
    })
    .await;
    assert!(
        heard.is_ok(),
        "source audio did not survive RTMP encode/decode"
    );
    let before = runtime.formats.lock().await[0].frames.clone();
    workspace
        .scenes
        .iter_mut()
        .find(|s| s.id == scene)
        .unwrap()
        .layers[0]
        .x = 160;
    workspace.scenes[0].layers[1].x = 0;
    let _ = configure_current(State(runtime.clone()), Json(workspace.clone()))
        .await
        .unwrap();
    until(|| colors(&sink, "blue", "red")).await;
    assert!(Arc::ptr_eq(
        &before,
        &runtime.formats.lock().await[0].frames
    ));
    // Apply crop, alpha blending and z-order edits without restarting encoders.
    let mut edited = workspace.clone();
    for layer in &mut edited.scenes[0].layers {
        layer.x = 0;
        layer.width = 320;
    }
    edited.scenes[0].layers[1].crop.left = 80;
    edited.scenes[0].layers[1]
        .effects
        .push(acheflow_media_api::workspace::LayerEffect {
            chroma_key: None,
            id: acheflow_media_api::workspace::new_entity_id(),
            shader: String::new(),
            kind: acheflow_media_api::workspace::EffectKind::Opacity,
            amount: 50,
            enabled: true,
        });
    let _ = configure_current(State(runtime.clone()), Json(edited.clone()))
        .await
        .unwrap();
    until(|| {
        let inputs = sink.inputs.lock().unwrap();
        let input = &inputs[&1];
        let frame = input.frame.read().unwrap();
        frame.as_ref().is_some_and(|f| {
            let p = f.image.get_pixel(80, 90).0;
            p[0] > 70 && p[2] > 70 && p[1] < 40
        })
    })
    .await;
    edited.move_layer(scene, 0, 1);
    let _ = configure_current(State(runtime.clone()), Json(edited))
        .await
        .unwrap();
    until(|| colors(&sink, "red", "red")).await;
    let _ = configure_current(State(runtime.clone()), Json(workspace.clone()))
        .await
        .unwrap();
    until(|| colors(&sink, "blue", "red")).await;
    assert!(Arc::ptr_eq(
        &before,
        &runtime.formats.lock().await[0].frames
    ));
    let retrying_output = runtime.formats.lock().await[0].outputs[1].clone();
    until(|| {
        retrying_output.lock().unwrap().state == acheflow_media_api::live::OutputState::Failed
    })
    .await;
    let status = runtime.status().await;
    assert_eq!(
        status.formats[0].outputs[0].encoder.as_deref(),
        Some(encoder)
    );
    assert_eq!(status.formats[0].outputs[0].hardware, hardware);
    assert_eq!(
        status.formats[0].outputs[0].state,
        acheflow_media_api::live::OutputState::Streaming
    );
    assert_eq!(
        status.formats[0].outputs[1].state,
        acheflow_media_api::live::OutputState::Failed
    );
    assert!(
        status.formats[0].outputs[1]
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("Reconnect attempts exhausted:"))
    );
    assert!(
        status
            .sources
            .iter()
            .all(|s| s.received_bytes > 0 && s.bitrate_kbps > 0.)
    );
    assert!(status.formats[0].outputs[0].sent_bytes > 0);
    assert!(status.formats[0].outputs[0].bitrate_kbps > 0.);
    let (recovered, recovered_task) =
        restart_failed_output(&runtime, &workspace, caps.clone(), refused_port).await;
    recovered.close().await;
    recovered_task.await.unwrap();
    let healthy_output = runtime.formats.lock().await[0].outputs[0].clone();
    let healthy_frames = healthy_output.lock().unwrap().frames;
    until(|| {
        let output = retrying_output.lock().unwrap();
        output.reconnect_attempt.is_some()
    })
    .await;
    let (recovered, recovered_task) =
        bind_recovery_sink(&workspace, caps.clone(), refused_port).await;
    until(|| colors(&recovered, "blue", "red")).await;
    until(|| {
        let output = healthy_output.lock().unwrap();
        output.state == acheflow_media_api::live::OutputState::Streaming
            && output.frames > healthy_frames
    })
    .await;
    recovered.close().await;
    recovered_task.await.unwrap();
    until(|| retrying_output.lock().unwrap().reconnect_attempt.is_some()).await;
    let (recovered, recovered_task) =
        bind_recovery_sink(&workspace, caps.clone(), refused_port).await;
    let restarted = live::manage_outputs(
        State(runtime.clone()),
        Json(ManageOutputsRequest {
            workspace_id: workspace.id.clone(),
            action: OutputAction::Restart,
            output: Some(OutputTarget {
                orientation: Orientation::Horizontal,
                index: 2,
            }),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(restarted.formats[0].outputs[1].reconnect_attempt, None);
    until(|| colors(&recovered, "blue", "red")).await;
    let mut changed_profile = workspace.clone();
    changed_profile.horizontal.fps = 30;
    assert!(
        configure_current(State(runtime.clone()), Json(changed_profile))
            .await
            .is_err()
    );
    blue.kill().await.unwrap();
    let _ = blue.wait().await;
    until(|| colors(&sink, "black", "red")).await;
    blue = publisher(&url, &workspace.sources[1].key, "blue", 880);
    until(|| colors(&sink, "blue", "red")).await;
    let old_token = workspace.sources[1].key.clone();
    workspace.sources[1].key = acheflow_media_api::workspace::new_token();
    let _ = configure_current(State(runtime.clone()), Json(workspace.clone()))
        .await
        .unwrap();
    until(|| colors(&sink, "black", "red")).await;
    assert!(
        runtime
            .authorize("live", &old_token, &runtime.shutdown)
            .is_err()
    );
    let _ = blue.kill().await;
    let _ = blue.wait().await;
    blue = publisher(&url, &workspace.sources[1].key, "blue", 880);
    until(|| colors(&sink, "blue", "red")).await;
    let wrong_scope = live::preview(
        State(runtime.clone()),
        axum::extract::Path("horizontal.jpg".into()),
        axum::extract::Query(std::collections::HashMap::from([(
            "workspace".into(),
            "wrong-workspace".into(),
        )])),
    )
    .await;
    assert_eq!(wrong_scope.status(), axum::http::StatusCode::NOT_FOUND);
    let response = live::preview(
        State(runtime.clone()),
        axum::extract::Path("horizontal.jpg".into()),
        axum::extract::Query(std::collections::HashMap::new()),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 320);
    let restarted_all = live::manage_outputs(
        State(runtime.clone()),
        Json(ManageOutputsRequest {
            workspace_id: workspace.id.clone(),
            action: OutputAction::Restart,
            output: None,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(restarted_all.formats[0].outputs.len(), 2);
    assert!(
        restarted_all.formats[0]
            .outputs
            .iter()
            .all(|output| matches!(output.state, OutputState::Starting | OutputState::Streaming))
    );
    until(|| {
        [&healthy_output, &retrying_output]
            .into_iter()
            .all(|status| {
                let status = status.lock().unwrap();
                status.state == OutputState::Streaming && status.sent_bytes > 0
            })
    })
    .await;
    until(|| colors(&sink, "blue", "red") && colors(&recovered, "blue", "red")).await;
    recovered.close().await;
    recovered_task.await.unwrap();
    until(|| {
        let output = retrying_output.lock().unwrap();
        output.reconnect_attempt.is_some()
    })
    .await;
    let _ = live::stop(
        State(runtime.clone()),
        Json(acheflow_media_api::live::ControlRequest {
            workspace_id: workspace.id.clone(),
        }),
    )
    .await;
    assert!(runtime.status().await.formats.is_empty());
    assert_eq!(
        retrying_output.lock().unwrap().state,
        acheflow_media_api::live::OutputState::Stopped
    );
    let (recovered, recovered_task) = bind_recovery_sink(&workspace, caps, refused_port).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                if recovered.inputs.lock().unwrap()[&1]
                    .connected
                    .load(Ordering::Relaxed)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .is_err(),
        "the stopped destination reconnected"
    );
    let _ = red.kill().await;
    let _ = red.wait().await;
    let _ = blue.kill().await;
    let _ = blue.wait().await;
    runtime.close().await;
    sink.close().await;
    recovered.close().await;
    recovered_task.await.unwrap();
    task.await.unwrap();
    sink_task.await.unwrap();
}

async fn spatial_chat_roundtrip(runtime: &RuntimeState, sink: &RuntimeState, scene_id: u32) {
    use acheflow_extension_api::interaction::CommandRule;
    use acheflow_media_api::{
        control::{ControlAction, ControlRequest},
        spatial::*,
    };
    use serde_json::json;
    let mut host = crate::control::ControlHost::new(runtime.clone());
    Arc::get_mut(&mut host).unwrap().installed =
        std::sync::RwLock::new(crate::extension_registry::discover_at(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions"),
        ));
    let workspace = runtime.workspace.read().unwrap().clone().unwrap();
    let mut spatial = SpatialScene::from_canvas(&workspace.horizontal.base);
    spatial.objects.push(SceneObject {
        id: 2,
        name: "Interactive cube".into(),
        visible: true,
        transform: Transform {
            position: [0., 0., 100.],
            scale: [64.; 3],
            ..Default::default()
        },
        kind: ObjectKind::Cube {
            color: [0., 1., 0., 1.],
        },
    });
    let request = |action, suffix| ControlRequest {
        workspace_id: workspace.id.clone(),
        request_id: format!("{}:{suffix}", crate::control::now_ms()),
        expected_revision: host.snapshot(0, 1).revision,
        action,
    };
    host.execute(request(
        ControlAction::SetScene { scene_id, spatial },
        "scene-3d",
    ))
    .await
    .unwrap();
    host.execute(request(
        ControlAction::SetCommand {
            command: CommandRule {
                name: "turn".into(),
                channel: "twitch:fixture".into(),
                roles: vec!["moderator".into()],
                cooldown_ms: 5000,
                scene_id,
                object_id: 2,
                rotation: [0., 45., 0.],
                enabled: true,
            },
        },
        "chat-command",
    ))
    .await
    .unwrap();
    let manifest: acheflow_control::extensions::GuestManifest =
        serde_json::from_str(include_str!("../../../extensions/twitch/manifest.json")).unwrap();
    let manifest = manifest
        .select_part(acheflow_extension_api::parts::ExecutionHost::Server)
        .unwrap()
        .unwrap();
    let mut guest = acheflow_control::extensions::Guest::load_part(
        include_bytes!("../../../extensions/twitch/twitch.wasm"),
        &manifest,
    )
    .unwrap();
    let effects=guest.call(json!({"type":"websocket","now_ms":crate::control::now_ms(),"payload":{"metadata":{"message_type":"notification","subscription_type":"channel.chat.message"},"payload":{"event":{"broadcaster_user_id":"fixture","message_id":"turn-once","chatter_user_id":"moderator","chatter_user_name":"Moderator","message":{"text":"!turn"},"badges":[{"set_id":"moderator"}]}}}})).unwrap();
    for effect in effects {
        // The real provider event enters the normalized provider path. Device
        // authentication and ownership are covered by bridge/collector tests.
        host.publish_inner(&workspace.id, "twitch", "", effect, 0, true)
            .unwrap();
    }
    let actual = host.snapshot(0, 1);
    assert_eq!(
        actual
            .scenes
            .iter()
            .find(|s| s.id == scene_id)
            .unwrap()
            .spatial
            .as_ref()
            .unwrap()
            .objects[1]
            .transform
            .rotation,
        [0., 45., 0.]
    );
    until(|| {
        sink.inputs.lock().unwrap()[&1]
            .frame
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|f| {
                let pixel = f.image.get_pixel(160, 90).0;
                pixel[1] > 160 && pixel[0] < 50 && pixel[2] < 50
            })
    })
    .await;
    let mut restored = runtime.workspace.read().unwrap().clone().unwrap();
    restored
        .scenes
        .iter_mut()
        .find(|s| s.id == scene_id)
        .unwrap()
        .spatial = None;
    let _ = configure_current(State(runtime.clone()), Json(restored))
        .await
        .unwrap();
    until(|| colors(sink, "red", "blue")).await;
}
