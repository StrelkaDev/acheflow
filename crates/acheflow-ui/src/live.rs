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
    let client = use_context::<LiveClient>();
    let mut tab = use_signal(|| "Studio");
    let preview = use_signal(std::collections::HashSet::<u32>::new);
    use_context_provider(|| crate::visual_sources::ChatPreview(preview));
    rsx! {div {class:"live-root",
        nav {class:"subnav",for name in ["Analytics","Sources","Scenes","Mixer","Studio","Chat","Interactive","Settings"] {
            button {class:if tab()==name {"selected"}else{""},onclick:move |_|tab.set(name),"{name}"}
        }}
        LiveNotice {}
        div {class:"live-pages",for name in ["Analytics","Sources","Scenes","Mixer","Studio","Chat","Interactive","Settings"] {
            div {key:"{name}",class:if visible && tab()==name {"tab-scroll active"}else{"tab-scroll"},"aria-hidden":if visible && tab()==name {"false"}else{"true"},
                div {class:"live-content",
                    if name=="Analytics" {Analytics {}}
                    if name=="Sources" {Sources {project}}
                    if name=="Scenes" {Scenes {project,visible:visible && tab()==name}}
                    if name=="Mixer" {crate::audio_mixer::AudioMixer {project}}
                    if name=="Studio" {Studio {project,visible:visible && tab()==name}}
                    if name=="Chat" {
                        StandaloneChat {running:client.running()}
                    }
                    if name=="Interactive" {crate::interaction::InteractivePage {}}
                    if name=="Settings" {Settings {project}}
                }
            }
        }}
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
pub fn Sources(mut project: Signal<Workspace>) -> Element {
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
                        on_added:move |_|picking.set(false),
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
            button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply sources / listen"}
            button {disabled:(client.busy)() || !client.owns(),onclick:move |_|client.command(Action::Deactivate),"Deactivate sources"}
            span {class:"form-help",if let Some(state)=client.status.read().as_ref(){"RTMP listener: {state.rtmp_address}"}}
        }
        if project.read().sources.is_empty(){div {class:"panel empty-state",h3 {"No sources yet"}p {"Add a source to make it available in scenes."}}}
        for source in project.read().sources.iter().filter(|s|s.kind.is_video_input()).cloned().collect::<Vec<_>>() {
            {
            let srt = matches!(source.kind, acheflow_media_api::visual::SourceKind::Srt { .. });
            let camera = matches!(source.kind, acheflow_media_api::visual::SourceKind::Camera { .. });
            let input = state.as_ref().and_then(|s|s.sources.iter().find(|i|i.id==source.id));
            let preview_class = "source-card-preview";
            let key_label = if srt {"SRT passphrase"}else{"RTMP stream key"};
            rsx!{section {key:"{source.id}",class:"panel",
                div {class:"panel-heading",button{class:"destination-toggle",onkeydown:move |event|{if !event.is_auto_repeating() && (event.key()==Key::Named(NamedKey::Enter) || event.key()==Key::Character(" ".into())){event.prevent_default();let mut open=expanded.write();if !open.remove(&source.id){open.insert(source.id);}}},"aria-expanded":expanded.read().contains(&source.id).to_string(),onclick:move |_|{let mut open=expanded.write();if !open.remove(&source.id){open.insert(source.id);}},span{if expanded.read().contains(&source.id){"▾"}else{"▸"}}h2 {"{source.name}"}}
                    span {class:"badge",if camera {"Camera"} else if srt {"SRT"} else {"RTMP"}}
                    span {class:"badge",if srt && !srt_available {"Unavailable"} else if input.is_some_and(|s|s.connected){"Connected"}else if input.is_some_and(|s|s.error.is_some()){"Source error"}else if input.is_some(){"Waiting for input"}else{"Apply to activate"}}
                    button {onclick:move |_|project.write().remove_source(source.id),"Remove"}
                }
                div {style:if expanded.read().contains(&source.id){"display:block"}else{"display:none"},
                if expanded.read().contains(&source.id) && input.is_some_and(|status|status.connected) {
                    VideoPreview {url:client.preview(&format!("/api/live/source/{}",source.id)),class:preview_class}
                }
                div {class:"form-grid",
                    Field {label:"Name",value:source.name,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.name=v;}}}
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
                        Field {label:"RTMP ingest URL",value:source.url,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.url=v;}}}
                    }
                    if !camera {
                    Field {label:key_label,secret:true,value:source.key,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.key=v;}}}
                    div {class:"field",button {onclick:move |_|{let key=project.peek().sources.iter().find(|s|s.id==source.id).map(|s|s.key.clone());if let Some(key)=key {spawn(async move {if client.copy_text(key).await {copied.set(Some(source.id));}});}},if copied()==Some(source.id){"Copied"}else if srt {"Copy passphrase"}else{"Copy stream key"}}button {onclick:move |_|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.key=crate::project::new_token();copied.set(None);}},"Generate new key"}}
                    }
                    if camera && expanded.read().contains(&source.id) {
                        CameraSettings{project,source_id:source.id,inventory:camera_inventory,refresh:refresh_cameras}
                    }
                    NumberField {label:"Offline preview width",value:source.resolution.width,min:16,max:((1920*1080/source.resolution.height.max(1)).min(3840)/2)*2,step:2,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.resolution.width=v;}}}
                    NumberField {label:"Offline preview height",value:source.resolution.height,min:16,max:((1920*1080/source.resolution.width.max(1)).min(3840)/2)*2,step:2,change:move |v|{if let Some(s)=project.write().sources.iter_mut().find(|s|s.id==source.id){s.resolution.height=v;}}}
                }
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
                div {class:"panel-note","Incoming video keeps its original resolution. Resize and fit it in the scene layer. Offline preview dimensions only size the test pattern while no video is available."}
                }
            }}
            }
        }
        crate::microphone_sources::MicrophoneSources{
            project,inventory:microphone_inventory,refresh:refresh_microphones,status:state,
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
                if let Some(source)=project.write().sources.iter_mut().find(|source|source.id==source_id)
                    && let SourceKind::Camera{device_id,mode,..}=&mut source.kind
                    && *device_id==mode_device_id {*mode = *next;}
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

#[component]
fn Settings(mut project: Signal<Workspace>) -> Element {
    let (capabilities, refresh) = use_capabilities(project);
    let snapshot = capabilities.read().clone();
    let available = snapshot.as_ref().and_then(|r| r.as_ref().ok()).cloned();
    rsx! {
        div {class:"page-heading",div {h1 {"Stream settings"}p {"Separate canvases and RTMP outputs for each format."}}}
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
        div {class:"panel-heading",h2 {"Restream outputs"}button {class:"primary",onclick:move |_|{let size=project.read().stream(orientation).base.clone();project.write().stream_mut(orientation).outputs.push(Output{name:"New destination".into(),url:String::new(),key:String::new(),resolution:size,quality:"Standard".into(),codec:"h264".into(),encoder:None,bitrate_kbps:4500, bitrate_mode: Default::default(),keyframe_interval_secs:2,enabled:true,audio:Default::default()});expanded.set(Some(project.peek().stream(orientation).outputs.len()-1));},"+ Output"}}
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
                    NumberField {label:"Keyframe interval (seconds)",value:out.keyframe_interval_secs,min:1,max:20,change:move |v|project.write().stream_mut(orientation).outputs[index].keyframe_interval_secs=v}
                    EncoderPicker {project,orientation,index,capabilities:capabilities.clone()}
                    Choice {label:"Output",value:if out.enabled {"Enabled"}else{"Disabled"},choices:vec!["Enabled".into(),"Disabled".into()],change:move |v:String|project.write().stream_mut(orientation).outputs[index].enabled=v=="Enabled"}
                }
                p {class:"form-help padded","CBR keeps a fixed video rate. VBR varies with image complexity up to the configured limit. The bitrate setting applies to video; traffic includes audio and FLV overhead. Audio uses the scene mixer. Keyframe interval changes require Stop/Start."}
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
                div {class:"row padded",button {disabled:(client.busy)(),onclick:move |_|client.command(Action::Apply),"Apply scene & sources"}span {class:"form-help","Drag a corner to resize. Hold Shift for the layer ratio or Shift + Option/Alt for the source ratio. The 2D canvas snaps to its edges. Releasing applies live changes."}}
                div {class:"panel-heading",h3 {"Add a source layer"}}
                div {class:"row padded",for source in project.read().sources.iter().filter(|s|!s.kind.is_audio_only()).cloned().collect::<Vec<_>>(){button {onclick:move |_|project.write().add_layer(scene.id,source.id),"+ {source.name}"}}if project.read().sources.is_empty(){p {"Add a video source or overlay in Live / Sources."}}}
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
        if let Some(status)=status.as_ref() && !status.server_load.is_empty() {
            ServerLoad {samples:status.server_load.clone()}
        }
        p {class:"studio-help form-help","Start runs the selected scenes and all enabled outputs. With no outputs it runs preview only. Adjust Preview FPS in Live / Settings."}
        div {class:"studio-grid",for o in Orientation::ALL {
            section {class:"panel",div {class:"panel-heading",h2 {"{o.label()}"}
                span {class:"badge",if let Some(format)=status.as_ref().and_then(|s|s.formats.iter().find(|f|f.orientation==o)) {
                    if format.outputs.is_empty(){"Preview only"}else if format.outputs.iter().any(|out|out.state==OutputState::Streaming){"Broadcasting"}else if format.outputs.iter().any(|out|out.reconnect_attempt.is_some()){"Reconnecting"}else if format.outputs.iter().any(|out|out.state==OutputState::Starting){"Starting"}else{"Output failed"}
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
                        if format.render_load.target_fps > 0 {
                            div {class:"studio-format-load",
                                span {"{format.render_load.observed_fps:.1} / {format.render_load.target_fps} FPS"}
                                span {"Render p95 {format.render_load.p95_ms:.1} ms"}
                                span {"{format.render_load.deadline_miss_percent:.1}% over frame budget"}
                            }
                        }
                    }

                    for out in &format.outputs {
                        div {class:"output-status",strong {"{out.name}"}span {class:"badge",role:"status",if let Some(attempt)=out.reconnect_attempt {"Reconnecting · attempt {attempt}"}else{"{out.state:?}"}}span {"{out.frames} frames · {out.bitrate_kbps:.0} kbps"}
                            button {disabled:(client.busy)(),onclick:{let index=out.index;move |_|client.command(Action::RestartOutput(o,index))},"Restart output"}
                            if let Some(encoder)=out.encoder.as_ref(){span {"{encoder} · ",if out.hardware {"Hardware"}else{"Software"}}}
                            if let Some(bus)=out.audio_bus {span {"Audio: scene {out.audio_scene.unwrap_or(0)} → bus {bus}"}crate::audio_mixer::Meters {meter:out.audio_meter.clone()}}
                            if out.repeated_video_frames > 0 || out.inserted_audio_blocks > 0 || out.reconnects > 0 {
                                span {"{out.repeated_video_frames} repeated frames · {out.inserted_audio_blocks} silence blocks · {out.reconnects} reconnects"}
                            }
                            if let Some(error)=out.error.as_ref(){p {class:"field-error","{error}"}}
                        }
                    }
                }else {Canvas {project,scene_id:project.read().active(o),orientation:o,live_images:visible}div {class:"panel-note","Scene layout · start live to render the composition"}}
                crate::studio_mixer::StudioMixer {project,orientation:o}
            }
        }}
    }
}

#[component]
fn ServerLoad(samples: Vec<acheflow_media_api::live::ServerLoadSample>) -> Element {
    let Some(latest) = samples.last() else {
        return rsx! {};
    };
    let ram_gib = latest.ram_used_bytes as f64 / (1u64 << 30) as f64;
    let total_gib = latest.ram_total_bytes as f64 / (1u64 << 30) as f64;
    rsx! {
        section {class:"panel studio-server-load",
            div {class:"panel-heading",h2 {"Server load"}span {class:"form-help","Last 60 seconds"}}
            div {class:"studio-load-history",
                div {
                    span {"CPU {latest.cpu_percent:.0}%"}
                    div {class:"studio-load-bars",role:"img","aria-label":"Server CPU usage history, up to 60 seconds",
                        for sample in &samples {
                            span {key:"{sample.time_ms}",style:format!("height:{:.1}%",sample.cpu_percent.clamp(0.,100.))}
                        }
                    }
                }
                div {
                    span {"RAM {ram_gib:.1} / {total_gib:.1} GiB"}
                    div {class:"studio-load-bars",role:"img","aria-label":"Server memory usage history, up to 60 seconds",
                        for sample in &samples {
                            span {key:"{sample.time_ms}",style:format!("height:{:.1}%",(sample.ram_used_bytes as f64 * 100. / sample.ram_total_bytes.max(1) as f64).clamp(0.,100.))}
                        }
                    }
                }
                if let Some(gpu)=latest.busiest_gpu_percent {
                    div {
                        span {"Busiest GPU {gpu:.0}%"}
                        div {class:"studio-load-bars",role:"img","aria-label":"Busiest reported server GPU usage history, up to 60 seconds",
                            for sample in &samples {
                                span {key:"{sample.time_ms}",style:format!("height:{:.1}%",sample.busiest_gpu_percent.unwrap_or(0.).clamp(0.,100.))}
                            }
                        }
                    }
                }
            }
        }
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
            button {id:"apply-sources","Apply sources"}
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
