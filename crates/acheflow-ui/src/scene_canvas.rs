use crate::{
    live_client::{Action, LiveClient},
    project::{Orientation, Workspace},
    scene_geometry::{self, Bounds, Corner, DragConstraints, DragKind},
    scene_preview::ScenePreview,
};
use dioxus_native::prelude::*;
use std::collections::VecDeque;

#[derive(Clone)]
struct LayerDrag {
    scene: u32,
    layer_id: u64,
    client_x: f64,
    client_y: f64,
    bounds: Bounds,
    kind: DragKind,
    scale_x: f64,
    scale_y: f64,
    source_aspect: Option<f64>,
    before: LayerGeometry,
}

const EDGE_SNAP_PX: f64 = 8.;
const HISTORY_LIMIT: usize = 64;

#[derive(Clone, PartialEq)]
struct LayerGeometry {
    bounds: Bounds,
    css: String,
}

impl LayerGeometry {
    fn read(layer: &crate::project::Layer) -> Self {
        Self {
            bounds: layer_bounds(layer, None),
            css: layer.css.clone(),
        }
    }

    fn restore(&self, layer: &mut crate::project::Layer) {
        layer.x = self.bounds.x;
        layer.y = self.bounds.y;
        layer.width = self.bounds.width;
        layer.height = self.bounds.height;
        layer.css.clone_from(&self.css);
    }
}

struct GeometryEdit {
    scene: u32,
    layer: u64,
    before: LayerGeometry,
    after: LayerGeometry,
}

#[derive(Default)]
pub(crate) struct SceneHistory {
    undo: VecDeque<SceneEdit>,
    redo: VecDeque<SceneEdit>,
}

impl SceneHistory {
    fn record(&mut self, edit: SceneEdit) {
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(edit);
        self.redo.clear();
    }

    pub(crate) fn record_swap(&mut self, scene: u32, indices: [usize; 2], before: [u64; 2]) {
        self.record(SceneEdit::Swap {
            scene,
            indices,
            before,
        });
    }

    fn can_step(&self, project: &Workspace, redo: bool) -> bool {
        let entries = if redo { &self.redo } else { &self.undo };
        entries.back().is_some_and(|edit| {
            project
                .scenes
                .iter()
                .find(|scene| scene.id == edit.scene())
                .is_some_and(|scene| edit.matches(&scene.layers, redo))
        })
    }

    fn step(&mut self, project: &mut Workspace, redo: bool) -> bool {
        if !self.can_step(project, redo) {
            return false;
        }
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let edit = from.pop_back().unwrap();
        let scene = project
            .scenes
            .iter_mut()
            .find(|scene| scene.id == edit.scene())
            .unwrap();
        edit.apply(&mut scene.layers, redo);
        to.push_back(edit);
        true
    }
}

enum SceneEdit {
    Geometry(GeometryEdit),
    Swap {
        scene: u32,
        indices: [usize; 2],
        before: [u64; 2],
    },
}
impl SceneEdit {
    fn scene(&self) -> u32 {
        match self {
            Self::Geometry(edit) => edit.scene,
            Self::Swap { scene, .. } => *scene,
        }
    }

    fn matches(&self, layers: &[crate::project::Layer], redo: bool) -> bool {
        match self {
            Self::Geometry(edit) => {
                let expected = if redo { &edit.before } else { &edit.after };
                layers
                    .iter()
                    .find(|layer| layer.id == edit.layer)
                    .is_some_and(|layer| &LayerGeometry::read(layer) == expected)
            }
            Self::Swap {
                indices, before, ..
            } => {
                let ids = if redo {
                    *before
                } else {
                    [before[1], before[0]]
                };
                indices
                    .iter()
                    .zip(ids)
                    .all(|(&index, id)| layers.get(index).is_some_and(|layer| layer.id == id))
            }
        }
    }

    fn apply(&self, layers: &mut [crate::project::Layer], redo: bool) {
        match self {
            Self::Geometry(edit) => {
                let layer = layers
                    .iter_mut()
                    .find(|layer| layer.id == edit.layer)
                    .unwrap();
                let replacement = if redo { &edit.after } else { &edit.before };
                replacement.restore(layer);
            }
            Self::Swap { indices, .. } => layers.swap(indices[0], indices[1]),
        }
    }
}

