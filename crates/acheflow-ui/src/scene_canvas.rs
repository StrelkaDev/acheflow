use crate::{
    live_client::{Action, LiveClient},
    project::{Orientation, Workspace},
    scene_geometry::{self, Bounds, Corner, DragKind},
    scene_preview::ScenePreview,
};
use dioxus_native::prelude::*;

#[derive(Clone, Copy)]
struct LayerDrag {
    scene: u32,
    index: usize,
    client_x: f64,
    client_y: f64,
    bounds: Bounds,
    kind: DragKind,
    scale_x: f64,
    scale_y: f64,
}

#[component]
pub fn Canvas(
    mut project: Signal<Workspace>,
    scene_id: Option<u32>,
    orientation: Orientation,
    #[props(default = false)] editable: bool,
    #[props(default = true)] live_images: bool,
    #[props(default)] selection: Option<Signal<Option<usize>>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let mut mounted: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);
    let dragging: Signal<Option<LayerDrag>> = use_signal(|| None);
    let metrics = use_signal(Vec::<acheflow_composition::gpu::LayerBox>::new);
    let local_selection = use_signal(|| None::<usize>);
    let selection = selection.unwrap_or(local_selection);
    let p = project.read();
    let base = p.stream(orientation).base.clone();
    let scene = p
        .scenes
        .iter()
        .find(|s| Some(s.id) == scene_id && s.orientation == orientation)
        .cloned();
    let state = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    rsx! {div {class:"canvas-wrap",div {
        class:if orientation==Orientation::Vertical{"scene-canvas portrait"}else{"scene-canvas"},
        style:format!("aspect-ratio: {} / {};",base.width,base.height),
        onmounted:move |event:MountedEvent|mounted.set(Some(event.data())),
        onmousemove:move |event| {
            if let Some(drag)=dragging() {
                let point=event.client_coordinates();
                let b=scene_geometry::transform(drag.bounds,drag.kind,(point.x-drag.client_x)*drag.scale_x,(point.y-drag.client_y)*drag.scale_y,(base.width,base.height));
                if let Some(scene)=project.write().scenes.iter_mut().find(|s|s.id==drag.scene) && let Some(layer)=scene.layers.get_mut(drag.index) {
                    let css=if layer.css.is_empty(){acheflow_media_api::appearance::default_layer_css(layer)}else{layer.css.clone()};
                    layer.x=b.x;layer.y=b.y;layer.width=b.width;layer.height=b.height;
                    layer.css=acheflow_media_api::appearance::geometry_css(&css,b.x,b.y,b.width,b.height);
                }
            }
        },
        onmouseup:move |_|finish_drag(dragging,client),
        onmouseleave:move |_|finish_drag(dragging,client),
        if let Some(scene)=scene {
        if live_images {ScenePreview {project,scene_id:scene.id,orientation,metrics:Some(metrics)}}
        for (index,layer) in scene.layers.iter().enumerate(){
            div {key:"{index}",class:format!("scene-layer {} {} {}",if editable {"editable"}else{""},if editable && selection()==Some(index){"selected-layer"}else{""},if live_images {"has-video"}else{""}),
                style:{let b=layer_bounds(layer,metrics.read().get(index).copied());format!("left:{}%;top:{}%;width:{}%;height:{}%;",b.x as f64/base.width as f64*100.,b.y as f64/base.height as f64*100.,b.width as f64/base.width as f64*100.,b.height as f64/base.height as f64*100.)},
                onmousedown:{let layer=layer.clone();let scene_id=scene.id;let base=base.clone();move |event:MouseEvent| {
                    if !editable{return;}
                    event.prevent_default();event.stop_propagation();
                    begin_drag(event,mounted,dragging,selection,DragStart::new(scene_id,index,layer_bounds(&layer,metrics.peek().get(index).copied()),&base,DragKind::Move));
                }},
                div {class:"layer-label",span {{p.sources.iter().find(|s|s.id==layer.source_id).map(|s|s.name.clone()).unwrap_or_else(||"Missing source".into())}}small {if state.as_ref().and_then(|s|s.sources.iter().find(|s|s.id==layer.source_id)).is_some_and(|s|s.connected){"RTMP connected"}else{"Waiting for RTMP"}}}
                if editable && selection()==Some(index) {for corner in Corner::ALL {
                    button {class:format!("resize-handle {}",corner.class()),"aria-label":corner.label(),title:corner.label(),
                        onmousedown:{let layer=layer.clone();let scene_id=scene.id;let base=base.clone();move |event:MouseEvent| {
                            event.prevent_default();event.stop_propagation();
                            begin_drag(event,mounted,dragging,selection,DragStart::new(scene_id,index,layer_bounds(&layer,metrics.peek().get(index).copied()),&base,DragKind::Resize(corner)));
                        }}
                    }
                }}
            }
        }if scene.layers.is_empty(){span {class:"canvas-empty","Empty scene"}}}else{span {class:"canvas-empty","No scene selected"}}
    }p {class:"form-help","{base.width} × {base.height} / {orientation.label()}"}}}
}
fn finish_drag(mut dragging: Signal<Option<LayerDrag>>, client: LiveClient) {
    if dragging().is_some() {
        dragging.set(None);
        if client.running() {
            client.command(Action::Apply);
        }
    }
}
// All pointer coordinates are converted with the mounted canvas's current size.
struct DragStart {
    scene: u32,
    index: usize,
    bounds: Bounds,
    kind: DragKind,
    base: (u32, u32),
}
fn begin_drag(
    event: MouseEvent,
    mounted: Signal<Option<std::rc::Rc<MountedData>>>,
    mut dragging: Signal<Option<LayerDrag>>,
    mut selection: Signal<Option<usize>>,
    start: DragStart,
) {
    selection.set(Some(start.index));
    let point = event.client_coordinates();
    spawn(async move {
        if let Some(handle) = mounted()
            && let Ok(rect) = handle.get_client_rect().await
            && rect.width() > 0.
            && rect.height() > 0.
        {
            dragging.set(Some(LayerDrag {
                scene: start.scene,
                index: start.index,
                client_x: point.x,
                client_y: point.y,
                bounds: start.bounds,
                kind: start.kind,
                scale_x: start.base.0 as f64 / rect.width(),
                scale_y: start.base.1 as f64 / rect.height(),
            }));
        }
    });
}

impl DragStart {
    fn new(
        scene: u32,
        index: usize,
        bounds: Bounds,
        base: &crate::project::Resolution,
        kind: DragKind,
    ) -> Self {
        Self {
            scene,
            index,
            bounds,
            kind,
            base: (base.width, base.height),
        }
    }
}

fn layer_bounds(
    layer: &crate::project::Layer,
    metrics: Option<acheflow_composition::gpu::LayerBox>,
) -> Bounds {
    metrics
        .filter(|b| b.width > 0. && b.height > 0.)
        .map(|b| Bounds {
            x: b.x.max(0.).round() as u32,
            y: b.y.max(0.).round() as u32,
            width: b.width.max(1.).round() as u32,
            height: b.height.max(1.).round() as u32,
        })
        .unwrap_or(Bounds {
            x: layer.x,
            y: layer.y,
            width: layer.width,
            height: layer.height,
        })
}
