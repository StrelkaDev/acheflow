use crate::{
    audio_mixer::Meters,
    live::{Choice, DeviceInventory, Field},
    project::Workspace,
};
use acheflow_media_api::{
    live::{LiveStatus, MicrophoneDevice, MicrophoneInventory},
    visual::{MicrophoneMode, SourceKind},
};
use dioxus_native::prelude::*;

type Inventory = DeviceInventory<MicrophoneInventory>;

#[component]
pub(crate) fn MicrophoneChoices(
    mut project: Signal<Workspace>,
    inventory: Inventory,
    mut refresh: Signal<u32>,
    on_added: EventHandler<()>,
) -> Element {
    let mut open = use_signal(|| false);
    let snapshot = inventory.read();
    let pending = snapshot.is_none();
    if snapshot
        .as_ref()
        .is_some_and(|result| result.as_ref().is_ok_and(|data| !data.supported))
    {
        return rsx! {};
    }
    let devices = snapshot
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|data| data.devices.as_slice())
        .unwrap_or_default();
    rsx! {
        div {class:"camera-choice",
            button {"aria-expanded":open().to_string(),onclick:move |_| {
                let opening=!open();
                open.set(opening);
                if opening && !pending {*refresh.write()+=1;}
            },
                strong{"Microphone"}span{"Local audio capture"}
            }
            if open() {
                div {class:"camera-device-list",
                    if pending {p {class:"form-help","Looking for server microphones…"}}
                    else if let Some(Err(error))=snapshot.as_ref() {p {class:"field-error","{error}"}}
                    else if devices.is_empty() {p {class:"form-help","No microphones detected on the server."}}
                    for (index, device) in devices.iter().enumerate() {
                        {
                            let added = assigned(&project.read(), &device.id, None);
                            let label = device_name(devices, index);
                            let device = device.clone();
                            rsx! {button {key:"{device.id}",disabled:pending || added,
                                onclick:move |_|{
                                    let mut workspace=project.write();
                                    if workspace.sources.len()>=8 || assigned(&workspace,&device.id,None) {return;}
                                    workspace.add_source();
                                    if let Some(source)=workspace.sources.last_mut() {
                                        source.name=device.name.clone();
                                        source.kind=SourceKind::Microphone{device_id:device.id.clone(),mode:None};
                                        source.key.clear();
                                        source.url.clear();
                                    }
                                    on_added.call(());
                                },
                                "{label}" if added {span {"Already added"}}
                            }}
                        }
                    }
                    button {disabled:pending,onclick:move |_|*refresh.write()+=1,"Refresh microphones"}
                }
            }
        }
    }
}

