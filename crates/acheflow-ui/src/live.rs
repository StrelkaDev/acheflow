use crate::fps::FrameRateInput;
use crate::layer_editor::LayerEditor;
use crate::live_client::{Action, LiveClient, LiveNotice};
use crate::media::{EncoderPicker, FfmpegStatus, use_capabilities};
use crate::project::{Orientation, Output, Resolution, Workspace};
use crate::scene_canvas::Canvas;
use crate::video_preview::VideoPreview;
use acheflow_media_api::live::OutputState;
use acheflow_media_api::{MediaCapabilities, workspace::BitrateMode};
use dioxus_native::prelude::*;

#[component]
pub fn Live(project: Signal<Workspace>, visible: bool) -> Element {
    let mut tab = use_signal(|| "Studio");
    rsx! {div {class:"live-root",
        nav {class:"subnav",for name in ["Analytics","Sources","Scenes","Mixer","Studio","Interactive","Settings"] {
            button {class:if tab()==name {"selected"}else{""},onclick:move |_|tab.set(name),"{name}"}
        }}
        LiveNotice {}
        div {class:"live-pages",for name in ["Analytics","Sources","Scenes","Mixer","Studio","Interactive","Settings"] {
            div {key:"{name}",class:if visible && tab()==name {"tab-scroll active"}else{"tab-scroll"},"aria-hidden":if visible && tab()==name {"false"}else{"true"},
                div {class:"live-content",
                    if name=="Analytics" {Analytics {}}
                    if name=="Sources" {Sources {project}}
                    if name=="Scenes" {Scenes {project,visible:visible && tab()==name}}
                    if name=="Mixer" {crate::audio_mixer::AudioMixer {project}}
                    if name=="Studio" {Studio {project,visible:visible && tab()==name}}
                    if name=="Interactive" {crate::interaction::InteractivePage {}}
                    if name=="Settings" {Settings {project}}
                }
            }
        }}
    }}
}
#[component]
fn Analytics() -> Element {
    let client = use_context::<LiveClient>();
    let state = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    let sources = state
        .as_ref()
        .map(|s| s.sources.iter().filter(|s| s.connected).count())
        .unwrap_or(0);
    let rendered = state
        .as_ref()
        .map(|s| s.formats.iter().map(|f| f.frames).sum::<u64>())
        .unwrap_or(0);
    let outputs = state
        .as_ref()
        .map(|s| {
            s.formats
                .iter()
                .flat_map(|f| &f.outputs)
                .filter(|o| o.state == OutputState::Streaming)
                .count()
        })
        .unwrap_or(0);
    let encoded = state
        .as_ref()
        .map(|s| {
            s.formats
                .iter()
                .flat_map(|f| &f.outputs)
                .map(|o| o.frames)
                .sum::<u64>()
        })
        .unwrap_or(0);
    rsx! {
        crate::interaction::Notice {}
        crate::interaction::Audience {}
        div {class:"page-heading",div {h1 {"Analytics"}p {"Server media worker counters"}}span {class:"badge",if client.running(){"Running"}else{"Idle"}}}
        div {class:"stats",for (label,value) in [("CONNECTED SOURCES",sources.to_string()),("RENDERED FRAMES",rendered.to_string()),("STREAMING OUTPUTS",outputs.to_string()),("ENCODED FRAMES",encoded.to_string())] {
            div {class:"stat",span {class:"stat-label","{label}"}strong {"{value}"}span {class:"stat-detail","Reported by server"}}
        }}
        crate::traffic::TrafficCharts {}
    }
}
#[component]
pub fn Sources(mut project: Signal<Workspace>) -> Element {
    let mut copied = use_signal(|| None::<u32>);
    let client = use_context::<LiveClient>();
    let state = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    rsx! {
        div {class:"page-heading",div {h1 {"Sources"}p {"Incoming RTMP publishers connect with their source token."}}button {class:"primary",disabled:client.status.read().is_none(),onclick:move |_|client.add_source(),"+ RTMP source"}}
        div {class:"source-actions row",
            button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply sources / listen"}
            button {disabled:(client.busy)() || !client.owns(),onclick:move |_|client.command(Action::Deactivate),"Deactivate sources"}
            span {class:"form-help",if let Some(state)=client.status.read().as_ref(){"RTMP listener: {state.rtmp_address}"}}
        }
        p {class:"form-help source-help","Use the ingest URL as the server and the connection token as the RTMP stream key in OBS/FFmpeg. Apply changes after editing sources."}
        if project.read().sources.is_empty(){div {class:"panel empty-state",h3 {"No expected sources"}p {"Add a source, then apply it to authorize its token on the server."}}}
        for source in project.read().sources.clone() {
            section {key:"{source.id}",class:"panel",
                div {class:"panel-heading",h2 {"{source.name}"}
                    span {class:"badge",if state.as_ref().and_then(|s|s.sources.iter().find(|i|i.id==source.id)).is_some_and(|s|s.connected){"Connected"}else{"Waiting for token"}}
                    button {onclick:move |_|project.write().remove_source(source.id),"Remove"}
                }
                div {class:"form-grid",
                    Field {label:"Name",value:source.name,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.name=v;}}}
                    Field {label:"RTMP ingest URL",value:source.url,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.url=v;}}}
                    Field {label:"Connection token / RTMP key",secret:true,value:source.key,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.key=v;}}}
                    div {class:"field",span {"Token"}button {onclick:move |_|{let key=project.peek().sources.iter().find(|s|s.id==source.id).map(|s|s.key.clone());if let Some(key)=key {spawn(async move {if client.copy_text(key).await {copied.set(Some(source.id));}});}},if copied()==Some(source.id){"Copied token"}else{"Copy token"}}button {onclick:move |_|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.key=crate::project::new_token();copied.set(None);}},"Generate new token"}span {class:"form-help","Apply to revoke the old token and reconnect this source."}}
                    NumberField {label:"Offline preview width",value:source.resolution.width,min:16,max:((1920*1080/source.resolution.height.max(1)).min(3840)/2)*2,step:2,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.resolution.width=v;}}}
                    NumberField {label:"Offline preview height",value:source.resolution.height,min:16,max:((1920*1080/source.resolution.width.max(1)).min(3840)/2)*2,step:2,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.resolution.height=v;}}}
                }
                if let Some(input)=state.as_ref().and_then(|s|s.sources.iter().find(|i|i.id==source.id)) {
                    div {class:"source-footer",
                        span {"Decoded frames: {input.frames}"}
                        span {"Incoming: {input.width.unwrap_or(0)} × {input.height.unwrap_or(0)} / {input.fps.unwrap_or(0.):.1} FPS"}
                        span {if input.audio{"Audio received"}else{"No audio received"}}
                        for track in &input.audio_tracks {span {"Track {track.track_id}: {track.codec} / {track.sample_rate} Hz / {track.channels} ch · ",if track.active {"receiving"}else{"waiting"}}}
                    }
                    if let Some(error)=input.error.as_ref(){p {class:"field-error padded","{error}"}}
                }
                div {class:"panel-note","Incoming video keeps its original resolution. Resize and fit it in the scene layer. Offline preview dimensions only size the test pattern while no video is available."}
            }
        }
    }
}
#[component]
fn Settings(mut project: Signal<Workspace>) -> Element {
    let (capabilities, refresh) = use_capabilities(project);
    let snapshot = capabilities.read().clone();
    let available = snapshot.as_ref().and_then(|r| r.as_ref().ok()).cloned();
    rsx! {
        div {class:"page-heading",div {h1 {"Stream settings"}p {"Separate canvases and RTMP outputs for each format."}}}
        FfmpegStatus {capabilities,refresh}
        for orientation in Orientation::ALL {StreamSettings {key:"{orientation.label()}",project,orientation,capabilities:available.clone()}}
    }
}
#[component]
fn StreamSettings(
    mut project: Signal<Workspace>,
    orientation: Orientation,
    capabilities: Option<MediaCapabilities>,
) -> Element {
    let stream = project.read().stream(orientation).clone();
    let mut expanded = use_signal(|| None::<usize>);
    let sizes = if orientation == Orientation::Horizontal {
        ["1280x720", "1920x1080", "2560x1440", "3840x2160"]
    } else {
        ["720x1280", "1080x1920", "1440x2560", "2160x3840"]
    };
    rsx! {section {class:"panel",
        div {class:"panel-heading",h2 {"{orientation.label()}"}span {class:"badge","RTMP / FFmpeg"}}
        div {class:"form-grid",
            Choice {label:"Base canvas",value:format!("{}x{}",stream.base.width,stream.base.height),choices:sizes.into_iter().map(String::from).collect(),change:move |v:String|{if let Some(r)=resolution(&v){project.write().resize_base(orientation,r);}}}
            NumberField {label:"Base width",value:stream.base.width,change:move |v:u32|{if (16..=7680).contains(&v) && v.is_multiple_of(2){let mut r=project.read().stream(orientation).base.clone();r.width=v;project.write().resize_base(orientation,r);}}}
            NumberField {label:"Base height",value:stream.base.height,change:move |v:u32|{if (16..=7680).contains(&v) && v.is_multiple_of(2){let mut r=project.read().stream(orientation).base.clone();r.height=v;project.write().resize_base(orientation,r);}}}
            FrameRateInput {value:stream.fps,change:move |v|project.write().stream_mut(orientation).fps=v}
        }
        div {class:"panel-heading",h2 {"Restream outputs"}button {class:"primary",onclick:move |_|{let size=project.read().stream(orientation).base.clone();project.write().stream_mut(orientation).outputs.push(Output{name:"New destination".into(),url:String::new(),key:String::new(),resolution:size,quality:"Standard".into(),codec:"h264".into(),encoder:None,bitrate_kbps:4500, bitrate_mode: Default::default(),enabled:true,audio:Default::default()});expanded.set(Some(project.peek().stream(orientation).outputs.len()-1));},"+ Output"}}
        for (index,out) in stream.outputs.iter().enumerate() {
            div {key:"{index}",class:"output-config",
                div {class:"panel-heading destination-heading",
                    button {class:"destination-toggle","aria-expanded":(expanded()==Some(index)).to_string(),onclick:move |_|expanded.set(if expanded()==Some(index){None}else{Some(index)}),
                        span {if expanded()==Some(index){"▾"}else{"▸"}} strong {"{out.name}"}
                    }
                    span {class:"destination-summary","{out.resolution.width} × {out.resolution.height} · {out.bitrate_mode.label()} · {out.bitrate_kbps} kbps"}
                    span {class:"badge",if out.enabled {"Enabled"}else{"Disabled"}}
                    button {onclick:move |_|{project.write().stream_mut(orientation).outputs.remove(index);expanded.set(expanded().and_then(|selected|if selected==index{None}else{Some(if selected>index{selected-1}else{selected})}));},"Remove"}
                }
                if expanded()==Some(index) {div {class:"form-grid",
                    Field {label:"Destination",value:out.name.clone(),change:move |v|project.write().stream_mut(orientation).outputs[index].name=v}
                    Field {label:"RTMP URL",value:out.url.clone(),change:move |v|project.write().stream_mut(orientation).outputs[index].url=v}
                    Field {label:"Stream key",secret:true,value:out.key.clone(),change:move |v|project.write().stream_mut(orientation).outputs[index].key=v}
                    NumberField {label:"Output width",value:out.resolution.width,change:move |v|project.write().stream_mut(orientation).outputs[index].resolution.width=v}
                    NumberField {label:"Output height",value:out.resolution.height,change:move |v|project.write().stream_mut(orientation).outputs[index].resolution.height=v}
                    Choice {label:"Quality",value:out.quality.clone(),choices:["Low","Standard","High","Custom"].into_iter().map(String::from).collect(),change:move |q:String|{let mut p=project.write();let out=&mut p.stream_mut(orientation).outputs[index];out.bitrate_kbps=match q.as_str(){"Low"=>2500,"High"=>8000,"Standard"=>4500,_=>out.bitrate_kbps};out.quality=q;}}
                    Choice {label:"Bitrate mode",value:out.bitrate_mode.label(),choices:vec!["CBR".into(),"VBR".into()],change:move |v:String|project.write().stream_mut(orientation).outputs[index].bitrate_mode=if v=="CBR"{BitrateMode::Cbr}else{BitrateMode::Vbr}}
                    NumberField {label:"Video bitrate / limit (kbps)",value:out.bitrate_kbps,min:100,max:100000,change:move |v|{let mut p=project.write();let out=&mut p.stream_mut(orientation).outputs[index];out.bitrate_kbps=v;out.quality="Custom".into();}}
                    EncoderPicker {project,orientation,index,capabilities:capabilities.clone()}
                    Choice {label:"Output",value:if out.enabled {"Enabled"}else{"Disabled"},choices:vec!["Enabled".into(),"Disabled".into()],change:move |v:String|project.write().stream_mut(orientation).outputs[index].enabled=v=="Enabled"}
                }
                p {class:"form-help padded","CBR keeps a fixed video rate. VBR varies with image complexity up to the configured limit. Audio uses the scene mixer."}
                }
            }
        }
        if stream.outputs.is_empty(){div {class:"empty-state",p {"No RTMP outputs configured for this format."}}}
        div {class:"panel-note","Changing the base canvas resizes matching scenes and scales their layers. Output dimensions are independent. Video encoders come from the server FFmpeg. Audio profile: AAC."}
    }}
}
#[component]
fn Scenes(mut project: Signal<Workspace>, visible: bool) -> Element {
    let client = use_context::<LiveClient>();
    let mut selected = use_signal(|| None::<u32>);
    let mut selected_layer = use_signal(|| None::<u64>);
    let current = project
        .read()
        .scenes
        .iter()
        .find(|s| Some(s.id) == selected())
        .cloned()
        .or_else(|| project.read().scenes.first().cloned());
    rsx! {
        div {class:"page-heading scene-page-heading",div {h1 {"Scenes"}}div {class:"row",for o in Orientation::ALL{button {onclick:move |_|{let id=project.write().add_scene(o);project.write().migrate_spatial();selected.set(Some(id));},"+ {o.label()} scene"}}}}
        div {class:"scene-list",for s in project.read().scenes.clone(){button {class:if current.as_ref().is_some_and(|c|c.id==s.id){"selected"}else{""},onclick:move |_|{selected.set(Some(s.id));selected_layer.set(None);},"{s.name} / {s.orientation.label()}"}}}
        if let Some(scene)=current {
            section {class:"panel scene-editor-panel",div {class:"scene-metadata",
                    Field {label:"Scene name",value:scene.name.clone(),change:move |v|{if let Some(s)=project.write().scenes.iter_mut().find(|s|s.id==scene.id){s.name=v;}}}
                    span {class:"form-help","{project.read().stream(scene.orientation).base.width} × {project.read().stream(scene.orientation).base.height} / {scene.orientation.label()}"}
                    button {onclick:move |_|project.write().remove_scene(scene.id),"Delete scene"}
                }
                crate::spatial_editor::SpatialEditor {key:"spatial-{project.read().id}-{scene.id}",project,scene_id:scene.id,
                div {class:"panel-heading",h3 {"Primary viewport content"}}
                Canvas {project,scene_id:Some(scene.id),orientation:scene.orientation,editable:true,live_images:visible,selection:Some(selected_layer)}
                div {class:"row padded",button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene & sources"}span {class:"form-help","Select a layer, drag to move, or drag its corners to resize. Releasing applies live changes. CSS layout, crop and effects are in the layer inspector."}}
                div {class:"panel-heading",h3 {"Add a source layer"}}
                div {class:"row padded",for source in project.read().sources.clone(){button {onclick:move |_|project.write().add_layer(scene.id,source.id),"+ {source.name}"}}if project.read().sources.is_empty(){p {"Register an RTMP source in Live / Sources first."}}}
                div {class:"panel-heading",h3 {"Primary viewport layers"}}
                LayerEditor {project,scene_id:scene.id,selection:selected_layer}
                }

            }
        }else{div {class:"panel empty-state",h3 {"Create your first scene"}p {"Horizontal and vertical scenes inherit their respective base canvas size."}}}
    }
}
#[component]
fn Studio(mut project: Signal<Workspace>, visible: bool) -> Element {
    let client = use_context::<LiveClient>();
    let status = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    let has_scene = Orientation::ALL
        .into_iter()
        .any(|o| project.read().active(o).is_some());
    rsx! {
        div {class:"page-heading",div {h1 {"Studio"}p {"Compose both formats and publish to your RTMP destinations."}}
            div {class:"row",
                button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene & sources"}
                if client.running(){button {class:"stop-button",disabled:(client.busy)(),onclick:move |_|client.command(Action::Stop),"Stop live"}}
                else {button {class:"primary",disabled:(client.busy)() || !has_scene,onclick:move |_|client.command(Action::Start),"Start live"}}
            }
        }
        p {class:"studio-help form-help","Start runs the selected scenes and all enabled outputs. With no outputs it runs preview only. Scene placement can change live; restart for FPS/resolution/encoding changes. Preview refreshes twice per second."}
        div {class:"studio-grid",for o in Orientation::ALL {
            section {class:"panel",div {class:"panel-heading",h2 {"{o.label()}"}
                span {class:"badge",if let Some(format)=status.as_ref().and_then(|s|s.formats.iter().find(|f|f.orientation==o)) {
                    if format.outputs.is_empty(){"Preview only"}else if format.outputs.iter().any(|out|out.state==OutputState::Streaming){"Broadcasting"}else if format.outputs.iter().any(|out|out.state==OutputState::Starting){"Starting"}else{"Output failed"}
                }else{"Offline"}}
            }
                div {class:"padded field",span {"Scene"}div {class:"row",
                    button {class:if project.read().active(o).is_none(){"selected"}else{""},onclick:move |_|project.write().set_active(o,None),"No scene"}
                    for scene in project.read().scenes.iter().filter(|s|s.orientation==o){button {class:if project.read().active(o)==Some(scene.id){"selected"}else{""},onclick:{let id=scene.id;move |_|project.write().set_active(o,Some(id))},"{scene.name}"}}
                }}
                if let Some(format)=status.as_ref().and_then(|s|s.formats.iter().find(|f|f.orientation==o)) {
                    div {class:"canvas-wrap",div {class:if o==Orientation::Vertical{"live-preview-frame portrait"}else{"live-preview-frame"},style:format!("aspect-ratio: {} / {};",project.read().stream(o).base.width,project.read().stream(o).base.height),if visible {VideoPreview {url:client.preview(&format!("/api/live/preview/{}.jpg",o.label().to_lowercase()))}}}}
                    div {class:"panel-note",SceneInputStatus {project,orientation:o}
                        if let Some(renderer)=format.renderer.as_ref(){p {"Renderer: {renderer}"}}
                        if let Some(error)=format.error.as_ref(){p {class:"field-error",role:"alert","{error}"}}
                        span {"Rendered frames: {format.frames} · audio is mixed into RTMP outputs"}
                    }

                    for out in &format.outputs {
                        div {class:"output-status",strong {"{out.name}"}span {class:"badge","{out.state:?}"}span {"{out.frames} frames · {out.bitrate_kbps:.0} kbps"}
                            button {disabled:(client.busy)(),onclick:{let index=out.index;move |_|client.command(Action::RestartOutput(o,index))},"Restart output"}
                            if let Some(encoder)=out.encoder.as_ref(){span {"{encoder} · ",if out.hardware {"Hardware"}else{"Software"}}}
                            if let Some(bus)=out.audio_bus {span {"Audio: scene {out.audio_scene.unwrap_or(0)} → bus {bus}"}crate::audio_mixer::Meters {meter:out.audio_meter.clone()}}
                            if let Some(error)=out.error.as_ref(){p {class:"field-error","{error}"}}
                        }
                    }
                }else {Canvas {project,scene_id:project.read().active(o),orientation:o,live_images:visible}div {class:"panel-note","Scene layout · start live to render the composition"}}
                crate::studio_mixer::StudioMixer {project,orientation:o}
            }
        }}
        if !client.running(){crate::interaction::ChatPanel {}}
    }
}
#[component]
fn SceneInputStatus(project: Signal<Workspace>, orientation: Orientation) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let scene = workspace
        .scenes
        .iter()
        .find(|s| Some(s.id) == workspace.active(orientation));
    let status = client.status.read();
    let inputs: Vec<_> = scene
        .into_iter()
        .flat_map(|s| &s.layers)
        .map(|layer| layer.source_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|id| {
            let name = workspace
                .sources
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Missing source".into());
            let connected = status
                .as_ref()
                .and_then(|s| s.sources.iter().find(|s| s.id == id))
                .is_some_and(|s| s.connected && s.frames > 0);
            (name, connected)
        })
        .collect();
    rsx! {div {class:"scene-input-status",
        if inputs.is_empty() {span {"No sources in the selected scene"}}
        for (name,connected) in inputs {span {class:if connected {"input-connected"}else{"input-waiting"},
            if connected {"{name}: receiving"}else{"{name}: waiting for RTMP · no video"}
        }}
    }}
}
#[component]
pub fn Field(
    label: &'static str,
    value: String,
    change: EventHandler<String>,
    #[props(default = false)] secret: bool,
) -> Element {
    let mut revealed = use_signal(|| false);
    rsx! {div {class:"field",
        label {"{label}",
            if !secret || revealed() {input {r#type:"text","aria-label":label,value:value.clone(),oninput:move |e|change.call(e.value())}}
            else {span {class:"masked-key",if value.is_empty(){"No key configured"}else{"••••••••"}}}
        }
        if secret {button {class:"key-toggle",onclick:move |_|revealed.set(!revealed()),if revealed(){"Hide key"}else{"Edit key"}}}
    }}
}
#[component]
pub(crate) fn NumberField(
    label: &'static str,
    value: u32,
    change: EventHandler<u32>,
    #[props(default = 0)] min: u32,
    #[props(default=u32::MAX)] max: u32,
    #[props(default = 1)] step: u32,
) -> Element {
    let mut text = use_signal(|| value.to_string());
    // Keep the native editor's value while typing. Reapplying each oninput
    // snapshot can overwrite later keystrokes already queued by Blitz.
    let mut displayed = use_signal(|| value.to_string());
    let mut reset = use_signal(|| 0u64);
    let mut error = use_signal(|| false);
    let mut applied = use_signal(|| value);
    use_effect(use_reactive((&value,), move |(value,)| {
        if *text.peek() == applied.peek().to_string() {
            text.set(value.to_string());
            displayed.set(value.to_string());
            error.set(false);
        }
        applied.set(value);
    }));
    let commit = move || {
        let number = valid_number(&text.peek(), min, max, step);
        match number {
            Some(number) => {
                text.set(number.to_string());
                displayed.set(number.to_string());
                change.call(number);
            }
            None => {
                text.set(value.to_string());
                displayed.set(value.to_string());
                error.set(true);
                reset += 1;
            }
        }
    };
    rsx! {div {class:"field",label {"{label}",input {key:"{reset}",r#type:"text",inputmode:"numeric","aria-label":label,value:{let _=displayed();text.peek().clone()},"aria-invalid":error().to_string(),
        oninput:move |e|{text.set(e.value());if *error.peek(){error.set(false);}},
        onblur:move |_| {let mut commit=commit;commit();},
        onkeydown:move |e| {
            if e.key()==Key::Named(NamedKey::Enter) {e.prevent_default();let mut commit=commit;commit();}
            if e.key()==Key::Named(NamedKey::Escape){text.set(value.to_string());displayed.set(value.to_string());error.set(false);reset += 1;}
        }
    }}if error(){span {class:"field-error",role:"alert","Use {min}–{max}, step {step}. Kept {value}."}}}}
}

fn valid_number(text: &str, min: u32, max: u32, step: u32) -> Option<u32> {
    let number = text.trim().parse::<u32>().ok()?;
    ((min..=max).contains(&number) && step > 0 && number.is_multiple_of(step)).then_some(number)
}

fn resolution(s: &str) -> Option<Resolution> {
    let (w, h) = s.split_once('x')?;
    Some(Resolution {
        width: w.parse().ok()?,
        height: h.parse().ok()?,
    })
}

#[component]
pub(crate) fn Choice(
    label: &'static str,
    value: String,
    choices: Vec<String>,
    #[props(default)] values: Option<Vec<String>>,
    change: EventHandler<String>,
) -> Element {
    let choices = choices
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let key = values
                .as_ref()
                .and_then(|v| v.get(i))
                .cloned()
                .unwrap_or_else(|| label.clone());
            (key, label)
        })
        .collect::<Vec<_>>();
    rsx! {div {class:"field",span {"{label}"}div {class:"row",role:"group","aria-label":label,
        for (key,label) in choices {button {class:if value==key {"selected"}else{""},"aria-pressed":(value==key).to_string(),onclick:{let key=key.clone();move |_|change.call(key.clone())},"{label}"}}
    }}}
}
