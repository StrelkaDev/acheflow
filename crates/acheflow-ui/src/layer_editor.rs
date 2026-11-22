use crate::{
    code_editor::{CodeEditor, CodeKind},
    live_client::{Action, LiveClient},
    project::{EffectKind, Layer, LayerEffect, Workspace},
    scene_canvas::SceneHistory,
    slider::Slider,
};
use acheflow_media_api::layer::ChromaKey;
use dioxus_native::prelude::*;

#[derive(Clone, Copy, PartialEq)]
struct LayerTarget {
    scene: u32,
    viewport: Option<u64>,
}
impl LayerTarget {
    fn layers<'a>(
        &self,
        scene: &'a acheflow_media_api::workspace::Scene,
    ) -> Option<&'a Vec<Layer>> {
        match self.viewport {
            None => Some(&scene.layers),
            Some(id) => scene
                .spatial
                .as_ref()?
                .objects
                .iter()
                .find(|o| o.id == id)
                .and_then(|o| match &o.kind {
                    acheflow_media_api::spatial::ObjectKind::Viewport {
                        primary: false,
                        layers,
                        ..
                    } => Some(layers),
                    _ => None,
                }),
        }
    }
    fn layers_mut<'a>(
        &self,
        scene: &'a mut acheflow_media_api::workspace::Scene,
    ) -> Option<&'a mut Vec<Layer>> {
        match self.viewport {
            None => Some(&mut scene.layers),
            Some(id) => scene
                .spatial
                .as_mut()?
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .and_then(|o| match &mut o.kind {
                    acheflow_media_api::spatial::ObjectKind::Viewport {
                        primary: false,
                        layers,
                        ..
                    } => Some(layers),
                    _ => None,
                }),
        }
    }
}
fn edit(
    mut project: Signal<Workspace>,
    target: LayerTarget,
    layer_id: u64,
    change: impl FnOnce(&mut Layer),
) {
    if let Some(scene) = project
        .write()
        .scenes
        .iter_mut()
        .find(|s| s.id == target.scene)
        && let Some(layer) = target
            .layers_mut(scene)
            .and_then(|layers| layers.iter_mut().find(|l| l.id == layer_id))
    {
        change(layer);
    }
}

#[component]
pub(crate) fn LayerEditor(
    mut project: Signal<Workspace>,
    scene_id: u32,
    mut selection: Signal<Option<u64>>,
    #[props(default)] viewport_id: Option<u64>,
    #[props(default)] history: Option<Signal<SceneHistory>>,
) -> Element {
    let target = LayerTarget {
        scene: scene_id,
        viewport: viewport_id,
    };
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|s| s.id == scene_id) else {
        return rsx! {};
    };
    let Some(layers) = target.layers(scene) else {
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
        div {class:"layer-stack",for (index,layer) in layers.iter().enumerate().rev() {
            {let layer_id = layer.id; rsx! {
            div {key:"{layer_id}",class:if selection()==Some(layer_id){"layer-stack-row active"}else{"layer-stack-row"},
                button {class:"layer-select","aria-pressed":(selection()==Some(layer_id)).to_string(),onclick:move |_|selection.set(Some(layer_id)),
                    "{index + 1} · "
                    {{workspace.sources.iter().find(|s|s.id==layer.source_id).map(|s|s.name.clone()).unwrap_or_else(||"Missing source".into())}}
                }
                button {disabled:index+1==layers.len() || (client.busy)(),title:"Bring forward",onclick:move |_| {
                    let changed = move_layer(project,target,layer_id,1,history); if changed && client.running(){client.command(Action::Apply);}
                },"Forward"}
                button {disabled:index==0 || (client.busy)(),title:"Send backward",onclick:move |_| {
                    let changed = move_layer(project,target,layer_id,-1,history); if changed && client.running(){client.command(Action::Apply);}
                },"Backward"}
            }
        }}}if layers.is_empty(){p {class:"padded form-help","Add a source above to create a layer."}}}
        for layer in layers.iter().filter(|l| visited.read().contains(&l.id)) {
            LayerInspector { key:"{layer.id}", project, target, layer_id:layer.id, selected:selection()==Some(layer.id), selection }
        }
        if selection().is_none() && !layers.is_empty(){p {class:"padded form-help","Select a layer on the canvas or in this list to resize, crop and add effects."}}
    }
}

