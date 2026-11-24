use crate::{
    layer_editor::LayerTarget,
    live_client::LiveClient,
    project::{Layer, LayerEffect, Orientation, Scene, Source, Workspace},
    scene_geometry::{self, Bounds, Corner, DragConstraints, DragKind},
    scene_preview::ScenePreview,
};
use acheflow_media_api::spatial::{ObjectKind, SceneObject, SpatialScene, Transform};
use dioxus_native::prelude::*;
use std::collections::VecDeque;
use web_time::{Duration, Instant};

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
const EFFECT_EDIT_INTERVAL: Duration = Duration::from_millis(300);

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

    pub(crate) fn record_swap(
        &mut self,
        target: LayerTarget,
        indices: [usize; 2],
        before: [u64; 2],
    ) {
        self.record(SceneEdit::Swap {
            target,
            indices,
            before,
        });
    }

    pub(crate) fn record_css(
        &mut self,
        target: LayerTarget,
        layer: u64,
        before: String,
        after: String,
    ) {
        if before != after {
            self.record(SceneEdit::Css {
                target,
                layer,
                before,
                after,
            });
        }
    }

    pub(crate) fn record_effects(
        &mut self,
        target: LayerTarget,
        layer: u64,
        before: Vec<LayerEffect>,
        after: Vec<LayerEffect>,
        control: Option<(u64, &'static str)>,
    ) {
        if before == after {
            return;
        }
        let now = Instant::now();
        // Merge adjacent samples from the same control only while the previous
        // value still matches this edit. A change after Undo starts a new step.
        if let Some(control) = control
            && self.redo.is_empty()
            && let Some(SceneEdit::Effects(previous)) = self.undo.back_mut()
            && (previous.target, previous.layer, previous.control) == (target, layer, Some(control))
            && previous.after == before
            && now.duration_since(previous.time) <= EFFECT_EDIT_INTERVAL
        {
            if previous.before == after {
                self.undo.pop_back();
            } else {
                previous.after = after;
                previous.time = now;
            }
            return;
        }
        self.record(SceneEdit::Effects(EffectsEdit {
            target,
            layer,
            before,
            after,
            control,
            time: now,
        }));
    }

    // Called with the layer present, immediately before removing it or after inserting it.
    pub(crate) fn record_presence(
        &mut self,
        target: LayerTarget,
        layers: &[Layer],
        index: usize,
        added: bool,
    ) {
        self.record(SceneEdit::Presence(LayerPresence {
            target,
            layer: layers[index].clone(),
            index,
            other_ids: layers
                .iter()
                .enumerate()
                .filter_map(|(i, layer)| (i != index).then_some(layer.id))
                .collect(),
            added,
        }));
    }

    pub(crate) fn record_name(&mut self, scene: u32, before: String, after: String) {
        if before != after {
            self.record(SceneEdit::Name {
                scene,
                before,
                after,
            });
        }
    }

    pub(crate) fn record_spatial(&mut self, scene: u32, before: SpatialScene, after: SpatialScene) {
        if before != after {
            self.record(SceneEdit::Spatial {
                scene,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
    }

    pub(crate) fn record_transform(
        &mut self,
        scene: u32,
        object: u64,
        before: Transform,
        after: Transform,
    ) {
        if before != after {
            self.record(SceneEdit::Transform {
                scene,
                object,
                before,
                after,
            });
        }
    }

    pub(crate) fn record_chat_object(
        &mut self,
        scene: u32,
        source: Source,
        source_index: usize,
        object: SceneObject,
        object_index: usize,
    ) {
        self.record(SceneEdit::Chat(Box::new(ChatObjectEdit {
            scene,
            source,
            source_index,
            object,
            object_index,
        })));
    }

    fn next(&self, redo: bool) -> Option<&SceneEdit> {
        if redo {
            self.redo.back()
        } else {
            self.undo.back()
        }
    }

    pub(crate) fn can_step(&self, project: &Workspace, redo: bool) -> bool {
        self.next(redo).is_some_and(|edit| {
            project
                .scenes
                .iter()
                .find(|scene| scene.id == edit.target().scene)
                .is_some_and(|scene| edit.matches(project, scene, redo))
        })
    }

    pub(crate) fn step(&mut self, project: &mut Workspace, redo: bool) -> bool {
        if !self.can_step(project, redo) {
            return false;
        }
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let mut edit = from.pop_back().unwrap();
        let target = edit.target();
        let scene = project
            .scenes
            .iter_mut()
            .find(|scene| scene.id == target.scene)
            .unwrap();
        edit.apply(scene, &mut project.sources, redo);
        if let SceneEdit::Effects(edit) = &mut edit {
            edit.control = None;
        }
        to.push_back(edit);
        true
    }
}

enum SceneEdit {
    Chat(Box<ChatObjectEdit>),
    Name {
        scene: u32,
        before: String,
        after: String,
    },
    Spatial {
        scene: u32,
        before: Box<SpatialScene>,
        after: Box<SpatialScene>,
    },
    Transform {
        scene: u32,
        object: u64,
        before: Transform,
        after: Transform,
    },
    Geometry(GeometryEdit),
    Css {
        target: LayerTarget,
        layer: u64,
        before: String,
        after: String,
    },
    Effects(EffectsEdit),
    Presence(LayerPresence),
    Swap {
        target: LayerTarget,
        indices: [usize; 2],
        before: [u64; 2],
    },
}

struct EffectsEdit {
    target: LayerTarget,
    layer: u64,
    before: Vec<LayerEffect>,
    after: Vec<LayerEffect>,
    control: Option<(u64, &'static str)>,
    time: Instant,
}

struct LayerPresence {
    target: LayerTarget,
    layer: Layer,
    index: usize,
    other_ids: Vec<u64>,
    added: bool,
}

impl LayerPresence {
    fn removes_layer(&self, redo: bool) -> bool {
        redo != self.added
    }

    fn matches(&self, project: &Workspace, layers: &[Layer], redo: bool) -> bool {
        if self.removes_layer(redo) {
            layers.get(self.index) == Some(&self.layer)
                && layers
                    .iter()
                    .enumerate()
                    .filter_map(|(i, layer)| (i != self.index).then_some(layer.id))
                    .eq(self.other_ids.iter().copied())
        } else {
            layers
                .iter()
                .map(|layer| layer.id)
                .eq(self.other_ids.iter().copied())
                && project
                    .sources
                    .iter()
                    .any(|source| source.id == self.layer.source_id && !source.kind.is_audio_only())
        }
    }
}

struct ChatObjectEdit {
    scene: u32,
    source: Source,
    source_index: usize,
    object: SceneObject,
    object_index: usize,
}

impl ChatObjectEdit {
    fn matches(&self, project: &Workspace, scene: &Scene, redo: bool) -> bool {
        let Some(spatial) = scene.spatial.as_ref() else {
            return false;
        };
        let id = self.source.id;
        let source = project.sources.iter().find(|source| source.id == id);
        let object = spatial
            .objects
            .iter()
            .find(|object| object.id == self.object.id);
        if redo {
            if source.is_some() || object.is_some() || project.sources.len() >= 8 {
                return false;
            }
            let mut restored = spatial.clone();
            restored.objects.push(self.object.clone());
            let source_ids = project
                .sources
                .iter()
                .filter(|source| !source.kind.is_audio_only())
                .map(|source| source.id)
                .chain(std::iter::once(id))
                .collect::<Vec<_>>();
            return restored.validate(&source_ids).is_ok();
        }

        source == Some(&self.source)
            && object == Some(&self.object)
            && !project.scenes.iter().any(|scene| {
                scene.audio.strips.iter().any(|strip| strip.source_id == id)
                    || scene.layers.iter().any(|layer| layer.source_id == id)
                    || scene
                        .spatial
                        .iter()
                        .flat_map(|spatial| &spatial.objects)
                        .filter(|object| scene.id != self.scene || object.id != self.object.id)
                        .any(|object| {
                            matches!(&object.kind, ObjectKind::Viewport { layers, .. }
                            if layers.iter().any(|layer| layer.source_id == id))
                        })
            })
    }

    fn apply(&self, spatial: &mut SpatialScene, sources: &mut Vec<Source>, redo: bool) {
        if redo {
            sources.insert(self.source_index.min(sources.len()), self.source.clone());
            spatial.objects.insert(
                self.object_index.min(spatial.objects.len()),
                self.object.clone(),
            );
        } else {
            spatial.objects.retain(|object| object.id != self.object.id);
            sources.retain(|source| source.id != self.source.id);
        }
    }
}

impl SceneEdit {
    fn target(&self) -> LayerTarget {
        match self {
            Self::Geometry(edit) => LayerTarget {
                scene: edit.scene,
                viewport: None,
            },
            Self::Chat(edit) => LayerTarget {
                scene: edit.scene,
                viewport: None,
            },
            Self::Effects(edit) => edit.target,
            Self::Presence(edit) => edit.target,
            Self::Swap { target, .. } | Self::Css { target, .. } => *target,
            Self::Name { scene, .. }
            | Self::Spatial { scene, .. }
            | Self::Transform { scene, .. } => LayerTarget {
                scene: *scene,
                viewport: None,
            },
        }
    }

    fn matches(&self, project: &Workspace, scene: &Scene, redo: bool) -> bool {
        let Some(layers) = self.target().layers(scene) else {
            return false;
        };
        match self {
            Self::Chat(edit) => edit.matches(project, scene, redo),
            Self::Name { before, after, .. } => &scene.name == if redo { before } else { after },
            Self::Spatial { before, after, .. } => {
                let (expected, replacement) = if redo {
                    (before, after)
                } else {
                    (after, before)
                };
                scene.spatial.as_ref() == Some(expected.as_ref())
                    && replacement
                        .validate(
                            &project
                                .sources
                                .iter()
                                .filter(|source| !source.kind.is_audio_only())
                                .map(|source| source.id)
                                .collect::<Vec<_>>(),
                        )
                        .is_ok()
            }

            Self::Transform {
                object,
                before,
                after,
                ..
            } => {
                scene
                    .spatial
                    .as_ref()
                    .and_then(|spatial| crate::spatial_view::selected_transform(spatial, *object))
                    == Some(if redo { before } else { after })
            }
            Self::Geometry(edit) => {
                let expected = if redo { &edit.before } else { &edit.after };
                layers
                    .iter()
                    .find(|layer| layer.id == edit.layer)
                    .is_some_and(|layer| &LayerGeometry::read(layer) == expected)
            }
            Self::Css {
                layer,
                before,
                after,
                ..
            } => {
                let expected = if redo { before } else { after };
                layers
                    .iter()
                    .find(|entry| entry.id == *layer)
                    .is_some_and(|entry| &entry.css == expected)
            }
            Self::Effects(edit) => {
                let expected = if redo { &edit.before } else { &edit.after };
                layers
                    .iter()
                    .find(|layer| layer.id == edit.layer)
                    .is_some_and(|layer| &layer.effects == expected)
            }
            Self::Presence(edit) => edit.matches(project, layers, redo),
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

    fn apply(&self, scene: &mut Scene, sources: &mut Vec<Source>, redo: bool) {
        match self {
            Self::Chat(edit) => {
                edit.apply(scene.spatial.as_mut().unwrap(), sources, redo);
                return;
            }
            Self::Name { before, after, .. } => {
                scene.name.clone_from(if redo { after } else { before });
                return;
            }
            Self::Spatial { before, after, .. } => {
                scene.spatial = Some(
                    if redo {
                        after.as_ref()
                    } else {
                        before.as_ref()
                    }
                    .clone(),
                );
                return;
            }
            Self::Transform {
                object,
                before,
                after,
                ..
            } => {
                let transform = crate::spatial_view::selected_transform_mut(
                    scene.spatial.as_mut().unwrap(),
                    *object,
                )
                .unwrap();
                transform.clone_from(if redo { after } else { before });
                return;
            }
            _ => {}
        }
        let layers = self.target().layers_mut(scene).unwrap();
        match self {
            Self::Chat(_) | Self::Name { .. } | Self::Spatial { .. } | Self::Transform { .. } => {
                unreachable!()
            }
            Self::Geometry(edit) => {
                let layer = layers
                    .iter_mut()
                    .find(|layer| layer.id == edit.layer)
                    .unwrap();
                let replacement = if redo { &edit.after } else { &edit.before };
                replacement.restore(layer);
            }
            Self::Css {
                layer,
                before,
                after,
                ..
            } => {
                let layer = layers.iter_mut().find(|entry| entry.id == *layer).unwrap();
                layer.css.clone_from(if redo { after } else { before });
            }
            Self::Effects(edit) => {
                let layer = layers
                    .iter_mut()
                    .find(|layer| layer.id == edit.layer)
                    .unwrap();
                layer
                    .effects
                    .clone_from(if redo { &edit.after } else { &edit.before });
            }
            Self::Presence(edit) => {
                if edit.removes_layer(redo) {
                    layers.remove(edit.index);
                } else {
                    layers.insert(edit.index, edit.layer.clone());
                }
            }
            Self::Swap { indices, .. } => layers.swap(indices[0], indices[1]),
        }
    }
}

pub(crate) fn history_shortcut(event: &KeyboardEvent) -> Option<bool> {
    let modifiers = event.modifiers();
    if !modifiers.intersects(Modifiers::META | Modifiers::CONTROL)
        || modifiers.contains(Modifiers::ALT)
    {
        return None;
    }
    match event.key() {
        Key::Character(key) if key.eq_ignore_ascii_case("z") => {
            Some(modifiers.contains(Modifiers::SHIFT))
        }
        Key::Character(key)
            if key.eq_ignore_ascii_case("y") && modifiers.contains(Modifiers::CONTROL) =>
        {
            Some(true)
        }
        _ => None,
    }
}

#[component]
pub(crate) fn HistoryControls(
    project: Signal<Workspace>,
    history: Signal<SceneHistory>,
) -> Element {
    let workspace = project.read();
    let timeline = history.read();
    let undo = timeline.can_step(&workspace, false);
    let redo = timeline.can_step(&workspace, true);
    let blocked =
        (!undo && timeline.next(false).is_some()) || (!redo && timeline.next(true).is_some());
    rsx! {
        div {class:"row", role:"group","aria-label":"Scene history",
            onkeydown:move |event:KeyboardEvent| {
                if let Some(redo)=history_shortcut(&event) {
                    event.prevent_default();
                    step_history(project,history,redo);
                }
            },
            crate::server_settings::ActionButton {disabled:!undo,onclick:move |_|step_history(project,history,false),"Undo"}
            crate::server_settings::ActionButton {disabled:!redo,onclick:move |_|step_history(project,history,true),"Redo"}
            if blocked {span {class:"form-help",role:"status","History conflicts with the current scene."}}
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
    rsx! {div {class:"canvas-wrap",div {
        class:if orientation==Orientation::Vertical{"scene-canvas portrait"}else{"scene-canvas"},
        style:format!("aspect-ratio: {} / {};",base.width,base.height),
        tabindex:if editable { "0" } else { "-1" },
        onmounted:move |event:MountedEvent|mounted.set(Some(event.data())),
        onkeydown:move |event:KeyboardEvent| {
            if editable && dragging.peek().is_none() && let Some(redo)=history_shortcut(&event) {
                event.prevent_default();
                step_history(project,history,redo);
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
        onmouseup:move |_|finish_drag(dragging,project,history),
        onmouseleave:move |_|finish_drag(dragging,project,history),
        if let Some(scene)=scene {
        if live_images {ScenePreview {key:"{scene.id}",project,scene_id:scene.id,orientation,metrics:Some(metrics),diagnostic,placeholders:editable,viewport_content:editable}}
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
        if let Some(error)=diagnostic.read().as_ref(){pre {class:"code-error",role:"alert","{error}"}}
        if empty.is_none(){p {class:"form-help","No scene selected"}}else if empty==Some(true){p {class:"form-help","Empty scene"}}
        p {class:"form-help","{base.width} × {base.height} / {orientation.label()}"}}}
}
fn finish_drag(
    mut dragging: Signal<Option<LayerDrag>>,
    project: Signal<Workspace>,
    mut history: Signal<SceneHistory>,
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
    }
}

pub(crate) fn step_history(
    mut project: Signal<Workspace>,
    mut history: Signal<SceneHistory>,
    redo: bool,
) {
    if history.peek().can_step(&project.peek(), redo) {
        history.write().step(&mut project.write(), redo);
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
                .map(|source| {
                    let size = source
                        .camera_mode_resolution()
                        .unwrap_or_else(|| source.resolution.clone());
                    (size.width, size.height)
                })
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

    #[tokio::test]
    async fn active_preview_switches_to_editing_and_resizes_the_selected_layer() {
        use blitz_dom::{Document, DocumentConfig};
        use blitz_traits::{
            events::{
                BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons,
                PointerCoords, UiEvent,
            },
            shell::Viewport,
        };
        use dioxus_native::DioxusDocument;
        use std::{cell::Cell, rc::Rc};

        #[derive(Default)]
        struct Harness {
            project: Cell<Option<Signal<Workspace>>>,
            editing: Cell<Option<Signal<bool>>>,
        }
        fn app() -> Element {
            let harness = consume_context::<Rc<Harness>>();
            let project = use_signal(|| {
                let mut workspace = Workspace::default();
                workspace.add_source();
                let source = workspace.sources[0].id;
                workspace.sources[0].resolution.width = 800;
                workspace.sources[0].resolution.height = 800;
                workspace.add_scene(Orientation::Horizontal);
                workspace.scenes[0].layers = [1, 2]
                    .map(|id| Layer {
                        id,
                        source_id: source,
                        x: 80,
                        y: 100,
                        width: 400,
                        height: 200,
                        ..Default::default()
                    })
                    .to_vec();
                workspace
            });
            crate::live_client::use_live_client(project);
            crate::control_client::use_control_client(project);
            harness.project.set(Some(project));
            let editing = use_signal(|| false);
            harness.editing.set(Some(editing));
            let selection = use_signal(|| Some(1));
            let scene_id = project.peek().scenes[0].id;
            rsx! {
                style {{include_str!("dashboard.css")}}
                Canvas {project, scene_id:Some(scene_id), orientation:Orientation::Horizontal,
                    editable:editing(), selection:Some(selection)}
            }
        }

        let harness = Rc::new(Harness::default());
        let mut doc = DioxusDocument::new(
            VirtualDom::new(app).with_root_context(harness.clone()),
            DocumentConfig {
                font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                    "../assets/DejaVuSans.woff2"
                ))),
                ..Default::default()
            },
        );
        doc.initial_build();
        doc.inner.borrow_mut().set_viewport(Viewport {
            window_size: (1000, 800),
            ..Default::default()
        });
        let settle = |doc: &mut DioxusDocument| {
            for _ in 0..30 {
                if !doc.poll(None) {
                    doc.inner.borrow_mut().resolve(0.);
                    return;
                }
            }
            panic!("Canvas did not settle");
        };
        settle(&mut doc);
        let preview_version = |doc: &DioxusDocument| {
            let inner = doc.inner.borrow();
            let preview = inner.query_selector(".scene-gpu-preview").unwrap().unwrap();
            let version = inner
                .get_node(preview)
                .unwrap()
                .attr("data-frame".into())
                .unwrap();
            (preview, version.parse::<u64>().unwrap())
        };
        let (preview, before) = preview_version(&doc);
        harness.editing.get().unwrap().set(true);
        settle(&mut doc);
        let (editing_preview, after) = preview_version(&doc);
        assert_eq!(editing_preview, preview);
        assert!(after > before);

        let pointer = |x, y, pressed, mods| BlitzPointerEvent {
            id: BlitzPointerId::Mouse,
            is_primary: true,
            coords: PointerCoords {
                page_x: x,
                page_y: y,
                screen_x: x,
                screen_y: y,
                client_x: x,
                client_y: y,
            },
            button: MouseEventButton::Main,
            buttons: if pressed {
                MouseEventButtons::Primary
            } else {
                MouseEventButtons::None
            },
            mods,
            details: Default::default(),
            element: Default::default(),
            active_pointers: Default::default(),
        };
        let project = harness.project.get().unwrap();
        for (mods, ratio) in [
            (Modifiers::SHIFT, 2.),
            (Modifiers::SHIFT | Modifiers::ALT, 1.),
            (Modifiers::empty(), 0.),
        ] {
            let point = {
                let inner = doc.inner.borrow();
                let handle = inner
                    .query_selector(".selected-layer .resize-handle.bottom-right")
                    .unwrap()
                    .unwrap();
                inner.get_node(handle).unwrap().absolute_position(8., 8.)
            };
            doc.handle_ui_event(UiEvent::PointerMove(pointer(point.x, point.y, false, mods)));
            doc.handle_ui_event(UiEvent::PointerDown(pointer(point.x, point.y, true, mods)));
            settle(&mut doc);
            let (x, y) = (point.x + 34., point.y + 16.);
            doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, true, mods)));
            doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, false, mods)));
            settle(&mut doc);

            let workspace = project.peek();
            let layers = &workspace.scenes[0].layers;
            assert_eq!(
                (layers[1].x, layers[1].y, layers[1].width, layers[1].height),
                (80, 100, 400, 200)
            );
            let selected = &layers[0];
            assert_eq!((selected.x, selected.y), (80, 100));
            assert!(selected.width > 400);
            if ratio > 0. {
                assert!(
                    (f64::from(selected.width) - ratio * f64::from(selected.height)).abs() <= 1.
                );
            } else {
                assert!(selected.width > selected.height);
            }
        }
        let (_, before) = preview_version(&doc);
        harness.editing.get().unwrap().set(false);
        settle(&mut doc);
        let (viewing_preview, after) = preview_version(&doc);
        assert_eq!(viewing_preview, preview);
        assert!(after > before);
    }

    #[test]
    fn offline_camera_aspect_follows_the_preset_and_auto_restores_the_manual_size() {
        use acheflow_media_api::visual::{CameraMode, SourceKind};

        let mut project = Workspace::default();
        project.add_source();
        let id = project.sources[0].id;
        project.sources[0].resolution = crate::project::Resolution {
            width: 1280,
            height: 720,
        };
        project.sources[0].kind = SourceKind::Camera {
            device_id: "fixture-camera".into(),
            mode: Some(CameraMode {
                width: 2160,
                height: 3840,
                duration_value: 1,
                duration_scale: 30,
            }),
            mirrored: false,
        };

        assert_eq!(source_aspect(&project, None, id), Some(9. / 16.));

        let SourceKind::Camera { mode, .. } = &mut project.sources[0].kind else {
            unreachable!();
        };
        *mode = None;
        assert_eq!(source_aspect(&project, None, id), Some(16. / 9.));
    }

    #[test]
    fn layer_order_history_restores_order_without_overwriting_layer_fields() {
        let mut project = Workspace::default();
        let scene = project.add_scene(Orientation::Horizontal);
        let first = Layer::default();
        let second = Layer::default();
        let original = [first.id, second.id];
        project.scenes[0].layers.extend([second, first]);
        let mut history = SceneHistory::default();
        history.record_swap(
            LayerTarget {
                scene,
                viewport: None,
            },
            [0, 1],
            original,
        );
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
    fn secondary_viewport_presence_history_keeps_other_edits_and_rejects_conflicts() {
        use acheflow_media_api::spatial::{ObjectKind, SceneObject, Transform};
        let mut project = Workspace::default();
        project.add_source();
        let scene = project.add_scene(Orientation::Horizontal);
        let source_id = project.sources[0].id;
        project.add_layer(scene, source_id);
        project.migrate_spatial();
        let target = LayerTarget {
            scene,
            viewport: Some(42),
        };
        let layer = Layer {
            source_id,
            ..Layer::default()
        };
        let mut history = SceneHistory::default();
        project.scenes[0]
            .spatial
            .as_mut()
            .unwrap()
            .objects
            .push(SceneObject {
                id: 42,
                name: "Second viewport".into(),
                visible: true,
                transform: Transform::default(),
                kind: ObjectKind::Viewport {
                    primary: false,
                    canvas: crate::project::Resolution {
                        width: 320,
                        height: 180,
                    },
                    layers: vec![layer.clone()],
                },
            });
        history.record_presence(target, target.layers(&project.scenes[0]).unwrap(), 0, true);
        project.scenes[0].layers[0].x = 42;
        assert!(history.step(&mut project, false));
        assert!(target.layers(&project.scenes[0]).unwrap().is_empty());
        assert_eq!(project.scenes[0].layers[0].x, 42);
        assert!(history.step(&mut project, true));
        let layers = target.layers_mut(&mut project.scenes[0]).unwrap();
        assert_eq!(layers.as_slice(), std::slice::from_ref(&layer));
        layers[0].x += 10;
        assert!(!history.step(&mut project, false));

        let layers = target.layers_mut(&mut project.scenes[0]).unwrap();
        history.record_presence(target, layers, 0, false);
        let removed = layers.remove(0);
        assert!(history.step(&mut project, false));
        assert_eq!(target.layers(&project.scenes[0]).unwrap(), &[removed]);
        assert!(history.step(&mut project, true));
        project.remove_source(source_id);
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