#[component]
pub(crate) fn Canvas(
    mut project: Signal<Workspace>,
    scene_id: Option<u32>,
    orientation: Orientation,
    #[props(default = false)] editable: bool,
    #[props(default = true)] live_images: bool,
    #[props(default)] selection: Option<Signal<Option<u64>>>,
    #[props(default)] history: Option<Signal<SceneHistory>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let mut mounted: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);
    let dragging: Signal<Option<LayerDrag>> = use_signal(|| None);
    let local_history = use_signal(SceneHistory::default);
    let history = history.unwrap_or(local_history);
    let metrics = use_signal(Vec::<acheflow_composition::gpu::LayerBox>::new);
    let diagnostic = use_signal(|| None::<String>);
    let local_selection = use_signal(|| None::<u64>);
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
    let empty = scene.as_ref().map(|s| s.layers.is_empty());
    let can_edit_history = editable && !(client.busy)() && dragging.read().is_none();
    let can_undo = can_edit_history && history.read().can_step(&p, false);
    let can_redo = can_edit_history && history.read().can_step(&p, true);
    rsx! {div {class:"canvas-wrap",div {
        class:if orientation==Orientation::Vertical{"scene-canvas portrait"}else{"scene-canvas"},
        style:format!("aspect-ratio: {} / {};",base.width,base.height),
        tabindex:if editable { "0" } else { "-1" },
        onmounted:move |event:MountedEvent|mounted.set(Some(event.data())),
        onkeydown:move |event:KeyboardEvent| {
            let modifiers=event.modifiers();
            if editable && modifiers.intersects(Modifiers::META | Modifiers::CONTROL)
                && !modifiers.contains(Modifiers::ALT)
                && matches!(event.key(), Key::Character(key) if key.eq_ignore_ascii_case("z"))
            {
                event.prevent_default();
                step_history(project, history, dragging, client, modifiers.contains(Modifiers::SHIFT));
            }
        },
        onmousemove:move |event| {
            if let Some(drag)=dragging.peek().as_ref() {
                let point=event.client_coordinates();
                let modifiers=event.modifiers();
                let aspect=modifiers.contains(Modifiers::SHIFT).then(|| {
                    let current=f64::from(drag.bounds.width.max(1))/f64::from(drag.bounds.height.max(1));
                    if modifiers.contains(Modifiers::ALT) {drag.source_aspect.unwrap_or(current)} else {current}
                });
                // This canvas edits the viewport content in 2D. Spatial view transforms
                // handle their own pointer gestures and never use canvas-edge snapping.
                let constraints=DragConstraints{aspect,snap:(EDGE_SNAP_PX*drag.scale_x,EDGE_SNAP_PX*drag.scale_y)};
                let b=scene_geometry::transform(drag.bounds,drag.kind,(point.x-drag.client_x)*drag.scale_x,(point.y-drag.client_y)*drag.scale_y,(base.width,base.height),constraints);
                if let Some(scene)=project.write().scenes.iter_mut().find(|s|s.id==drag.scene) && let Some(layer)=scene.layers.iter_mut().find(|l|l.id==drag.layer_id) {
                    let css=if layer.css.is_empty(){acheflow_media_api::appearance::default_layer_css(layer)}else{layer.css.clone()};
                    layer.x=b.x;layer.y=b.y;layer.width=b.width;layer.height=b.height;
                    layer.css=acheflow_media_api::appearance::geometry_css(&css,b.x,b.y,b.width,b.height);
                }
            }
        },
        onmouseup:move |_|finish_drag(dragging,project,history,client),
        onmouseleave:move |_|finish_drag(dragging,project,history,client),
        if let Some(scene)=scene {
        if live_images {ScenePreview {project,scene_id:scene.id,orientation,metrics:Some(metrics),diagnostic,placeholders:editable,viewport_content:editable}}
        if editable {for (index,layer) in scene.layers.iter().enumerate(){
            div {key:"{layer.id}",class:format!("scene-layer {} {} {}",if editable {"editable"}else{""},if editable && selection()==Some(layer.id){"selected-layer"}else{""},if live_images {"has-video"}else{""}),
                style:{let b=layer_bounds(layer,metrics.read().get(index).copied());format!("left:{}%;top:{}%;width:{}%;height:{}%;",b.x as f64/base.width as f64*100.,b.y as f64/base.height as f64*100.,b.width as f64/base.width as f64*100.,b.height as f64/base.height as f64*100.)},
                onmousedown:{let layer=layer.clone();let scene_id=scene.id;let base=base.clone();move |event:MouseEvent| {
                    if !editable{return;}
                    event.prevent_default();event.stop_propagation();
                    begin_drag(event,mounted,dragging,selection,DragStart::new(scene_id,&layer,layer_bounds(&layer,metrics.peek().get(index).copied()),&base,DragKind::Move));
                }},
                div {class:"layer-label",span {{p.sources.iter().find(|s|s.id==layer.source_id).map(|s|s.name.clone()).unwrap_or_else(||"Missing source".into())}}small {if state.as_ref().and_then(|s|s.sources.iter().find(|s|s.id==layer.source_id)).is_some_and(|s|s.connected){"RTMP connected"}else{"Waiting for RTMP"}}}
                if editable && selection()==Some(layer.id) {for corner in Corner::ALL {
                    {let source_aspect=source_aspect(&p,state.as_ref(),layer.source_id); rsx! {
                    button {class:format!("resize-handle {}",corner.class()),"aria-label":corner.label(),title:corner.label(),
                        onmousedown:{let layer=layer.clone();let scene_id=scene.id;let base=base.clone();move |event:MouseEvent| {
                            event.prevent_default();event.stop_propagation();
                            let mut start=DragStart::new(scene_id,&layer,layer_bounds(&layer,metrics.peek().get(index).copied()),&base,DragKind::Resize(corner));
                            start.source_aspect=source_aspect;
                            begin_drag(event,mounted,dragging,selection,start);
                        }}
                    }
                    }}
                }}
            }
        }}}
    }
        if editable {
            div {class:"row",
                button {disabled:!can_undo,onclick:move |_|step_history(project,history,dragging,client,false),title:"Undo layer edit (⌘/Ctrl+Z)","Undo"}
                button {disabled:!can_redo,onclick:move |_|step_history(project,history,dragging,client,true),title:"Redo layer edit (⌘/Ctrl+Shift+Z)","Redo"}
            }
        }
        if let Some(error)=diagnostic.read().as_ref(){pre {class:"code-error",role:"alert","{error}"}}
        if empty.is_none(){p {class:"form-help","No scene selected"}}else if empty==Some(true){p {class:"form-help","Empty scene"}}
        p {class:"form-help","{base.width} × {base.height} / {orientation.label()}"}}}
}
fn finish_drag(
    mut dragging: Signal<Option<LayerDrag>>,
    project: Signal<Workspace>,
    mut history: Signal<SceneHistory>,
    client: LiveClient,
) {
    if dragging.peek().is_none() {
        return;
    }
    let Some(drag) = dragging.write().take() else {
        return;
    };
    let after = project
        .peek()
        .scenes
        .iter()
        .find(|scene| scene.id == drag.scene)
        .and_then(|scene| scene.layers.iter().find(|layer| layer.id == drag.layer_id))
        .map(LayerGeometry::read);
    if let Some(after) = after.filter(|after| *after != drag.before) {
        history.write().record(SceneEdit::Geometry(GeometryEdit {
            scene: drag.scene,
            layer: drag.layer_id,
            before: drag.before,
            after,
        }));
        if client.running() {
            client.command(Action::Apply);
        }
    }
}

