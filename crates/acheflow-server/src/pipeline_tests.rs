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
    let _ = live::configure(State(runtime.clone()), Json(workspace))
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
    let listener = TcpListener::bind(("127.0.0.1", refused_port))
        .await
        .unwrap();
    let mut recovered_workspace = Workspace::default();
    recovered_workspace.add_source();
    recovered_workspace.sources[0].url = workspace.horizontal.outputs[2].url.clone();
    recovered_workspace.sources[0].key = workspace.horizontal.outputs[2].key.clone();
    let recovered = Runtime::new(caps.clone(), listener.local_addr().unwrap().to_string());
    let _ = live::configure(State(recovered.clone()), Json(recovered_workspace))
        .await
        .unwrap();
    let recovered_runtime = recovered.clone();
    let recovered_task = tokio::spawn(async move {
        rtmp::serve(listener, recovered_runtime, None)
            .await
            .unwrap();
    });
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
    let _ = live::start(
        State(runtime.clone()),
        Json(StartRequest {
            workspace: workspace.clone(),
            formats: vec![Orientation::Horizontal, Orientation::Vertical],
        }),
    )
    .await
    .unwrap();
    until(|| colors(&sink, "red", "blue")).await;
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
    let _ = live::configure(State(runtime.clone()), Json(workspace.clone()))
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
            id: acheflow_media_api::workspace::new_entity_id(),
            shader: String::new(),
            kind: acheflow_media_api::workspace::EffectKind::Opacity,
            amount: 50,
            enabled: true,
        });
    let _ = live::configure(State(runtime.clone()), Json(edited.clone()))
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
    let _ = live::configure(State(runtime.clone()), Json(edited))
        .await
        .unwrap();
    until(|| colors(&sink, "red", "red")).await;
    let _ = live::configure(State(runtime.clone()), Json(workspace.clone()))
        .await
        .unwrap();
    until(|| colors(&sink, "blue", "red")).await;
    assert!(Arc::ptr_eq(
        &before,
        &runtime.formats.lock().await[0].frames
    ));
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
        status
            .sources
            .iter()
            .all(|s| s.received_bytes > 0 && s.bitrate_kbps > 0.)
    );
    assert!(status.formats[0].outputs[0].sent_bytes > 0);
    assert!(status.formats[0].outputs[0].bitrate_kbps > 0.);
    let (recovered, recovered_task) =
        restart_failed_output(&runtime, &workspace, caps.clone(), refused_port).await;
    let mut changed_profile = workspace.clone();
    changed_profile.horizontal.fps = 30;
    assert!(
        live::configure(State(runtime.clone()), Json(changed_profile))
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
    let _ = live::configure(State(runtime.clone()), Json(workspace.clone()))
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
    let _ = live::stop(
        State(runtime.clone()),
        Json(acheflow_media_api::live::ControlRequest {
            workspace_id: workspace.id.clone(),
        }),
    )
    .await;
    assert!(runtime.status().await.formats.is_empty());
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
    let host = crate::control::ControlHost::new(runtime.clone());
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
    let mut guest = acheflow_control::extensions::Guest::load(
        include_bytes!("../../../extensions/twitch/twitch.wat"),
        manifest.capabilities,
    )
    .unwrap();
    let effects=guest.call(json!({"type":"websocket","now_ms":crate::control::now_ms(),"payload":{"metadata":{"message_type":"notification","subscription_type":"channel.chat.message"},"payload":{"event":{"broadcaster_user_id":"fixture","message_id":"turn-once","chatter_user_id":"moderator","chatter_user_name":"Moderator","message":{"text":"!turn"},"badges":[{"set_id":"moderator"}]}}}})).unwrap();
    for effect in effects {
        host.publish(&workspace.id, effect).await.unwrap();
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
    let _ = live::configure(State(runtime.clone()), Json(restored))
        .await
        .unwrap();
    until(|| colors(sink, "red", "blue")).await;
}
