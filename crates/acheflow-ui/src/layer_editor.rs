use crate::{
    code_editor::{CodeEditor, CodeKind},
    live::NumberField,
    live_client::{Action, LiveClient},
    project::{EffectKind, Layer, LayerEffect, Workspace},
};
use dioxus_native::prelude::*;

fn edit(
    mut project: Signal<Workspace>,
    scene: u32,
    layer_id: u64,
    change: impl FnOnce(&mut Layer),
) {
    if let Some(scene) = project.write().scenes.iter_mut().find(|s| s.id == scene)
        && let Some(layer) = scene.layers.iter_mut().find(|l| l.id == layer_id)
    {
        change(layer);
    }
}

#[component]
pub fn LayerEditor(
    mut project: Signal<Workspace>,
    scene_id: u32,
    mut selection: Signal<Option<u64>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|s| s.id == scene_id) else {
        return rsx! {};
    };
    let mut visited = use_signal(std::collections::HashSet::<u64>::new);
    use_effect(move || {
        if let Some(id) = selection()
            && !visited.peek().contains(&id)
        {
            visited.write().insert(id);
        }
    });

    rsx! {
        div {class:"panel-heading",h3 {"Layers"}span {class:"form-help","Top of list = front"}}
        div {class:"layer-stack",for (index,layer) in scene.layers.iter().enumerate().rev() {
            {let layer_id = layer.id; rsx! {
            div {key:"{layer_id}",class:if selection()==Some(layer_id){"layer-stack-row active"}else{"layer-stack-row"},
                button {class:"layer-select","aria-pressed":(selection()==Some(layer_id)).to_string(),onclick:move |_|selection.set(Some(layer_id)),
                    "{index + 1} · "
                    {{workspace.sources.iter().find(|s|s.id==layer.source_id).map(|s|s.name.clone()).unwrap_or_else(||"Missing source".into())}}
                }
                button {disabled:index+1==scene.layers.len() || (client.busy)(),title:"Bring forward",onclick:move |_| {
                    let changed = move_layer(project,scene_id,layer_id,1); if changed && client.running(){client.command(Action::Apply);}
                },"Forward"}
                button {disabled:index==0 || (client.busy)(),title:"Send backward",onclick:move |_| {
                    let changed = move_layer(project,scene_id,layer_id,-1); if changed && client.running(){client.command(Action::Apply);}
                },"Backward"}
            }
        }}}if scene.layers.is_empty(){p {class:"padded form-help","Add a source above to create a layer."}}}
        for layer in scene.layers.iter().filter(|l| visited.read().contains(&l.id)) {
            LayerInspector { key:"{layer.id}", project, scene_id, layer_id:layer.id, selected:selection()==Some(layer.id), selection }
        }
        if selection().is_none() && !scene.layers.is_empty(){p {class:"padded form-help","Select a layer on the canvas or in this list to resize, crop and add effects."}}
    }
}

