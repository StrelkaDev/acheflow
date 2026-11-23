use crate::{
    live::CommittedName, live_client::LiveClient, project::Workspace, workspace_sync::WorkspaceSync,
};
use acheflow_media_api::{
    access::Permission,
    live::{SourceCapture, SourceEdit, SourceStatus},
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

#[component]
pub(crate) fn SourcePreview(
    source_id: u32,
    video: bool,
    activated: bool,
    input: Option<SourceStatus>,
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
    let previewing = can_request && registered && open();
    let workspace = client.project.read();
    rsx! {
        if can_request {
            div {class:"source-footer",
                button {disabled:!registered,title:if registered {""} else {"Add this source to the server first."},"aria-pressed":previewing.to_string(),onclick:move |_|open.set(!open()),
                    if previewing {"Close preview"} else if video {"Preview source"} else {"Check microphone level"}
                }
            }
        }
        if previewing {
            KeepSourcePreview {key:"{workspace.id}-{workspace.server_url}-{source_id}",source_id,
                waiting:input.as_ref().is_none_or(|status| !status.connected && status.error.is_none())}
        }
        if (activated || previewing) && let Some(status)=input.as_ref() {
            if video && status.connected {
                crate::video_preview::VideoPreview {url:client.preview(&format!("/api/live/source/{source_id}")),class:"source-card-preview"}
            } else if !video && let Some(track)=status.audio_tracks.iter().find(|track|track.track_id==0) {
                div {class:"padded",crate::audio_mixer::Meters {meter:track.meter.clone()}}
            }
        }
    }
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
