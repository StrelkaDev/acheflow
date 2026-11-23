use crate::fps::FrameRateInput;
use crate::layer_editor::LayerEditor;
use crate::live_client::{Action, LiveClient, LiveNotice};
use crate::media::{EncoderPicker, FfmpegStatus, use_capabilities};
use crate::project::{Orientation, Output, OutputProtocol, Resolution, Workspace};
use crate::scene_canvas::Canvas;
use crate::server_settings::ActionButton;
use crate::video_preview::VideoPreview;
use acheflow_media_api::live::{
    FormatSettingsEdit, MeterTarget, OutputAction, OutputProfile, OutputState, OutputTarget,
    SourceEdit,
};
use acheflow_media_api::{MediaCapabilities, workspace::BitrateMode};
use dioxus_native::prelude::*;

#[component]
pub fn Live(project: Signal<Workspace>, visible: bool, mut monitor_tab: Signal<bool>) -> Element {
    let client = use_context::<LiveClient>();
    let histories = use_scene_histories(project);
    use_context_provider(|| histories);
    let mut tab = use_signal(|| "Studio");
    let mut separate = use_signal(|| None::<u32>);
    use_context_provider(|| crate::source_settings::SeparateSourcePreview(separate));
    let showing_source = separate().is_some();
    use_effect(move || {
        let monitored = separate().is_some() || matches!(tab(), "Studio" | "Mixer" | "Sources");
        if *monitor_tab.peek() != monitored {
            monitor_tab.set(monitored);
        }
    });
    use_effect(use_reactive((&visible,), move |(visible,)| {
        if !visible && separate.peek().is_some() {
            separate.set(None);
        }
    }));
    let preview = use_signal(std::collections::HashSet::<u32>::new);
    use_context_provider(|| crate::visual_sources::ChatPreview(preview));
    rsx! {div {class:"live-root",
        nav {class:"subnav",for name in ["Analytics","Sources","Scenes","Mixer","Studio","Chat","Interactive","Settings"] {
            button {class:if tab()==name {"selected"}else{""},onclick:move |_|{separate.set(None);tab.set(name);},"{name}"}
        }}
        LiveNotice {}
        // Keep the mounted tabs in the layout tree when swapping views. Blitz can
        // retain stale layout children if we hide the tree while updating its contents.
        div {class:"live-pages",
            for name in ["Analytics","Sources","Scenes","Mixer","Studio","Chat","Interactive","Settings"] {
                div {key:"{name}",class:if visible && !showing_source && tab()==name {"tab-scroll active"}else{"tab-scroll"},"aria-hidden":if visible && !showing_source && tab()==name {"false"}else{"true"},
                    div {class:"live-content",
                        if name=="Analytics" {Analytics {}}
                        if name=="Sources" {Sources {key:"{project.read().id}-{project.read().server_url}",project,visible:visible && !showing_source && tab()==name}}
                        if name=="Scenes" {Scenes {key:"{project.read().id}-{project.read().server_url}",project,visible:visible && !showing_source && tab()==name}}
                        if name=="Mixer" {crate::audio_mixer::AudioMixer {project,visible:visible && !showing_source && tab()==name}}
                        if name=="Studio" {Studio {key:"{project.read().id}-{project.read().server_url}",project,visible:visible && !showing_source && tab()==name}}
                        if name=="Chat" {
                            StandaloneChat {running:client.running()}
                        }
                        if name=="Interactive" {crate::interaction::InteractivePage {}}
                        if name=="Settings" {Settings {key:"{project.read().id}-{project.read().server_url}",project}}
                    }
                }
            }
            if visible && let Some(source_id)=separate() {
                div {class:"tab-scroll active",
                    crate::source_settings::SeparateSourceView {key:"{project.read().id}-{project.read().server_url}-{source_id}",project,source_id}
                }
            }
        }
    }}
}

#[component]
pub(crate) fn StandaloneChat(running: bool) -> Element {
    if running {
        rsx! {p{class:"form-help chat-docked-notice","Chat is docked beside the workspace while live."}}
    } else {
        rsx! {crate::interaction::ChatPanel {}}
    }
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
pub fn Sources(mut project: Signal<Workspace>, #[props(default = true)] visible: bool) -> Element {
    let mut copied = use_signal(|| None::<u32>);
    let mut picking = use_signal(|| false);
    let mut expanded = use_signal(std::collections::HashSet::<u32>::new);
    let mut camera_picker = use_signal(|| false);
    let client = use_context::<LiveClient>();
    let connected = use_memo(move || client.status.read().is_some());
    let (camera_scan, camera_inventory, mut refresh_cameras) =
        use_device_inventory(project, connected, "/api/media/cameras", "cameras");
    let (microphone_scan, microphone_inventory, mut refresh_microphones) =
        use_device_inventory(project, connected, "/api/media/microphones", "microphones");
    let (mut capabilities, mut rescan_ffmpeg) = use_capabilities(project);
    let checking_srt = capabilities.read().is_none();
    let srt_available = capabilities
        .read()
        .as_ref()
        .is_some_and(|r| r.as_ref().is_ok_and(|caps| caps.srt_input));
    let full = project.read().sources.len() >= 8;
    use_effect(move || {
        if connected() {
            capabilities.restart();
        }
    });
    use_future(move || async move {
        loop {
            crate::scene_preview::pause(4_000).await;
            let picker_open = *picking.peek();
            let workspace = project.peek();
            let camera_visible = picker_open
                || workspace.sources.iter().any(|source| {
                    matches!(
                        source.kind,
                        acheflow_media_api::visual::SourceKind::Camera { .. }
                    ) && expanded.peek().contains(&source.id)
                });
            let microphone_visible = picker_open
                || workspace
                    .sources
                    .iter()
                    .any(|source| source.kind.is_audio_only());
            if *connected.peek() {
                if camera_visible && *camera_scan.peek() != UseResourceState::Pending {
                    *refresh_cameras.write() += 1;
                }
                if microphone_visible && *microphone_scan.peek() != UseResourceState::Pending {
                    *refresh_microphones.write() += 1;
                }
            }
        }
    });
    let state = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    let activated = state.as_ref().is_some_and(|status| status.activated);
    rsx! {
        div {class:"page-heading",h1 {"Sources"}button {id:"add-source",class:"primary",disabled:full,"aria-expanded":picking().to_string(),"aria-controls":"source-types",onclick:move |_|{
            let opening=!picking();
            picking.set(opening);
            camera_picker.set(false);
            if opening {
                *refresh_cameras.write()+=1;
                *refresh_microphones.write()+=1;
                capabilities.restart();
            }
        },if picking(){"Close source types"}else{"+ Add source"}}}
        if picking() && !full {
            crate::visual_sources::AddSourceOptions{
                project,server_connected:connected(),srt_available,checking_srt,
                on_refresh_ffmpeg:move |_|*rescan_ffmpeg.write()+=1,
                camera_choices:rsx!{
                    CameraChoices{project,open:camera_picker,inventory:camera_inventory,refresh:refresh_cameras,on_add:move |camera:acheflow_media_api::live::CameraDevice|{
                            let mut workspace=project.write();
                            workspace.add_source();
                            if let Some(source)=workspace.sources.last_mut(){
                                source.name=camera.name;
                                source.kind=acheflow_media_api::visual::SourceKind::Camera{device_id:camera.id,mode:None,mirrored:false};
                                source.url.clear();
                                source.key.clear();
                                expanded.write().insert(source.id);
                            }
                            picking.set(false);
                            camera_picker.set(false);
                        }}
                },
                microphone_choices:rsx! {
                    crate::microphone_sources::MicrophoneChoices{
                        project,inventory:microphone_inventory,refresh:refresh_microphones,
                        on_added:move |id|{expanded.write().insert(id);picking.set(false);},
                    }
                },
                on_rtmp:move |_|{if let Some(id)=client.add_source(){expanded.write().insert(id);picking.set(false);}},
                on_srt:move |_|{
                    let mut workspace=project.write();
                    let port=(9000..=65535).find(|port| !workspace.sources.iter().any(|source|
                        matches!(source.kind, acheflow_media_api::visual::SourceKind::Srt { port:used } if used==*port)));
                    if let Some(port)=port {
                        workspace.add_source();
                        if let Some(source)=workspace.sources.last_mut(){
                            source.kind=acheflow_media_api::visual::SourceKind::Srt{port};
                            source.url.clear();
                            expanded.write().insert(source.id);
                        }
                        picking.set(false);
                    }
                },
                on_added:move |_|picking.set(false),
            }
        }
        div {class:"source-actions row",
            if !activated {
                button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Activate workspace sources"}
            }
            button {disabled:(client.busy)() || !activated,onclick:move |_|client.command(Action::Deactivate),"Deactivate sources"}
            span {class:"form-help",if let Some(state)=client.status.read().as_ref(){"RTMP listener: {state.rtmp_address}"}}
        }
        if project.read().sources.is_empty(){div {class:"panel empty-state",h3 {"No sources yet"}p {"Add a source to make it available in scenes."}}}
        for source in project.read().sources.iter().filter(|s|s.kind.is_video_input()).cloned().collect::<Vec<_>>() {
            {
            let srt = matches!(source.kind, acheflow_media_api::visual::SourceKind::Srt { .. });
            let camera = matches!(source.kind, acheflow_media_api::visual::SourceKind::Camera { .. });
            let input = state.as_ref().and_then(|s|s.sources.iter().find(|i|i.id==source.id));
            let key_label = if srt {"SRT passphrase"}else{"RTMP stream key"};
            rsx!{section {key:"{source.id}",class:"panel",
                div {class:"panel-heading",button{class:"destination-toggle",onkeydown:move |event|{if !event.is_auto_repeating() && (event.key()==Key::Named(NamedKey::Enter) || event.key()==Key::Character(" ".into())){event.prevent_default();let mut open=expanded.write();if !open.remove(&source.id){open.insert(source.id);}}},"aria-expanded":expanded.read().contains(&source.id).to_string(),onclick:move |_|{let mut open=expanded.write();if !open.remove(&source.id){open.insert(source.id);}},span{if expanded.read().contains(&source.id){"▾"}else{"▸"}}h2 {"{source.name}"}}
                    span {class:"badge",if camera {"Camera"} else if srt {"SRT"} else {"RTMP"}}
                    span {class:"badge",if srt && !srt_available {"Unavailable"} else if input.is_some_and(|s|s.connected){"Connected"}else if input.is_some_and(|s|s.error.is_some()){"Source error"}else if input.is_some(){"Waiting for input"}else{"Inactive"}}
                    crate::source_settings::OpenSourcePreview {source_id:source.id}
                    crate::source_settings::RemoveSource {project,source_id:source.id}
                }
                div {style:if expanded.read().contains(&source.id){"display:block"}else{"display:none"},
                if visible && expanded.read().contains(&source.id) {
                    crate::source_settings::SourcePreview {source_id:source.id,video:true,activated,input:input.cloned()}
                }
                div {class:"form-grid",
                    crate::source_settings::SourceName {project,source_id:source.id,value:source.name.clone()}
                    if let acheflow_media_api::visual::SourceKind::Srt { port } = source.kind {
                        NumberField {label:"UDP port",value:u32::from(port),min:1024,max:65535,step:1,change:move |port|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.kind=acheflow_media_api::visual::SourceKind::Srt{port:port as u16};}}}
                        if let Ok(url) = client.srt_url(port) {
                            div {class:"field",span {"Publisher URL"},code{"{url}"},button {onclick:move |_|{
                                let connection=project.peek().sources.iter().find(|s|s.id==source.id).and_then(|s| {
                                    let mut url=reqwest::Url::parse(&url).ok()?;
                                    url.query_pairs_mut().append_pair("pbkeylen","16").append_pair("passphrase",&s.key);
                                    Some(url.to_string())
                                });
                                if let Some(connection)=connection {spawn(async move {client.copy_text(connection).await;});}
                            },"Copy URL with passphrase"}}
                        }
                    } else if !camera {
                        Field {label:"RTMP ingest URL",value:source.url.clone(),change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.url=v;}}}
                    }
                    if !camera {
                    Field {label:key_label,secret:true,value:source.key.clone(),change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.key=v;}}}
                    div {class:"field",button {onclick:move |_|{let key=project.peek().sources.iter().find(|s|s.id==source.id).map(|s|s.key.clone());if let Some(key)=key {spawn(async move {if client.copy_text(key).await {copied.set(Some(source.id));}});}},if copied()==Some(source.id){"Copied"}else if srt {"Copy passphrase"}else{"Copy stream key"}}button {onclick:move |_|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.key=crate::project::new_token();copied.set(None);}},"Generate new key"}}
                    }
                    if camera && expanded.read().contains(&source.id) {
                        CameraSettings{project,source_id:source.id,inventory:camera_inventory,refresh:refresh_cameras}
                    }
                    if let Some(size)=source.camera_mode_resolution() {
                        div {class:"field",span {"Offline preview size"}
                            div {class:"readonly-field","{size.width} × {size.height}"}
                        }
                    } else {
                        NumberField {label:"Offline preview width",value:source.resolution.width,min:16,max:((1920*1080/source.resolution.height.max(1)).min(3840)/2)*2,step:2,change:move |v|crate::source_settings::edit_source(project,source.id,SourceEdit::PreviewWidth(v))}
                        NumberField {label:"Offline preview height",value:source.resolution.height,min:16,max:((1920*1080/source.resolution.width.max(1)).min(3840)/2)*2,step:2,change:move |v|crate::source_settings::edit_source(project,source.id,SourceEdit::PreviewHeight(v))}
                    }
                }
                crate::source_settings::SourceCaptureActions {source:source.clone()}
                if let Some(input)=input {
                    div {class:"source-footer",
                        span {"Decoded frames: {input.frames}"}
                        if let Some((width,height))=input.width.zip(input.height) {
                            span {"Incoming: {width} × {height}" if let Some(fps)=input.fps {" / {fps:.2} FPS"}}
                        }
                        if !camera {span {if input.audio{"Audio received"}else{"No audio received"}}}
                        for track in &input.audio_tracks {span {"Track {track.track_id}: {track.codec} / {track.sample_rate} Hz / {track.channels} ch · ",if track.active {"receiving"}else{"waiting"}}}
                    }
                    if let Some(error)=input.error.as_ref(){p {class:"field-error padded","{error}"}}
                }
                div {class:"panel-note",
                    if camera {"The offline test pattern follows the selected capture mode. Auto uses the manual preview size."}
                    else {"Offline preview dimensions only size the test pattern while no video is available."}
                }
                }
            }}
            }
        }
        crate::microphone_sources::MicrophoneSources{
            project,inventory:microphone_inventory,refresh:refresh_microphones,status:state,expanded,visible,
        }
        crate::visual_sources::VisualSources{project}
    }
}
pub(crate) type DeviceInventory<T> = Memo<Option<Result<T, String>>>;
type CameraInventory = DeviceInventory<acheflow_media_api::live::CameraInventory>;