fn move_layer(mut project: Signal<Workspace>, scene: u32, id: u64, delta: isize) -> bool {
    let mut w = project.write();
    let Some(index) = w
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .and_then(|s| s.layers.iter().position(|l| l.id == id))
    else {
        return false;
    };
    let Some(to) = index.checked_add_signed(delta) else {
        return false;
    };
    w.move_layer(scene, index, to)
}
fn edit_effect(
    project: Signal<Workspace>,
    scene: u32,
    layer: u64,
    id: u64,
    change: impl FnOnce(&mut LayerEffect),
) {
    edit(project, scene, layer, |l| {
        if let Some(e) = l.effects.iter_mut().find(|e| e.id == id) {
            change(e);
        }
    });
}
fn move_effect(project: Signal<Workspace>, scene: u32, layer: u64, id: u64, delta: isize) {
    edit(project, scene, layer, |l| {
        if let Some(from) = l.effects.iter().position(|e| e.id == id)
            && let Some(to) = from.checked_add_signed(delta)
            && to < l.effects.len()
        {
            l.effects.swap(from, to);
        }
    });
}
#[component]
fn LayerInspector(
    project: Signal<Workspace>,
    scene_id: u32,
    layer_id: u64,
    selected: bool,
    mut selection: Signal<Option<u64>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let Some((index, layer)) = workspace
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .and_then(|s| s.layers.iter().enumerate().find(|(_, l)| l.id == layer_id))
    else {
        return rsx! {};
    };
    rsx! {
            div {class:"layer-inspector",style:if selected {"display:block"}else{"display:none"},
                div {class:"panel-heading",h3 {"Selected layer · {index + 1}"}button {onclick:move |_| {
                    if let Some(scene)=project.write().scenes.iter_mut().find(|s|s.id==scene_id) && let Some(index)=scene.layers.iter().position(|l|l.id==layer_id){scene.layers.remove(index);}
                    selection.set(None);
                },"Remove layer"}}
                div {class:"panel-heading",h3 {"Layer CSS"}}
                p {class:"inspector-help form-help","One stylesheet controls position, size, crop and appearance. Target .layer (or :layer). Crop uses --crop-left/top/right/bottom in source px or %. Use object-fit and object-position to place the image inside."}
                div {class:"row padded",for fit in ["fill","contain","cover","none","scale-down"] {button {onclick:move |_|edit(project,scene_id,layer_id,|l| {
                    let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                    l.css=acheflow_media_api::appearance::managed_css(&css,"fit",&format!(".layer {{object-fit:{fit};}}"));
                }),"{fit}"}}
                    button {onclick:move |_|edit(project,scene_id,layer_id,|l| {
                        let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                        l.css=acheflow_media_api::appearance::managed_css(&css,"frame",".layer {border:4px solid #ff9944;border-radius:24px;}");
                    }),"+ Border"}
                    button {onclick:move |_|edit(project,scene_id,layer_id,|l| {
                        let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                        l.css=acheflow_media_api::appearance::managed_css(&css,"animation","@keyframes layer_pulse {from {opacity:0.45;} to {opacity:1;}} .layer {animation:layer_pulse 2s ease-in-out infinite alternate;}");
                    }),"+ Animation"}
                }
                CodeEditor {label:"Layer CSS",kind:CodeKind::Css,value:if layer.css.is_empty(){acheflow_media_api::appearance::default_layer_css(layer)}else{layer.css.clone()},change:move |v|edit(project,scene_id,layer_id,|l|l.css=v)}
                div {class:"panel-heading",h3 {"Layer effects"}span {class:"badge","{layer.effects.len()} / 8"}}
                p {class:"inspector-help form-help","GPU effects run from top to bottom after cropping. CSS decorates the processed layer. Audio is unchanged."}
                div {class:"row padded effect-add",for kind in EffectKind::ALL {button {disabled:layer.effects.len()>=8,onclick:move |_|edit(project,scene_id,layer_id,|l|{if l.effects.len()<8{l.effects.push(LayerEffect::new(kind));}}),"+ {kind.label()}"}}}
                for (effect_index,entry) in layer.effects.iter().enumerate() {
                    {let effect_id=entry.id; let effect=entry; rsx! {
                    div {key:"{effect_id}",class:"effect-row",
                        div {class:"row",strong {"{effect.kind.label()}"}
                            button {"aria-pressed":effect.enabled.to_string(),onclick:move |_|edit_effect(project,scene_id,layer_id,effect_id,|e|e.enabled = !e.enabled),if effect.enabled {"Enabled"}else{"Disabled"}}
                            button {disabled:effect_index==0,"aria-label":"Move effect earlier",onclick:move |_|move_effect(project,scene_id,layer_id,effect_id,-1),"↑"}
                            button {disabled:effect_index+1==layer.effects.len(),"aria-label":"Move effect later",onclick:move |_|move_effect(project,scene_id,layer_id,effect_id,1),"↓"}
                            button {onclick:move |_|edit(project,scene_id,layer_id,|l|{l.effects.retain(|e| e.id!=effect_id);}),"Remove"}
                        }
                        NumberField {label:effect.kind.amount_label(),value:effect.amount,max:effect.kind.max(),change:move |v|edit_effect(project,scene_id,layer_id,effect_id,|e|e.amount=v)}
                        if effect.kind==EffectKind::Wgsl {
                            p {class:"form-help","fn effect(uv, color) → vec4f · sample_source(uv) · params.time (seconds), params.strength, params.size. Bounded fragment functions; no loops or custom bindings."}
                            CodeEditor {label:"WGSL shader",kind:CodeKind::Wgsl,value:effect.shader.clone(),change:move |v|edit_effect(project,scene_id,layer_id,effect_id,|e|e.shader=v)}
                        }

                    }
                }}}
                div {class:"row padded",button {class:"primary",disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene & sources"}span {class:"form-help","Crop, size and effects are saved locally. Apply to update the live composition."}}
            }

    }
}
