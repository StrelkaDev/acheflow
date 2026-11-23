use crate::{
    live::CommittedName, live_client::LiveClient, project::Workspace, workspace_sync::WorkspaceSync,
};
use acheflow_media_api::{
    access::Permission,
    live::{MeterTarget, SourceCapture, SourceEdit, SourceStatus},
    visual::SourceKind,
    workspace::Source,
};
use dioxus_native::prelude::*;

pub(crate) fn edit_source(mut project: Signal<Workspace>, id: u32, edit: SourceEdit) {
    if let Some(client) = try_consume_context::<LiveClient>() {
        client.source_edit(id, edit);
    } else {
        let _ = edit.apply(&mut project.write(), id);
    }
}

#[component]
pub(crate) fn SourceName(project: Signal<Workspace>, source_id: u32, value: String) -> Element {
    rsx! {CommittedName {label:"Name",value,change:move |value|edit_source(project,source_id,SourceEdit::Name(value))}}
}

#[component]
pub(crate) fn RemoveSource(project: Signal<Workspace>, source_id: u32) -> Element {
    let busy = try_consume_context::<LiveClient>()
        .is_some_and(|client| (client.busy)() || (client.scoped_pending)());
    rsx! {button {disabled:busy,onclick:move |_|edit_source(project,source_id,SourceEdit::Remove),"Remove"}}
}

#[component]
pub(crate) fn SourceCaptureActions(source: Source) -> Element {
    let Some(client) = try_consume_context::<LiveClient>() else {
        return rsx! {};
    };
    let Some(sync) = try_consume_context::<WorkspaceSync>() else {
        return rsx! {};
    };
    if !client.owns() || !sync.can_edit_workspace() {
        return rsx! {};
    }
    let id = source.id;
    let busy = (client.busy)() || (client.scoped_pending)();
    let Some(saved) = sync.saved_source(id) else {
        return rsx! {
            div {class:"source-footer",
                button {disabled:busy,onclick:move |_|client.source_edit(id,SourceEdit::Register(source.clone())),"Add to server"}
            }
        };
    };
    if !source.kind.is_media_input() {
        return if source.kind == saved.kind {
            rsx! {}
        } else {
            rsx! {div {class:"source-footer",
                button {disabled:busy,onclick:move |_|client.source_edit(id,SourceEdit::VisualSettings(source.kind.clone())),"Save overlay settings"}
            }}
        };
    }
    let capture = SourceCapture::of(&source);
    if capture.matches(&saved) {
        return rsx! {};
    }
    let active = client
        .status
        .read()
        .as_ref()
        .is_some_and(|status| status.activated);
    let label = if !active {
        "Save capture settings"
    } else {
        match source.kind {
            SourceKind::Camera { .. } => "Reconnect camera",
            SourceKind::Microphone { .. } => "Reconnect microphone",
            _ => "Reconnect input",
        }
    };
    rsx! {
        div {class:"source-footer",
            button {disabled:busy,onclick:move |_|client.source_edit(id,SourceEdit::Capture(capture.clone())),"{label}"}
        }
    }
}

pub(crate) fn can_activate_preview(video: bool) -> bool {
    crate::server_access::allows(Permission::MediaManage)
        && (!video || crate::server_access::allows(Permission::StudioView))
}

pub(crate) fn preview_source_registered(source_id: u32) -> bool {
    try_consume_context::<WorkspaceSync>()
        .is_none_or(|sync| !sync.can_edit_workspace() || sync.saved_source(source_id).is_some())
}

#[derive(Clone, Copy)]
pub(crate) struct SeparateSourcePreview(pub Signal<Option<u32>>);

#[component]
pub(crate) fn OpenSourcePreview(source_id: u32) -> Element {
    let Some(SeparateSourcePreview(mut view)) = try_consume_context::<SeparateSourcePreview>()
    else {
        return rsx! {};
    };
    rsx! {crate::server_settings::ActionButton {onclick:move |_|view.set(Some(source_id)), "Open preview"}}
}