fn use_device_inventory<T: Clone + PartialEq + serde::de::DeserializeOwned + 'static>(
    project: Signal<Workspace>,
    connected: Memo<bool>,
    path: &'static str,
    devices: &'static str,
) -> (
    ReadSignal<UseResourceState>,
    DeviceInventory<T>,
    Signal<u32>,
) {
    let refresh = use_signal(|| 0u32);
    let target = use_memo(move || {
        (
            crate::media::server_url(&project.read(), path),
            crate::server_access::identity(),
        )
    });
    let scan = use_resource(move || {
        refresh();
        let scope = target();
        let online = connected();
        async move {
            let result = if online {
                match &scope.0 {
                    Ok(url) => fetch_device_inventory(url, devices).await,
                    Err(error) => Err(error.clone()),
                }
            } else {
                Err("Connect to the workspace server to list devices".into())
            };
            (scope, result)
        }
    });
    // Keep the last response while refreshing the same server. A device ID from
    // another server or access session must never populate this picker.
    let inventory = use_memo(move || {
        let target = target.read();
        scan.read()
            .as_ref()
            .filter(|(scope, _)| scope == &*target)
            .map(|(_, result)| result.clone())
    });
    (scan.state(), inventory, refresh)
}

async fn fetch_device_inventory<T: serde::de::DeserializeOwned>(
    url: &str,
    devices: &str,
) -> Result<T, String> {
    let response = crate::server_access::request(reqwest::Method::GET, url)?
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| format!("Could not reach the server to list {devices}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not list {devices} (HTTP {})",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .await
        .map_err(|_| format!("Invalid {devices} list"))
}

#[component]
fn CameraChoices(
    project: Signal<Workspace>,
    mut open: Signal<bool>,
    inventory: CameraInventory,
    mut refresh: Signal<u32>,
    on_add: EventHandler<acheflow_media_api::live::CameraDevice>,
) -> Element {
    let snapshot = inventory.read();
    let loading = snapshot.is_none();
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
            button {id:"camera-source-options", "aria-expanded":open().to_string(),onclick:move |_| {
                let opening=!open();
                open.set(opening);
                if opening && !loading {*refresh.write()+=1;}
            },strong{"Camera"}span{"Local video capture"}}
            if open() {
                div {class:"camera-device-list",
                    if loading && devices.is_empty() {p {class:"form-help","Detecting cameras…"}}
                    else if let Some(Err(error))=snapshot.as_ref() {p {class:"field-error","{error}"}}
                    else if devices.is_empty() {p {class:"form-help","No cameras detected on the server."}}
                    for (index, camera) in devices.iter().enumerate() {
                        {
                            let added=camera_assigned(&project.read(), &camera.id, None);
                            let label=camera_name(devices,index);
                            let camera=camera.clone();
                            rsx!{button{key:"{camera.id}",disabled:loading||added,onclick:move |_|on_add.call(camera.clone()),"{label}",if added{span{"Already added"}}}}
                        }
                    }
                    button{disabled:loading,onclick:move |_| *refresh.write()+=1,"Refresh cameras"}
                }
            }
        }
    }
}

fn camera_assigned(workspace: &Workspace, id: &str, except: Option<u32>) -> bool {
    use acheflow_media_api::visual::SourceKind;
    workspace.sources.iter().any(|source| {
        Some(source.id) != except
            && matches!(&source.kind, SourceKind::Camera { device_id, .. } if device_id == id)
    })
}

fn camera_name(devices: &[acheflow_media_api::live::CameraDevice], index: usize) -> String {
    let name = &devices[index].name;
    if devices.iter().filter(|device| &device.name == name).count() > 1 {
        format!("{name} ({})", index + 1)
    } else {
        name.clone()
    }
}

#[component]
fn CameraSettings(
    mut project: Signal<Workspace>,
    source_id: u32,
    inventory: CameraInventory,
    mut refresh: Signal<u32>,
) -> Element {
    use acheflow_media_api::visual::SourceKind;
    let workspace = project.read();
    let Some(SourceKind::Camera {
        device_id,
        mode,
        mirrored,
    }) = workspace
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .map(|source| source.kind.clone())
    else {
        return rsx! {};
    };
    let snapshot = inventory.read();
    let loading = snapshot.is_none();
    let devices = snapshot
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|inventory| inventory.devices.as_slice())
        .unwrap_or_default();
    let active = devices.iter().position(|device| device.id == device_id);
    let device_value = active
        .map(|index| camera_name(devices, index))
        .unwrap_or_else(|| "Saved camera unavailable".into());
    let device_options = devices
        .iter()
        .enumerate()
        .filter(|(_, device)| !camera_assigned(&workspace, &device.id, Some(source_id)))
        .map(|(index, device)| (camera_name(devices, index), device.id.clone()))
        .collect::<Vec<_>>();
    let modes = active
        .map(|index| devices[index].modes.as_slice())
        .unwrap_or_default();
    let mut mode_options = vec![("Auto".to_owned(), None)];
    for mode in modes {
        let label = format!("{} × {} · {:.2} fps", mode.width, mode.height, mode.fps());
        let label = if mode_options.iter().any(|(choice, _)| *choice == label) {
            format!(
                "{label} ({}/{} s)",
                mode.duration_value, mode.duration_scale
            )
        } else {
            label
        };
        mode_options.push((label, Some(*mode)));
    }
    let mode_value = mode_options
        .iter()
        .find(|(_, choice)| *choice == mode)
        .map(|(label, _)| label.clone())
        .unwrap_or_else(|| "Saved mode unavailable".into());
    let mode_device_id = device_id.clone();
    let device_labels = device_options
        .iter()
        .map(|(label, _)| label.clone())
        .collect();
    let mode_labels = mode_options
        .iter()
        .map(|(label, _)| label.clone())
        .collect();
    rsx! {
        div {class:"field",
            if loading {
                span {"Camera device"}
                p {class:"form-help","Detecting cameras…"}
            } else {
                crate::extension_panel::PanelSelect {label:"Camera device",value:device_value,options:device_labels,onselect:move |label:String|{
                    let Some((_, next))=device_options.iter().find(|(choice, _)| *choice==label) else {return};
                    if camera_assigned(&project.peek(), next, Some(source_id)) {return;}
                    if let Some(source)=project.write().sources.iter_mut().find(|source|source.id==source_id)
                        && let SourceKind::Camera{device_id,mode,..}=&mut source.kind
                        && device_id!=next {
                        device_id.clone_from(next);
                        *mode=None;
                    }
                }}
                if let Some(Err(error))=snapshot.as_ref(){span{class:"field-error","{error}"}}
            }
            button{disabled:loading,onclick:move |_| *refresh.write()+=1,"Refresh cameras"}
        }
        if loading {
            div{class:"field",span{"Capture mode"},p{class:"form-help","Reading camera modes…"}}
        } else {
            crate::extension_panel::PanelSelect {label:"Capture mode",value:mode_value,options:mode_labels,onselect:move |label:String|{
                let Some((_, next))=mode_options.iter().find(|(choice, _)| *choice==label) else {return};
                change_camera_mode(project, source_id, &mode_device_id, *next);
            }}
        }
        div {class:"field",
            span{"Mirror horizontally"}
            button{
                class:if mirrored {"selected"}else{""},
                role:"switch","aria-label":"Mirror horizontally","aria-checked":mirrored.to_string(),
                onclick:move |_|{
                    if let Some(source)=project.write().sources.iter_mut().find(|source|source.id==source_id)
                        && let SourceKind::Camera{mirrored,..}=&mut source.kind {*mirrored = !*mirrored;}
                },
                if mirrored {"On"} else {"Off"}
            }
        }
    }
}

