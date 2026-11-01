use crate::{
    live_client::LiveClient,
    project::{Orientation, Workspace},
};
use acheflow_composition::gpu::{GpuScene, LayerBox, SceneFrame, SourceFrame};
use anyrender::{PaintScene, RenderContext, Scene};
use blitz_dom::{Widget, node::ComputedStyles};
use dioxus_native::{CustomWidgetAttr, prelude::*};
use peniko::kurbo::Affine;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use web_time::Instant;

struct Snapshot {
    owner: String,
    server: String,
    frame: Option<SceneFrame>,
    patterns: HashMap<u32, SourceFrame>,
    anchor: Instant,
    boxes: Vec<LayerBox>,
    error: Option<String>,
    revision: u64,
}
type Shared = Arc<Mutex<Snapshot>>;
#[component]
pub fn ScenePreview(
    project: Signal<Workspace>,
    scene_id: u32,
    orientation: Orientation,
    mut diagnostic: Signal<Option<String>>,
    #[props(default = false)] placeholders: bool,
    #[props(default)] metrics: Option<Signal<Vec<LayerBox>>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let start = use_hook(Instant::now);
    let shared: Shared = use_hook(|| {
        Arc::new(Mutex::new(Snapshot {
            owner: String::new(),
            server: String::new(),
            frame: None,
            patterns: HashMap::new(),
            anchor: Instant::now(),
            boxes: vec![],
            error: None,
            revision: 0,
        }))
    });
    let mut version = use_signal(|| 0u64);
    let widget_shared = shared.clone();
    let widget = use_hook(move || {
        CustomWidgetAttr::new(SceneWidget {
            shared: widget_shared,
            gpu: None,
            painted: 0,
        })
    });
    let config_shared = shared.clone();
    use_effect(move || {
        let workspace = project.read();
        let Some(scene) = workspace.scenes.iter().find(|s| s.id == scene_id) else {
            return;
        };
        let status = client.status.read();
        let time = status
            .as_ref()
            .filter(|s| s.workspace_id.as_deref() == Some(workspace.id.as_str()))
            .and_then(|s| s.formats.iter().find(|f| f.orientation == orientation))
            .map(|f| f.time_seconds)
            .unwrap_or_else(|| start.elapsed().as_secs_f64());
        let mut state = config_shared.lock().unwrap();
        if state.owner != workspace.id || state.server != workspace.server_url {
            state.frame = None;
            state.owner = workspace.id.clone();
            state.server = workspace.server_url.clone();
            state.boxes.clear();
            state.patterns.clear();
        }
        if placeholders {
            for source in &workspace.sources {
                if state
                    .patterns
                    .get(&source.id)
                    .is_none_or(|p| p.size != source.resolution)
                {
                    state
                        .patterns
                        .insert(source.id, SourceFrame::test_pattern(&source.resolution));
                }
            }
            state
                .patterns
                .retain(|id, _| workspace.sources.iter().any(|s| s.id == *id));
        }
        let mut inputs = state
            .frame
            .as_ref()
            .map(|f| f.inputs.clone())
            .unwrap_or_default();
        if placeholders {
            for (id, pattern) in &state.patterns {
                inputs.entry(*id).or_insert_with(|| pattern.clone());
            }
        }
        state.frame = Some(SceneFrame {
            scene: scene.clone(),
            size: workspace.stream(orientation).base.clone(),
            inputs,
            time,
        });
        state.anchor = Instant::now();
        state.revision += 1;
        version.set(state.revision);
    });
    let fetch_shared = shared.clone();
    use_future(move || {
        let shared = fetch_shared.clone();
        async move {
            let client = reqwest::Client::new();
            loop {
                let request = {
                    let state = shared.lock().unwrap();
                    state.frame.as_ref().map(|f| {
                        (
                            state.owner.clone(),
                            state.server.clone(),
                            f.scene
                                .layers
                                .iter()
                                .map(|l| l.source_id)
                                .collect::<std::collections::BTreeSet<_>>(),
                        )
                    })
                };
                if let Some((owner, server, ids)) = request {
                    let workspace = project.peek().clone();
                    let downloaded = futures_util::future::join_all(ids.into_iter().map(|id| {
                        let client = client.clone();
                        let owner = owner.clone();
                        let workspace = workspace.clone();
                        async move {
                            let url = crate::media::server_url(
                                &workspace,
                                &format!("/api/live/source/{id}?workspace={owner}"),
                            )
                            .ok()?;
                            let (image, size) =
                                crate::video_preview::download_snapshot(&client, &url).await?;
                            Some((
                                id,
                                SourceFrame {
                                    image: Arc::new(image),
                                    size,
                                },
                            ))
                        }
                    }))
                    .await;
                    let mut state = shared.lock().unwrap();
                    if state.owner == owner && state.server == server {
                        let patterns = if placeholders {
                            state.patterns.clone()
                        } else {
                            HashMap::new()
                        };
                        if let Some(frame) = state.frame.as_mut() {
                            frame.inputs = downloaded.into_iter().flatten().collect();
                            for (id, pattern) in patterns {
                                frame.inputs.entry(id).or_insert(pattern);
                            }
                        }
                        state.revision += 1;
                        version.set(state.revision);
                    }
                }
                pause(500).await;
            }
        }
    });
    use_future(move || {
        let shared = shared.clone();
        async move {
            loop {
                let (boxes, next_error) = {
                    let state = shared.lock().unwrap();
                    (state.boxes.clone(), state.error.clone())
                };
                if let Some(mut metrics) = metrics
                    && *metrics.peek() != boxes
                {
                    metrics.set(boxes);
                }
                if *diagnostic.peek() != next_error {
                    diagnostic.set(next_error);
                }
                pause(100).await;
            }
        }
    });
    rsx! {object {class:"scene-gpu-preview","data":widget,"data-frame":"{version}",role:"img","aria-label":"GPU scene preview"}
    }
}
async fn pause(ms: u32) {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(ms).await;
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_millis(u64::from(ms))).await;
}
struct SceneWidget {
    shared: Shared,
    gpu: Option<GpuScene>,
    painted: u64,
}
impl Widget for SceneWidget {
    fn can_create_surfaces(&mut self, ctx: &mut dyn RenderContext) {
        if let Some(handle) = ctx
            .renderer_specific_context()
            .and_then(|c| c.downcast::<wgpu_context::DeviceHandle>().ok())
        {
            self.gpu = Some(GpuScene::new(handle.device.clone(), handle.queue.clone()));
        }
    }
    fn destroy_surfaces(&mut self) {
        self.gpu = None;
    }
    fn requires_redraw(&self) -> bool {
        let state = self.shared.lock().unwrap();
        state.revision != self.painted
            || state.frame.as_ref().is_some_and(|f| {
                f.scene.layers.iter().any(|l| {
                    l.css.contains("animation")
                        || l.css.contains("transition")
                        || l.effects
                            .iter()
                            .any(|e| e.enabled && e.kind == crate::project::EffectKind::Wgsl)
                })
            })
    }
    fn paint(
        &mut self,
        ctx: &mut dyn RenderContext,
        _styles: &ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        let (frame, revision) = {
            let state = self.shared.lock().unwrap();
            (
                state.frame.clone().map(|mut f| {
                    f.time += state.anchor.elapsed().as_secs_f64();
                    f
                }),
                state.revision,
            )
        };
        let Some(frame) = frame else {
            return Scene::new();
        };
        if self.gpu.is_none() {
            self.can_create_surfaces(ctx);
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return Scene::new();
        };
        let mut scene = Scene::new();
        match gpu.prepare(ctx, &frame) {
            Ok(boxes) => {
                self.shared.lock().unwrap().boxes = boxes;
                self.shared.lock().unwrap().error = None;
                gpu.draw(&mut scene, &frame.size);
            }
            Err(error) => {
                self.shared.lock().unwrap().error = Some(error);
            }
        }
        self.painted = revision;
        let mut scaled = Scene::new();
        scaled.append_scene(
            scene,
            Affine::scale_non_uniform(
                width as f64 / frame.size.width as f64,
                height as f64 / frame.size.height as f64,
            ),
        );
        scaled
    }
}
