use crate::{
    project::{Orientation, Workspace},
    spatial_editor::edit_transform,
    spatial_view::{self, AxisHandle, EditorView, Tool},
};
use acheflow_media_api::spatial::Transform;
use dioxus_native::prelude::dioxus_html::{geometry::WheelDelta, input_data::MouseButton};
use dioxus_native::{NodeHandle, prelude::*};
use glam::Vec2;

#[derive(Clone)]
enum Gesture {
    Orbit,
    Pan,
    Object {
        id: u64,
        start: Transform,
        axis: AxisHandle,
        tool: Tool,
    },
}
#[derive(Clone)]
struct Drag {
    origin: Vec2,
    size: Vec2,
    view: EditorView,
    gesture: Gesture,
}
#[component]
pub fn SpatialViewport(
    project: Signal<Workspace>,
    scene_id: u32,
    orientation: Orientation,
    mut view: Signal<EditorView>,
    camera_preview: bool,
) -> Element {
    let mut handle = use_signal(|| None::<NodeHandle>);
    let mut drag = use_signal(|| None::<Drag>);
    let mut pressed = use_signal(|| false);
    let mut generation = use_signal(|| 0u64);
    let diagnostic = use_signal(|| None::<String>);
    let cancel = move || {
        generation += 1;
        if let Some(start) = drag.peek().as_ref() {
            match &start.gesture {
                Gesture::Object { id, start, .. } => {
                    edit_transform(project, scene_id, *id, start.clone())
                }
                _ => view.set(start.view.clone()),
            }
        }
        drag.set(None);
        pressed.set(false);
    };
    rsx! {div{class:if camera_preview{"spatial-stage camera-stage"}else{"spatial-stage editor-stage"},tabindex:0,
        role:"region","aria-label":if camera_preview{"Active camera preview"}else{"3D scene editor"},
        onmounted:move|e|handle.set(e.data().downcast::<NodeHandle>().cloned()),
        oncontextmenu:move|e|{if !camera_preview{e.prevent_default();}},
        onpointerdown:move|e|{
            if camera_preview{return;}
            e.prevent_default();e.stop_propagation();pressed.set(true);generation+=1;let token=*generation.peek();
            let point=e.client_coordinates();let point=Vec2::new(point.x as f32,point.y as f32);
            let navigate=e.trigger_button()!=Some(MouseButton::Primary)||e.modifiers().contains(Modifiers::ALT);
            let pan=e.modifiers().contains(Modifiers::SHIFT);
            spawn(async move {
                let bounds=handle.peek().as_ref().and_then(|h|h.doc().get_client_bounding_rect(h.node_id()));
                if *generation.peek()!=token {return;}
                let Some(bounds)=bounds else{return;};
                if let Some(h)=handle.peek().as_ref(){h.doc_mut().set_focus_to(h.node_id());}
                let size=Vec2::new(bounds.width as f32,bounds.height as f32);if size.min_element()<=0.{return;}
                let local=point-Vec2::new(bounds.x as f32,bounds.y as f32);
                let snapshot=view.peek().clone();
                let workspace=project.peek();let Some(scene)=workspace.scenes.iter().find(|s|s.id==scene_id).and_then(|s|s.spatial.as_ref())else{return;};
                let gesture=if navigate {if pan{Gesture::Pan}else{Gesture::Orbit}}
                    else if let Some(axis)=spatial_view::pick_axis(&snapshot,scene,local,size) {
                        let Some(id)=snapshot.selected else{return;};let Some(start)=spatial_view::selected_transform(scene,id)else{return;};
                        Gesture::Object{id,start:start.clone(),axis,tool:snapshot.tool}
                    }else{
                        let picked=spatial_view::pick(&snapshot,scene,local,size);
                        view.write().selected=picked;
                        if picked.is_some(){return;}
                        if pan{Gesture::Pan}else{Gesture::Orbit}
                    };
                if *pressed.peek(){drag.set(Some(Drag{origin:point,size,view:snapshot,gesture}));}
            });
        },
        onpointermove:move|e|{
            let Some(start)=drag.peek().clone()else{return;};
            e.prevent_default();let p=e.client_coordinates();let delta=Vec2::new(p.x as f32,p.y as f32)-start.origin;
            match start.gesture {
                Gesture::Orbit=>{let mut next=start.view;next.orbit(delta);view.set(next);},
                Gesture::Pan=>{let mut next=start.view;next.pan(delta,start.size.y);view.set(next);},
                Gesture::Object{id,start,axis,tool}=>edit_transform(project,scene_id,id,spatial_view::manipulate(&start,tool,axis,delta,e.modifiers().contains(Modifiers::SHIFT))),
            }
        },
        onpointerup:move|_|{
            pressed.set(false);drag.set(None);
            if !camera_preview {spawn(async move {if let Some(h)=handle.peek().as_ref(){h.doc_mut().set_focus_to(h.node_id());}});}
        },
        onpointercancel:move|_|{let mut cancel=cancel;cancel();},
        onwheel:move|e|{
            if camera_preview{return;}
            e.prevent_default();e.stop_propagation();
            let amount=match e.delta(){WheelDelta::Pixels(v)=>v.y,WheelDelta::Lines(v)=>v.y*40.,WheelDelta::Pages(v)=>v.y*500.};
            // Blitz forwards winit deltas: positive Y is wheel-up, unlike DOM WheelEvent.
            view.write().zoom(-amount as f32);
        },
        onkeydown:move|e|{
            if camera_preview{return;}
            let key=e.key();
            match key {
                Key::Named(NamedKey::Escape)=>{e.prevent_default();let mut cancel=cancel;cancel();},
                Key::Character(ref c) if ["g","r","s"].contains(&c.to_lowercase().as_str())=>{e.prevent_default();view.write().tool=match c.to_lowercase().as_str(){"r"=>Tool::Rotate,"s"=>Tool::Scale,_=>Tool::Move};},
                Key::Character(ref c) if c.eq_ignore_ascii_case("f")=>{
                    e.prevent_default();let p=project.peek();if let Some(scene)=p.scenes.iter().find(|s|s.id==scene_id).and_then(|s|s.spatial.as_ref()) {
                        let transform=view.peek().selected_transform(scene).cloned();if let Some(transform)=transform{view.write().frame(&transform);}
                    }
                },_=>{},
            }
        },
        crate::scene_preview::ScenePreview{project,scene_id,orientation,diagnostic,placeholders:true,editor:if camera_preview{None}else{Some(view)}}
        if !camera_preview {div{class:"stage-caption",if view.read().perspective{"PERSPECTIVE"}else{"ORTHOGRAPHIC"}}}
        if !camera_preview {div{class:"stage-axis-legend",span{class:"axis-x","X"}span{class:"axis-y","Y"}span{class:"axis-z","Z"}}}
        if drag.read().is_some(){div{class:"spatial-drag-shield"}}
        if let Some(error)=diagnostic.read().as_ref(){pre{class:"stage-error",role:"alert","{error}"}}
    }}
}