fn change_camera_mode(
    mut project: Signal<Workspace>,
    source_id: u32,
    expected_device: &str,
    next: Option<acheflow_media_api::visual::CameraMode>,
) {
    use acheflow_media_api::{live::SourceCapture, visual::SourceKind};
    let capture = {
        let mut workspace = project.write();
        let Some(source) = workspace
            .sources
            .iter_mut()
            .find(|source| source.id == source_id)
        else {
            return;
        };
        let SourceKind::Camera {
            device_id, mode, ..
        } = &mut source.kind
        else {
            return;
        };
        if device_id != expected_device || *mode == next {
            return;
        }
        *mode = next;
        SourceCapture::of(source)
    };
    if let Some(client) = try_consume_context::<LiveClient>()
        .filter(|client| client.owns() && !*client.busy.peek() && !*client.scoped_pending.peek())
        && let Some(mut saved) = try_consume_context::<crate::workspace_sync::WorkspaceSync>()
            .and_then(|sync| sync.saved_source(source_id))
        && let SourceKind::Camera { mode, .. } = &mut saved.kind
    {
        *mode = next;
        // Apply a preset change on its own. Device and mirror edits use the
        // existing capture settings action, which applies those drafts together.
        if capture.matches(&saved) {
            client.source_edit(source_id, SourceEdit::Capture(capture));
        }
    }
}

#[component]
fn Settings(mut project: Signal<Workspace>) -> Element {
    let (capabilities, refresh) = use_capabilities(project);
    let snapshot = capabilities.read().clone();
    let available = snapshot.as_ref().and_then(|r| r.as_ref().ok()).cloned();
    rsx! {
        div {class:"page-heading",div {h1 {"Stream settings"}p {"Separate canvases and outputs for each format."}}}
        FfmpegStatus {capabilities,refresh}
        crate::video_preview::PreviewRate {}
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
    let client = use_context::<LiveClient>();
    let sync = try_consume_context::<crate::workspace_sync::WorkspaceSync>();
    let saved = sync.and_then(|sync| sync.saved_stream(orientation));
    let saved_count = saved.as_ref().map(|stream| stream.outputs.len());
    let owned = client.owns();
    let editing_local = sync.is_some_and(|sync| !(sync.following)());
    let can_edit_list = saved_count.map_or(!owned || editing_local, |count| {
        count <= stream.outputs.len()
    });
    let can_add_output =
        can_edit_list && saved_count.is_none_or(|count| count == stream.outputs.len());
    let can_save_canvas = saved
        .as_ref()
        .is_some_and(|saved| stream.base != saved.base || stream.fps != saved.fps);
    let canvas_edit = FormatSettingsEdit::Canvas {
        base: stream.base.clone(),
        fps: stream.fps,
    };
    let running = client.owns()
        && client.status.read().as_ref().is_some_and(|status| {
            status
                .formats
                .iter()
                .any(|format| format.orientation == orientation)
        });
    let busy = (client.busy)() || (client.scoped_pending)();
    let sizes = if orientation == Orientation::Horizontal {
        ["1280x720", "1920x1080", "2560x1440", "3840x2160"]
    } else {
        ["720x1280", "1080x1920", "1440x2560", "2160x3840"]
    };
    rsx! {section {class:"panel",
        div {class:"panel-heading",h2 {"{orientation.label()}"}span {class:"badge","FFmpeg"}}
        div {class:"form-grid",
            Choice {label:"Base canvas",value:format!("{}x{}",stream.base.width,stream.base.height),choices:sizes.into_iter().map(String::from).collect(),change:move |v:String|{if let Some(r)=resolution(&v){project.write().resize_base(orientation,r);}}}
            NumberField {label:"Base width",value:stream.base.width,change:move |v:u32|{if (16..=7680).contains(&v) && v.is_multiple_of(2){let mut r=project.read().stream(orientation).base.clone();r.width=v;project.write().resize_base(orientation,r);}}}
            NumberField {label:"Base height",value:stream.base.height,change:move |v:u32|{if (16..=7680).contains(&v) && v.is_multiple_of(2){let mut r=project.read().stream(orientation).base.clone();r.height=v;project.write().resize_base(orientation,r);}}}
            FrameRateInput {value:stream.fps,change:move |v|project.write().stream_mut(orientation).fps=v}
        }
        if saved.is_some() {
            div {class:"row padded",
                button {disabled:running || busy || !can_save_canvas, onclick:move |_|client.save_format_settings(orientation, canvas_edit.clone()),"Save canvas and FPS"}
            }
        }
        if running {
            p {class:"form-help", "Stop this format in Studio to save its canvas or change its destination list."}
        }
        div {class:"panel-heading",h2 {"Restream outputs"}button {class:"primary",disabled:running || busy || !can_add_output,onclick:move |_|{let size=project.read().stream(orientation).base.clone();project.write().stream_mut(orientation).outputs.push(Output{ protocol: Default::default(),name:"New destination".into(),url:String::new(),key:String::new(),resolution:size,quality:"Standard".into(),codec:"h264".into(),encoder:None,bitrate_kbps:4500, bitrate_mode: Default::default(),keyframe_interval_secs:2,enabled:true,audio:Default::default()});expanded.set(Some(project.peek().stream(orientation).outputs.len()-1));},"+ Output"}}
        for (index,out) in stream.outputs.iter().enumerate() {
            {
                let saved_output = saved.as_ref().and_then(|saved| saved.outputs.get(index));
                let is_saved = saved_output.is_some();
                let is_new = saved.is_some() && !is_saved;
                let previous_enabled = saved_output.map_or(out.enabled, |saved| saved.enabled);
                rsx! {
            div {key:"{index}",class:"output-config",
                div {class:"panel-heading destination-heading",
                    button {class:"destination-toggle","aria-expanded":(expanded()==Some(index)).to_string(),onclick:move |_|expanded.set(if expanded()==Some(index){None}else{Some(index)}),
                        span {if expanded()==Some(index){"▾"}else{"▸"}} strong {"{out.name}"}
                    }
                    span {class:"destination-summary","{out.resolution.width} × {out.resolution.height} · {out.bitrate_mode.label()} · {out.bitrate_kbps} kbps"}
                    span {class:"badge","{out.protocol.label()}"}
                    span {class:"badge",if is_new {"Unsaved"} else if previous_enabled {"Enabled"}else{"Disabled"}}
                    button {disabled:running || busy || !can_edit_list,onclick:move |_|{
                        expanded.set(None);
                        if is_saved {
                            client.save_format_settings(orientation, FormatSettingsEdit::RemoveOutput {index});
                        } else {
                            project.write().stream_mut(orientation).outputs.remove(index);
                        }
                    },"Remove"}
                }
                if expanded()==Some(index) {
                    {
                        let target = OutputTarget {orientation, index};
                        let (url_label, key_label) = match out.protocol {
                            OutputProtocol::Rtmp => ("RTMP URL", "Stream key"),
                            OutputProtocol::Srt => ("SRT receiver URL", "SRT passphrase (optional)"),
                        };
                        let can_save = is_saved && sync.is_some_and(|sync| sync.changed_output_profile(target).is_some());
                        let profile = OutputProfile::of(out);
                        rsx! {div {class:"form-grid",
                    Field {label:"Destination",value:out.name.clone(),change:move |v|project.write().stream_mut(orientation).outputs[index].name=v}
                    Choice {label:"Output protocol",value:out.protocol.label().to_owned(),
                        choices:OutputProtocol::ALL.into_iter().map(|protocol|protocol.label().to_owned()).collect(),
                        change:move |value:String| {
                            let Some(protocol)=OutputProtocol::ALL.into_iter().find(|protocol|protocol.label()==value) else {return};
                            let mut workspace=project.write();
                            let out=&mut workspace.stream_mut(orientation).outputs[index];
                            if out.protocol != protocol {
                                out.protocol=protocol;
                                out.url.clear();
                                out.key.clear();
                                if !protocol.video_codecs().contains(&out.codec.as_str()) {
                                    out.codec="h264".into();
                                    out.encoder=None;
                                }
                            }
                        }
                    }
                    Field {label:url_label,value:out.url.clone(),change:move |v|project.write().stream_mut(orientation).outputs[index].url=v}
                    Field {label:key_label,secret:true,value:out.key.clone(),change:move |v|project.write().stream_mut(orientation).outputs[index].key=v}
                    if out.protocol==OutputProtocol::Srt {
                        p {class:"form-help","SRT caller · MPEG-TS. Enter srt://host:port. Optional URL settings: latency in microseconds and streamid."}
                    }
                    NumberField {label:"Output width",value:out.resolution.width,change:move |v|project.write().stream_mut(orientation).outputs[index].resolution.width=v}
                    NumberField {label:"Output height",value:out.resolution.height,change:move |v|project.write().stream_mut(orientation).outputs[index].resolution.height=v}
                    Choice {label:"Quality",value:out.quality.clone(),choices:["Low","Standard","High","Custom"].into_iter().map(String::from).collect(),change:move |q:String|{let mut p=project.write();let out=&mut p.stream_mut(orientation).outputs[index];out.bitrate_kbps=match q.as_str(){"Low"=>2500,"High"=>8000,"Standard"=>4500,_=>out.bitrate_kbps};out.quality=q;}}
                    Choice {label:"Bitrate mode",value:out.bitrate_mode.label(),choices:vec!["CBR".into(),"VBR".into()],change:move |v:String|project.write().stream_mut(orientation).outputs[index].bitrate_mode=if v=="CBR"{BitrateMode::Cbr}else{BitrateMode::Vbr}}
                    NumberField {label:"Video bitrate / limit (kbps)",value:out.bitrate_kbps,min:100,max:100000,change:move |v|{let mut p=project.write();let out=&mut p.stream_mut(orientation).outputs[index];out.bitrate_kbps=v;out.quality="Custom".into();}}
                    NumberField {label:"Keyframe interval (seconds)",value:out.keyframe_interval_secs,min:1,max:20,change:move |v|project.write().stream_mut(orientation).outputs[index].keyframe_interval_secs=v}
                    EncoderPicker {project,orientation,index,capabilities:capabilities.clone()}
                    Choice {label:"Output at next start",disabled:busy || !can_edit_list || (is_saved && running),value:if previous_enabled {"Enabled"}else{"Disabled"},choices:vec!["Enabled".into(),"Disabled".into()],change:move |v:String|{
                        let enabled = v=="Enabled";
                        if enabled == previous_enabled {
                            return;
                        }
                        if is_saved {
                            client.save_format_settings(orientation, FormatSettingsEdit::SetOutputEnabled {index,enabled});
                        } else {
                            project.write().stream_mut(orientation).outputs[index].enabled=enabled;
                        }
                    }}
                }
                div {class:"row padded",
                    if is_new {
                        button {disabled:busy || running || saved_count!=Some(index), onclick:move |_|client.save_format_settings(orientation, FormatSettingsEdit::AddOutput {index,profile:profile.clone(),enabled:previous_enabled}),"Save new output"}
                    } else if is_saved {
                        button {disabled:busy || !can_save, onclick:move |_|client.save_output_profile(target),"Save output profile"}
                    }
                }
                if is_saved && running {
                    p {class:"form-help padded","Restart this output in Studio to use saved encoder settings."}
                }
                    }
                    }
                }
            }}
            }
        }
        if stream.outputs.is_empty(){div {class:"empty-state",p {"No outputs configured for this format."}}}
        div {class:"panel-note","Changing the base canvas resizes matching scenes and scales their layers. Output dimensions are independent. Video encoders come from the server FFmpeg. Audio profile: AAC."}
    }}
}
#[component]
fn Scenes(mut project: Signal<Workspace>, visible: bool) -> Element {
    let mut selected = use_signal(|| None::<u32>);
    let mut selected_layer = use_signal(|| None::<u64>);
    let histories = use_scene_histories(project);
    let workspace = project.read();
    let current = workspace
        .scenes
        .iter()
        .find(|s| Some(s.id) == selected())
        .or_else(|| workspace.scenes.first());
    let editor = current.map(|scene| (scene.id, histories.get(&workspace, scene.id)));
    rsx! {
        div {class:"page-heading scene-page-heading",div {h1 {"Scenes"}}div {class:"row",for o in Orientation::ALL{button {onclick:move |_|{let id=project.write().add_scene(o);project.write().migrate_spatial();selected.set(Some(id));},"+ {o.label()} scene"}}}}
        div {class:"scene-list",for s in project.read().scenes.clone(){button {class:if current.as_ref().is_some_and(|c|c.id==s.id){"selected"}else{""},onclick:move |_|{selected.set(Some(s.id));selected_layer.set(None);},"{s.name} / {s.orientation.label()}"}}}
        if let Some((scene_id,history))=editor {
            SceneEditor {key:"{scene_id}",project,scene_id,selected_layer,visible,history}
        }else{div {class:"panel empty-state",h3 {"Create your first scene"}p {"Horizontal and vertical scenes inherit their respective base canvas size."}}}
    }
}

#[derive(Clone, Copy)]
struct SceneHistories {
    owner: dioxus_core::ScopeId,
    entries: CopyValue<
        std::collections::HashMap<(String, String, u32), Signal<crate::scene_canvas::SceneHistory>>,
    >,
}

impl SceneHistories {
    fn get(
        mut self,
        workspace: &Workspace,
        scene_id: u32,
    ) -> Signal<crate::scene_canvas::SceneHistory> {
        let key = (workspace.id.clone(), workspace.server_url.clone(), scene_id);
        *self.entries.write().entry(key).or_insert_with(|| {
            Signal::new_in_scope(crate::scene_canvas::SceneHistory::default(), self.owner)
        })
    }
}

fn use_scene_histories(project: Signal<Workspace>) -> SceneHistories {
    let local = SceneHistories {
        owner: dioxus_core::current_scope_id(),
        entries: use_hook(|| CopyValue::new(Default::default())),
    };
    let shared = try_consume_context::<SceneHistories>();
    let mut histories = shared.unwrap_or(local);
    use_effect(move || {
        if shared.is_some() {
            return;
        }
        let workspace = project.read();
        histories
            .entries
            .write()
            .retain(|(id, server, scene_id), history| {
                let keep = *id == workspace.id
                    && *server == workspace.server_url
                    && workspace.scenes.iter().any(|scene| scene.id == *scene_id);
                if !keep {
                    history.manually_drop();
                }
                keep
            });
    });
    histories
}
#[component]
fn SceneEditor(
    mut project: Signal<Workspace>,
    scene_id: u32,
    selected_layer: Signal<Option<u64>>,
    visible: bool,
    mut history: Signal<crate::scene_canvas::SceneHistory>,
) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|scene| scene.id == scene_id) else {
        return rsx! {};
    };
    let orientation = scene.orientation;
    let base = &workspace.stream(orientation).base;
    rsx! {
        section {class:"panel scene-editor-panel",
            div {class:"scene-metadata",
                CommittedName {label:"Scene name",value:scene.name.clone(),change:move |name:String|{
                    if let Some(scene)=project.write().scenes.iter_mut().find(|scene|scene.id==scene_id)
                        && scene.name!=name {
                        let previous=std::mem::replace(&mut scene.name,name.clone());
                        history.write().record_name(scene_id,previous,name);
                    }
                }}
                span {class:"form-help","{base.width} × {base.height} / {orientation.label()}"}
                div {class:"row scene-actions",
                    crate::scene_canvas::HistoryControls {project,history}
                    SaveSceneButton {scene_id}
                    button {disabled:(client.busy)() || (client.scoped_pending)() || scene_on_air(client,scene_id).is_some(),
                        title:if scene_on_air(client,scene_id).is_some() {"Switch away from this scene or stop its format before deleting it."} else {"Delete scene"},
                        onclick:move |_|client.remove_scene(scene_id),"Delete scene"}
                }
            }
            crate::spatial_editor::SpatialEditor {project,scene_id,history,visible,
                PrimaryViewportEditor {project,scene_id,orientation,selected_layer,visible,history}
            }
        }
    }
}

