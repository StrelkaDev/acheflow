//! Real audio acceptance: independent sources/tracks/channels and live scene buses.
use crate::{
    audio_input::tests::tone_amplitude,
    live,
    pipeline_tests::{server, until},
};
use acheflow_media_api::{audio::*, live::StartRequest, workspace::*};
use axum::{Json, extract::State};
use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    sync::broadcast,
};

fn publish(url: &str, key: &str, tones: &[[u32; 2]]) -> Child {
    let mut c = Command::new(crate::ffmpeg::executable());
    c.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-re",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=160x90:r=10",
    ]);
    for [left, right] in tones {
        c.args(["-f", "lavfi", "-i"]).arg(format!(
            "aevalsrc=0.12*sin(2*PI*{left}*t)|0.12*sin(2*PI*{right}*t):s=48000"
        ));
    }
    c.args(["-map", "0:v"]);
    for n in 1..=tones.len() {
        c.arg("-map").arg(format!("{n}:a"));
    }
    c.args([
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
    .arg(format!("{url}/{key}"))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::inherit())
    .kill_on_drop(true)
    .spawn()
    .unwrap()
}
async fn signal(
    receiver: &mut broadcast::Receiver<Arc<Vec<i16>>>,
    expected: [Option<f64>; 2],
    absent: &[f64],
) {
    // Drain packets from before the live edit, then allow bounded codec/pipe delay.
    tokio::time::sleep(Duration::from_millis(700)).await;
    while receiver.try_recv().is_ok() {}
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let mut samples = vec![];
            while samples.len() < AUDIO_SAMPLES * 10 {
                match receiver.recv().await {
                    Ok(p) => samples.extend_from_slice(&p),
                    Err(broadcast::error::RecvError::Lagged(_)) => samples.clear(),
                    Err(e) => panic!("sink audio: {e}"),
                }
            }
            let okay = (0..2).all(|channel| {
                let wanted = expected[channel]
                    .is_none_or(|hz| tone_amplitude(&samples, channel, hz) > 0.035);
                let no_crosstalk = absent
                    .iter()
                    .all(|&hz| tone_amplitude(&samples, channel, hz) < 0.015);
                let quiet = expected[channel].is_some()
                    || samples
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| f64::from(s[channel]).powi(2))
                        .sum::<f64>()
                        / ((samples.len() / 2) as f64)
                        < 10000.;
                wanted && no_crosstalk && quiet
            });
            if okay {
                break;
            }
        }
    })
    .await
    .expect("expected stereo tones / isolation did not reach destination");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires FFmpeg Enhanced RTMP, Metal and loopback TCP; run explicitly"]
