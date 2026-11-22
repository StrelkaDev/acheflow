use crate::{
    av_sync::{FormatSync, Presentation, VideoHistory},
    ingest::Input,
    live::RuntimeState,
    media_monitor::RenderMonitor,
};
use acheflow_media_api::{
    MediaCapabilities,
    live::{FormatStatus, OutputState, OutputStatus},
    workspace::{Orientation, Scene},
};
use image::RgbImage;
use std::{
    collections::HashMap,
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
    pub(crate) preview_profiles: Vec<acheflow_media_api::live::OutputPreviewProfile>,
    pub(crate) preview_cache: Arc<tokio::sync::Mutex<crate::live::PreviewCache>>,
    encoded_previews: crate::encoded_preview::EncodedPreviews,
    output_workers: Vec<OutputWorker>,
    encoder_frame: watch::Receiver<crate::output::RenderedVideo>,
    spec: crate::output::VideoSpec,
    audio_buses: Arc<Mutex<Vec<acheflow_media_api::audio::AudioBusStatus>>>,
    sync: Arc<FormatSync>,
    render: Arc<Mutex<RenderMonitor>>,
    tasks: Vec<JoinHandle<()>>,
    started: Instant,
    renderer: String,
    error: Arc<Mutex<Option<String>>>,
}
struct OutputWorker {
    index: usize,
    config: acheflow_media_api::workspace::Output,
    audio: broadcast::Sender<Arc<crate::audio_mixer::PcmBlock>>,
    status: Arc<Mutex<OutputStatus>>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}
impl OutputWorker {
    async fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.as_mut()
            && tokio::time::timeout(Duration::from_secs(3), &mut *task)
                .await
                .is_err()
        {
            task.abort();
            let _ = task.await;
        }
        self.task = None;
    }

    async fn start(
        &mut self,
        format_cancel: &CancellationToken,
        spec: &crate::output::VideoSpec,
        frame: &watch::Receiver<crate::output::RenderedVideo>,
        capabilities: &MediaCapabilities,
        restart: bool,
    ) -> Result<(), String> {
        if !restart
            && matches!(
                self.status.lock().unwrap().state,
                OutputState::Starting | OutputState::Streaming
            )
        {
            return Ok(());
        }
        self.stop().await;
        {
            let mut status = self.status.lock().unwrap();
            status.state = OutputState::Starting;
            status.reconnect_attempt = None;
            status.error = None;
            status.frames = 0;
            status.sent_bytes = 0;
            status.bitrate_kbps = 0.;
            status.repeated_video_frames = 0;
            status.inserted_audio_blocks = 0;
            status.reconnects = 0;
            status.encoder = None;
            status.hardware = false;
        }
        self.cancel = format_cancel.child_token();
        let result = match crate::live::validate_destination(&self.config, capabilities) {
            Ok(()) => {
                crate::output::start(
                    self.config.clone(),
                    spec.clone(),
                    capabilities,
                    frame.clone(),
                    self.audio.clone(),
                    self.status.clone(),
                    self.cancel.clone(),
                )
                .await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(task) => {
                self.task = Some(task);
                Ok(())
            }
            Err(error) => {
                let mut status = self.status.lock().unwrap();
                status.state = OutputState::Failed;
                status.error = Some(error.clone());
                Err(error)
            }
        }
    }
}

impl RunningFormat {
    pub fn has_output(&self, index: usize) -> bool {
        self.output_workers
            .iter()
            .any(|worker| worker.index == index)
    }

    pub(crate) fn encoded_view(
        &mut self,
        index: usize,
        output: acheflow_media_api::workspace::Output,
        capabilities: &MediaCapabilities,
    ) -> Result<Arc<crate::encoded_preview::EncodedView>, axum::http::StatusCode> {
        self.encoded_previews.view(
            index,
            output,
            crate::encoded_preview::PreviewSource {
                spec: self.spec.clone(),
                frames: self.frame.clone(),
                cancel: self.cancel.clone(),
            },
            capabilities,
        )
    }

    pub async fn restart_output(
        &mut self,
        index: usize,
        capabilities: &MediaCapabilities,
    ) -> Result<(), String> {
        let worker = self
            .output_workers
            .iter_mut()
            .find(|worker| worker.index == index)
            .ok_or("This destination is unavailable or disabled")?;
        worker
            .start(
                &self.cancel,
                &self.spec,
                &self.encoder_frame,
                capabilities,
                true,
            )
            .await
    }

    pub async fn run_outputs(
        &mut self,
        index: Option<usize>,
        capabilities: &MediaCapabilities,
        restart: bool,
    ) -> Vec<Result<(), String>> {
        let cancel = &self.cancel;
        let spec = &self.spec;
        let frame = &self.encoder_frame;
        futures_util::future::join_all(
            self.output_workers
                .iter_mut()
                .filter(|worker| index.is_none_or(|index| worker.index == index))
                .map(|worker| worker.start(cancel, spec, frame, capabilities, restart)),
        )
        .await
    }