#[component]
fn PrimaryViewportEditor(
    project: Signal<Workspace>,
    scene_id: u32,
    orientation: Orientation,
    mut selected_layer: Signal<Option<u64>>,
    visible: bool,
    history: Signal<crate::scene_canvas::SceneHistory>,
) -> Element {
    let target = crate::layer_editor::LayerTarget {
        scene: scene_id,
        viewport: None,
    };
    rsx! {
        div {class:"panel-heading",h3 {"Primary viewport content"}}
        Canvas {project,scene_id:Some(scene_id),orientation:orientation,editable:true,live_images:visible,selection:Some(selected_layer),history:Some(history)}
        p {class:"form-help padded","Drag a corner to resize. Hold Shift for the layer ratio or Shift + Option/Alt for the source ratio. The 2D canvas snaps to its edges."}
        div {class:"panel-heading",h3 {"Add a source layer"}}
        div {class:"row padded",for source in project.read().sources.iter().filter(|s|!s.kind.is_audio_only()).cloned().collect::<Vec<_>>() {
            button {onclick:move |_|{
                if let Some(id)=crate::layer_editor::add_layer(project,target,source.id,history){selected_layer.set(Some(id));}
            },"+ {source.name}"}
        }if project.read().sources.is_empty(){p {"Add a video source or overlay in Live / Sources."}}}
        div {class:"panel-heading",h3 {"Primary viewport layers"}}
        LayerEditor {project,scene_id,selection:selected_layer,history:Some(history)}
    }
}

