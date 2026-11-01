use crate::{ingest::Input, live::RuntimeState};
type AudioReceiver = (Arc<Input>, broadcast::Receiver<Arc<Vec<i16>>>);
use acheflow_media_api::{
    MediaCapabilities,
    live::{FormatStatus, OutputState, OutputStatus},
    workspace::{Orientation, Scene},
};
use image::RgbImage;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{broadcast, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub struct RunningFormat {
    pub orientation: Orientation,
    pub cancel: CancellationToken,
    pub frame: watch::Receiver<Arc<RgbImage>>,
    pub frames: Arc<AtomicU64>,
    pub outputs: Vec<Arc<Mutex<OutputStatus>>>,
    tasks: Vec<JoinHandle<()>>,
    started: Instant,
    renderer: String,
    error: Arc<Mutex<Option<String>>>,
}
impl RunningFormat {
    pub fn status(&self) -> FormatStatus {
        FormatStatus {
            time_seconds: self.started.elapsed().as_secs_f64(),
            renderer: Some(self.renderer.clone()),
            error: self.error.lock().unwrap().clone(),
            orientation: self.orientation,
            frames: self.frames.load(Ordering::Relaxed),
            outputs: self
                .outputs
                .iter()
                .map(|o| o.lock().unwrap().clone())
                .collect(),
        }
    }
    pub async fn join(self) {
        for mut task in self.tasks {
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

pub async fn start(
    runtime: RuntimeState,
    orientation: Orientation,
    capabilities: &MediaCapabilities,
) -> Result<RunningFormat, String> {
    let workspace = runtime
        .workspace
        .read()
        .unwrap()
        .clone()
        .ok_or("No workspace configured")?;
    let stream = workspace.stream(orientation).clone();
    let cancel = runtime.shutdown.child_token();
    let (video_tx, video_rx) = watch::channel(Arc::new(RgbImage::new(
        stream.base.width,
        stream.base.height,
    )));
    let (audio_tx, _) = broadcast::channel::<Arc<Vec<u8>>>(64);
    let frames = Arc::new(AtomicU64::new(0));
    let video_runtime = runtime.clone();
    let video_cancel = cancel.clone();
    let video_frames = frames.clone();
    let fps = stream.fps;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let error = Arc::new(Mutex::new(None));
    let video_error = error.clone();
    let started = Instant::now();
    let size = stream.base.clone();
    let video = tokio::task::spawn_blocking(move || {
        let mut renderer = match acheflow_composition::headless::Headless::new(size) {
            Ok(renderer) => renderer,
            Err(error) => {
                let _ = ready_tx.send(Err(error));
                return;
            }
        };
        let _ = ready_tx.send(Ok(renderer.backend()));
        let period = Duration::from_secs_f64(1. / fps as f64);
        let mut deadline = Instant::now();
        while !video_cancel.is_cancelled() {
            let workspace = video_runtime.workspace.read().unwrap().clone();
            if let Some(workspace) = workspace {
                let inputs = video_runtime.inputs.lock().unwrap().clone();
                let frame = gpu_frame(
                    &workspace,
                    orientation,
                    &inputs,
                    started.elapsed().as_secs_f64(),
                );
                match renderer.render(&frame) {
                    Ok(image) => {
                        video_tx.send_replace(Arc::new(image));
                        video_frames.fetch_add(1, Ordering::Relaxed);
                        *video_error.lock().unwrap() = None;
                    }
                    Err(error) => {
                        *video_error.lock().unwrap() = Some(error);
                    }
                }
            }
            deadline += period;
            if let Some(delay) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            } else {
                deadline = Instant::now();
            }
        }
    });
    let renderer = match tokio::time::timeout(Duration::from_secs(30), ready_rx).await {
        Ok(Ok(Ok(backend))) => backend,
        result => {
            cancel.cancel();
            let error = match result {
                Ok(Ok(Err(error))) => error,
                _ => "GPU compositor did not initialize".into(),
            };
            return Err(error);
        }
    };
    let audio_runtime = runtime.clone();
    let audio_cancel = cancel.clone();
    let audio_sender = audio_tx.clone();
    let audio = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(20));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut receivers: HashMap<u32, AudioReceiver> = HashMap::new();
        loop {
            tokio::select! {_=audio_cancel.cancelled()=>break,_=ticker.tick()=>{}}
            let selected = {
                let workspace = audio_runtime.workspace.read().unwrap();
                workspace
                    .as_ref()
                    .and_then(|w| {
                        w.scenes
                            .iter()
                            .find(|s| Some(s.id) == w.active(orientation))
                    })
                    .map(|s| s.layers.iter().map(|l| l.source_id).collect::<HashSet<_>>())
                    .unwrap_or_default()
            };
            let inputs = audio_runtime.inputs.lock().unwrap().clone();
            receivers.retain(|id, _| selected.contains(id));
            for id in selected {
                if let Some(input) = inputs
                    .get(&id)
                    .filter(|i| i.connected.load(Ordering::Relaxed))
                {
                    if receivers
                        .get(&id)
                        .is_none_or(|(old, _)| !Arc::ptr_eq(old, input))
                    {
                        receivers.insert(id, (input.clone(), input.audio.subscribe()));
                    }
                } else {
                    receivers.remove(&id);
                }
            }
            let mut sum = vec![0i32; 1920];
            let mut count = 0;
            for (input, receiver) in receivers.values_mut() {
                if input.has_audio.load(Ordering::Relaxed) {
                    count += 1;
                }
                let samples = match receiver.try_recv() {
                    Ok(samples) => Some(samples),
                    Err(broadcast::error::TryRecvError::Lagged(_)) => receiver.try_recv().ok(),
                    _ => None,
                };
                if let Some(samples) = samples {
                    for (out, sample) in sum.iter_mut().zip(samples.iter()) {
                        *out += i32::from(*sample);
                    }
                }
            }
            let divisor = count.max(1);
            let bytes = sum
                .into_iter()
                .flat_map(|sample| {
                    ((sample / divisor).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
                        .to_le_bytes()
                })
                .collect();
            let _ = audio_sender.send(Arc::new(bytes));
        }
    });
    let mut running = RunningFormat {
        orientation,
        cancel,
        frame: video_rx.clone(),
        frames,
        outputs: vec![],
        tasks: vec![video, audio],
        started,
        renderer,
        error,
    };
    for output in stream.outputs.into_iter().filter(|o| o.enabled) {
        let status = Arc::new(Mutex::new(OutputStatus {
            name: output.name.clone(),
            state: OutputState::Starting,
            frames: 0,
            error: None,
        }));
        match crate::output::start(
            output,
            crate::output::VideoSpec {
                base: stream.base.clone(),
                fps,
            },
            capabilities,
            video_rx.clone(),
            audio_tx.subscribe(),
            status.clone(),
            running.cancel.child_token(),
        )
        .await
        {
            Ok(task) => {
                running.outputs.push(status);
                running.tasks.push(task);
            }
            Err(error) => {
                running.cancel.cancel();
                running.join().await;
                return Err(error);
            }
        }
    }
    Ok(running)
}

fn gpu_frame(
    workspace: &acheflow_media_api::workspace::Workspace,
    orientation: Orientation,
    inputs: &HashMap<u32, Arc<Input>>,
    time: f64,
) -> acheflow_composition::gpu::SceneFrame {
    use acheflow_composition::gpu::{SceneFrame, SourceFrame};
    SceneFrame {
        scene: workspace
            .scenes
            .iter()
            .find(|s| Some(s.id) == workspace.active(orientation))
            .cloned()
            .unwrap_or(Scene {
                id: 0,
                name: "Empty".into(),
                orientation,
                layers: vec![],
            }),
        size: workspace.stream(orientation).base.clone(),
        time,
        inputs: inputs
            .iter()
            .filter_map(|(id, input)| {
                if !input.connected.load(Ordering::Relaxed) {
                    return None;
                }
                let frame = input.frame.read().unwrap();
                frame
                    .as_ref()
                    .filter(|f| f.received.elapsed() < Duration::from_secs(2))
                    .map(|f| {
                        (
                            *id,
                            SourceFrame {
                                image: f.image.clone(),
                                size: input.config.resolution.clone(),
                            },
                        )
                    })
            })
            .collect(),
    }
}

#[cfg(test)]
#[derive(Default)]
struct Renderer {
    scene: Option<Scene>,
    sources: Vec<Option<Arc<RgbImage>>>,
    resized: Vec<Option<Arc<image::RgbaImage>>>,
    image: Option<Arc<RgbImage>>,
}

#[cfg(test)]
impl Renderer {
    fn render(
        &mut self,
        width: u32,
        height: u32,
        scene: Option<&Scene>,
        inputs: &HashMap<u32, Arc<Input>>,
    ) -> Arc<RgbImage> {
        let layers = scene.map(|s| s.layers.as_slice()).unwrap_or_default();
        let sources: Vec<_> = layers
            .iter()
            .map(|layer| {
                inputs
                    .get(&layer.source_id)
                    .filter(|i| i.connected.load(Ordering::Relaxed))
                    .and_then(|input| {
                        input
                            .frame
                            .read()
                            .unwrap()
                            .as_ref()
                            .filter(|f| f.received.elapsed() < Duration::from_secs(2))
                            .map(|f| f.image.clone())
                    })
            })
            .collect();
        let same_scene = self.scene.as_ref() == scene;
        let same_sources = self.sources.len() == sources.len()
            && self
                .sources
                .iter()
                .zip(&sources)
                .all(|(a, b)| match (a, b) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                });
        if same_scene
            && same_sources
            && let Some(image) = self
                .image
                .as_ref()
                .filter(|i| i.width() == width && i.height() == height)
        {
            return image.clone();
        }
        if !same_scene {
            self.resized.clear();
        }
        self.resized.resize_with(layers.len(), || None);
        let mut image = RgbImage::new(width, height);
        for (index, (layer, source)) in layers.iter().zip(&sources).enumerate() {
            let Some(source) = source else {
                self.resized[index] = None;
                continue;
            };
            let unchanged = self
                .sources
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|old| Arc::ptr_eq(old, source));
            let resized = if same_scene && unchanged {
                self.resized[index].clone()
            } else {
                None
            }
            .unwrap_or_else(|| {
                let size = &inputs[&layer.source_id].config.resolution;
                let prepared =
                    acheflow_composition::prepare_source(source, size, layer.crop, &layer.effects);
                if prepared.width() == layer.width && prepared.height() == layer.height {
                    Arc::new(prepared)
                } else {
                    Arc::new(image::imageops::resize(
                        &prepared,
                        layer.width,
                        layer.height,
                        image::imageops::FilterType::Triangle,
                    ))
                }
            });
            acheflow_composition::composite(&mut image, &resized, layer.x, layer.y);
            self.resized[index] = Some(resized);
        }
        self.scene = scene.cloned();
        self.sources = sources;
        let image = Arc::new(image);
        self.image = Some(image.clone());
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::workspace::{Layer, Workspace};
    #[test]
    fn scene_edits_invalidate_cached_crop_effects_and_layer_order() {
        use acheflow_media_api::workspace::{EffectKind, LayerEffect, Resolution, SourceCrop};
        let mut w = Workspace::default();
        w.add_source();
        w.add_source();
        let mut inputs = HashMap::new();
        for source in &mut w.sources {
            source.resolution = Resolution {
                width: 16,
                height: 16,
            };
            let input = Arc::new(Input::new(source.clone()));
            input.connected.store(true, Ordering::Relaxed);
            let image = if source.id == 1 {
                RgbImage::from_pixel(16, 16, image::Rgb([255, 0, 0]))
            } else {
                RgbImage::from_fn(16, 16, |x, _| {
                    if x < 8 {
                        image::Rgb([0, 255, 0])
                    } else {
                        image::Rgb([0, 0, 255])
                    }
                })
            };
            *input.frame.write().unwrap() = Some(crate::ingest::VideoFrame {
                received: Instant::now(),
                image: Arc::new(image),
            });
            inputs.insert(source.id, input);
        }
        let id = w.add_scene(Orientation::Horizontal);
        w.add_layer(id, 1);
        w.add_layer(id, 2);
        for l in &mut w.scenes[0].layers {
            l.width = 16;
            l.height = 16;
        }
        let mut renderer = Renderer::default();
        assert_eq!(
            renderer
                .render(16, 16, Some(&w.scenes[0]), &inputs)
                .get_pixel(0, 0)
                .0,
            [0, 255, 0]
        );
        w.scenes[0].layers[1].crop = SourceCrop {
            left: 8,
            ..Default::default()
        };
        assert_eq!(
            renderer
                .render(16, 16, Some(&w.scenes[0]), &inputs)
                .get_pixel(0, 0)
                .0,
            [0, 0, 255]
        );
        w.scenes[0].layers[1].effects.push(LayerEffect {
            id: acheflow_media_api::workspace::new_entity_id(),
            shader: String::new(),
            kind: EffectKind::Opacity,
            amount: 50,
            enabled: true,
        });
        let blended = renderer.render(16, 16, Some(&w.scenes[0]), &inputs);
        assert_eq!(blended.get_pixel(0, 0).0, [127, 0, 128]);
        w.move_layer(id, 0, 1);
        assert_eq!(
            renderer
                .render(16, 16, Some(&w.scenes[0]), &inputs)
                .get_pixel(0, 0)
                .0,
            [255, 0, 0]
        );
    }
    #[test]
    fn live_geometry_repositions_pixels_without_restarting_the_renderer() {
        let mut workspace = Workspace::default();
        workspace.add_source();
        let input = Arc::new(Input::new(workspace.sources[0].clone()));
        input.connected.store(true, Ordering::Relaxed);
        *input.frame.write().unwrap() = Some(crate::ingest::VideoFrame {
            received: Instant::now(),
            image: Arc::new(RgbImage::from_pixel(16, 16, image::Rgb([255, 0, 0]))),
        });
        let inputs = HashMap::from([(1, input)]);
        let mut scene = Scene {
            id: 1,
            name: "Test".into(),
            orientation: Orientation::Horizontal,
            layers: vec![Layer {
                source_id: 1,
                x: 0,
                y: 0,
                width: 16,
                height: 16,
                ..Layer::default()
            }],
        };
        let mut renderer = Renderer::default();
        let first = renderer.render(32, 16, Some(&scene), &inputs);
        assert_eq!(first.get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(first.get_pixel(20, 0).0, [0, 0, 0]);
        scene.layers[0].x = 16;
        let second = renderer.render(32, 16, Some(&scene), &inputs);
        assert_eq!(second.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(second.get_pixel(20, 0).0, [255, 0, 0]);
    }
}