#[component]
pub(crate) fn MicrophoneSources(
    mut project: Signal<Workspace>,
    inventory: Inventory,
    mut refresh: Signal<u32>,
    status: Option<LiveStatus>,
) -> Element {
    let workspace = project.read();
    let snapshot = inventory.read();
    let devices = snapshot
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|data| data.devices.as_slice())
        .unwrap_or_default();
    let pending = snapshot.is_none();
    rsx! {
        for source in &workspace.sources {
            if let SourceKind::Microphone{device_id,mode}=&source.kind {
                {
                    let id=source.id;
                    let active=devices.iter().position(|device| device.id==*device_id);
                    let selected=active.map(|index|device_name(devices,index)).unwrap_or_else(||"Saved microphone unavailable".into());
                    let options=devices.iter().enumerate()
                        .filter(|(_,device)|!assigned(&workspace,&device.id,Some(id)))
                        .map(|(index,device)|(device_name(devices,index),device.id.clone())).collect::<Vec<_>>();
                    let input=status.as_ref().and_then(|status|status.sources.iter().find(|input|input.id==id));
                    let track=input.and_then(|input|input.audio_tracks.iter().find(|track|track.track_id==0));
                    rsx! {section {key:"microphone-{id}",class:"panel",
                        div {class:"panel-heading",
                            h2 {"{source.name}"}
                            span {class:"badge","Microphone"}
                            span {class:"badge",
                                if track.is_some_and(|track|track.active) {"Receiving"}
                                else if input.is_some_and(|input|input.error.is_some()) {"Source error"}
                                else if input.is_some() {"Waiting for input"}
                                else {"Apply to activate"}
                            }
                            button {onclick:move |_|project.write().remove_source(id),"Remove"}
                        }
                        div {class:"form-grid",
                            Field {label:"Name",value:source.name.clone(),change:move |value|{
                                if let Some(source)=project.write().sources.iter_mut().find(|source|source.id==id) {source.name=value;}
                            }}
                            if pending {
                                p {class:"form-help","Looking for server microphones…"}
                            } else if !options.is_empty() {
                                Choice {label:"Device",value:selected,choices:options.iter().map(|(name,_)|name.clone()).collect(),change:move |label:String|{
                                    let Some((_,device))=options.iter().find(|(name,_)|name==&label) else {return;};
                                    let mut workspace=project.write();
                                    if !assigned(&workspace,device,Some(id))
                                        && let Some(source)=workspace.sources.iter_mut().find(|source|source.id==id)
                                        && !matches!(&source.kind, SourceKind::Microphone { device_id, .. } if device_id==device)
                                    {
                                        source.kind=SourceKind::Microphone{device_id:device.clone(),mode:None};
                                    }
                                }}
                            } else {
                                p {class:"form-help","{selected}"}
                            }
                            if !pending {
                                CaptureQuality {
                                    project,source_id:id,device_id:device_id.clone(),selected:*mode,
                                    device_available:active.is_some(),
                                    modes:active.map(|index|devices[index].modes.clone()).unwrap_or_default(),
                                }
                            }
                            button {disabled:pending,onclick:move |_|*refresh.write()+=1,"Refresh microphones"}
                        }
                        if let Some(error)=input.and_then(|input|input.error.as_ref()){p {class:"field-error padded","{error}"}}
                        if let Some(track)=track {div {class:"padded",Meters {meter:track.meter.clone()}}}
                        p {class:"panel-note","Choose a scene in Live / Mixer and set this microphone’s send to Program or another bus."}
                    }}
                }
            }
        }
    }
}

#[component]
fn CaptureQuality(
    mut project: Signal<Workspace>,
    source_id: u32,
    device_id: String,
    selected: Option<MicrophoneMode>,
    device_available: bool,
    modes: Vec<MicrophoneMode>,
) -> Element {
    let unavailable = selected.is_some_and(|mode| !modes.contains(&mode));
    let auto_only = device_available && modes.is_empty();
    let value = selected.map(mode_label).unwrap_or_else(|| "Auto".into());
    let mut choices = vec!["Auto".to_owned()];
    choices.extend(modes.iter().copied().map(mode_label));
    rsx! {
        Choice {label:"Capture quality",value,choices,change:move |label:String| {
            let mode = if label == "Auto" {
                None
            } else {
                let Some(mode) = modes.iter().copied().find(|mode| mode_label(*mode)==label) else {
                    return;
                };
                Some(mode)
            };
            if let Some(source)=project.write().sources.iter_mut().find(|source|source.id==source_id)
                && let SourceKind::Microphone{device_id:current,mode:saved}=&mut source.kind
                && current==&device_id
            {
                *saved=mode;
            }
        }}
        if unavailable {p {class:"field-error","Saved capture quality is unavailable. Select Auto or another available mode."}}
        else if auto_only {p {class:"form-help","This microphone reports no selectable capture modes."}}
    }
}

fn mode_label(mode: MicrophoneMode) -> String {
    let channels = match mode.channels {
        1 => "Mono".to_owned(),
        2 => "Stereo".to_owned(),
        channels => format!("{channels} channels"),
    };
    format!(
        "{} kHz · {}-bit · {channels}",
        f64::from(mode.sample_rate) / 1000.,
        mode.bit_depth
    )
}

fn assigned(workspace: &Workspace, device: &str, except: Option<u32>) -> bool {
    workspace.sources.iter().any(|source| {
        Some(source.id) != except
            && matches!(&source.kind, SourceKind::Microphone { device_id, .. } if device_id == device)
    })
}

fn device_name(devices: &[MicrophoneDevice], index: usize) -> String {
    let name = &devices[index].name;
    if devices.iter().filter(|device| &device.name == name).count() > 1 {
        format!("{name} ({})", index + 1)
    } else {
        name.clone()
    }
}