fn step_history(
    mut project: Signal<Workspace>,
    mut history: Signal<SceneHistory>,
    dragging: Signal<Option<LayerDrag>>,
    client: LiveClient,
    redo: bool,
) {
    if *client.busy.peek()
        || dragging.peek().is_some()
        || !history.peek().can_step(&project.peek(), redo)
    {
        return;
    }
    if history.write().step(&mut project.write(), redo) && client.running() {
        client.command(Action::Apply);
    }
}
// All pointer coordinates are converted with the mounted canvas's current size.
struct DragStart {
    scene: u32,
    layer_id: u64,
    bounds: Bounds,
    kind: DragKind,
    base: (u32, u32),
    source_aspect: Option<f64>,
    before: LayerGeometry,
}
fn begin_drag(
    event: MouseEvent,
    mounted: Signal<Option<std::rc::Rc<MountedData>>>,
    mut dragging: Signal<Option<LayerDrag>>,
    mut selection: Signal<Option<u64>>,
    start: DragStart,
) {
    selection.set(Some(start.layer_id));
    let point = event.client_coordinates();
    spawn(async move {
        if let Some(handle) = mounted()
            && let Ok(rect) = handle.get_client_rect().await
            && rect.width() > 0.
            && rect.height() > 0.
        {
            dragging.set(Some(LayerDrag {
                scene: start.scene,
                layer_id: start.layer_id,
                client_x: point.x,
                client_y: point.y,
                bounds: start.bounds,
                kind: start.kind,
                scale_x: start.base.0 as f64 / rect.width(),
                scale_y: start.base.1 as f64 / rect.height(),
                source_aspect: start.source_aspect,
                before: start.before,
            }));
            let _ = handle.set_focus(true).await;
        }
    });
}