    pub fn status(&self, scene: Option<&Scene>) -> FormatStatus {
        FormatStatus {
            applied_scene: scene.map(|scene| acheflow_media_api::live::AppliedScene {
                id: scene.id,
                name: scene.name.clone(),
            }),
            audio_buses: self.audio_buses.lock().unwrap().clone(),
            audio_sync: self.sync.audio.lock().unwrap().clone(),
            render_load: self
                .render
                .lock()
                .unwrap()
                .status(Instant::now(), self.spec.fps),
            time_seconds: self.started.elapsed().as_secs_f64(),
            renderer: Some(self.renderer.clone()),
            error: self.error.lock().unwrap().clone(),
            orientation: self.orientation,
            frames: self.frames.load(Ordering::Relaxed),
            preview_profiles: self.preview_profiles.clone(),
            outputs: self
                .outputs
                .iter()
                .map(|o| o.lock().unwrap().clone())
                .collect(),
        }
    }
    pub async fn join(mut self) {
        for worker in &mut self.output_workers {
            worker.stop().await;
        }
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
    preview_only: bool,
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
    let (encoder_tx, encoder_frame) = watch::channel(crate::output::RenderedVideo {
        image: video_rx.borrow().clone(),
        rendered_at: Instant::now(),
    });
    let audio_buses = Arc::new(Mutex::new(vec![]));
    let frames = Arc::new(AtomicU64::new(0));
    let video_runtime = runtime.clone();
    let video_cancel = cancel.clone();
    let video_frames = frames.clone();
    let fps = stream.fps;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let error = Arc::new(Mutex::new(None));
    let video_error = error.clone();
    let sync = Arc::new(FormatSync::default());
    let video_sync = sync.clone();
    let render = Arc::new(Mutex::new(RenderMonitor::default()));
    let video_render = render.clone();
    let started = Instant::now();
    let visual_epoch_ms = crate::control::now_ms();
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
        let mut history = VideoHistory::default();
        while !video_cancel.is_cancelled() {
            let render_start = Instant::now();
            let workspace = video_runtime.workspace.read().unwrap().clone();
            if let Some(workspace) = workspace {
                let inputs = video_runtime.inputs.lock().unwrap().clone();
                let (mut frame, mut presented) = gpu_frame(
                    &workspace,
                    orientation,
                    &inputs,
                    started.elapsed().as_secs_f64(),
                    &mut history,
                );
                frame.epoch_ms = visual_epoch_ms;
                if let Some(services) = video_runtime.visual_services.read().unwrap().as_ref() {
                    frame.source_documents = services.lock().unwrap().visual_documents(
                        &workspace,
                        visual_epoch_ms.saturating_add((frame.time * 1000.) as u64),
                    );
                }
                let result = renderer.render(&frame);
                let shown = Instant::now();
                video_render.lock().unwrap().record(
                    shown,
                    shown.duration_since(render_start),
                    result.is_ok(),
                );
                match result {
                    Ok(image) => {
                        let image = Arc::new(image);
                        #[cfg(test)]
                        crate::av_timing_tests::trace_image(
                            "compositor",
                            orientation,
                            shown,
                            presented.get(&1).and_then(|source| source.pts),
                            &image,
                        );
                        video_tx.send_replace(image.clone());
                        encoder_tx.send_replace(crate::output::RenderedVideo {
                            image,
                            rendered_at: shown,
                        });
                        for timing in presented.values_mut() {
                            timing.shown = shown;
                        }
                        *video_sync.presented.lock().unwrap() = presented;
                        video_frames.fetch_add(1, Ordering::Relaxed);
                        *video_error.lock().unwrap() = None;
                    }
                    Err(error) => {
                        *video_sync.presented.lock().unwrap() = HashMap::new();
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
    let mut running = RunningFormat {
        orientation,
        cancel,
        frame: video_rx,
        encoder_frame,
        frames,
        outputs: vec![],
        preview_profiles: stream
            .outputs
            .iter()
            .enumerate()
            .map(
                |(index, output)| acheflow_media_api::live::OutputPreviewProfile {
                    index,
                    name: output.name.clone(),
                    resolution: output.resolution.clone(),
                    codec: output.codec.clone(),
                    bitrate_kbps: output.bitrate_kbps,
                    bitrate_mode: output.bitrate_mode,
                    fps,
                },
            )
            .collect(),
        output_workers: vec![],
        preview_cache: Default::default(),
        encoded_previews: Default::default(),
        spec: crate::output::VideoSpec {
            base: stream.base.clone(),
            fps,
            #[cfg(test)]
            orientation,
        },
        audio_buses: audio_buses.clone(),
        sync: sync.clone(),
        render,
        tasks: vec![video],
        started,
        renderer,
        error,
    };
    let mut destinations = vec![];
    for (index, output) in stream
        .outputs
        .into_iter()
        .enumerate()
        .filter(|(_, o)| o.enabled)
    {
        let (audio_tx, _) = broadcast::channel(8);
        let status = Arc::new(Mutex::new(OutputStatus {
            index,
            sent_bytes: 0,
            bitrate_kbps: 0.,
            name: output.name.clone(),
            encoder: None,
            hardware: false,
            audio_scene: None,
            audio_bus: None,
            audio_meter: Default::default(),
            state: OutputState::Stopped,
            reconnect_attempt: None,
            repeated_video_frames: 0,
            inserted_audio_blocks: 0,
            reconnects: 0,
            frames: 0,
            error: None,
        }));
        destinations.push(crate::audio_mixer::Destination {
            index,
            sender: audio_tx.clone(),
            status: status.clone(),
        });
        running.outputs.push(status.clone());
        running.output_workers.push(OutputWorker {
            index,
            config: output,
            audio: audio_tx,
            status,
            cancel: running.cancel.child_token(),
            task: None,
        });
    }
    if !preview_only {
        if let Some(error) = running
            .run_outputs(None, capabilities, false)
            .await
            .into_iter()
            .find_map(Result::err)
        {
            running.cancel.cancel();
            running.join().await;
            return Err(error);
        }
    }
    running.tasks.push(tokio::spawn(crate::audio_mixer::run(
        runtime,
        orientation,
        destinations,
        audio_buses,
        sync,
        running.cancel.clone(),
    )));
    Ok(running)
}

fn gpu_frame(
    workspace: &acheflow_media_api::workspace::Workspace,
    orientation: Orientation,
    inputs: &HashMap<u32, Arc<Input>>,
    time: f64,
    history: &mut VideoHistory,
) -> (
    acheflow_composition::gpu::SceneFrame,
    HashMap<u32, Presentation>,
) {
    use acheflow_composition::gpu::{SceneFrame, SourceFrame};
    let scene = workspace
        .scenes
        .iter()
        .find(|s| Some(s.id) == workspace.active(orientation))
        .cloned()
        .unwrap_or(Scene {
            id: 0,
            name: "Empty".into(),
            orientation,
            layers: vec![],
            audio: Default::default(),
            spatial: None,
        });
    let now = Instant::now();
    history.retain(|id| {
        scene.audio.video_delay_ms(id) > 0
            && inputs
                .get(&id)
                .is_some_and(|input| input.connected.load(Ordering::Acquire))
    });
    let mut timing = HashMap::new();
    let sources = inputs
        .iter()
        .filter_map(|(&id, input)| {
            if !input.connected.load(Ordering::Acquire) {
                return None;
            }
            let frame = input.frame.read().unwrap().clone()?;
            if frame.generation != input.generation.load(Ordering::Acquire)
                || now.saturating_duration_since(frame.received) >= Duration::from_secs(2)
            {
                return None;
            }
            let (frame, presented) = history.select(id, frame, scene.audio.video_delay_ms(id), now);
            // Expose timing only for sources that enter this scene.
            if scene.visual_layers().any(|layer| layer.source_id == id) {
                timing.insert(id, presented);
            }
            Some((
                id,
                SourceFrame {
                    size: acheflow_media_api::workspace::Resolution {
                        width: frame.image.width(),
                        height: frame.image.height(),
                    },
                    image: frame.image,
                },
            ))
        })
        .collect();
    (
        SceneFrame {
            source_documents: Default::default(),
            epoch_ms: 0,
            scene,
            size: workspace.stream(orientation).base.clone(),
            time,
            inputs: sources,
        },
        timing,
    )
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
                let size = &acheflow_media_api::workspace::Resolution {
                    width: source.width(),
                    height: source.height(),
                };
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
    fn gpu_scene_uses_native_frame_dimensions_instead_of_offline_preview() {
        let mut workspace = Workspace::default();
        workspace.add_source();
        let scene = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(scene, 1);
        let input = Arc::new(Input::new(workspace.sources[0].clone()));
        input.connected.store(true, Ordering::Relaxed);
        *input.metadata.lock().unwrap() = (Some(320), Some(180), Some(30.));
        let image = Arc::new(RgbImage::from_pixel(160, 90, image::Rgb([255, 0, 0])));
        *input.frame.write().unwrap() = Some(crate::ingest::VideoFrame {
            pts: None,
            generation: 0,
            received: Instant::now(),
            image: image.clone(),
        });
        let inputs = HashMap::from([(1, input.clone())]);
        let (frame, _) = gpu_frame(
            &workspace,
            Orientation::Horizontal,
            &inputs,
            0.,
            &mut VideoHistory::default(),
        );
        assert_eq!(
            frame.inputs[&1].size,
            acheflow_media_api::workspace::Resolution {
                width: 160,
                height: 90
            }
        );
        assert!(Arc::ptr_eq(&frame.inputs[&1].image, &image));
        assert_eq!(
            (input.status().width, input.status().height),
            (Some(160), Some(90))
        );
    }
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
                pts: None,
                generation: 0,
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
            chroma_key: None,
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
            pts: None,
            generation: 0,
            received: Instant::now(),
            image: Arc::new(RgbImage::from_pixel(16, 16, image::Rgb([255, 0, 0]))),
        });
        let inputs = HashMap::from([(1, input)]);
        let mut scene = Scene {
            id: 1,
            name: "Test".into(),
            audio: Default::default(),
            spatial: None,
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