#[component]
fn Studio(project: Signal<Workspace>, visible: bool) -> Element {
    let client = use_context::<LiveClient>();
    let status = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    let preview_selection = [use_signal(|| None), use_signal(|| None)];
    let initial_scenes: [Option<u32>; 2] = std::array::from_fn(|index| {
        let orientation = Orientation::ALL[index];
        let workspace = project.read();
        studio_scene(
            &workspace,
            orientation,
            preview_selection[index](),
            workspace.active(orientation),
        )
    });
    let sync = try_consume_context::<crate::workspace_sync::WorkspaceSync>();
    let saved_streams =
        Orientation::ALL.map(|orientation| sync.and_then(|sync| sync.saved_stream(orientation)));
    let owned = client.owns();
    let start_scenes = std::array::from_fn(|index| {
        initial_scenes[index].filter(|scene| {
            !owned || sync.is_some_and(|sync| sync.can_take_scene(Orientation::ALL[index], *scene))
        })
    });
    let can_start_selected =
        start_scenes == initial_scenes && start_scenes.iter().any(Option::is_some);
    let running = client.running();
    let outputs = || {
        status
            .iter()
            .flat_map(|status| &status.formats)
            .flat_map(|format| &format.outputs)
    };
    let publishing = outputs().any(|output| output_is_active(output.state));
    let can_start_all = if running {
        outputs().any(|output| !output_is_active(output.state))
    } else {
        let workspace = project.read();
        can_start_selected
            && Orientation::ALL
                .into_iter()
                .enumerate()
                .any(|(index, orientation)| {
                    start_scenes[index].is_some()
                        && saved_streams[index]
                            .as_ref()
                            .unwrap_or_else(|| workspace.stream(orientation))
                            .outputs
                            .iter()
                            .any(|output| output.enabled)
                })
    };
    rsx! {
        div {class:"page-heading",div {h1 {"Studio"}p {"Compose both formats and publish to your RTMP and SRT destinations."}}
            div {class:"row",
                if running {
                    button {class:"stop-button",disabled:(client.busy)(),onclick:move |_|client.command(Action::Stop),if publishing {"Stop live"}else{"Stop program preview"}}
                } else {
                    button {disabled:(client.busy)() || !can_start_selected,onclick:move |_|client.command(Action::Start {output:None,preview_only:true,scenes:start_scenes}),"Start program preview"}
                }
                button {class:"primary",disabled:(client.busy)() || !can_start_all,
                    onclick:move |_|client.command(if running {Action::Outputs(OutputAction::Start,None)} else {Action::Start {output:None,preview_only:false,scenes:start_scenes}}),
                    "Start all outputs"
                }
                if publishing {
                    button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Outputs(OutputAction::Restart,None)),"Restart all outputs"}
                }
            }
        }
        if running {crate::video_preview::PreviewRate {compact:true}}
        if visible {ServerLoadPanel {}}
        p {class:"studio-help form-help","Use Start program preview to run the server composition. Start an output to go live."}
        if visible {crate::source_settings::StudioSourcePreview {project}}
        div {class:"studio-grid",for (index,o) in Orientation::ALL.into_iter().enumerate() {
            section {key:"{o.label()}-{project.read().id}-{project.read().server_url}",class:"panel",div {class:"panel-heading",h2 {"{o.label()}"}
                span {class:"badge",if let Some(format)=status.as_ref().and_then(|s|s.formats.iter().find(|f|f.orientation==o)) {
                    if format.outputs.iter().any(|out|out.state==OutputState::Streaming){"Broadcasting"}else if format.outputs.iter().any(|out|out.reconnect_attempt.is_some()){"Reconnecting"}else if format.outputs.iter().any(|out|out.state==OutputState::Starting){"Starting"}else if format.outputs.iter().any(|out|out.state==OutputState::Failed){"Output failed"}else{"Preview only"}
                }else{"Offline"}}
                if status.as_ref().is_some_and(|s| s.formats.iter().any(|f| f.orientation==o)) {
                    button {class:"stop-button",disabled:(client.busy)(),onclick:move |_|client.command(Action::StopFormat(o)),"Stop format"}
                }
            }
                if let Some(format)=status.as_ref().and_then(|s|s.formats.iter().find(|f|f.orientation==o)) {
                    div {class:"studio-preview-pair",
                    StudioSceneSelector {
                        project, orientation:o, visible, running:true, selection:preview_selection[index],
                        publishing:format.outputs.iter().any(|out|output_is_active(out.state)),
                        program:format.applied_scene.as_ref().map(|scene|scene.id),
                    }
                    div {class:"studio-program",
                    if let Some(scene)=&format.applied_scene {
                        div {class:"panel-heading",
                            h3 {if format.outputs.iter().any(|out|output_is_active(out.state)) {"On air"}else{"Program preview"}}
                            span {"{scene.name}"}
                        }
                    }
                    StudioPreview {orientation:o,base:saved_streams[index].as_ref().map_or_else(|| project.read().stream(o).base.clone(), |stream| stream.base.clone()),profiles:format.preview_profiles.clone(),render_fps:format.render_load.target_fps,visible}
                    div {class:"panel-note",SceneInputStatus {project,orientation:o}
                        if let Some(renderer)=format.renderer.as_ref(){p {"Renderer: {renderer}"}}
                        if let Some(error)=format.error.as_ref(){p {class:"field-error",role:"alert","{error}"}}
                        span {"Rendered frames: {format.frames} · audio is mixed into the outputs"}
                        if visible && format.render_load.target_fps > 0 {
                            StudioRenderTelemetry {orientation:o,time_seconds:format.time_seconds,load:format.render_load.clone()}
                        }
                    }

                    for out in &format.outputs {
                        div {key:"{out.index}",class:"output-status",strong {"{out.name}"}span {class:"badge","{out.protocol.label()}"}span {class:"badge",role:"status",if let Some(attempt)=out.reconnect_attempt {"Reconnecting · attempt {attempt}"}else{"{out.state:?}"}}span {"{out.frames} frames · {out.bitrate_kbps:.0} kbps"}
                            button {disabled:(client.busy)(),onclick:{
                                let target=OutputTarget {orientation:o,index:out.index};
                                let action=if output_is_active(out.state) {OutputAction::Restart} else {OutputAction::Start};
                                move |_|client.command(Action::Outputs(action,Some(target)))
                            },if output_is_active(out.state) {"Restart output"} else {"Start output"}}
                            if let Some(fps)=out.encoding_fps {span {"FFmpeg {fps:.1} FPS"}}
                            if let Some(speed)=out.realtime_speed {span {"{speed:.2}× real time"}}
                            if let Some(timing)=out.track_timing.as_ref() {
                                for (name,track) in [("Video",timing.video.as_ref()),("Audio",timing.audio.as_ref())] {
                                    if let Some(track)=track {
                                        span {key:"{name}-timing","{name} FFmpeg p95 {track.pipeline_p95_ms:.2} ms"
                                            if let Some(encode)=track.encode_p95_ms {" · encode stage {encode:.2} ms"}
                                        }
                                    }
                                }
                            }
                            if out.restart_required {span {class:"badge","Saved profile · restart to apply"}}
                            if let Some(encoder)=out.encoder.as_ref(){span {"{encoder} · ",if out.hardware {"Hardware"}else{"Software"}}}
                            if let Some(bus)=out.audio_bus {span {"Audio: scene {out.audio_scene.unwrap_or(0)} → bus {bus}"}if visible {crate::audio_mixer::LiveMeters {target:MeterTarget::Output {orientation:o,index:out.index},fallback:out.audio_meter.clone()}}}
                            if out.repeated_video_frames > 0 || out.inserted_audio_blocks > 0 || out.reconnects > 0 {
                                span {"{out.repeated_video_frames} repeated frames · {out.inserted_audio_blocks} silence blocks · {out.reconnects} reconnects"}
                            }
                            if let Some(error)=out.error.as_ref(){p {class:"field-error","{error}"}}
                        }
                    }
                    crate::studio_mixer::StudioMixer {project,orientation:o,visible,preview_scene:Some(initial_scenes[index])}
                    }
                    }
                }else {
                    StudioSceneSelector {project,orientation:o,visible,selection:preview_selection[index]}
                    if let Some(scene_id) = start_scenes[index] {
                        {
                            let scenes = std::array::from_fn(|i| (i == index).then_some(scene_id));
                            let destinations = {
                                let workspace = project.read();
                                saved_streams[index].as_ref().unwrap_or_else(|| workspace.stream(o))
                                    .outputs.iter().enumerate().filter(|(_, output)| output.enabled)
                                    .map(|(index, output)| (index, output.name.clone())).collect::<Vec<_>>()
                            };
                            rsx! {
                                button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Start {output:None,preview_only:true,scenes}),"Start format preview"}
                                for (output_index,name) in destinations {
                                    div {key:"{output_index}",class:"output-status",strong {"{name}"}
                                        button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Start {output:Some(OutputTarget {orientation:o,index:output_index}),preview_only:false,scenes}),"Start output"}
                                    }
                                }
                            }
                        }
                    } else if initial_scenes[index].is_some() {
                        p {class:"form-help","Save the selected scene to start its program preview or outputs."}
                    }
                    crate::studio_mixer::StudioMixer {project,orientation:o,visible,preview_scene:Some(initial_scenes[index])}
                }
            }
        }}
    }
}

#[component]
fn DraftPreviewInputs(project: Signal<Workspace>, scene_id: u32) -> Element {
    use crate::source_settings::{
        KeepSourcePreview, can_activate_preview, preview_source_registered,
    };
    let client = use_context::<LiveClient>();
    let mut open = use_signal(|| false);
    let can_request = client.owns()
        && can_activate_preview(true)
        && client
            .status
            .read()
            .as_ref()
            .is_some_and(|status| !status.activated);
    use_effect(use_reactive((&can_request,), move |(allowed,)| {
        if !allowed && open() {
            open.set(false);
        }
    }));
    if !can_request {
        return rsx! {};
    }
    let workspace = project.read();
    let Some(scene) = workspace.scenes.iter().find(|scene| scene.id == scene_id) else {
        return rsx! {};
    };
    let ids = crate::scene_preview::draft_capture_sources(&workspace, scene);
    let (available, missing): (Vec<_>, Vec<_>) = ids
        .into_iter()
        .partition(|id| preview_source_registered(*id));
    let status = client.status.read();
    let previewing = open();
    rsx! {
        if previewing || !available.is_empty() {
            div {class:"row padded draft-preview-controls",
                button {"aria-pressed":previewing.to_string(),onclick:move |_|open.set(!open()),
                    if previewing {"Stop draft capture"} else {"Preview draft inputs"}
                }
            }
        }
        if !missing.is_empty() {
            p {class:"form-help padded","Add the draft’s remaining sources to the server to preview them."}
        }
        if previewing {
            for id in available {
                {
                    let input = status.as_ref().and_then(|status|status.sources.iter().find(|source|source.id==id));
                    let name = workspace.sources.iter().find(|source|source.id==id).map(|source|source.name.clone());
                    rsx! {
                        KeepSourcePreview {key:"{id}",source_id:id,name,
                            waiting:input.is_none_or(|input|!input.connected && input.error.is_none()),
                            capture_error:input.and_then(|input|input.error.clone())}
                    }
                }
            }
        }
    }
}

fn studio_scene(
    workspace: &Workspace,
    orientation: Orientation,
    selection: Option<Option<u32>>,
    fallback: Option<u32>,
) -> Option<u32> {
    match selection {
        Some(None) => None,
        Some(Some(id))
            if workspace
                .scenes
                .iter()
                .any(|scene| scene.id == id && scene.orientation == orientation) =>
        {
            Some(id)
        }
        _ => fallback,
    }
}

#[component]
fn StudioSceneSelector(
    project: Signal<Workspace>,
    orientation: Orientation,
    visible: bool,
    mut selection: Signal<Option<Option<u32>>>,
    #[props(default)] running: bool,
    #[props(default)] publishing: bool,
    #[props(default)] program: Option<u32>,
) -> Element {
    let client = use_context::<LiveClient>();
    let sync = try_consume_context::<crate::workspace_sync::WorkspaceSync>();
    let mut editing = use_signal(|| None::<u32>);
    let workspace = project.read();
    let scenes: Vec<_> = workspace
        .scenes
        .iter()
        .filter(|scene| scene.orientation == orientation)
        .map(|scene| (scene.id, scene.name.clone()))
        .collect();
    let current = if running {
        studio_scene(&workspace, orientation, selection(), program).or(program)
    } else {
        studio_scene(
            &workspace,
            orientation,
            selection(),
            workspace.active(orientation),
        )
    };
    let changed = current
        .and_then(|scene| sync?.scene_save_state(scene))
        .is_some_and(|state| state.changed);
    let can_edit = !client.owns()
        || crate::server_access::allows(acheflow_media_api::access::Permission::ScenesManage);
    let editing_current = can_edit && current.is_some() && editing() == current;
    let draft = current.filter(|id| !running || Some(*id) != program || changed || editing_current);
    let switching = draft.is_some_and(|id| Some(id) != program);
    let can_take = switching
        && draft
            .is_some_and(|scene| sync.is_some_and(|sync| sync.can_take_scene(orientation, scene)));
    rsx! {
        div {class:"studio-draft",
        div {class:"padded field",
            span {"Draft scene"}
            div {class:"row",
                if !running {
                    ActionButton {class:if current.is_none(){"selected"}else{""},pressed:Some(current.is_none()),onclick:move |_|selection.set(Some(None)),"No scene"}
                }
                for (id,name) in scenes {
                    ActionButton {key:"{id}",class:if current==Some(id){"selected"}else{""},
                        pressed:Some(current==Some(id)),onclick:move |_|{
                            if editing.peek().is_some() {editing.set(Some(id));}
                            selection.set(Some(Some(id)));
                        },"{name}"
                    }
                }
                if can_edit && let Some(scene_id)=current {
                    ActionButton {class:"studio-edit-draft",expanded:Some(editing_current),onclick:move |_| {
                        editing.set((!editing_current).then_some(scene_id));
                    },if editing_current {"Close inspector"} else {"Edit draft"}}
                }
            }
        }
        if let Some(scene_id)=draft {
            div {class:"studio-draft-preview",
                if running || changed {
                    div {class:"panel-heading",
                        h3 {"Draft preview"}
                        div {class:"row",
                            if changed {SaveSceneButton {scene_id}}
                            if switching {
                                ActionButton {class:"primary",disabled:!can_take || (client.busy)() || (client.scoped_pending)(),
                                    onclick:move |_|client.take_scene(orientation,scene_id),
                                    if publishing {"Take live"}else{"Use scene"}
                                }
                            }
                        }
                    }
                }
                if switching && !can_take {
                    p {class:"padded form-help","Save this scene and its sources before taking it live."}
                }
                if !running && visible {DraftPreviewInputs {project,scene_id}}
                StudioDraftCanvas {key:"{scene_id}",project,scene_id,orientation,visible,editing:editing_current}
            }
        } else if !running {
            Canvas {project,scene_id:None,orientation,live_images:false}
        }
        }
    }
}