fn move_layer(
    mut project: Signal<Workspace>,
    target: LayerTarget,
    id: u64,
    delta: isize,
    history: Option<Signal<SceneHistory>>,
) -> bool {
    let mut workspace = project.write();
    let Some(layers) = workspace
        .scenes
        .iter_mut()
        .find(|s| s.id == target.scene)
        .and_then(|s| target.layers_mut(s))
    else {
        return false;
    };
    let Some(from) = layers.iter().position(|l| l.id == id) else {
        return false;
    };
    let Some(to) = from
        .checked_add_signed(delta)
        .filter(|to| *to < layers.len())
    else {
        return false;
    };
    let before = [layers[from].id, layers[to].id];
    layers.swap(from, to);
    if let Some(mut history) = history {
        history
            .write()
            .record_swap(target.scene, [from, to], before);
    }
    true
}
fn edit_effect(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer: u64,
    id: u64,
    change: impl FnOnce(&mut LayerEffect),
) {
    edit(project, target, layer, |l| {
        if let Some(e) = l.effects.iter_mut().find(|e| e.id == id) {
            change(e);
        }
    });
}
fn move_effect(project: Signal<Workspace>, target: LayerTarget, layer: u64, id: u64, delta: isize) {
    edit(project, target, layer, |l| {
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
    target: LayerTarget,
    layer_id: u64,
    selected: bool,
    mut selection: Signal<Option<u64>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let Some((index, layer)) = workspace
        .scenes
        .iter()
        .find(|s| s.id == target.scene)
        .and_then(|s| target.layers(s))
        .and_then(|layers| layers.iter().enumerate().find(|(_, l)| l.id == layer_id))
    else {
        return rsx! {};
    };
    rsx! {
            div {class:"layer-inspector",style:if selected {"display:block"}else{"display:none"},
                div {class:"panel-heading",h3 {"Selected layer · {index + 1}"}button {onclick:move |_| {
                    if let Some(layers)=project.write().scenes.iter_mut().find(|s|s.id==target.scene).and_then(|s|target.layers_mut(s)){layers.retain(|l|l.id!=layer_id);}
                    selection.set(None);
                },"Remove layer"}}
                if workspace.sources.iter().any(|s|s.id==layer.source_id && matches!(s.kind,acheflow_media_api::visual::SourceKind::Chat(_))){crate::visual_sources::ChatSettings{project,source_id:layer.source_id}}
                div {class:"panel-heading",h3 {"Layer CSS"}}
                p {class:"inspector-help form-help","One stylesheet controls position, size, crop and appearance. Target .layer (or :layer). Crop uses --crop-left/top/right/bottom in source px or %. Use object-fit and object-position to place the image inside."}
                div {class:"row padded",for fit in ["fill","contain","cover","none","scale-down"] {button {onclick:move |_|edit(project,target,layer_id,|l| {
                    let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                    l.css=acheflow_media_api::appearance::managed_css(&css,"fit",&format!(".layer {{object-fit:{fit};}}"));
                }),"{fit}"}}
                    button {onclick:move |_|edit(project,target,layer_id,|l| {
                        let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                        l.css=acheflow_media_api::appearance::managed_css(&css,"frame",".layer {border:4px solid #ff9944;border-radius:24px;}");
                    }),"+ Border"}
                    button {onclick:move |_|edit(project,target,layer_id,|l| {
                        let css=if l.css.is_empty(){acheflow_media_api::appearance::default_layer_css(l)}else{l.css.clone()};
                        l.css=acheflow_media_api::appearance::managed_css(&css,"animation","@keyframes layer_pulse {from {opacity:0.45;} to {opacity:1;}} .layer {animation:layer_pulse 2s ease-in-out infinite alternate;}");
                    }),"+ Animation"}
                }
                CodeEditor {label:"Layer CSS",kind:CodeKind::Css,value:if layer.css.is_empty(){acheflow_media_api::appearance::default_layer_css(layer)}else{layer.css.clone()},change:move |v|edit(project,target,layer_id,|l|l.css=v)}
                div {class:"panel-heading",h3 {"Layer effects"}span {class:"badge","{layer.effects.len()} / 8"}}
                p {class:"inspector-help form-help","GPU effects run from top to bottom after cropping. CSS decorates the processed layer. Audio is unchanged."}
                div {class:"row padded effect-add",for kind in EffectKind::ALL {button {disabled:layer.effects.len()>=8,onclick:move |_|edit(project,target,layer_id,|l|{if l.effects.len()<8{l.effects.push(LayerEffect::new(kind));}}),"+ {kind.label()}"}}}
                for (effect_index,entry) in layer.effects.iter().enumerate() {
                    {let effect_id=entry.id; let effect=entry; rsx! {
                    div {key:"{effect_id}",class:"effect-row",
                        div {class:"row",strong {"{effect.kind.label()}"}
                            button {"aria-pressed":effect.enabled.to_string(),onclick:move |_|edit_effect(project,target,layer_id,effect_id,|e|e.enabled = !e.enabled),if effect.enabled {"Enabled"}else{"Disabled"}}
                            button {disabled:effect_index==0,"aria-label":"Move effect earlier",onclick:move |_|move_effect(project,target,layer_id,effect_id,-1),"↑"}
                            button {disabled:effect_index+1==layer.effects.len(),"aria-label":"Move effect later",onclick:move |_|move_effect(project,target,layer_id,effect_id,1),"↓"}
                            button {onclick:move |_|edit(project,target,layer_id,|l|{l.effects.retain(|e| e.id!=effect_id);}),"Remove"}
                        }
                        Slider {label:effect.kind.amount_label(),unit:if effect.kind==EffectKind::Blur {"px"}else{"%"},value:effect.amount as i32,min:0,max:effect.kind.max() as i32,reset:Some(effect.kind.default_amount() as i32),change:move |v:i32|edit_effect(project,target,layer_id,effect_id,|e|e.amount=v as u32)}
                        if effect.kind==EffectKind::Wgsl {
                            p {class:"form-help","fn effect(uv, color) → vec4f · sample_source(uv) · params.time (seconds), params.strength, params.size. Bounded fragment functions; no loops or custom bindings."}
                            CodeEditor {label:"WGSL shader",kind:CodeKind::Wgsl,value:effect.shader.clone(),change:move |v|edit_effect(project,target,layer_id,effect_id,|e|e.shader=v)}
                        }
                        if effect.kind==EffectKind::ChromaKey {
                            ChromaKeyControls {project,target,layer_id,effect_id,key_settings:effect.chroma_key.unwrap_or_default()}
                        }

                    }
                }}}
                div {class:"row padded",button {class:"primary",disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene & sources"}span {class:"form-help","Crop, size and effects are saved locally. Apply to update the live composition."}}
            }

    }
}

#[component]
fn ChromaKeyControls(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer_id: u64,
    effect_id: u64,
    key_settings: ChromaKey,
) -> Element {
    let defaults = ChromaKey::default();
    let mut color = use_signal(|| color_hex(key_settings.color));
    use_effect(use_reactive((&key_settings.color,), move |(current,)| {
        if parse_key_color(&color.peek()) != Some(current) {
            color.set(color_hex(current));
        }
    }));
    let valid = parse_key_color(&color.read()).is_some();
    rsx! {
        div {class:"chroma-key-controls",
            crate::live::Field {label:"Key color (hex)",value:color(),change:move |value:String| {
                let parsed=parse_key_color(&value);
                color.set(value);
                if let Some(rgb)=parsed {
                    edit_key(project,target,layer_id,effect_id,|key| key.color=rgb);
                }
            }}
            if !valid {p {class:"field-error","Enter a color such as #0DBF14."}}
            Slider {label:"Tolerance",unit:"%",value:key_settings.tolerance as i32,min:0,max:100,reset:Some(defaults.tolerance as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,|key|key.tolerance=v as u8)}
            Slider {label:"Softness",unit:"%",value:key_settings.softness as i32,min:1,max:100,reset:Some(defaults.softness as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,|key|key.softness=v as u8)}
            Slider {label:"Edge smoothing",unit:" px",value:key_settings.edge as i32,min:0,max:ChromaKey::MAX_EDGE as i32,reset:Some(defaults.edge as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,|key|key.edge=v as u8)}
            Slider {label:"Despill",unit:"%",value:key_settings.despill as i32,min:0,max:100,reset:Some(defaults.despill as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,|key|key.despill=v as u8)}
        }
    }
}

fn edit_key(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer_id: u64,
    effect_id: u64,
    change: impl FnOnce(&mut ChromaKey),
) {
    edit_effect(project, target, layer_id, effect_id, |effect| {
        change(effect.chroma_key.get_or_insert_with(ChromaKey::default));
    });
}

fn color_hex([red, green, blue]: [u8; 3]) -> String {
    format!("#{red:02X}{green:02X}{blue:02X}")
}

fn parse_key_color(input: &str) -> Option<[u8; 3]> {
    let input = input.trim();
    let input = input.strip_prefix('#').unwrap_or(input);
    if input.len() != 6 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&input[..2], 16).ok()?,
        u8::from_str_radix(&input[2..4], 16).ok()?,
        u8::from_str_radix(&input[4..], 16).ok()?,
    ])
}

#[cfg(test)]
mod viewport_target_tests {
    use super::*;
    #[test]
    fn viewport_edits_do_not_touch_primary_or_other_viewports_even_with_shared_layer_ids() {
        use acheflow_media_api::spatial::{ObjectKind, SceneObject, Transform};
        let mut workspace = Workspace::default();
        workspace.add_source();
        let scene_id = workspace.add_scene(crate::project::Orientation::Horizontal);
        workspace.add_layer(scene_id, workspace.sources[0].id);
        workspace.migrate_spatial();
        let scene = &mut workspace.scenes[0];
        let original = scene.layers.clone();
        let spatial = scene.spatial.as_mut().unwrap();
        for id in [2, 3] {
            spatial.objects.push(SceneObject {
                id,
                name: format!("Viewport {id}"),
                visible: true,
                transform: Transform::default(),
                kind: ObjectKind::Viewport {
                    primary: false,
                    canvas: workspace.horizontal.base.clone(),
                    layers: original.clone(),
                },
            });
        }
        let target = LayerTarget {
            scene: scene_id,
            viewport: Some(2),
        };
        target.layers_mut(scene).unwrap()[0].css = ".layer {opacity:0.5}".into();
        assert_eq!(scene.layers, original);
        assert_eq!(
            *LayerTarget {
                scene: scene_id,
                viewport: Some(3)
            }
            .layers(scene)
            .unwrap(),
            original
        );
        assert_ne!(*target.layers(scene).unwrap(), original);
        assert!(
            LayerTarget {
                scene: scene_id,
                viewport: Some(999)
            }
            .layers_mut(scene)
            .is_none()
        );
    }
}
