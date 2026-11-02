use crate::{
    live_client::{Action, LiveClient},
    project::Workspace,
};
use acheflow_media_api::spatial::*;
use dioxus_native::prelude::*;
#[cfg(test)]
#[path = "float_input_tests.rs"]
mod tests;

#[component]
pub fn SpatialEditor(mut project: Signal<Workspace>, scene_id: u32, children: Element) -> Element {
    let client = use_context::<LiveClient>();
    let height = project
        .read()
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .map(|s| project.read().stream(s.orientation).base.height)
        .unwrap_or(1080);
    let mut view = use_signal(|| crate::spatial_view::EditorView::new(height as f32));
    let mut preview = use_signal(|| false);
    let mut content = use_signal(|| false);
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|s| s.id == scene_id) else {
        return rsx! {};
    };
    let base = workspace.stream(scene.orientation).base.clone();
    let orientation = scene.orientation;
    let Some(spatial) = scene.spatial.clone() else {
        return rsx! {p{class:"padded","This scene has no 3D container."}};
    };
    drop(workspace);
    let selected = view.read().selected;
    let current = spatial
        .objects
        .iter()
        .find(|o| Some(o.id) == selected)
        .cloned();
    let primary = current
        .as_ref()
        .is_some_and(|o| matches!(o.kind, ObjectKind::Viewport { primary: true, .. }));
    rsx! {section{class:"spatial-editor",
        div{class:"spatial-header",
            div{class:"editor-mode",role:"group","aria-label":"Scene view mode",
                button{class:if !preview(){"selected"}else{""},"aria-pressed":(!preview()).to_string(),onclick:move |_|preview.set(false),"Scene editor"}
                button{class:if preview(){"selected"}else{""},"aria-pressed":preview().to_string(),onclick:move |_|preview.set(true),"Camera preview"}
            }
            div{class:"row",span{class:"editor-save-hint","Local draft"}button{class:"primary",disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene"}}
        }
        if preview(){
            div{class:"camera-preview-heading",span{"Camera · Active"}button{onclick:move |_|{view.write().selected=Some(0);preview.set(false);},"Edit camera"}}
            div{class:"camera-preview-wrap",style:format!("aspect-ratio:{} / {};",base.width,base.height),crate::spatial_viewport::SpatialViewport{project,scene_id,orientation,view,camera_preview:true}}
            p{class:"editor-help","Output from the active scene camera. {base.width} × {base.height}"}
        }else{
            div{class:"editor-workspace",
                div{class:"editor-main",
                    div{class:"editor-tools",
                        div{class:"editor-tool-group",for tool in crate::spatial_view::Tool::ALL {button{class:if view.read().tool==tool{"selected"}else{""},"aria-pressed":(view.read().tool==tool).to_string(),onclick:move |_|view.write().tool=tool,"{tool.label()}"}}}
                        div{class:"editor-tool-group",
                            button{title:"Frame selected (F)",onclick:{let spatial=spatial.clone();move |_|{let transform=view.peek().selected_transform(&spatial).cloned();if let Some(t)=transform{view.write().frame(&t);}}},"Frame"}
                            button{class:if view.read().grid{"selected"}else{""},onclick:move |_|{let grid=view.peek().grid;view.write().grid= !grid;},"Grid"}
                            button{onclick:move |_|{let perspective=view.peek().perspective;view.write().perspective= !perspective;},if view.read().perspective{"Perspective"}else{"Orthographic"}}
                        }
                        div{class:"editor-tool-group",for (name,yaw,pitch) in [("Front",0.,0.),("Right",90.,0.),("Top",0.,-89.5)]{button{onclick:move |_|{let mut v=view.write();v.yaw=yaw;v.pitch=pitch;v.perspective=false;},"{name}"}}}
                    }
                    crate::spatial_viewport::SpatialViewport{project,scene_id,orientation,view,camera_preview:false}
                    p{class:"editor-help","Click to select · Drag an axis · Empty space / Alt + drag: orbit · Shift + drag: pan / snap · Wheel: zoom · F: frame · G / R / S: tools · Esc: cancel"}
                }
                aside{class:"spatial-hierarchy","aria-label":"Scene hierarchy",
                    div{class:"hierarchy-title",strong{"Scene collection"}span{"{spatial.objects.len()} objects"}}
                    button{class:if selected==Some(0){"hierarchy-item selected"}else{"hierarchy-item"},onclick:move |_|view.write().selected=Some(0),span{class:"object-type","CAM"}span{"Camera"}small{"Active"}}
                    for object in &spatial.objects {button{key:"{object.id}",class:if selected==Some(object.id){"hierarchy-item selected"}else{"hierarchy-item"},onclick:{let id=object.id;move |_|view.write().selected=Some(id)},span{class:"object-type",match object.kind{ObjectKind::Viewport{..}=>"VP",_=>"MESH"}}span{"{object.name}"}if !object.visible{small{"Hidden"}}}}
                    div{class:"hierarchy-add",span{"Add object"}for kind in ["Viewport","Plane","Cube"]{button{disabled:spatial.objects.len()>=32,onclick:{let base=base.clone();move |_|{
                        let mut p=project.write();let Some(scene)=p.scenes.iter_mut().find(|s|s.id==scene_id)else{return;};let layers=scene.layers.clone();let Some(s)=scene.spatial.as_mut()else{return;};if s.objects.len()>=32{return;}
                        let id=crate::project::new_entity_id();let (kind_data,scale)=match kind{"Viewport"=>(ObjectKind::Viewport{primary:false,canvas:base.clone(),layers},[base.width as f32/2.,base.height as f32/2.,1.]),"Plane"=>(ObjectKind::Plane{color:[0.9,0.3,0.05,1.]},[200.,200.,1.]),_=>(ObjectKind::Cube{color:[0.9,0.3,0.05,1.]},[150.;3])};
                        let transform=Transform{position:[0.,0.,100.],scale,..Default::default()};s.objects.push(SceneObject{id,name:format!("{kind} {}",s.objects.len()+1),visible:true,transform:transform.clone(),kind:kind_data});
                        let mut v=view.write();v.selected=Some(id);v.frame(&transform);
                    }},"+ {kind}"}}}
                }
            }
            section{class:"spatial-inspector","aria-label":"Selected object inspector",
                div{class:"inspector-heading",strong{"Inspector"}span{{if selected==Some(0){"Camera · Active".to_owned()}else{current.as_ref().map(|o|o.name.clone()).unwrap_or_else(||"No selection".into())}}}}
                if selected==Some(0){CameraInspector{key:"camera-{scene_id}",project,scene_id,camera:spatial.camera.clone(),canvas_height:base.height as f32}}
                else if let Some(object)=current {ObjectInspector{key:"object-{object.id}",project,scene_id,object}}
                else{p{class:"padded form-help","Select an object in the view or scene collection."}}
                if primary {div{class:"inspector-content-toggle",button{class:if content(){"selected"}else{""},onclick:move |_|{let open=content();content.set(!open);},if content(){"Close viewport content"}else{"Edit viewport content"}}span{class:"form-help","Layers, sources, crop, CSS and effects belong to this viewport."}}}
            }
        }
        // Keep viewport code editors mounted so switching mode/object keeps drafts.
        div{class:"viewport-content-editor",style:if primary && content() && !preview(){"display:block"}else{"display:none"},{children}}
    }}
}
#[component]
fn CameraInspector(
    project: Signal<Workspace>,
    scene_id: u32,
    camera: Camera,
    canvas_height: f32,
) -> Element {
    rsx! {div{class:"inspector-body",
        div{class:"inspector-transform",
            VectorInput{label:"Position",value:camera.transform.position,change:move|value|edit_scene(project,scene_id,|s|s.camera.transform.position=value)}
            VectorInput{label:"Rotation (degrees)",value:camera.transform.rotation,change:move|value|edit_scene(project,scene_id,|s|s.camera.transform.rotation=value)}
            div{class:"camera-lens",
                crate::live::Choice{label:"Projection",value:if matches!(camera.projection,Projection::Perspective{..}){"Perspective".to_owned()}else{"Orthographic".to_owned()},choices:vec!["Orthographic".into(),"Perspective".into()],change:move|v:String|edit_scene(project,scene_id,|s|s.camera.projection=if v=="Perspective"{Projection::Perspective{fov_degrees:60.}}else{Projection::Orthographic{height:canvas_height}})}
                match camera.projection {Projection::Orthographic{height}=>rsx!{FloatInput{label:"View height",value:height,change:move|value:f32|edit_scene(project,scene_id,|s|s.camera.projection=Projection::Orthographic{height:value.clamp(0.01,100_000.)})}},Projection::Perspective{fov_degrees}=>rsx!{FloatInput{label:"Field of view",value:fov_degrees,change:move|value:f32|edit_scene(project,scene_id,|s|s.camera.projection=Projection::Perspective{fov_degrees:value.clamp(1.,175.)})}}}
            }
        }
    }}
}
#[component]
fn ObjectInspector(project: Signal<Workspace>, scene_id: u32, object: SceneObject) -> Element {
    let id = object.id;
    rsx! {div{class:"inspector-body",
        div{class:"inspector-identity",label{class:"field",span{"Name"}input{value:"{object.name}",maxlength:200,oninput:move|e|edit_object(project,scene_id,id,|o|o.name=e.value())}}
            button{class:if object.visible{"selected"}else{""},onclick:move |_|edit_object(project,scene_id,id,|o|o.visible= !o.visible),if object.visible{"Visible"}else{"Hidden"}}
            button{class:"remove-object",onclick:move |_|edit_scene(project,scene_id,|s|s.objects.retain(|o|o.id!=id)),"Remove object"}
        }
        div{class:"inspector-transform",
            VectorInput{label:"Position",value:object.transform.position,change:move|v|edit_object(project,scene_id,id,|o|o.transform.position=v)}
            VectorInput{label:"Rotation (degrees)",value:object.transform.rotation,change:move|v|edit_object(project,scene_id,id,|o|o.transform.rotation=v)}
            VectorInput{label:"Scale / dimensions",value:object.transform.scale,change:move|v:[f32;3]|edit_object(project,scene_id,id,|o|o.transform.scale=v.map(|n|n.clamp(0.01,100_000.)))}
        }
        match &object.kind {
            ObjectKind::Viewport{primary:true,..}=>rsx!{},
            ObjectKind::Viewport{..}=>rsx!{ViewportLayers{project,scene_id,object_id:id}},
            ObjectKind::Plane{color}|ObjectKind::Cube{color}=>rsx!{div{class:"material-properties",VectorInput{label:"Color RGB",value:[color[0],color[1],color[2]],change:move|v:[f32;3]|edit_object(project,scene_id,id,|o|match &mut o.kind{ObjectKind::Plane{color}|ObjectKind::Cube{color}=>color[..3].copy_from_slice(&v.map(|n|n.clamp(0.,1.))),_=>{}})}}},
        }
    }}
}
pub(crate) fn edit_transform(
    project: Signal<Workspace>,
    scene: u32,
    id: u64,
    transform: Transform,
) {
    if id == 0 {
        edit_scene(project, scene, |s| s.camera.transform = transform);
    } else {
        edit_object(project, scene, id, |o| o.transform = transform);
    }
}
fn edit_scene(mut project: Signal<Workspace>, id: u32, edit: impl FnOnce(&mut SpatialScene)) {
    if let Some(scene) = project
        .write()
        .scenes
        .iter_mut()
        .find(|s| s.id == id)
        .and_then(|s| s.spatial.as_mut())
    {
        edit(scene);
    }
}
fn edit_object(
    project: Signal<Workspace>,
    scene: u32,
    id: u64,
    edit: impl FnOnce(&mut SceneObject),
) {
    edit_scene(project, scene, |s| {
        if let Some(object) = s.objects.iter_mut().find(|o| o.id == id) {
            edit(object);
        }
    });
}

#[component]
fn FloatInput(label: &'static str, value: f32, change: EventHandler<f32>) -> Element {
    let mut draft = use_signal(|| value.to_string());
    let mut shown = use_signal(|| value.to_string());
    let mut applied = use_signal(|| value);
    let mut invalid = use_signal(|| false);
    let mut reset = use_signal(|| 0u64);
    use_effect(use_reactive((&value,), move |(value,)| {
        if *draft.peek() == applied.peek().to_string() {
            draft.set(value.to_string());
            shown.set(value.to_string());
            invalid.set(false);
        }
        applied.set(value);
    }));
    let commit = move || {
        let parsed = draft.peek().trim().parse::<f32>();
        match parsed {
            Ok(number) if number.is_finite() => {
                draft.set(number.to_string());
                shown.set(number.to_string());
                change.call(number);
                invalid.set(false);
            }
            _ => {
                draft.set(value.to_string());
                shown.set(value.to_string());
                invalid.set(true);
                reset += 1;
            }
        }
    };
    rsx! {label{class:"field",span{"{label}"}input{key:"{reset}",r#type:"text",inputmode:"decimal","aria-label":label,value:{let _=shown();draft.peek().clone()},"aria-invalid":invalid().to_string(),oninput:move|e|draft.set(e.value()),onblur:move |_|{let mut commit=commit;commit();},onkeydown:move |e|{
        if e.key()==Key::Named(NamedKey::Enter){e.prevent_default();let mut commit=commit;commit();}
        if e.key()==Key::Named(NamedKey::Escape){draft.set(value.to_string());shown.set(value.to_string());invalid.set(false);reset+=1;}
    }}if invalid(){span{class:"field-error",role:"alert","Enter a finite number"}}}}
}
#[component]
fn VectorInput(label: &'static str, value: [f32; 3], change: EventHandler<[f32; 3]>) -> Element {
    rsx! {div{class:"vector-field",span{"{label}"}div{class:"vector-inputs",for (axis,name) in ["X","Y","Z"].into_iter().enumerate(){FloatInput{label:name,value:value[axis],change:move|n|{let mut next=value;next[axis]=n;change.call(next);}}}}}}
}
#[component]
fn ViewportLayers(mut project: Signal<Workspace>, scene_id: u32, object_id: u64) -> Element {
    let workspace = project.read();
    let sources = workspace.sources.clone();
    let layers = workspace
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .and_then(|s| s.spatial.as_ref())
        .and_then(|s| s.objects.iter().find(|o| o.id == object_id))
        .and_then(|o| match &o.kind {
            ObjectKind::Viewport { layers, .. } => Some(layers.clone()),
            _ => None,
        })
        .unwrap_or_default();
    drop(workspace);
    rsx! {div{class:"viewport-layers",h3{"Viewport content"}
        div{class:"row",for source in sources{button{onclick:move |_|{
            let mut p=project.write();let Some(index)=p.scenes.iter().position(|s|s.id==scene_id) else{return;};
            p.add_layer(scene_id,source.id);let Some(layer)=p.scenes[index].layers.pop() else{return;};
            if let Some(ObjectKind::Viewport{layers,..})=p.scenes[index].spatial.as_mut().and_then(|s|s.objects.iter_mut().find(|o|o.id==object_id)).map(|o|&mut o.kind){layers.push(layer);}
        },"+ {source.name}"}}}
        for layer in layers {div{key:"{layer.id}",class:"viewport-layer",strong{"Source {layer.source_id}"}
            label{class:"field",span{"CSS"}textarea{class:"viewport-css",value:"{layer.css}",rows:5,oninput:move|e|edit_object(project,scene_id,object_id,|o|if let ObjectKind::Viewport{layers,..}=&mut o.kind && let Some(layer)=layers.iter_mut().find(|l|l.id==layer.id){layer.css=e.value();})}}
            button{onclick:move |_|edit_object(project,scene_id,object_id,|o|if let ObjectKind::Viewport{layers,..}=&mut o.kind{layers.retain(|l|l.id!=layer.id);}),"Remove layer"}
        }}
    }}
}