#[component]
fn StudioDraftCanvas(
    project: Signal<Workspace>,
    scene_id: u32,
    orientation: Orientation,
    visible: bool,
    editing: bool,
) -> Element {
    let mut selection = use_signal(|| None::<u64>);
    let history = use_scene_histories(project).get(&project.peek(), scene_id);
    let target = crate::layer_editor::LayerTarget {
        scene: scene_id,
        viewport: None,
    };
    rsx! {
        if editing {
            div {class:"panel-heading",h3 {"Primary viewport"}}
        }
        Canvas {project,scene_id:Some(scene_id),orientation,editable:editing,live_images:visible,
            selection:editing.then_some(selection),history:editing.then_some(history)}
        if editing {
            div {class:"studio-draft-inspector",
                div {class:"panel-heading",h3 {"Draft inspector"}
                    div {class:"row",crate::scene_canvas::HistoryControls {project,history}}
                }
                div {class:"row padded",for source in project.read().sources.iter().filter(|source|!source.kind.is_audio_only()) {
                    ActionButton {key:"{source.id}",onclick:{
                        let source_id=source.id;
                        move |_| if let Some(layer)=crate::layer_editor::add_layer(project,target,source_id,history) {selection.set(Some(layer));}
                    },"+ {source.name}"}
                }}
                LayerEditor {project,scene_id,selection,history:Some(history)}
            }
        }
    }
}

fn scene_on_air(client: LiveClient, scene_id: u32) -> Option<bool> {
    if !client.owns() {
        return None;
    }
    let status = client.status.read();
    let format = status.as_ref()?.formats.iter().find(|format| {
        format
            .applied_scene
            .as_ref()
            .is_some_and(|scene| scene.id == scene_id)
    })?;
    Some(
        format
            .outputs
            .iter()
            .any(|output| output_is_active(output.state)),
    )
}

#[component]
fn SaveSceneButton(scene_id: u32) -> Element {
    let client = use_context::<LiveClient>();
    let save_state = try_consume_context::<crate::workspace_sync::WorkspaceSync>()
        .and_then(|sync| sync.scene_save_state(scene_id));
    let Some(state) = save_state.filter(|_| client.owns()) else {
        return rsx! {span {class:"form-help","Local draft"}};
    };
    let label = if scene_on_air(client, scene_id) == Some(true) {
        "Update on air"
    } else if state.saved {
        "Save scene"
    } else {
        "Add scene to server"
    };
    rsx! {
        if !state.available {
            span {class:"form-help","Save the scene’s sources and format settings first."}
        }
        ActionButton {
            disabled:!state.available || (client.busy)() || (client.scoped_pending)(),
            onclick:move |_|client.save_scene(scene_id),"{label}"
        }
    }
}

fn output_is_active(state: OutputState) -> bool {
    matches!(state, OutputState::Starting | OutputState::Streaming)
}

#[component]
fn StudioPreview(
    orientation: Orientation,
    base: Resolution,
    profiles: Vec<acheflow_media_api::live::OutputPreviewProfile>,
    render_fps: u32,
    visible: bool,
) -> Element {
    let client = use_context::<LiveClient>();
    let mut selected = use_signal(|| None::<usize>);
    let mut encoded = use_signal(|| false);
    let mut notice = use_signal(|| Some("Starting encoded preview…"));
    let active = profiles
        .iter()
        .find(|profile| Some(profile.index) == selected());
    let size = active.map_or(&base, |profile| &profile.resolution);
    let show_encoded = active.is_some() && encoded();
    let mode_query = if show_encoded {
        "&mode=encoded"
    } else {
        "&mode=lossless"
    };
    let url = client.preview(&format!(
        "/api/live/preview/{}.jpg",
        orientation.label().to_lowercase()
    ));
    let url = active.map_or_else(
        || format!("{url}{mode_query}"),
        |profile| format!("{url}&output={}{mode_query}", profile.index),
    );
    let path = active
        .map(|profile| profile.index.to_string())
        .unwrap_or_else(|| "canvas".into());
    let mut choices = vec!["Scene canvas".to_owned()];
    choices.extend(
        profiles
            .iter()
            .map(|profile| format!("{} · {}", profile.index + 1, profile.name)),
    );
    let indices: Vec<_> = profiles.iter().map(|profile| profile.index).collect();
    let selected_label = active.map_or_else(
        || choices[0].clone(),
        |profile| format!("{} · {}", profile.index + 1, profile.name),
    );
    rsx! {
        if !profiles.is_empty() {
            div {class:"padded",
                crate::extension_panel::PanelSelect {label:"Preview profile",value:selected_label,options:choices.clone(),onselect:move |label:String|{
                    if let Some(choice)=choices.iter().position(|choice| choice == &label) {
                        selected.set(choice.checked_sub(1).and_then(|choice|indices.get(choice)).copied());
                        notice.set(Some("Starting encoded preview…"));
                    }
                }}
                if let Some(profile)=active {
                    p {class:"form-help",
                        "{profile.protocol.label()} · {profile.resolution.width} × {profile.resolution.height} · {profile.fps} FPS · {profile.codec} · {profile.bitrate_mode.label()} {profile.bitrate_kbps} kbps"
                    }
                    div {class:"row",
                        button {class:if show_encoded {"selected"}else{""},"aria-pressed":show_encoded.to_string(),onclick:move |_| {
                            if !encoded() {
                                encoded.set(true);
                                notice.set(Some("Starting encoded preview…"));
                            }
                        },"After encoding"}
                        button {class:if show_encoded {""}else{"selected"},"aria-pressed":(!show_encoded).to_string(),onclick:move |_|encoded.set(false),"Before encoding"}
                    }
                    if show_encoded {
                        p {class:"form-help studio-preview-notice",role:"status",
                            if visible && let Some(message)=notice() { "{message}" }
                            else { "Local video encode · snapshots up to {acheflow_media_api::live::ENCODED_PREVIEW_SNAPSHOT_FPS} FPS" }
                        }
                    } else if u64::from(profile.resolution.width) * u64::from(profile.resolution.height) > 3840 * 2160 {
                        p {class:"form-help","The preview is scaled down to fit the 4K pixel budget."}
                    }
                }
            }
        }
        div {class:"canvas-wrap",div {
            class:if orientation==Orientation::Vertical{"live-preview-frame portrait"}else{"live-preview-frame"},
            style:format!("aspect-ratio: {} / {};",size.width,size.height),
            if visible {VideoPreview {key:"{path}-{show_encoded}",url,max_fps:Some(render_fps),notice:show_encoded.then_some(notice)}}
        }}
    }
}

#[derive(Clone, Copy)]
struct RenderSample {
    time_ms: u64,
    target_fps: u32,
    p95_ms: f64,
}

#[component]
fn StudioRenderTelemetry(
    orientation: Orientation,
    time_seconds: f64,
    load: acheflow_media_api::live::RenderLoad,
) -> Element {
    let client = use_context::<LiveClient>();
    let snapshot = client.telemetry.read();
    let (time_seconds, load) = snapshot
        .as_ref()
        .filter(|snapshot| {
            snapshot.workspace_id.as_deref() == Some(client.project.read().id.as_str())
        })
        .and_then(|snapshot| {
            snapshot
                .render
                .iter()
                .find(|format| format.orientation == orientation)
        })
        .map_or((time_seconds, load), |sample| {
            (sample.time_seconds, sample.load.clone())
        });
    rsx! {StudioRenderProfile {time_seconds,load,project:client.project}}
}

#[component]
fn StudioRenderProfile(
    time_seconds: f64,
    load: acheflow_media_api::live::RenderLoad,
    project: Signal<Workspace>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut history = use_signal(std::collections::VecDeque::<RenderSample>::new);
    use_effect(use_reactive(
        (&time_seconds, &load.target_fps, &load.p95_ms),
        move |(time_seconds, target_fps, p95_ms)| {
            let sample = RenderSample {
                time_ms: (time_seconds * 1000.) as u64,
                target_fps,
                p95_ms,
            };
            if history.peek().back().is_some_and(|last| {
                last.time_ms > sample.time_ms || last.target_fps != sample.target_fps
            }) {
                history.write().clear();
            }
            if history
                .peek()
                .back()
                .is_some_and(|last| sample.time_ms - last.time_ms < 250)
            {
                return;
            }
            let mut history = history.write();
            history.retain(|point| sample.time_ms - point.time_ms <= 60_000);
            history.push_back(sample);
        },
    ));
    let budget = 1000. / f64::from(load.target_fps.max(1));
    rsx! {
        div {class:"studio-format-load",
            span {"{load.observed_fps:.1} / {load.target_fps} FPS"}
            span {"Render p95 {load.p95_ms:.1} ms / {budget:.1} ms budget"}
            span {"{load.deadline_miss_percent:.1}% over frame budget"}
            ActionButton {expanded:Some(open()),onclick:move |_|open.set(!open()),
                if open() {"Hide render graph"} else {"Render graph"}
            }
        }
        if open() {
            {
                let history = history.read();
                let ceiling = history.iter().map(|point| point.p95_ms).fold(budget * 1.25, f64::max);
                let path = history.back().map(|last| plot_path(
                    history.iter().map(|point| (point.time_ms, Some(100. * point.p95_ms / ceiling))),
                    last.time_ms,
                )).unwrap_or_default();
                rsx! {div {class:"studio-render-graph",
                    span {class:"form-help","Render p95 · last 60 seconds · dashed line is the frame budget"}
                    LoadPlot {path,stroke:"#ffab70",label:"Frame render time over the last 60 seconds",threshold:Some(100. * budget / ceiling)}
                    if let Some(profile) = load.gpu_profile.clone() {
                        GpuRenderPasses {project,profile}
                    } else {
                        p {class:"form-help","GPU pass timing requires a timestamp-capable adapter and a rendered sample."}
                    }
                }}
            }
        }
    }
}

