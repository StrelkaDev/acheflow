use crate::{
    code_editor::{CodeEditor, CodeKind},
    project::{EffectKind, Layer, LayerEffect, Workspace},
    scene_canvas::SceneHistory,
    slider::Slider,
};
use acheflow_media_api::layer::ChromaKey;
use dioxus_native::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct LayerTarget {
    pub scene: u32,
    pub viewport: Option<u64>,
}
impl LayerTarget {
    pub(crate) fn layers<'a>(
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
    pub(crate) fn layers_mut<'a>(
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
pub(crate) fn add_layer(
    mut project: Signal<Workspace>,
    target: LayerTarget,
    source: u32,
    mut history: Signal<SceneHistory>,
) -> Option<u64> {
    let mut workspace = project.write();
    let scene_index = workspace
        .scenes
        .iter()
        .position(|scene| scene.id == target.scene)?;
    target.layers(&workspace.scenes[scene_index])?;
    let original_len = workspace.scenes[scene_index].layers.len();
    workspace.add_layer(target.scene, source);
    let scene = &mut workspace.scenes[scene_index];
    if original_len == scene.layers.len() {
        return None;
    }
    if target.viewport.is_some() {
        let layer = scene.layers.pop()?;
        target.layers_mut(scene)?.push(layer);
    }
    let layers = target.layers(scene)?;
    let index = layers.len() - 1;
    history.write().record_presence(target, layers, index, true);
    Some(layers[index].id)
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
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|s| s.id == scene_id) else {
        return rsx! {};
    };
    let Some(layers) = target.layers(scene) else {
        return rsx! {};
    };
    let mut visited = use_signal(std::collections::HashSet::<u64>::new);
    use_effect(move || {
        if let Some(id) = selection() {
            let workspace = project.read();
            let exists = workspace
                .scenes
                .iter()
                .find(|scene| scene.id == target.scene)
                .and_then(|scene| target.layers(scene))
                .is_some_and(|layers| layers.iter().any(|layer| layer.id == id));
            if !exists {
                selection.set(None);
            } else if !visited.peek().contains(&id) {
                visited.write().insert(id);
            }
        }
    });