#[component]
pub(crate) fn SeparateSourceView(project: Signal<Workspace>, source_id: u32) -> Element {
    let SeparateSourcePreview(mut view) = use_context::<SeparateSourcePreview>();
    let client = use_context::<LiveClient>();
    let workspace = project.read();
    let source = workspace
        .sources
        .iter()
        .find(|source| source.id == source_id && source.kind.is_media_input());
    let status = client.status.read();
    let status = status
        .as_ref()
        .filter(|status| status.workspace_id.as_deref() == Some(workspace.id.as_str()));
    rsx! {
        div {class:"live-content separate-source-view",
            div {class:"page-heading",
                div {h1 {if let Some(source)=source {"{source.name}"} else {"Source preview"}}}
                crate::server_settings::ActionButton {onclick:move |_|view.set(None),"Close preview"}
            }
            if let Some(source)=source {
                section {class:"panel",
                    SourcePreview {source_id,video:source.kind.is_video_input(),
                        activated:status.is_some_and(|status|status.activated),
                        input:status.and_then(|status|status.sources.iter().find(|input|input.id==source_id)).cloned(),
                        inspector:true,separate:true}
                }
            } else {
                p {class:"form-help","This source is no longer available."}
            }
        }
    }
}

#[component]
pub(crate) fn StudioSourcePreview(project: Signal<Workspace>) -> Element {
    let client = use_context::<LiveClient>();
    let mut open = use_signal(|| false);
    let mut selected = use_signal(|| None::<u32>);
    use_effect(move || {
        if let Some(id) = selected()
            && !project
                .read()
                .sources
                .iter()
                .any(|source| source.id == id && source.kind.is_media_input())
        {
            selected.set(None);
        }
    });
    let workspace = project.read();
    let sources = workspace
        .sources
        .iter()
        .filter(|source| source.kind.is_media_input())
        .collect::<Vec<_>>();
    let source = sources
        .iter()
        .copied()
        .find(|source| Some(source.id) == selected());
    let status = client.status.read();
    let status = status
        .as_ref()
        .filter(|status| status.workspace_id.as_deref() == Some(workspace.id.as_str()));

    if sources.is_empty() {
        return rsx! {};
    }
    rsx! {
        section {class:"panel studio-source-preview",
            div {class:"panel-heading", h2 {"Source preview"}
                crate::server_settings::ActionButton {expanded:Some(open()), onclick:move |_| {
                    let next = !open();
                    open.set(next);
                    if !next { selected.set(None); }
                }, if open() {"Close source preview"} else {"Choose source"}}
            }
            if open() {
                div {class:"row padded studio-source-options",role:"group","aria-label":"Sources to preview",
                    for source in sources {
                        {
                            let id = source.id;
                            let chosen = selected() == Some(id);
                            rsx! {
                                crate::server_settings::ActionButton {key:"{id}",class:if chosen {"selected"} else {""},pressed:Some(chosen),
                                    onclick:move |_|selected.set((!chosen).then_some(id)),
                                    "{source.name}"
                                }
                            }
                        }
                    }
                }
                if let Some(source) = source {
                    div {key:"{source.id}",
                        div {class:"row padded", OpenSourcePreview {source_id:source.id}}
                        SourcePreview {source_id:source.id,video:source.kind.is_video_input(),
                            activated:status.is_some_and(|status|status.activated),
                            input:status.and_then(|status|status.sources.iter().find(|input|input.id==source.id)).cloned(),
                            inspector:true
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn SourcePreview(
    source_id: u32,
    video: bool,
    activated: bool,
    input: Option<SourceStatus>,
    #[props(default)] inspector: bool,
    #[props(default)] separate: bool,
) -> Element {
    let Some(client) = try_consume_context::<LiveClient>() else {
        return rsx! {};
    };
    let allowed = can_activate_preview(video);
    let mut open = use_signal(|| false);
    let can_request = !activated && allowed && client.owns();
    use_effect(use_reactive((&can_request,), move |(can_request,)| {
        if !can_request && open() {
            open.set(false);
        }
    }));
    let registered = preview_source_registered(source_id);
    let previewing = can_request && registered && (inspector || open());
    let workspace = client.project.read();
    if inspector {
        let unavailable = if !client.owns() {
            Some("Save or follow this workspace on the server to preview its sources.")
        } else if video && !crate::server_access::allows(Permission::StudioView) {
            Some("This device needs Studio viewing access to see video.")
        } else if !registered {
            Some("Add this source to the server before previewing it.")
        } else if !activated && !allowed {
            Some("This device needs media management access to start an inactive source.")
        } else {
            None
        };
        if let Some(reason) = unavailable {
            return rsx! {p {class:"padded form-help",role:"status","{reason}"}};
        }
    }
    let showing = activated || previewing;
    let connected = input.as_ref().is_some_and(|status| status.connected);
    let image = showing
        && video
        && connected
        && (!inspector || input.as_ref().is_some_and(|status| status.frames > 0));
    let error = input.as_ref().and_then(|status| status.error.clone());
    let image_class = if separate {
        "studio-source-image separate-source-image"
    } else {
        "studio-source-image"
    };
    rsx! {
        if can_request && !inspector {
            div {class:"source-footer",
                button {disabled:!registered,title:if registered {""} else {"Add this source to the server first."},"aria-pressed":previewing.to_string(),onclick:move |_|open.set(!open()),
                    if previewing {"Close preview"} else if video {"Preview source"} else {"Check microphone level"}
                }
            }
        }
        if previewing {
            KeepSourcePreview {key:"{workspace.id}-{workspace.server_url}-{source_id}",source_id,
                waiting:!inspector && !connected && error.is_none(),
                capture_error:if inspector {None} else {error.clone()}}
        }
        if inspector {
            div {class:"padded studio-source-readout",
                div {class:"row",
                    span {class:"badge",role:"status",if error.is_some() {"Source error"} else if connected {"Connected"} else {"Waiting for source"}}
                    if let Some(status) = input.as_ref() {
                        if let (Some(width), Some(height)) = (status.width, status.height) {
                            span {class:"form-help","{width} × {height}"}
                        }
                        if let Some(fps) = status.fps {
                            span {class:"form-help","{fps:.1} FPS"}
                        }
                    }
                }
                if let Some(error) = error.as_ref() {
                    p {class:"field-error",role:"alert","{error}"}
                }
                if video && !image {
                    div {class:format!("{image_class} source-preview-waiting"),"Waiting for video"}
                }
                if image {
                    crate::video_preview::VideoPreview {url:client.preview(&format!("/api/live/source/{source_id}")),class:image_class,fit:true}
                }
                if !video && input.as_ref().is_none_or(|status| status.audio_tracks.is_empty()) {
                    p {class:"form-help",role:"status","Waiting for microphone samples"}
                }
                if let Some(status) = input.as_ref() {
                    for track in &status.audio_tracks {
                        PreviewAudioTrack {key:"{track.track_id}",source_id,track:track.clone()}
                    }
                }
            }
        } else if image {
            crate::video_preview::VideoPreview {url:client.preview(&format!("/api/live/source/{source_id}")),class:"source-card-preview",fit:true}
        } else if showing && !video && let Some(status)=input.as_ref() {
            div {class:"padded",
                for track in &status.audio_tracks {
                    PreviewAudioTrack {key:"{track.track_id}",source_id,track:track.clone()}
                }
            }
        }
    }
}

#[component]
fn PreviewAudioTrack(
    source_id: u32,
    track: acheflow_media_api::audio::AudioTrackStatus,
) -> Element {
    rsx! {div {class:"studio-source-track",
        div {class:"source-track-heading",
            span {class:"form-help","Track {track.track_id} · {track.sample_rate} Hz"}
            crate::audio_monitor::MonitorButton {
                target:crate::audio_monitor::Target::Source {source:source_id,track:track.track_id},
                disabled:!track.active,
            }
        }
        crate::audio_mixer::LiveMeters {target:MeterTarget::Source {source_id,track_id:track.track_id},fallback:track.meter}
    }}
}

#[component]
pub(crate) fn KeepSourcePreview(
    source_id: u32,
    waiting: bool,
    #[props(default)] name: Option<String>,
    #[props(default)] capture_error: Option<String>,
) -> Element {
    let client = use_context::<LiveClient>();
    let mut error = use_signal(|| None::<String>);
    use_future(move || {
        let workspace = client.project.peek().clone();
        async move {
            loop {
                let result = crate::live_client::renew_source_preview(&workspace, source_id)
                    .await
                    .err();
                if *error.peek() != result {
                    error.set(result);
                }
                crate::scene_preview::pause(2_000).await;
            }
        }
    });
    let prefix = name.map_or_else(String::new, |name| format!("{name} · "));
    rsx! {
        if let Some(error)=error.read().as_ref().or(capture_error.as_ref()){
            p {class:"field-error padded",role:"alert","{prefix}{error}"}
        } else if waiting {
            p {class:"form-help padded",role:"status","{prefix}Waiting for preview input…"}
        }
    }
}