#[component]
fn GpuRenderPasses(
    project: Signal<Workspace>,
    profile: acheflow_media_api::live::GpuFrameProfile,
) -> Element {
    let workspace = project.read();
    let scene = workspace
        .scenes
        .iter()
        .find(|scene| scene.id == profile.scene_id);
    let mut layers = std::collections::BTreeMap::new();
    let mut combined = Vec::new();
    for pass in &profile.passes {
        if let Some(layer_id) = pass.layer_id {
            layers
                .entry((pass.viewport_id, layer_id, pass.source_id))
                .or_insert_with(Vec::new)
                .push(pass);
        } else {
            combined.push(pass);
        }
    }
    let duration = |passes: &[&acheflow_media_api::live::GpuPassTiming]| {
        passes.iter().map(|pass| pass.elapsed_ms).sum::<f64>()
    };
    let mut layers = layers.into_iter().collect::<Vec<_>>();
    layers.sort_by(|(_, a), (_, b)| duration(b).total_cmp(&duration(a)));
    let viewport_name = |id: u64| {
        scene
            .and_then(|scene| scene.spatial.as_ref())
            .and_then(|spatial| spatial.objects.iter().find(|object| object.id == id))
            .map(|object| object.name.clone())
            .unwrap_or_else(|| format!("Viewport {id}"))
    };
    rsx! {
        div {class:"studio-gpu-profile",
            h3 {"GPU render passes"}
            p {class:"form-help","Latest sampled frame. Layer totals include crop and effects executed on this frame. Composition is batched per viewport."}
            if profile.truncated {p {class:"form-help","This frame exceeded the 128-pass sampling limit."}}
            for ((viewport, layer, source), passes) in layers {
                {
                    let name = workspace.sources.iter().find(|item| Some(item.id)==source)
                        .map(|item| item.name.as_str()).unwrap_or("Source");
                    let total = duration(&passes);
                    rsx! {
                        div {key:"{viewport:?}-{layer}",class:"studio-gpu-layer",
                            strong {"{name} · layer {layer}"}
                            if let Some(id)=viewport {span {class:"form-help","{viewport_name(id)}"}}
                            span {"{total:.3} ms"}
                            div {class:"studio-gpu-pass-list",
                                for (index,pass) in passes.iter().enumerate() {
                                    span {key:"{index}","{pass.stage} · {pass.elapsed_ms:.3} ms"}
                                }
                            }
                        }
                    }
                }
            }
            for pass in combined {
                div {class:"studio-gpu-layer",
                    strong {"{pass.stage}"}
                    if let Some(id)=pass.viewport_id {span {class:"form-help","{viewport_name(id)}"}}
                    span {"{pass.elapsed_ms:.3} ms"}
                }
            }
        }
    }
}

#[component]
pub(crate) fn ServerLoadPanel() -> Element {
    let client = use_context::<LiveClient>();
    let history = client.server_load.read();
    let samples = if history.is_empty() {
        client
            .status
            .read()
            .as_ref()
            .map(|status| status.server_load.clone())
            .unwrap_or_default()
    } else {
        history.clone()
    };
    rsx! {if !samples.is_empty() {ServerLoad {samples}}}
}

#[component]
pub(crate) fn ServerLoad(samples: Vec<acheflow_media_api::live::ServerLoadSample>) -> Element {
    let Some(latest) = samples.last() else {
        return rsx! {};
    };
    let ram_gib = latest.ram_used_bytes as f64 / (1u64 << 30) as f64;
    let total_gib = latest.ram_total_bytes as f64 / (1u64 << 30) as f64;
    let cpu_path = load_path(&samples, |sample| Some(f64::from(sample.cpu_percent)));
    let ram_path = load_path(&samples, |sample| {
        (sample.ram_total_bytes > 0)
            .then(|| 100. * sample.ram_used_bytes as f64 / sample.ram_total_bytes as f64)
    });
    let current_gpu = samples
        .iter()
        .rev()
        .take_while(|sample| latest.time_ms.saturating_sub(sample.time_ms) <= 2200)
        .find(|sample| !sample.gpus.is_empty());
    rsx! {
        section {class:"panel studio-server-load",
            div {class:"panel-heading",h2 {"Server load"}span {class:"form-help","Last 60 seconds"}}
            div {class:"studio-load-history",
                div {
                    span {"CPU {latest.cpu_percent:.0}%"}
                    LoadPlot {path:cpu_path,stroke:"#ffab70",label:"Server CPU usage over the last 60 seconds"}
                }
                div {
                    span {"RAM {ram_gib:.1} / {total_gib:.1} GiB"}
                    LoadPlot {path:ram_path,stroke:"#78c9b1",label:"Server memory usage over the last 60 seconds"}
                }
                if let Some(sample)=current_gpu {
                        for gpu in &sample.gpus {
                            div {key:"{gpu.id}",class:"studio-gpu-load",
                                strong {"{gpu.name}"}
                                if let Some(utilization)=gpu.utilization_percent {
                                    span {"Activity {utilization:.0}%"}
                                    LoadPlot {path:gpu_load_path(&samples,&gpu.id,|gpu|gpu.utilization_percent.map(f64::from)),stroke:"#aab7ff",label:"GPU utilization over the last 60 seconds"}
                                }
                                if let Some(used)=gpu.memory_used_bytes {
                                    {
                                        let used = used as f64 / (1u64<<30) as f64;
                                        rsx!{
                                            if gpu.unified_memory {
                                                span {"GPU shared RAM {used:.1} GiB"}
                                            } else if let Some(total)=gpu.memory_total_bytes {
                                                {let total = total as f64 / (1u64<<30) as f64;rsx!{span {"VRAM {used:.1} / {total:.1} GiB"}}}
                                                LoadPlot {path:gpu_load_path(&samples,&gpu.id,|gpu|Some(100. * gpu.memory_used_bytes? as f64 / gpu.memory_total_bytes?.max(1) as f64)),stroke:"#78c9b1",label:"Video memory usage over the last 60 seconds"}
                                            } else {
                                                span {"VRAM {used:.1} GiB"}
                                            }
                                        }
                                    }
                                }
                                if let Some(encoder)=gpu.encoder_percent {
                                    span {"Video encoder {encoder:.0}%"}
                                    LoadPlot {path:gpu_load_path(&samples,&gpu.id,|gpu|gpu.encoder_percent.map(f64::from)),stroke:"#aab7ff",label:"Video encoder utilization over the last 60 seconds"}
                                }
                                if let Some(decoder)=gpu.decoder_percent {
                                    span {"Video decoder {decoder:.0}%"}
                                    LoadPlot {path:gpu_load_path(&samples,&gpu.id,|gpu|gpu.decoder_percent.map(f64::from)),stroke:"#aab7ff",label:"Video decoder utilization over the last 60 seconds"}
                                }
                            }
                        }
                } else if let Some(gpu)=latest.busiest_gpu_percent {
                    div {
                        span {"Busiest GPU {gpu:.0}%"}
                        LoadPlot {path:load_path(&samples,|sample|sample.busiest_gpu_percent.map(f64::from)),stroke:"#aab7ff",label:"Highest reported GPU usage over the last 60 seconds"}
                    }
                }
            }
        }
    }
}

#[component]
fn LoadPlot(
    path: String,
    stroke: &'static str,
    label: &'static str,
    #[props(default)] threshold: Option<f64>,
) -> Element {
    rsx! {
        svg {class:"studio-load-plot",xmlns:"http://www.w3.org/2000/svg",view_box:"-1 -1 242 62",preserve_aspect_ratio:"none",role:"img","aria-label":label,
            // Blitz serializes the SVG subtree for painting. Keep strokes in the SVG.
            for y in [0, 30, 60] {line {x1:"0",x2:"240",y1:y.to_string(),y2:y.to_string(),stroke:"#303b4e",stroke_width:"0.6"}}
            if let Some(threshold)=threshold {
                {let y=(60. * (1. - threshold / 100.)).to_string();rsx!{
                    line {x1:"0",x2:"240",y1:y.clone(),y2:y,stroke:"#b4bec9",stroke_dasharray:"4 3","aria-hidden":"true"}
                }}
            }
            path {d:path,fill:"none",stroke,stroke_width:"1.5",vector_effect:"non-scaling-stroke"}
        }
    }
}

fn gpu_load_path(
    samples: &[acheflow_media_api::live::ServerLoadSample],
    id: &str,
    value: impl Fn(&acheflow_media_api::live::GpuLoadSample) -> Option<f64>,
) -> String {
    let Some(last) = samples.last() else {
        return String::new();
    };
    plot_path(
        samples
            .iter()
            .filter(|sample| !sample.gpus.is_empty())
            .map(|sample| {
                (
                    sample.time_ms,
                    sample.gpus.iter().find(|gpu| gpu.id == id).and_then(&value),
                )
            }),
        last.time_ms,
    )
}

fn load_path(
    samples: &[acheflow_media_api::live::ServerLoadSample],
    value: impl Fn(&acheflow_media_api::live::ServerLoadSample) -> Option<f64>,
) -> String {
    let Some(last) = samples.last() else {
        return String::new();
    };
    plot_path(
        samples.iter().map(|sample| (sample.time_ms, value(sample))),
        last.time_ms,
    )
}

