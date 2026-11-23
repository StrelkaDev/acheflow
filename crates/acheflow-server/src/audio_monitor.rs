//! Authenticated, bounded PCM taps for client-side listening.
use crate::{access::MonitorGrant, audio_mixer::MixedBus, live::RuntimeState};
use acheflow_media_api::workspace::Orientation;
use axum::{
    Extension, Router,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;

const BACKLOG: usize = 8;

type BusSender = broadcast::Sender<Arc<Vec<i16>>>;

#[derive(Default)]
pub(crate) struct BusMonitors(Mutex<BTreeMap<(u32, u32), BusSender>>);

impl BusMonitors {
    fn subscribe(&self, scene: u32, bus: u32) -> broadcast::Receiver<Arc<Vec<i16>>> {
        self.0
            .lock()
            .unwrap()
            .entry((scene, bus))
            .or_insert_with(|| broadcast::channel(BACKLOG).0)
            .subscribe()
    }

    pub(crate) fn publish(&self, scene: Option<u32>, buses: &BTreeMap<u32, MixedBus>) {
        self.0.lock().unwrap().retain(|&(scene_id, bus), sender| {
            if scene != Some(scene_id) || sender.receiver_count() == 0 {
                return false;
            }
            let Some(bus) = buses.get(&bus) else {
                return false;
            };
            let samples = bus
                .samples
                .iter()
                .map(|sample| crate::audio_mixer::pcm_sample(*sample))
                .collect();
            let _ = sender.send(Arc::new(samples));
            true
        });
    }
}

pub(crate) fn routes() -> Router<RuntimeState> {
    Router::new()
        .route("/api/live/monitor/source/{source}/{track}", get(source))
        .route(
            "/api/live/monitor/bus/{orientation}/{scene}/{bus}",
            get(bus),
        )
}

#[derive(serde::Deserialize)]
struct WorkspaceQuery {
    workspace: String,
}

async fn source(
    State(runtime): State<RuntimeState>,
    Extension(grant): Extension<MonitorGrant>,
    Path((source, track)): Path<(u32, u8)>,
    Query(query): Query<WorkspaceQuery>,
) -> Response {
    let receiver = {
        let owner = runtime.workspace.read().unwrap();
        owner
            .as_ref()
            .filter(|owner| owner.id == query.workspace)
            .and_then(|_| runtime.inputs.lock().unwrap().get(&source).cloned())
            .and_then(|input| {
                input
                    .audio_tracks
                    .lock()
                    .unwrap()
                    .get(&track)
                    .map(|track| track.frames.subscribe())
            })
    };
    match receiver {
        Some(rx) => pcm(runtime, grant, query.workspace, rx, |frame| &frame.samples),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn bus(
    State(runtime): State<RuntimeState>,
    Extension(grant): Extension<MonitorGrant>,
    Path((orientation, scene, bus)): Path<(Orientation, u32, u32)>,
    Query(query): Query<WorkspaceQuery>,
) -> Response {
    let receiver = {
        let formats = runtime.formats.lock().await;
        let workspace = runtime.workspace.read().unwrap();
        let valid = workspace.as_ref().is_some_and(|owner| {
            owner.id == query.workspace
                && owner.active(orientation) == Some(scene)
                && owner.scenes.iter().any(|candidate| {
                    candidate.id == scene
                        && candidate
                            .audio
                            .buses
                            .iter()
                            .any(|candidate| candidate.id == bus)
                })
        });
        formats
            .iter()
            .find(|format| valid && format.orientation == orientation)
            .map(|format| format.bus_monitors.subscribe(scene, bus))
    };
    match receiver {
        Some(rx) => pcm(runtime, grant, query.workspace, rx, Vec::as_slice),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn pcm<T: Send + Sync + 'static>(
    runtime: RuntimeState,
    grant: MonitorGrant,
    workspace: String,
    receiver: broadcast::Receiver<Arc<T>>,
    samples: fn(&T) -> &[i16],
) -> Response {
    let stream = futures_util::stream::unfold(
        (
            receiver,
            tokio::time::Instant::now(),
            grant,
            runtime,
            workspace,
        ),
        move |(mut receiver, mut checked, grant, runtime, workspace)| async move {
            loop {
                match tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await {
                    Ok(Ok(frame)) => {
                        if checked.elapsed() >= Duration::from_millis(500) {
                            if !grant.allowed()
                                || !runtime
                                    .workspace
                                    .read()
                                    .unwrap()
                                    .as_ref()
                                    .is_some_and(|owner| owner.id == workspace)
                            {
                                return None;
                            }
                            checked = tokio::time::Instant::now();
                        }
                        let bytes = samples(&frame)
                            .iter()
                            .flat_map(|sample| sample.to_le_bytes())
                            .collect::<Vec<_>>();
                        return Some((
                            Ok::<_, Infallible>(Bytes::from(bytes)),
                            (receiver, checked, grant, runtime, workspace),
                        ));
                    }
                    Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                        // Resume at the live edge instead of playing the old backlog.
                        receiver = receiver.resubscribe();
                    }
                    _ => return None,
                }
            }
        },
    );
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_yields_only_the_selected_scene_bus_and_closes_on_scene_change() {
        let monitors = BusMonitors::default();
        let mut program = monitors.subscribe(1, 1);
        let mut aux = monitors.subscribe(1, 2);
        let buses = BTreeMap::from([
            (
                1,
                MixedBus {
                    samples: vec![0.5, -0.5],
                    meter: Default::default(),
                },
            ),
            (
                2,
                MixedBus {
                    samples: vec![1.2, 0.],
                    meter: Default::default(),
                },
            ),
        ]);
        monitors.publish(Some(1), &buses);
        assert_eq!(program.try_recv().unwrap().as_slice(), &[16384, -16384]);
        assert_eq!(aux.try_recv().unwrap().as_slice(), &[i16::MAX, 0]);
        monitors.publish(Some(2), &buses);
        assert_eq!(
            program.try_recv(),
            Err(broadcast::error::TryRecvError::Closed)
        );
        assert_eq!(aux.try_recv(), Err(broadcast::error::TryRecvError::Closed));
    }
}
