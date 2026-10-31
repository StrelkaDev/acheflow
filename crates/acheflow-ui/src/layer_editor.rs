use crate::{
    code_editor::{CodeEditor, CodeKind},
    live::NumberField,
    live_client::{Action, LiveClient},
    project::{EffectKind, Layer, LayerEffect, Workspace},
};
use dioxus_native::prelude::*;

fn edit(mut project: Signal<Workspace>, scene: u32, index: usize, change: impl FnOnce(&mut Layer)) {
    if let Some(scene) = project.write().scenes.iter_mut().find(|s| s.id == scene)
        && let Some(layer) = scene.layers.get_mut(index)
    {
        change(layer);
    }
}

#[component]
pub fn LayerEditor(
    mut project: Signal<Workspace>,
    scene_id: u32,
    mut selection: Signal<Option<usize>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|s| s.id == scene_id) else {
        return rsx! {};
    };
    let selected = selection().and_then(|i| scene.layers.get(i).map(|l| (i, l.clone())));

    rsx! {
        div {class:"panel-heading",h3 {"Layers"}span {class:"form-help","Top of list = front"}}
        div {class:"layer-stack",for (index,layer) in scene.layers.iter().enumerate().rev() {
            div {class:if selection()==Some(index){"layer-stack-row active"}else{"layer-stack-row"},
                button {class:"layer-select","aria-pressed":(selection()==Some(index)).to_string(),onclick:move |_|selection.set(Some(index)),
                    "{index + 1} · "
                    {{workspace.sources.iter().find(|s|s.id==layer.source_id).map(|s|s.name.clone()).unwrap_or_else(||"Missing source".into())}}
                }
                button {disabled:index+1==scene.layers.len() || (client.busy)(),title:"Bring forward",onclick:move |_| {
                    let changed = project.write().move_layer(scene_id,index,index+1); if changed {selection.set(Some(index+1));if client.running(){client.command(Action::Apply);}}
                },"Forward"}
                button {disabled:index==0 || (client.busy)(),title:"Send backward",onclick:move |_| {
                    let changed = index>0 && project.write().move_layer(scene_id,index,index-1); if changed {selection.set(Some(index-1));if client.running(){client.command(Action::Apply);}}
                },"Backward"}
            }
        }if scene.layers.is_empty(){p {class:"padded form-help","Add a source above to create a layer."}}}
        if let Some((index,layer))=selected {
            div {key:"{scene_id}-{index}",class:"layer-inspector",
                div {class:"panel-heading",h3 {"Selected layer · {index + 1}"}button {onclick:move |_| {
                    if let Some(scene)=project.write().scenes.iter_mut().find(|s|s.id==scene_id) && index<scene.layers.len(){scene.layers.remove(index);}
                    selection.set(None);
                },"Remove layer"}}
                div {class:"panel-heading",h3 {"Layer CSS"}}
                p {class:"inspector-help form-help","One stylesheet controls position, size, crop and appearance. Target .layer (or :layer). Crop uses --crop-left/top/right/bottom in source px or %. Use object-fit and object-position to place the image inside."}
                div {class:"row padded",for fit in ["fill","contain","cover","none","scale-down"] {button {onclick:move |_|edit(project,scene_id,index,|l| {
                    let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                    l.css=acheflow_media_api::appearance::managed_css(&css,"fit",&format!(".layer {{object-fit:{fit};}}"));
                }),"{fit}"}}
                    button {onclick:move |_|edit(project,scene_id,index,|l| {
                        let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                        l.css=acheflow_media_api::appearance::managed_css(&css,"frame",".layer {border:4px solid #ff9944;border-radius:24px;}");
                    }),"+ Border"}
                    button {onclick:move |_|edit(project,scene_id,index,|l| {
                        let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                        l.css=acheflow_media_api::appearance::managed_css(&css,"animation","@keyframes layer_pulse {from {opacity:0.45;} to {opacity:1;}} .layer {animation:layer_pulse 2s ease-in-out infinite alternate;}");
                    }),"+ Animation"}
                }
                CodeEditor {label:"Layer CSS",kind:CodeKind::Css,value:if layer.css.is_empty(){acheflow_media_api::appearance::default_layer_css(&layer)}else{layer.css.clone()},change:move |v|edit(project,scene_id,index,|l|l.css=v)}
                div {class:"panel-heading",h3 {"Layer effects"}span {class:"badge","{layer.effects.len()} / 8"}}
                p {class:"inspector-help form-help","GPU effects run from top to bottom after cropping. CSS decorates the processed layer. Audio is unchanged."}
                div {class:"row padded effect-add",for kind in EffectKind::ALL {button {disabled:layer.effects.len()>=8,onclick:move |_|edit(project,scene_id,index,|l|{if l.effects.len()<8{l.effects.push(LayerEffect::new(kind));}}),"+ {kind.label()}"}}}
                for (effect_index,effect) in layer.effects.iter().enumerate() {
                    div {class:"effect-row",
                        div {class:"row",strong {"{effect.kind.label()}"}
                            button {"aria-pressed":effect.enabled.to_string(),onclick:move |_|edit(project,scene_id,index,|l|l.effects[effect_index].enabled = !l.effects[effect_index].enabled),if effect.enabled {"Enabled"}else{"Disabled"}}
                            button {disabled:effect_index==0,"aria-label":"Move effect earlier",onclick:move |_|edit(project,scene_id,index,|l|l.effects.swap(effect_index,effect_index-1)),"↑"}
                            button {disabled:effect_index+1==layer.effects.len(),"aria-label":"Move effect later",onclick:move |_|edit(project,scene_id,index,|l|l.effects.swap(effect_index,effect_index+1)),"↓"}
                            button {onclick:move |_|edit(project,scene_id,index,|l|{l.effects.remove(effect_index);}),"Remove"}
                        }
                        NumberField {label:effect.kind.amount_label(),value:effect.amount,max:effect.kind.max(),change:move |v|edit(project,scene_id,index,|l|l.effects[effect_index].amount=v)}
                        if effect.kind==EffectKind::Wgsl {
                            p {class:"form-help","fn effect(uv, color) → vec4f · sample_source(uv) · params.time (seconds), params.strength, params.size. Bounded fragment functions; no loops or custom bindings."}
                            CodeEditor {label:"WGSL shader",kind:CodeKind::Wgsl,value:effect.shader.clone(),change:move |v|edit(project,scene_id,index,|l|l.effects[effect_index].shader=v)}
                        }

                    }
                }
                div {class:"row padded",button {class:"primary",disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene & sources"}span {class:"form-help","Crop, size and effects are saved locally. Apply to update the live composition."}}
            }
        }else if !scene.layers.is_empty(){p {class:"padded form-help","Select a layer on the canvas or in this list to resize, crop and add effects."}}
    }
}