    rsx! {
        div {class:"panel-heading",h3 {"Layers"}span {class:"form-help","Top of list = front"}}
        div {class:"layer-stack",for (index,layer) in layers.iter().enumerate().rev() {
            {let layer_id = layer.id; rsx! {
            div {key:"{layer_id}",class:if selection()==Some(layer_id){"layer-stack-row active"}else{"layer-stack-row"},
                crate::server_settings::ActionButton {class:"layer-select",pressed:Some(selection()==Some(layer_id)),onclick:move |_|selection.set(Some(layer_id)),
                    "{index + 1} · "
                    {{workspace.sources.iter().find(|s|s.id==layer.source_id).map(|s|s.name.clone()).unwrap_or_else(||"Missing source".into())}}
                }
                crate::server_settings::ActionButton {disabled:index+1==layers.len(),aria_label:Some("Bring forward".into()),onclick:move |_| {
                    move_layer(project,target,layer_id,1,history);
                },"Forward"}
                crate::server_settings::ActionButton {disabled:index==0,aria_label:Some("Send backward".into()),onclick:move |_| {
                    move_layer(project,target,layer_id,-1,history);
                },"Backward"}
            }
        }}}if layers.is_empty(){p {class:"padded form-help","Add a source above to create a layer."}}}
        for layer in layers.iter().filter(|l| visited.read().contains(&l.id)) {
            LayerInspector { key:"{layer.id}", project, target, layer_id:layer.id, selected:selection()==Some(layer.id), selection, history }
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
) {
    let mut workspace = project.write();
    let Some(layers) = workspace
        .scenes
        .iter_mut()
        .find(|s| s.id == target.scene)
        .and_then(|s| target.layers_mut(s))
    else {
        return;
    };
    let Some(from) = layers.iter().position(|l| l.id == id) else {
        return;
    };
    let Some(to) = from
        .checked_add_signed(delta)
        .filter(|to| *to < layers.len())
    else {
        return;
    };
    let before = [layers[from].id, layers[to].id];
    layers.swap(from, to);
    if let Some(mut history) = history {
        history.write().record_swap(target, [from, to], before);
    }
}
fn edit_css(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer_id: u64,
    history: Option<Signal<SceneHistory>>,
    change: impl FnOnce(&Layer) -> String,
) {
    edit(project, target, layer_id, |layer| {
        let next = change(layer);
        if next == layer.css
            || (layer.css.is_empty()
                && next == acheflow_media_api::appearance::default_layer_css(layer))
        {
            return;
        }
        let before = std::mem::replace(&mut layer.css, next);
        if let Some(mut history) = history {
            history
                .write()
                .record_css(target, layer_id, before, layer.css.clone());
        }
    });
}

fn layer_css(layer: &Layer) -> String {
    if layer.css.is_empty() {
        acheflow_media_api::appearance::default_layer_css(layer)
    } else {
        layer.css.clone()
    }
}

fn edit_effects(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer_id: u64,
    history: Option<Signal<SceneHistory>>,
    control: Option<(u64, &'static str)>,
    change: impl FnOnce(&mut Vec<LayerEffect>),
) {
    edit(project, target, layer_id, |layer| {
        let before = history.map(|_| layer.effects.clone());
        change(&mut layer.effects);
        if let (Some(mut history), Some(before)) = (history, before)
            && before != layer.effects
        {
            history.write().record_effects(
                target,
                layer_id,
                before,
                layer.effects.clone(),
                control,
            );
        }
    });
}

fn edit_effect(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer: u64,
    id: u64,
    history: Option<Signal<SceneHistory>>,
    control: Option<&'static str>,
    change: impl FnOnce(&mut LayerEffect),
) {
    edit_effects(
        project,
        target,
        layer,
        history,
        control.map(|name| (id, name)),
        |effects| {
            if let Some(effect) = effects.iter_mut().find(|effect| effect.id == id) {
                change(effect);
            }
        },
    );
}

fn move_effect(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer: u64,
    id: u64,
    delta: isize,
    history: Option<Signal<SceneHistory>>,
) {
    edit_effects(project, target, layer, history, None, |effects| {
        if let Some(from) = effects.iter().position(|effect| effect.id == id)
            && let Some(to) = from.checked_add_signed(delta)
            && to < effects.len()
        {
            effects.swap(from, to);
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
    history: Option<Signal<SceneHistory>>,
) -> Element {
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
                    if let Some(layers)=project.write().scenes.iter_mut().find(|s|s.id==target.scene).and_then(|s|target.layers_mut(s))
                        && let Some(index)=layers.iter().position(|layer|layer.id==layer_id) {
                        if let Some(mut history)=history {history.write().record_presence(target,layers,index,false);}
                        layers.remove(index);
                        selection.set(None);
                    }
                },"Remove layer"}}
                if workspace.sources.iter().any(|s|s.id==layer.source_id && matches!(s.kind,acheflow_media_api::visual::SourceKind::Chat(_))){crate::visual_sources::ChatSettings{project,source_id:layer.source_id}}
                div {class:"panel-heading",h3 {"Layer CSS"}}
                p {class:"inspector-help form-help","One stylesheet controls position, size, crop and appearance. Target .layer (or :layer). Crop uses --crop-left/top/right/bottom in source px or %. Use object-fit and object-position to place the image inside."}
                div {class:"row padded",
                    for fit in ["fill","contain","cover","none","scale-down"] {
                        button {onclick:move |_|edit_css(project,target,layer_id,history,|layer|
                            acheflow_media_api::appearance::managed_css(&layer_css(layer),"fit",&format!(".layer {{object-fit:{fit};}}"))
                        ),"{fit}"}
                    }
                    button {onclick:move |_|edit_css(project,target,layer_id,history,|layer|
                        acheflow_media_api::appearance::managed_css(&layer_css(layer),"frame",".layer {border:4px solid #ff9944;border-radius:24px;}")
                    ),"+ Border"}
                    button {onclick:move |_|edit_css(project,target,layer_id,history,|layer|
                        acheflow_media_api::appearance::managed_css(&layer_css(layer),"animation","@keyframes layer_pulse {from {opacity:0.45;} to {opacity:1;}} .layer {animation:layer_pulse 2s ease-in-out infinite alternate;}")
                    ),"+ Animation"}
                }
                CodeEditor {label:"Layer CSS",kind:CodeKind::Css,value:layer_css(layer),change:move |value|edit_css(project,target,layer_id,history,|_|value)}
                div {class:"panel-heading",h3 {"Layer effects"}span {class:"badge","{layer.effects.len()} / 8"}}
                p {class:"inspector-help form-help","GPU effects run from top to bottom after cropping. CSS decorates the processed layer. Audio is unchanged."}
                div {class:"row padded effect-add",for kind in EffectKind::ALL {button {disabled:layer.effects.len()>=8,onclick:move |_|edit_effects(project,target,layer_id,history,None,|effects|{if effects.len()<8{effects.push(LayerEffect::new(kind));}}),"+ {kind.label()}"}}}
                for (effect_index,entry) in layer.effects.iter().enumerate() {
                    {let effect_id=entry.id; let effect=entry; rsx! {
                    div {key:"{effect_id}",class:"effect-row",
                        div {class:"row",strong {"{effect.kind.label()}"}
                            button {"aria-pressed":effect.enabled.to_string(),onclick:move |_|edit_effect(project,target,layer_id,effect_id,history,None,|e|e.enabled = !e.enabled),if effect.enabled {"Enabled"}else{"Disabled"}}
                            button {disabled:effect_index==0,"aria-label":"Move effect earlier",onclick:move |_|move_effect(project,target,layer_id,effect_id,-1,history),"↑"}
                            button {disabled:effect_index+1==layer.effects.len(),"aria-label":"Move effect later",onclick:move |_|move_effect(project,target,layer_id,effect_id,1,history),"↓"}
                            button {onclick:move |_|edit_effects(project,target,layer_id,history,None,|effects|effects.retain(|e| e.id!=effect_id)),"Remove"}
                        }
                        Slider {label:effect.kind.amount_label(),unit:if effect.kind==EffectKind::Blur {"px"}else{"%"},value:effect.amount as i32,min:0,max:effect.kind.max() as i32,reset:Some(effect.kind.default_amount() as i32),change:move |v:i32|edit_effect(project,target,layer_id,effect_id,history,Some("amount"),|e|e.amount=v as u32)}
                        if effect.kind==EffectKind::Wgsl {
                            p {class:"form-help","fn effect(uv, color) → vec4f · sample_source(uv) · params.time (seconds), params.strength, params.size. Bounded fragment functions; no loops or custom bindings."}
                            CodeEditor {label:"WGSL shader",kind:CodeKind::Wgsl,value:effect.shader.clone(),change:move |v|edit_effect(project,target,layer_id,effect_id,history,None,|e|e.shader=v)}
                        }
                        if effect.kind==EffectKind::ChromaKey {
                            ChromaKeyControls {project,target,layer_id,effect_id,history,key_settings:effect.chroma_key.unwrap_or_default()}
                        }

                    }
                }}}
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
    history: Option<Signal<SceneHistory>>,
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
                    edit_key(project,target,layer_id,effect_id,history,"color",|key| key.color=rgb);
                }
            }}
            if !valid {p {class:"field-error","Enter a color such as #0DBF14."}}
            Slider {label:"Tolerance",unit:"%",value:key_settings.tolerance as i32,min:0,max:100,reset:Some(defaults.tolerance as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,history,"tolerance",|key|key.tolerance=v as u8)}
            Slider {label:"Softness",unit:"%",value:key_settings.softness as i32,min:1,max:100,reset:Some(defaults.softness as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,history,"softness",|key|key.softness=v as u8)}
            Slider {label:"Edge smoothing",unit:" px",value:key_settings.edge as i32,min:0,max:ChromaKey::MAX_EDGE as i32,reset:Some(defaults.edge as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,history,"edge",|key|key.edge=v as u8)}
            Slider {label:"Despill",unit:"%",value:key_settings.despill as i32,min:0,max:100,reset:Some(defaults.despill as i32),change:move |v:i32|edit_key(project,target,layer_id,effect_id,history,"despill",|key|key.despill=v as u8)}
        }
    }
}

fn edit_key(
    project: Signal<Workspace>,
    target: LayerTarget,
    layer_id: u64,
    effect_id: u64,
    history: Option<Signal<SceneHistory>>,
    control: &'static str,
    change: impl FnOnce(&mut ChromaKey),
) {
    edit_effect(
        project,
        target,
        layer_id,
        effect_id,
        history,
        Some(control),
        |effect| {
            change(effect.chroma_key.get_or_insert_with(ChromaKey::default));
        },
    );
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