impl DragStart {
    fn new(
        scene: u32,
        layer: &crate::project::Layer,
        bounds: Bounds,
        base: &crate::project::Resolution,
        kind: DragKind,
    ) -> Self {
        Self {
            scene,
            layer_id: layer.id,
            bounds,
            kind,
            base: (base.width, base.height),
            source_aspect: None,
            before: LayerGeometry::read(layer),
        }
    }
}

fn source_aspect(
    workspace: &Workspace,
    status: Option<&acheflow_media_api::live::LiveStatus>,
    id: u32,
) -> Option<f64> {
    let (width, height) = status
        .and_then(|state| state.sources.iter().find(|input| input.id == id))
        .and_then(|input| input.width.zip(input.height))
        .filter(|&(width, height)| width > 0 && height > 0)
        .or_else(|| {
            workspace
                .sources
                .iter()
                .find(|source| source.id == id)
                .map(|source| (source.resolution.width, source.resolution.height))
        })?;
    (width > 0 && height > 0).then(|| f64::from(width) / f64::from(height))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Layer;

    #[test]
    fn layer_order_history_restores_order_without_overwriting_layer_fields() {
        let mut project = Workspace::default();
        let scene = project.add_scene(Orientation::Horizontal);
        let first = Layer::default();
        let second = Layer::default();
        let original = [first.id, second.id];
        project.scenes[0].layers.extend([second, first]);
        let mut history = SceneHistory::default();
        history.record_swap(scene, [0, 1], original);
        project.scenes[0].layers[0].width = 42;

        assert!(history.step(&mut project, false));
        assert_eq!(project.scenes[0].layers[0].id, original[0]);
        assert_eq!(project.scenes[0].layers[1].width, 42);
        assert!(history.step(&mut project, true));
        assert_eq!(project.scenes[0].layers[0].id, original[1]);
        project.scenes[0].layers.swap(0, 1);
        assert!(!history.step(&mut project, false));
    }

    #[test]
    fn geometry_history_groups_gestures_and_preserves_other_edits() {
        let mut project = Workspace::default();
        let scene = project.add_scene(Orientation::Horizontal);
        let mut layer = Layer {
            width: 30,
            height: 20,
            ..Layer::default()
        };
        let before = LayerGeometry::read(&layer);
        layer.x = 12;
        layer.width = 42;
        layer.css = "left:12px;width:42px;".into();
        let after = LayerGeometry::read(&layer);
        let id = layer.id;
        project.scenes[0].layers.push(layer);

        let mut history = SceneHistory::default();
        history.record(SceneEdit::Geometry(GeometryEdit {
            scene,
            layer: id,
            before: before.clone(),
            after: after.clone(),
        }));
        project.scenes[0].layers[0].source_id = 7;
        assert!(history.step(&mut project, false));
        assert!(LayerGeometry::read(&project.scenes[0].layers[0]) == before);
        assert_eq!(project.scenes[0].layers[0].source_id, 7);
        assert!(!history.step(&mut project, false));
        assert!(history.step(&mut project, true));
        assert!(LayerGeometry::read(&project.scenes[0].layers[0]) == after);

        project.scenes[0].layers[0].x = 55;
        assert!(!history.step(&mut project, false));
        assert_eq!(project.scenes[0].layers[0].x, 55);
    }
}