async fn real_scene_mixer_multitrack_sources_and_live_destination_routes() {
    let caps = Arc::new(tokio::sync::Mutex::new(crate::ffmpeg::discover().await));
    let mut sink_workspace = Workspace::default();
    sink_workspace.add_source();
    sink_workspace.add_source();
    let (sink, sink_task, sink_url) = server(sink_workspace, caps.clone()).await;
    let sink_sources = sink.workspace.read().unwrap().clone().unwrap().sources;
    let (mut first_audio, mut second_audio) = {
        let inputs = sink.inputs.lock().unwrap();
        (inputs[&1].audio.subscribe(), inputs[&2].audio.subscribe())
    };
    let mut w = Workspace::default();
    w.add_source();
    w.add_source();
    w.horizontal.base = Resolution {
        width: 160,
        height: 90,
    };
    w.horizontal.fps = 10;
    let scene = w.add_scene(Orientation::Horizontal);
    w.add_layer(scene, 1);
    w.scenes[0].audio.buses.push(AudioBus {
        id: 2,
        name: "Track 2".into(),
        ..Default::default()
    });
    w.scenes[0].audio.strips = vec![
        AudioStrip::new(
            TrackKey {
                source_id: 1,
                track_id: 0,
            },
            true,
        ),
        AudioStrip {
            sends: BTreeMap::from([(2, 100)]),
            ..AudioStrip::new(
                TrackKey {
                    source_id: 1,
                    track_id: 1,
                },
                false,
            )
        },
        AudioStrip {
            gain: 50,
            ..AudioStrip::new(
                TrackKey {
                    source_id: 2,
                    track_id: 0,
                },
                true,
            )
        },
    ];
    let other = w.add_scene(Orientation::Horizontal);
    w.add_layer(other, 2);
    w.scenes[1].audio.strips.push(AudioStrip {
        pan: 100,
        ..AudioStrip::new(
            TrackKey {
                source_id: 2,
                track_id: 0,
            },
            true,
        )
    });
    for (index, source) in sink_sources.iter().enumerate() {
        w.horizontal.outputs.push(Output {
            name: format!("Sink {index}"),
            url: sink_url.clone(),
            key: source.key.clone(),
            resolution: w.horizontal.base.clone(),
            quality: "Custom".into(),
            codec: "h264".into(),
            encoder: Some("libx264".into()),
            bitrate_kbps: 500,
            enabled: true,
            audio: OutputAudio {
                scene_buses: BTreeMap::from([(scene, index as u32 + 1)]),
                ..Default::default()
            },
        });
    }
    let (runtime, task, url) = server(w, caps).await;
    let mut w = runtime.workspace.read().unwrap().clone().unwrap();
    let mut multi = publish(&url, &w.sources[0].key, &[[440, 660], [880, 1100]]);
    let mut single = publish(&url, &w.sources[1].key, &[[1320, 1540]]);
    until(|| {
        let inputs = runtime.inputs.lock().unwrap();
        inputs[&1].audio_tracks.lock().unwrap().len() == 2
            && inputs[&2].has_audio.load(Ordering::Relaxed)
    })
    .await;
    let _ = live::start(
        State(runtime.clone()),
        Json(StartRequest {
            workspace: w.clone(),
            formats: vec![Orientation::Horizontal],
        }),
    )
    .await
    .unwrap();
    signal(&mut first_audio, [Some(440.), Some(660.)], &[880., 1100.]).await;
    signal(
        &mut second_audio,
        [Some(880.), Some(1100.)],
        &[440., 660., 1320., 1540.],
    )
    .await;
    // The second source is mixed into Program at a fixed gain, not averaged away.
    signal(&mut first_audio, [Some(1320.), Some(1540.)], &[880., 1100.]).await;
    let frames = runtime.formats.lock().await[0].frames.clone();
    let before = frames.load(Ordering::Relaxed);
    w.horizontal.outputs[0].audio.scene_buses.insert(scene, 2);
    let _ = live::configure(State(runtime.clone()), Json(w.clone()))
        .await
        .unwrap();
    signal(
        &mut first_audio,
        [Some(880.), Some(1100.)],
        &[440., 660., 1320., 1540.],
    )
    .await;
    assert!(Arc::ptr_eq(
        &frames,
        &runtime.formats.lock().await[0].frames
    ));
    assert!(frames.load(Ordering::Relaxed) > before);
    w.horizontal.outputs[0].audio.muted = true;
    let _ = live::configure(State(runtime.clone()), Json(w.clone()))
        .await
        .unwrap();
    signal(
        &mut first_audio,
        [None, None],
        &[440., 660., 880., 1100., 1320., 1540.],
    )
    .await;
    w.horizontal.outputs[0].audio.muted = false;
    w.set_active(Orientation::Horizontal, Some(other));
    let _ = live::configure(State(runtime.clone()), Json(w.clone()))
        .await
        .unwrap();
    signal(
        &mut first_audio,
        [None, Some(1540.)],
        &[440., 660., 880., 1100., 1320.],
    )
    .await;
    signal(
        &mut second_audio,
        [None, Some(1540.)],
        &[440., 660., 880., 1100., 1320.],
    )
    .await;
    single.kill().await.unwrap();
    single.wait().await.unwrap();
    until(|| {
        !runtime.inputs.lock().unwrap()[&2]
            .connected
            .load(Ordering::Acquire)
    })
    .await;
    single = publish(&url, &w.sources[1].key, &[[1320, 1540]]);
    signal(
        &mut first_audio,
        [None, Some(1540.)],
        &[440., 660., 880., 1100., 1320.],
    )
    .await;
    let status = runtime.status().await;
    let out = &status.formats[0].outputs[0];
    assert_eq!((out.audio_scene, out.audio_bus), (Some(other), Some(1)));
    assert!(out.audio_meter.peak[1] > 0.05);
    assert!(out.audio_meter.peak[0] < 0.001);
    multi.kill().await.unwrap();
    multi.wait().await.unwrap();
    single.kill().await.unwrap();
    single.wait().await.unwrap();
    runtime.close().await;
    sink.close().await;
    task.await.unwrap();
    sink_task.await.unwrap();
}