fn plot_path(samples: impl IntoIterator<Item = (u64, Option<f64>)>, last_ms: u64) -> String {
    use std::fmt::Write;

    let mut path = String::new();
    let mut previous = None;
    for (time_ms, value) in samples {
        let age = last_ms.saturating_sub(time_ms);
        if age > 60_000 {
            continue;
        }
        if let Some(percent) = value.filter(|percent| percent.is_finite()) {
            let command = if previous.is_some_and(|time| time_ms.saturating_sub(time) <= 1500) {
                'L'
            } else {
                'M'
            };
            let x = 240. * (1. - age as f64 / 60_000.);
            let y = 60. * (1. - percent.clamp(0., 100.) / 100.);
            let _ = write!(path, "{command}{x:.2},{y:.2} ");
            previous = Some(time_ms);
        } else {
            previous = None;
        }
    }
    path
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
            let visual = workspace
                .sources
                .iter()
                .any(|s| s.id == id && !s.kind.is_video_input());
            let connected = visual
                || status
                    .as_ref()
                    .and_then(|s| s.sources.iter().find(|s| s.id == id))
                    .is_some_and(|s| s.connected && s.frames > 0);
            (name, connected, visual)
        })
        .collect();
    rsx! {div {class:"scene-input-status",
        if inputs.is_empty() {span {"No sources in the selected scene"}}
        for (name,connected,visual) in inputs {span {class:if connected {"input-connected"}else{"input-waiting"},
            if visual {"{name}: overlay"}else if connected {"{name}: receiving"}else{"{name}: waiting for RTMP · no video"}
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
/// Commit after typing so a saved workspace snapshot cannot overwrite queued keys.
#[component]
pub(crate) fn CommittedName(
    label: &'static str,
    value: String,
    change: EventHandler<String>,
    #[props(default)] max_bytes: Option<usize>,
) -> Element {
    let mut draft = use_signal(|| value.clone());
    let mut displayed = use_signal(|| value.clone());
    let mut applied = use_signal(|| value.clone());
    let mut reset = use_signal(|| 0u32);
    let mut error = use_signal(|| false);
    use_effect(use_reactive((&value,), move |(value,)| {
        if *draft.peek() == *applied.peek() {
            draft.set(value.clone());
            displayed.set(value.clone());
        }
        applied.set(value);
    }));
    let commit = move || {
        let text = draft.peek().trim().to_string();
        let valid = !text.is_empty() && max_bytes.is_none_or(|limit| text.len() <= limit);
        if valid {
            draft.set(text.clone());
            displayed.set(text.clone());
            change.call(text);
        } else {
            draft.set(applied.peek().clone());
            displayed.set(applied.peek().clone());
            reset += 1;
        }
        error.set(!valid);
    };
    rsx! {div {class:"field",label {"{label}",input {key:"{reset}",r#type:"text","aria-label":label,value:{let _=displayed();draft.peek().clone()},"aria-invalid":error().to_string(),
        oninput:move |e|{draft.set(e.value());},onblur:move |_|{let mut commit=commit;commit();},
        onkeydown:move |e|{if e.key()==Key::Named(NamedKey::Enter){e.prevent_default();let mut commit=commit;commit();}}
    }}if error(){
        span {class:"field-error",role:"alert",
            if let Some(limit)=max_bytes {"Use a name with 1–{limit} bytes. Kept the previous name."}
            else {"Enter a name. Kept the previous name."}
        }
    }}}
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
    #[props(default)] disabled: bool,
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
        for (key,label) in choices {button {disabled,class:if value==key {"selected"}else{""},"aria-pressed":(value==key).to_string(),onclick:{let key=key.clone();move |_|change.call(key.clone())},"{label}"}}
    }}}
}

#[cfg(test)]
mod camera_discovery_tests {
    use super::*;
    use acheflow_media_api::live::{CameraDevice, CameraInventory};
    use blitz_dom::{Document, DocumentConfig};
    use std::{cell::Cell, rc::Rc};

    #[derive(Default)]
    struct Harness {
        refresh: Cell<Option<Signal<u32>>>,
        open: Cell<Option<Signal<bool>>>,
    }

    fn app() -> Element {
        let harness = use_context::<Rc<Harness>>();
        let refresh = use_signal(|| 0);
        harness.refresh.set(Some(refresh));
        let project = use_signal(Workspace::default);
        let open = use_signal(|| true);
        harness.open.set(Some(open));
        let scan = use_resource(move || {
            let attempt = refresh();
            async move {
                if attempt == 0 {
                    return Err("Camera inventory temporarily unavailable".to_owned());
                }
                if attempt == 2 {
                    std::future::pending::<()>().await;
                }
                Ok(CameraInventory {
                    supported: true,
                    devices: vec![CameraDevice {
                        id: "virtual-camera".into(),
                        name: "OBS Virtual Camera".into(),
                        modes: Vec::new(),
                    }],
                })
            }
        });
        let inventory = use_memo(move || scan.read().clone());
        rsx! {
            style {{include_str!("dashboard.css")}}
            div {id:"picker-grid", class:"source-types",
                button {id:"rtmp-card","RTMP"}
                CameraChoices {project,open,inventory,refresh,on_add:|_|{}}
                button {"Chat"}
            }
            button {id:"apply-sources","Activate workspace sources"}
        }
    }

    fn settle(doc: &mut dioxus_native::DioxusDocument) {
        for _ in 0..40 {
            if !doc.poll(None) {
                doc.inner.borrow_mut().resolve(0.);
                return;
            }
        }
        panic!("camera inventory UI did not settle");
    }

    fn inventory_text(doc: &dioxus_native::DioxusDocument) -> String {
        let document = doc.inner.borrow();
        let node = document
            .query_selector(".camera-device-list")
            .unwrap()
            .unwrap();
        document.get_node(node).unwrap().text_content()
    }

    #[tokio::test]
    async fn camera_inventory_recovers_without_reflowing_the_grid() {
        let harness = Rc::new(Harness::default());
        let mut doc = dioxus_native::DioxusDocument::new(
            VirtualDom::new(app).with_root_context(harness.clone()),
            DocumentConfig::default(),
        );
        doc.initial_build();
        settle(&mut doc);
        assert!(inventory_text(&doc).contains("Camera inventory temporarily unavailable"));
        assert!(inventory_text(&doc).contains("Refresh cameras"));
        harness.open.get().unwrap().set(false);
        settle(&mut doc);
        let layout = |doc: &dioxus_native::DioxusDocument| {
            let dom = doc.inner.borrow();
            ["#rtmp-card", "#picker-grid", "#apply-sources"].map(|selector| {
                let id = dom.query_selector(selector).unwrap().unwrap();
                *dom.get_node(id).unwrap().final_layout()
            })
        };
        let closed = layout(&doc);
        harness.open.get().unwrap().set(true);
        harness.refresh.get().unwrap().set(1);
        settle(&mut doc);
        assert!(inventory_text(&doc).contains("OBS Virtual Camera"));
        assert_eq!(layout(&doc), closed);
        harness.refresh.get().unwrap().set(2);
        settle(&mut doc);
        assert!(inventory_text(&doc).contains("OBS Virtual Camera"));
        assert_eq!(layout(&doc), closed);
    }
}

#[cfg(test)]
mod studio_selection_tests {
    use super::*;
    use blitz_dom::{Document, DocumentConfig};
    use blitz_traits::{
        events::{
            BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords,
            UiEvent,
        },
        shell::Viewport,
    };
    use dioxus_native::DioxusDocument;
    use std::{cell::Cell, rc::Rc};

    struct Harness {
        workspace: Workspace,
        project: Cell<Option<Signal<Workspace>>>,
    }

    fn app() -> Element {
        let harness = consume_context::<Rc<Harness>>();
        let project = use_signal(|| harness.workspace.clone());
        let selection = use_signal(|| None);
        crate::live_client::use_live_client(project);
        harness.project.set(Some(project));
        rsx! {
            style {{include_str!("dashboard.css")}}
            StudioSceneSelector {
                project, orientation:Orientation::Horizontal, selection, running:true, visible:false,
                program:harness.workspace.active(Orientation::Horizontal),
            }
        }
    }

    fn settle(doc: &mut DioxusDocument) {
        for _ in 0..30 {
            if !doc.poll(None) {
                doc.inner.borrow_mut().resolve(0.);
                return;
            }
        }
        panic!("Studio scene selector did not settle");
    }

    #[tokio::test]
    async fn studio_preview_click_leaves_the_workspace_program_unchanged() {
        let mut workspace = Workspace::default();
        let program = workspace.add_scene(Orientation::Horizontal);
        workspace.add_scene(Orientation::Horizontal);
        let harness = Rc::new(Harness {
            workspace,
            project: Cell::new(None),
        });
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
        settle(&mut doc);
        let point = {
            let inner = doc.inner.borrow();
            let id = inner
                .query_selector("button:nth-of-type(2)")
                .unwrap()
                .unwrap();
            inner.get_node(id).unwrap().absolute_position(4., 4.)
        };
        let pointer = |pressed| BlitzPointerEvent {
            id: BlitzPointerId::Mouse,
            is_primary: true,
            coords: PointerCoords {
                page_x: point.x,
                page_y: point.y,
                screen_x: point.x,
                screen_y: point.y,
                client_x: point.x,
                client_y: point.y,
            },
            button: MouseEventButton::Main,
            buttons: if pressed {
                MouseEventButtons::Primary
            } else {
                MouseEventButtons::None
            },
            mods: Modifiers::empty(),
            details: Default::default(),
            element: Default::default(),
            active_pointers: Default::default(),
        };
        doc.handle_ui_event(UiEvent::PointerMove(pointer(false)));
        doc.handle_ui_event(UiEvent::PointerDown(pointer(true)));
        doc.handle_ui_event(UiEvent::PointerUp(pointer(false)));
        settle(&mut doc);
        let inner = doc.inner.borrow();
        assert!(
            inner
                .query_selector("button:nth-of-type(2)[aria-pressed='true']")
                .unwrap()
                .is_some()
        );
        assert!(
            inner
                .query_selector(".studio-draft-preview")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            harness
                .project
                .get()
                .unwrap()
                .read()
                .active(Orientation::Horizontal),
            Some(program)
        );
    }
}

#[cfg(test)]
mod load_history_tests {
    use super::*;
    use acheflow_media_api::live::ServerLoadSample;

    #[test]
    fn blitz_paints_the_grid_and_metric_line() {
        use blitz_dom::DocumentConfig;
        use dioxus_native::DioxusDocument;

        fn chart() -> Element {
            rsx! {LoadPlot {path:"M0,40 L240,0",stroke:"#ffab70",label:"CPU usage"}}
        }
        let mut doc = DioxusDocument::new(VirtualDom::new(chart), DocumentConfig::default());
        doc.initial_build();
        let mut dom = doc.inner.borrow_mut();
        dom.resolve(0.);
        let id = dom.query_selector(".studio-load-plot").unwrap().unwrap();
        let tree = dom
            .get_node(id)
            .unwrap()
            .element_data()
            .unwrap()
            .svg_data()
            .unwrap();
        let bounds = tree.root().abs_stroke_bounding_box();
        assert!(
            bounds.top() < 1. && bounds.bottom() > 61.,
            "grid strokes at both ends of the chart"
        );
        assert!(
            bounds.left() < 1.,
            "the diagonal history stroke extends past the horizontal grid"
        );
    }

    #[test]
    fn gpu_history_keeps_sparse_samples_separate_per_device() {
        let device = |id: &str, utilization_percent| acheflow_media_api::live::GpuLoadSample {
            id: id.into(),
            utilization_percent: Some(utilization_percent),
            ..Default::default()
        };
        let sample = |time_ms, gpus| ServerLoadSample {
            time_ms,
            cpu_percent: 0.,
            ram_used_bytes: 1,
            ram_total_bytes: 2,
            busiest_gpu_percent: None,
            gpus,
        };
        let samples = [
            sample(1000, vec![device("first", 20.), device("second", 90.)]),
            sample(1250, vec![]),
            sample(2000, vec![device("first", 40.), device("second", 80.)]),
            sample(2250, vec![]),
        ];
        assert_eq!(
            gpu_load_path(&samples, "first", |gpu| gpu
                .utilization_percent
                .map(f64::from)),
            "M235.00,48.00 L239.00,36.00 "
        );
    }

    #[test]
    fn load_path_uses_elapsed_time_and_breaks_on_missing_measurements() {
        let sample = |time_ms, busiest_gpu_percent| ServerLoadSample {
            time_ms,
            cpu_percent: 0.,
            ram_used_bytes: 1,
            ram_total_bytes: 2,
            busiest_gpu_percent,
            gpus: vec![],
        };
        let samples = [
            sample(0, Some(100.)),
            sample(63_000, Some(50.)),
            sample(63_500, Some(75.)),
            sample(64_000, None),
            sample(65_000, Some(100.)),
        ];
        assert_eq!(
            load_path(&samples, |sample| sample.busiest_gpu_percent.map(f64::from)),
            "M232.00,30.00 L234.00,15.00 M240.00,0.00 "
        );
    }
}

#[cfg(test)]
#[path = "scene_history_tests.rs"]
mod scene_history_tests;

#[cfg(test)]
#[path = "draft_capture_tests.rs"]
mod draft_capture_tests;
